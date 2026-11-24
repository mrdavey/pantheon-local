//! Forward real browser requests and inject one response or concurrent update.
use anyhow::{Context, Result};
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use std::{
    net::TcpListener,
    sync::{Arc, Mutex},
};
use tokio::sync::oneshot;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Once {
    InitialSelection,
    MoreInitialFiles,
    FolderSelected,
    FolderCancelled,
    SnapshotFailure,
    MissingGit,
    DestinationsFailure,
    DestinationsSuccess,
    EvidenceFailure,
    SnapshotRace,
    CombinationRace,
}
#[derive(Clone)]
struct Forward {
    origin: String,
    upstream: String,
    token: String,
    http: reqwest::Client,
    control: Arc<Mutex<Control>>,
}
#[derive(Default)]
struct Control {
    pending: Vec<Once>,
    cursor: Option<i64>,
    first_poll: Option<i64>,
}
pub struct Proxy {
    pub url: String,
    control: Arc<Mutex<Control>>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Proxy {
    pub fn new(info: &Value) -> Result<Self> {
        let socket = TcpListener::bind("127.0.0.1:0")?;
        socket.set_nonblocking(true)?;
        let url = format!("http://{}", socket.local_addr()?);
        let control = Arc::new(Mutex::new(Control::default()));
        let state = Forward {
            origin: url.clone(),
            upstream: info["url"].as_str().context("service URL")?.into(),
            token: info["token"].as_str().context("service token")?.into(),
            control: control.clone(),
            http: reqwest::Client::builder().no_proxy().build()?,
        };
        let (stop, stopped) = oneshot::channel();
        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move{
                let listener=tokio::net::TcpListener::from_std(socket).unwrap();let router=Router::new().fallback(forward).with_state(state);
                tokio::select!{_ = axum::serve(listener,router).into_future()=>{}, _ = stopped => {}}
            });
        });
        Ok(Self {
            url,
            control,
            stop: Some(stop),
            thread: Some(thread),
        })
    }
    pub fn inject(&self, once: Once) {
        self.control.lock().unwrap().pending.push(once);
    }
    pub fn cursor_checked(&self) -> Option<bool> {
        let c = self.control.lock().unwrap();
        Some(c.cursor? > 0 && c.first_poll? == c.cursor?)
    }
}
impl Drop for Proxy {
    fn drop(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
async fn forward(State(state): State<Forward>, request: Request) -> Response {
    match handle(state, request).await {
        Ok(res) => res,
        Err(e) => (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    }
}
async fn handle(st: Forward, request: Request) -> Result<Response> {
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, 4 << 20).await?;
    let cmd: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let path = parts.uri.path();
    let injection = {
        let mut control = st.control.lock().unwrap();
        if path == "/api/events" && control.cursor.is_some() && control.first_poll.is_none() {
            control.first_poll = parts
                .uri
                .query()
                .and_then(|q| q.strip_prefix("after="))
                .and_then(|n| n.parse().ok());
        }
        control
            .pending
            .iter()
            .position(|p| match p {
                Once::SnapshotFailure | Once::SnapshotRace => path == "/api/snapshot",
                Once::InitialSelection | Once::MoreInitialFiles => {
                    path == "/api/pick-initial-files"
                }
                Once::FolderSelected | Once::FolderCancelled => {
                    path == "/api/project-folder" && parts.method == "POST"
                }
                Once::MissingGit => path == "/api/setup/status",
                Once::DestinationsFailure | Once::DestinationsSuccess => {
                    path == "/api/destinations"
                }
                Once::EvidenceFailure => path.contains("/evidence/"),
                Once::CombinationRace => parts.method == "POST" && cmd["kind"] == "combine",
            })
            .map(|i| control.pending.remove(i))
    };
    let reply = |code: StatusCode, value: Value| (code, axum::Json(value)).into_response();
    match injection {
        Some(Once::InitialSelection) => return Ok(reply(StatusCode::OK, json!(["example.txt"]))),
        Some(Once::MoreInitialFiles) => {
            return Ok(reply(
                StatusCode::OK,
                json!(["example.txt", "nested/notes.md"]),
            ));
        }
        Some(Once::FolderSelected) => {
            return Ok(reply(StatusCode::OK, json!("/tmp/Project With Spaces")));
        }
        Some(Once::FolderCancelled) => return Ok(reply(StatusCode::OK, Value::Null)),
        Some(Once::SnapshotFailure | Once::EvidenceFailure | Once::DestinationsFailure) => {
            return Ok(reply(
                StatusCode::SERVICE_UNAVAILABLE,
                json!({"error":"Temporarily unavailable"}),
            ));
        }
        Some(Once::DestinationsSuccess) => {
            return Ok(reply(StatusCode::OK, json!(["test-staging"])));
        }
        _ => (),
    }
    let inject_chat = |path: String, text: &'static str| {
        let st = st.clone();
        async move {
            st.http
                .post(st.upstream + &path)
                .bearer_auth(st.token)
                .json(&json!({"kind":"chat","text":text}))
                .send()
                .await?
                .error_for_status()?;
            Ok::<(), reqwest::Error>(())
        }
    };
    if injection == Some(Once::CombinationRace) {
        inject_chat(path.into(), "Summarize the exporter milestone.").await?;
    }
    let mut req = st
        .http
        .request(parts.method, format!("{}{}", st.upstream, parts.uri))
        .body(bytes);
    for name in [header::AUTHORIZATION, header::CONTENT_TYPE] {
        if let Some(value) = parts.headers.get(&name) {
            req = req.header(name, value);
        }
    }
    if let Some(origin) = parts.headers.get(header::ORIGIN) {
        req = if origin.as_bytes() == st.origin.as_bytes() {
            req.header(header::ORIGIN, &st.upstream)
        } else {
            req.header(header::ORIGIN, origin)
        };
    }
    let res = req.send().await?;
    let code = res.status();
    let headers = res.headers().clone();
    let body = res.bytes().await?;
    if matches!(injection, Some(Once::SnapshotRace | Once::MissingGit)) {
        let mut data: Value = serde_json::from_slice(&body)?;
        if injection == Some(Once::MissingGit) {
            data["git"] = json!(false);
            data["profiles"]["codex"]["state"] = json!("sign-in-unknown");
        } else {
            st.control.lock().unwrap().cursor = data["cursor"].as_i64();
            let id = data["runs"][0]["id"]
                .as_str()
                .context("missing snapshot run")?;
            inject_chat(
                format!("/api/runs/{id}"),
                "Explain the result after reopening.",
            )
            .await?;
        }
        return Ok(reply(code, data));
    }
    let mut reply = Response::builder().status(code);
    for name in [
        header::CONTENT_TYPE,
        header::REFERRER_POLICY,
        header::X_CONTENT_TYPE_OPTIONS,
        header::CONTENT_SECURITY_POLICY,
        header::CACHE_CONTROL,
    ] {
        if let Some(v) = headers.get(&name) {
            reply = reply.header(name, v);
        }
    }
    Ok(reply.body(Body::from(body))?)
}
