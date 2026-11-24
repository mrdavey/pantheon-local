use crate::{
    adapter, assets, eval, project,
    run::{Engine, comparison, experiment, release},
    store::Store,
};
use anyhow::{Context, Result, bail, ensure};
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::Child,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;

#[derive(Debug)]
pub struct AlreadyRunning;
impl fmt::Display for AlreadyRunning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Pantheon Local is already running for this state directory")
    }
}
impl std::error::Error for AlreadyRunning {}

pub fn state_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("SWARM_STATE_DIR").filter(|s| !s.is_empty()) {
        return Ok(std::path::absolute(path)?);
    }
    let home = || {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("HOME is unset")
    };
    let config = if cfg!(target_os = "macos") {
        home()?.join("Library/Application Support")
    } else if let Some(path) = std::env::var_os("XDG_CONFIG_HOME").filter(|s| !s.is_empty()) {
        path.into()
    } else {
        home()?.join(".config")
    };
    Ok(config.join("local-swarm"))
}
pub fn setup(dir: &Path) -> Result<Value> {
    let mut cfg: Value = match fs::read(dir.join("profiles.json")) {
        Ok(raw) => serde_json::from_slice(&raw)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            json!({"profiles":{"codex":{"kind":"codex"},"claude":{"kind":"claude"},"hermes":{"kind":"hermes"},"antigravity":{"kind":"antigravity"}}})
        }
        Err(e) => return Err(e.into()),
    };
    let profiles = cfg["profiles"]
        .as_object()
        .context("profiles must be an object")?;
    if let Some(roles) = fs::read(dir.join("roles.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
    {
        let coordinator = roles["coordinator"]
            .as_str()
            .filter(|s| profiles.contains_key(*s));
        let workers = roles["workers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|s| profiles.contains_key(*s))
            .collect::<Vec<_>>();
        let workers = json!(workers);
        if let Some(c) = coordinator {
            cfg["coordinator"] = c.into()
        }
        cfg["workers"] = workers;
    }
    Ok(cfg)
}
fn write_local(path: &Path, value: &Value) -> Result<()> {
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().context("missing state directory")?)?;
    serde_json::to_writer(&mut file, value)?;
    file.flush()?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}
struct Local {
    dir: PathBuf,
    host: String,
    origin: String,
    auth: String,
    engine: Engine,
    writes: Mutex<()>,
    profiles: Mutex<()>,
    previews: Mutex<Vec<Child>>,
    shutdown: AtomicBool,
}
impl Local {
    /// Serialize run mutations across HTTP and the scheduler. Read-only requests
    /// take SQLite snapshots and stay responsive while Git or an agent starts.
    fn mutate<T>(&self, action: impl FnOnce(&Engine) -> Result<T>) -> Result<T> {
        let _guard = self
            .writes
            .lock()
            .map_err(|_| anyhow::anyhow!("engine lock poisoned"))?;
        action(&self.engine)
    }
}
struct Metadata(PathBuf);
impl Drop for Metadata {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
pub fn serve(dir: &Path, listen: &str, browser: bool) -> Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(dir.join("daemon.lock"))?;
    if let Err(err) = fs2::FileExt::try_lock_exclusive(&lock) {
        if err.kind() == std::io::ErrorKind::WouldBlock {
            return Err(AlreadyRunning.into());
        }
        return Err(err.into());
    }
    let addr = listen
        .to_socket_addrs()?
        .find_map(|a| match a {
            SocketAddr::V4(v4) => Some(v4),
            _ => None,
        })
        .context("use an IPv4 loopback listen address")?;
    ensure!(addr.ip().is_loopback(), "use a loopback listen address");
    let listener = TcpListener::bind(addr)?;
    listener.set_nonblocking(true)?;
    let host = listener.local_addr()?.to_string();
    let origin = format!("http://{host}");
    let token = hex::encode(rand::random::<[u8; 32]>());
    let state = Arc::new(Local {
        dir: dir.into(),
        host,
        origin: origin.clone(),
        auth: format!("Bearer {token}"),
        engine: Engine {
            dir: dir.into(),
            store: Store::open(dir)?,
        },
        writes: Mutex::new(()),
        profiles: Mutex::new(()),
        previews: Mutex::new(vec![]),
        shutdown: AtomicBool::new(false),
    });
    let meta = Metadata(dir.join("service.json"));
    write_local(&meta.0, &json!({"url":origin,"token":token}))?;
    let link = format!("{origin}/#{token}");
    println!("{link}");
    if browser {
        thread::spawn(move || crate::cli::open_browser(&link));
    }
    let worker = state.clone();
    let scheduler = thread::spawn(move || {
        while !worker.shutdown.load(Ordering::Acquire) {
            if let Err(err) = worker.mutate(Engine::tick) {
                eprintln!("scheduler: {err:#}")
            }
            thread::park_timeout(Duration::from_millis(150));
        }
    });
    let result = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?
        .block_on(async {
            let listener = tokio::net::TcpListener::from_std(listener)?;
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            let shutting_down = state.clone();
            axum::serve(
                listener,
                Router::new().fallback(handle).with_state(state.clone()),
            )
            .with_graceful_shutdown(async move {
                tokio::select! {_=term.recv()=>{},_=tokio::signal::ctrl_c()=>{}}
                shutting_down.shutdown.store(true, Ordering::Release);
            })
            .await
        });
    state.shutdown.store(true, Ordering::Release);
    scheduler.thread().unpark();
    scheduler
        .join()
        .map_err(|_| anyhow::anyhow!("scheduler thread terminated unexpectedly"))?;
    if let Ok(mut children) = state.previews.lock() {
        for child in children.iter_mut() {
            stop_preview(child)
        }
    }
    Ok(result?)
}
fn stop_preview(child: &mut Child) {
    if matches!(child.try_wait(), Ok(None)) {
        let _ = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(child.id() as i32),
            nix::sys::signal::Signal::SIGKILL,
        );
        let _ = child.wait();
    }
}
async fn handle(State(state): State<Arc<Local>>, req: Request) -> Response {
    let address = req
        .headers()
        .get(header::HOST)
        .and_then(|s| s.to_str().ok());
    if address != Some(&state.host) {
        return (StatusCode::BAD_REQUEST, "unexpected local host").into_response();
    }
    let uri = req.uri().clone();
    let path = uri.path().to_owned();
    let api = path.starts_with("/api/");
    let method = req.method().clone();
    let mut response = if api {
        if req
            .headers()
            .get(header::ORIGIN)
            .is_some_and(|v| v.as_bytes() != state.origin.as_bytes())
        {
            (StatusCode::FORBIDDEN, "unexpected origin").into_response()
        } else if !bool::from(
            req.headers()
                .get(header::AUTHORIZATION)
                .map_or(&[][..], |v| v.as_bytes())
                .ct_eq(state.auth.as_bytes()),
        ) {
            (StatusCode::UNAUTHORIZED, "authentication required").into_response()
        } else if path == "/api/session" && method == Method::GET {
            StatusCode::NO_CONTENT.into_response()
        } else {
            let body = match tokio::time::timeout(
                Duration::from_secs(25),
                to_bytes(req.into_body(), 1024 * 1024),
            )
            .await
            {
                Ok(Ok(body)) => body,
                _ => {
                    return (
                        StatusCode::BAD_REQUEST,
                        "request body too large or timed out",
                    )
                        .into_response();
                }
            };
            let query = uri.query().unwrap_or("").to_owned();
            match tokio::task::spawn_blocking(move || state.api(method, &path, &query, &body)).await
            {
                Ok(Ok(value)) => axum::Json(value).into_response(),
                Ok(Err(err)) => {
                    let msg = err.to_string();
                    let status = if msg == "the run changed; review its current state and retry" {
                        StatusCode::CONFLICT
                    } else {
                        StatusCode::BAD_REQUEST
                    };
                    (status, axum::Json(json!({"error":msg}))).into_response()
                }
                Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        }
    } else if method == Method::GET || method == Method::HEAD {
        match assets::asset(if path == "/" {
            "index.html"
        } else {
            path.trim_start_matches('/')
        }) {
            Some((mime, data)) => (
                [(header::CONTENT_TYPE, mime)],
                if method == Method::HEAD {
                    Body::empty()
                } else {
                    Body::from(data)
                },
            )
                .into_response(),
            None if path == "/" => (
                StatusCode::SERVICE_UNAVAILABLE,
                "Build the web interface before launching the workbench.",
            )
                .into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        }
    } else {
        StatusCode::METHOD_NOT_ALLOWED.into_response()
    };
    let h = response.headers_mut();
    h.insert(header::REFERRER_POLICY, "no-referrer".parse().unwrap());
    h.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        "default-src 'self'; style-src 'self'; object-src 'none'; frame-ancestors 'none'"
            .parse()
            .unwrap(),
    );
    if api {
        h.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    }
    response
}

impl Local {
    fn api(&self, method: Method, path: &str, query: &str, body: &[u8]) -> Result<Value> {
        let path: Vec<_> = path.trim_start_matches("/api/").split('/').collect();
        let cursor = |key: &str| {
            url::form_urlencoded::parse(query.as_bytes())
                .find(|(k, _)| k == key)
                .and_then(|(_, v)| v.parse().ok())
                .unwrap_or(0)
        };
        match (method.as_str(), path.as_slice()) {
            ("GET", ["setup"]) => setup(&self.dir),
            ("GET", ["setup", "status"]) => {
                let cfg = setup(&self.dir)?;
                let profiles = cfg["profiles"].as_object().context("invalid profiles")?;
                let mut statuses = serde_json::Map::new();
                for (name, p) in profiles {
                    statuses.insert(name.clone(), adapter::availability(p));
                }
                Ok(json!({"setup":cfg,"profiles":statuses,"git":adapter::installed("git")}))
            }
            ("POST", ["setup", "model"]) => {
                let input = decode(body, "model")?;
                let _lock = self
                    .profiles
                    .lock()
                    .map_err(|_| anyhow::anyhow!("profile lock poisoned"))?;
                let mut cfg = setup(&self.dir)?;
                let p = cfg["profiles"]
                    .get_mut(input["profile"].as_str().unwrap_or(""))
                    .context("agent profile not found")?;
                for key in [
                    "model",
                    "benchmark_model",
                    "benchmark_agent",
                    "benchmark_effort",
                ] {
                    let value = input[key].as_str().unwrap_or("");
                    ensure!(
                        value.len() <= 150 && !value.contains(['\r', '\n', '\0']),
                        "model settings must be one line and at most 150 bytes"
                    );
                    if key != "model" || matches!(p["kind"].as_str(), Some("codex" | "claude")) {
                        p[key] = value.trim().into()
                    }
                }
                write_local(&self.dir.join("profiles.json"), &cfg)?;
                Ok(cfg)
            }
            ("GET", ["destinations"]) => Ok(json!(release::destinations(&self.dir)?)),
            ("GET", ["snapshot"]) => {
                let mut snap = self.engine.store.snapshot()?;
                if let Some(runs) = snap["runs"].as_array_mut() {
                    for r in runs {
                        *r = experiment::public(r.take());
                    }
                }
                Ok(snap)
            }
            ("GET", ["events"]) => {
                let mut events = self.engine.store.events(cursor("after"))?;
                for e in &mut events {
                    e["run"] = experiment::public(e["run"].take());
                }
                Ok(events.into())
            }
            ("GET", ["runs"]) => Ok(self
                .engine
                .store
                .list()?
                .into_iter()
                .map(experiment::public)
                .collect()),
            ("GET", ["runs", id]) => Ok(experiment::public(self.engine.store.get(id)?)),
            ("POST", ["runs"]) => {
                let input = decode(body, "start")?;
                let cfg = setup(&self.dir)?;
                let prefs = json!({"coordinator":input["coordinator"],"workers":input["workers"]});
                let run = self.mutate(|e| e.start(input, cfg["profiles"].clone()))?;
                if let Err(err) = write_local(&self.dir.join("roles.json"), &prefs) {
                    eprintln!("saving role preferences: {err}")
                }
                Ok(experiment::public(run))
            }
            ("POST", ["runs", id]) => Ok(experiment::public(
                self.mutate(|e| e.command(id, decode(body, "command")?))?,
            )),
            ("POST", ["advice"]) => {
                let v = decode(body, "advice")?;
                let cfg = setup(&self.dir)?;
                let profile = cfg["profiles"]
                    .get(v["profile"].as_str().unwrap_or(""))
                    .context("select an advisor profile")?;
                Ok(self
                    .advice(profile, v["prompt"].as_str().unwrap_or(""))?
                    .into())
            }
            (m, ["runs", id, rest @ ..]) => {
                let e = &self.engine;
                let run = e.store.get(id)?;
                let root = e.root(&run);
                let project = run["project"].as_str().unwrap_or("");
                match (m, rest) {
                    ("GET", ["history"]) => {
                        Ok(e.store.project_history(project, cursor("before"))?.into())
                    }
                    ("GET", ["cases"]) => {
                        let mut list = e.store.cases(project, cursor("before"), true)?;
                        for item in &mut list {
                            item["active"] = run["input"]["recipe"]["cases"]
                                .as_array()
                                .is_some_and(|v| {
                                    v.iter().any(|c| {
                                        c["id"] == item["id"] && c["revision"] == item["revision"]
                                    })
                                })
                                .into()
                        }
                        Ok(list.into())
                    }
                    ("GET", ["evidence", cid]) => comparison::evidence(&run, cid, &root),
                    ("GET", ["turns", tid]) => {
                        find(&run, "turns", tid)?;
                        let mut f = File::open(root.join("turns").join(tid).join("events.log"))?;
                        let len = f.metadata()?.len();
                        f.seek(SeekFrom::Start(len.saturating_sub(256 * 1024)))?;
                        let mut raw = vec![];
                        f.take(256 * 1024).read_to_end(&mut raw)?;
                        Ok(String::from_utf8_lossy(&raw).into_owned().into())
                    }
                    ("GET", ["diff", cid]) => {
                        let c = find(&run, "candidates", cid)?;
                        ensure!(c["commit"].is_string(), "candidate has no commit");
                        let diff = project::diff(
                            root.join("integration")
                                .to_str()
                                .context("invalid integration path")?,
                            c["base"].as_str().context("missing base")?,
                            c["commit"].as_str().unwrap(),
                        )?;
                        Ok(diff.into())
                    }
                    ("POST", ["preview", cid]) => self.preview(&run, &root, cid),
                    ("POST", ["blind", pair, side]) => {
                        let cid = experiment::preference_candidate(&run, pair, side)?;
                        self.preview(&run, &root, &cid)
                    }
                    _ => bail!("unknown endpoint"),
                }
            }
            _ => bail!("unknown endpoint"),
        }
    }
    fn preview(&self, r: &Value, root: &Path, id: &str) -> Result<Value> {
        use std::os::unix::process::CommandExt;
        let c = find(r, "candidates", id)?;
        let sha = c["commit"]
            .as_str()
            .context("candidate has no preview source")?;
        let command = if c["round"].as_i64().unwrap_or(0) == 0 {
            &r["input"]["recipe"]["preview"]
        } else {
            &c["preview"]
        };
        let args: Vec<String> = serde_json::from_value(command.clone())
            .context("this recipe has no preview command")?;
        ensure!(!args.is_empty(), "this recipe has no preview command");
        let source = root.join("previews").join(crate::uid());
        project::clone_repo(
            root.join("integration")
                .to_str()
                .context("invalid integration path")?,
            source.to_str().context("invalid preview path")?,
            sha,
            "swarm-preview",
        )?;
        let addr = TcpListener::bind("127.0.0.1:0")?.local_addr()?;
        let data = tempfile::Builder::new()
            .prefix("preview-")
            .tempdir_in(&self.dir)?
            .keep();
        let cwd = source.join(r["repo"]["Subdir"].as_str().unwrap_or("."));
        let mut cmd = eval::command(
            &cwd,
            &data,
            &args,
            &json!({"repository":r["repo"]["Root"],"state":self.dir}),
        )?;
        let log = File::create(data.join("preview.log"))?;
        let mut child = cmd
            .env("SWARM_PORT", addr.port().to_string())
            .env("PORT", addr.port().to_string())
            .env("HOST", "127.0.0.1")
            .stdout(log.try_clone()?)
            .stderr(log)
            .process_group(0)
            .spawn()?;
        let now = Instant::now();
        loop {
            if let Some(s) = child.try_wait()? {
                bail!(
                    "preview exited ({s}). See {}",
                    data.join("preview.log").display()
                )
            }
            if TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_ok() {
                let mut previews = self
                    .previews
                    .lock()
                    .map_err(|_| anyhow::anyhow!("preview lock poisoned"))?;
                previews.retain_mut(|p| matches!(p.try_wait(), Ok(None)));
                previews.push(child);
                return Ok(json!({"url":format!("http://{addr}")}));
            }
            if self.shutdown.load(Ordering::Acquire) || now.elapsed() > Duration::from_secs(12) {
                stop_preview(&mut child);
                bail!(
                    "preview did not listen on SWARM_PORT. See {}",
                    data.join("preview.log").display()
                )
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}
fn find<'a>(run: &'a Value, key: &str, id: &str) -> Result<&'a Value> {
    run[key]
        .as_array()
        .and_then(|arr| arr.iter().find(|v| v["id"] == id))
        .with_context(|| format!("{key} entry not found"))
}

fn decode(body: &[u8], schema: &str) -> Result<Value> {
    let v: Value = serde_json::from_slice(body)?;
    crate::wire::validate(&v, schema)?;
    Ok(v)
}

impl Local {
    fn advice(&self, profile: &Value, prompt: &str) -> Result<String> {
        let job = tempfile::Builder::new()
            .prefix("advice-")
            .tempdir_in(&self.dir)?;
        let work = job.path().join("work");
        fs::create_dir(&work)?;
        let spec = json!({"request":{"role":"advisor","dir":work,"profile":profile,"prompt":format!("Suggest a concise editable developer brief. Identify the highest-impact missing decision, if any. Preserve the developer's goal and distinguish assumptions from requirements. Respond in plain text.\n\n{prompt}")}});
        write_local(&job.path().join("job.json"), &spec)?;
        crate::supervisor::spawn_with_state(job.path(), &self.dir)?;
        let start = Instant::now();
        loop {
            if let Some(result) = crate::supervisor::reception(job.path()) {
                if let Some(err) = result["error"].as_str().filter(|s| !s.is_empty()) {
                    bail!("{err}")
                }
                return Ok(result["text"]
                    .as_str()
                    .context("advisor returned no response")?
                    .into());
            }
            if self.shutdown.load(Ordering::Acquire) || start.elapsed() >= Duration::from_secs(300)
            {
                crate::supervisor::stop_job(job.path())?;
                bail!("advisor timed out")
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}
