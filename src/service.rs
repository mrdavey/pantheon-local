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
    io::Write,
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
fn configured_profiles(dir: &Path) -> Result<Value> {
    Ok(match fs::read(dir.join("profiles.json")) {
        Ok(raw) => serde_json::from_slice(&raw)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            json!({"profiles":{"codex":{"kind":"codex"},"claude":{"kind":"claude"},"hermes":{"kind":"hermes","hermes_profile":"default"},"antigravity":{"kind":"antigravity"}}})
        }
        Err(e) => return Err(e.into()),
    })
}
pub fn setup(dir: &Path) -> Result<Value> {
    let mut cfg = configured_profiles(dir)?;
    let settings = cfg
        .as_object_mut()
        .and_then(|c| c.remove("hermes_models"))
        .unwrap_or_default();
    let profiles = cfg["profiles"]
        .as_object_mut()
        .context("profiles must be an object")?;
    if let Some(home) = std::env::var_os("HOME") {
        adapter::hermes::add_profiles(
            profiles,
            &adapter::hermes::root(Path::new(&home)),
            &settings,
        );
    }
    adapter::models::discover(profiles, dir);
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
    cfg["cwd"] = json!(
        std::env::current_dir()
            .ok()
            .filter(|p| p != Path::new("/"))
            .and_then(|p| p.into_os_string().into_string().ok())
    );
    if let Some(update) = crate::update::cached(dir) {
        cfg["update"] = update;
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
struct Preview {
    owner: String,
    pgid: nix::unistd::Pid,
    child: Child,
    source: PathBuf,
    data: PathBuf,
}
struct Local {
    dir: PathBuf,
    host: String,
    origin: String,
    auth: String,
    engine: Engine,
    writes: Mutex<()>,
    profiles: Mutex<()>,
    picker: Mutex<()>,
    previews: Mutex<Vec<Preview>>,
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
        picker: Mutex::new(()),
        previews: Mutex::new(vec![]),
        shutdown: AtomicBool::new(false),
    });
    for deleted in state.engine.store.deleted_runs()? {
        if let Some(id) = deleted["id"].as_str()
            && let Err(error) = state.remove_checkout(id)
        {
            eprintln!("could not clean up deleted run {id}: {error}");
        }
    }
    let meta = Metadata(dir.join("service.json"));
    write_local(&meta.0, &json!({"url":origin,"token":token}))?;
    let link = crate::cli::workbench_link(&origin, &token);
    let (notices, notifications) = tokio::sync::mpsc::unbounded_channel();
    let update_notice = notices.clone();
    let update_dir = dir.to_path_buf();
    thread::spawn(move || {
        if let Some(update) = crate::update::check(&update_dir) {
            let _ = update_notice.send(crate::launch::Notice::Update(update));
        }
    });
    let worker = state.clone();
    let scheduler = thread::spawn(move || {
        while !worker.shutdown.load(Ordering::Acquire) {
            if let Err(err) = worker.mutate(Engine::tick) {
                let _ = notices.send(crate::launch::Notice::Message(format!(
                    "Scheduler: {err:#}"
                )));
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
            let mut interrupt =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
            let shutting_down = state.clone();
            let server = axum::serve(
                listener,
                Router::new().fallback(handle).with_state(state.clone()),
            )
            .with_graceful_shutdown(async move {
                tokio::select! {_=term.recv()=>{},_=interrupt.recv()=>{}}
                shutting_down.shutdown.store(true, Ordering::Release);
            })
            .into_future();
            // A paused terminal can also block ordinary diagnostic output.
            // Accept browser connections independently of the foreground display.
            let mode = if browser {
                crate::launch::Mode::Owner
            } else {
                crate::launch::Mode::Serve
            };
            crate::launch::monitor(tokio::spawn(server), mode, &link, notifications).await?
        });
    state.shutdown.store(true, Ordering::Release);
    scheduler.thread().unpark();
    scheduler
        .join()
        .map_err(|_| anyhow::anyhow!("scheduler thread terminated unexpectedly"))?;
    if let Ok(mut children) = state.previews.lock() {
        for preview in children.iter_mut() {
            stop_preview(preview)
        }
    }
    Ok(result?)
}
fn stop_preview(preview: &mut Preview) {
    let _ = nix::sys::signal::killpg(preview.pgid, nix::sys::signal::Signal::SIGKILL);
    let _ = preview.child.wait();
    let _ = fs::remove_dir_all(&preview.source);
    let _ = fs::remove_dir_all(&preview.data);
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
            let limit =
                if method == Method::POST && matches!(path.as_str(), "/api/runs" | "/api/advice") {
                    crate::attachments::REQUEST_LIMIT
                } else {
                    1024 * 1024
                };
            let body = match tokio::time::timeout(
                Duration::from_secs(25),
                to_bytes(req.into_body(), limit),
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
            ("POST", ["project-folder"]) => {
                let _guard = self
                    .picker
                    .try_lock()
                    .map_err(|_| anyhow::anyhow!("A file or folder dialog is already open."))?;
                Ok(json!(crate::folder_picker::choose(&self.shutdown)?))
            }
            ("POST", ["pick-initial-files"]) => {
                let _guard = self
                    .picker
                    .try_lock()
                    .map_err(|_| anyhow::anyhow!("A file or folder dialog is already open."))?;
                let input = decode(body, "initial-files")?;
                Ok(json!(crate::folder_picker::choose_initial(
                    input["path"].as_str().context("Select a project folder")?,
                    &self.shutdown
                )?))
            }
            ("POST", ["initial-files"]) => {
                let input = decode(body, "initial-files")?;
                crate::project::initial_choices(
                    input["path"].as_str().context("Choose a project folder.")?,
                )
            }
            ("GET", ["setup"]) => setup(&self.dir),
            ("GET", ["setup", "status"]) => {
                let mut cfg = setup(&self.dir)?;
                if let Some(update) = crate::update::check(&self.dir) {
                    cfg["update"] = update;
                }
                let profiles = cfg["profiles"].as_object().context("invalid profiles")?;
                let mut statuses = serde_json::Map::new();
                for (name, p) in profiles {
                    statuses.insert(name.clone(), adapter::availability(p, Some(&self.dir)));
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
                    "effort",
                    "benchmark_model",
                    "benchmark_agent",
                    "benchmark_effort",
                ] {
                    let Some(value) = input[key].as_str() else {
                        continue;
                    };
                    ensure!(
                        value.len() <= 150 && !value.contains(['\r', '\n', '\0']),
                        "model settings must be one line and at most 150 bytes"
                    );
                    if !matches!(key, "model" | "effort")
                        || matches!(
                            p["kind"].as_str(),
                            Some("codex" | "claude" | "hermes" | "antigravity")
                        )
                    {
                        p[key] = value.trim().into()
                    }
                }
                adapter::validate(p)?;
                let mut config = configured_profiles(&self.dir)?;
                let name = input["profile"].as_str().context("missing profile name")?;
                if config["profiles"].get(name).is_some() {
                    for key in [
                        "model",
                        "effort",
                        "benchmark_model",
                        "benchmark_agent",
                        "benchmark_effort",
                    ] {
                        if let Some(value) = p.get(key) {
                            config["profiles"][name][key] = value.clone();
                        }
                    }
                } else {
                    config["hermes_models"][name] = json!({"model":p["model"],"effort":p["effort"],"benchmark_model":p["benchmark_model"],"benchmark_agent":p["benchmark_agent"],"benchmark_effort":p["benchmark_effort"]});
                }
                write_local(&self.dir.join("profiles.json"), &config)?;
                setup(&self.dir)
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
            ("POST", ["runs", id, "delete"]) => {
                let input: Value = serde_json::from_slice(body)?;
                let revision = input["expected_revision"]
                    .as_i64()
                    .context("missing run revision")?;
                self.mutate(|engine| {
                    ensure_run_id(id)?;
                    let mut previews = self
                        .previews
                        .lock()
                        .map_err(|_| anyhow::anyhow!("preview lock poisoned"))?;
                    let deleted = engine.store.delete(id, revision)?;
                    previews.retain_mut(|preview| {
                        if preview.owner == *id {
                            stop_preview(preview);
                            false
                        } else {
                            true
                        }
                    });
                    self.remove_checkout(id)?;
                    Ok(deleted)
                })
            }
            ("POST", ["runs", id]) => {
                let run = self.mutate(|e| e.command(id, decode(body, "command")?))?;
                if run["state"] == "stopped"
                    && let Ok(mut previews) = self.previews.lock()
                {
                    previews.retain_mut(|preview| {
                        if preview.owner == *id {
                            stop_preview(preview);
                            false
                        } else {
                            true
                        }
                    });
                }
                Ok(experiment::public(run))
            }
            ("POST", ["advice"]) => {
                let mut v = decode(body, "advice")?;
                let files = crate::attachments::unpack(&mut v)?;
                let cfg = setup(&self.dir)?;
                let profile = cfg["profiles"]
                    .get(v["profile"].as_str().unwrap_or(""))
                    .context("select an advisor profile")?;
                Ok(self
                    .advice(profile, v["prompt"].as_str().unwrap_or(""), files)?
                    .into())
            }
            (m, ["runs", id, rest @ ..]) => {
                let e = &self.engine;
                let run = e.store.get(id)?;
                let root = e.root(&run);
                let project = run["project"].as_str().unwrap_or("");
                match (m, rest) {
                    ("POST", ["open-folder"]) => {
                        #[derive(serde::Deserialize)]
                        #[serde(deny_unknown_fields)]
                        struct FolderRequest {
                            candidate: Option<String>,
                            #[serde(default)]
                            working: bool,
                        }
                        let request: FolderRequest = serde_json::from_slice(body)?;
                        let folder = if request.working {
                            ensure!(request.candidate.is_none(), "select one folder");
                            let work = run["working_folder"]
                                .as_str()
                                .map(PathBuf::from)
                                .unwrap_or_else(|| root.join("work"));
                            work.canonicalize()
                                .context("worker folders appear when the first milestone starts")?
                        } else {
                            output_folder(&run, &root, request.candidate.as_deref())?
                        };
                        let opener = if cfg!(target_os = "macos") {
                            "/usr/bin/open"
                        } else {
                            "xdg-open"
                        };
                        use wait_timeout::ChildExt;
                        let mut child = std::process::Command::new(opener)
                            .arg(&folder)
                            .spawn()
                            .with_context(|| {
                                format!("open {} in your file manager", folder.display())
                            })?;
                        // Some Linux launchers stay attached until the folder window closes.
                        if let Some(status) = child.wait_timeout(Duration::from_millis(500))? {
                            ensure!(
                                status.success(),
                                "could not open {} in your file manager",
                                folder.display()
                            );
                        } else {
                            thread::spawn(move || {
                                let _ = child.wait();
                            });
                        }
                        Ok(json!({"opened":true}))
                    }
                    ("POST", ["metrics", "prices"]) => self.mutate(|engine| {
                        let mut latest = engine.store.get(id)?;
                        engine.fetch_prices(&mut latest, 0, false)?;
                        Ok(json!({"status":latest["pricing_status"]}))
                    }),
                    ("GET", ["metrics"]) => {
                        crate::metrics::report(&run, &root, cursor("from").max(0) as usize)
                    }
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
                    ("GET", ["turns", tid, "log"]) => {
                        let turn = find(&run, "turns", tid)?;
                        let mut before = None;
                        let mut known = None;
                        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
                            match key.as_ref() {
                                "before" => {
                                    before =
                                        Some(value.parse::<u64>().context("invalid log cursor")?)
                                }
                                "known" => {
                                    known = Some(value.parse::<u64>().context("invalid log size")?)
                                }
                                _ => (),
                            }
                        }
                        crate::turn_log::page(
                            &root.join("turns").join(tid),
                            before,
                            known,
                            turn["state"] == "running",
                        )
                    }
                    ("GET", ["turns", tid, "reply"]) => {
                        let turn = find(&run, "turns", tid)?;
                        ensure!(turn["role"] == "chat", "this turn has no chat reply");
                        Ok(crate::chat_stream::preview(&root.join("turns").join(tid))?.into())
                    }
                    ("GET", ["turns", tid]) => {
                        find(&run, "turns", tid)?;
                        let path = root.join("turns").join(tid).join("events.log");
                        match crate::supervisor::log_tail(&path, 256 * 1024) {
                            Ok(log) => Ok(log.into()),
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                Ok(String::new().into())
                            }
                            Err(e) => Err(e.into()),
                        }
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
    fn remove_checkout(&self, id: &str) -> Result<()> {
        ensure_run_id(id)?;
        match fs::remove_dir_all(self.dir.join("runs").join(id)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            result => Ok(result?),
        }
    }

    fn preview(&self, r: &Value, root: &Path, id: &str) -> Result<Value> {
        use std::os::unix::process::CommandExt;
        let owner = r["id"].as_str().context("missing run ID")?;
        let latest = self.engine.store.get(owner)?;
        ensure!(
            latest["state"] != "stopped",
            "resume the run before starting a preview"
        );
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
        let data = tempfile::Builder::new()
            .prefix("preview-")
            .tempdir_in(root.join("previews"))?
            .keep();
        let start = (|| -> Result<(SocketAddr, Preview)> {
            let cwd = project::subdir(&source, r["repo"]["Subdir"].as_str().unwrap_or("."))?;
            let mut cmd = eval::command(
                &cwd,
                &data,
                &args,
                &json!({"repository":r["repo"]["Root"],"common":r["repo"]["Common"],"state":self.dir}),
            )?;
            let addr = TcpListener::bind("127.0.0.1:0")?.local_addr()?;
            let log = File::create(data.join("preview.log"))?;
            let child = cmd
                .env("SWARM_PORT", addr.port().to_string())
                .env("PORT", addr.port().to_string())
                .env("HOST", "127.0.0.1")
                .stdout(log.try_clone()?)
                .stderr(log)
                .process_group(0)
                .spawn()?;
            let pgid = nix::unistd::Pid::from_raw(child.id() as i32);
            let mut preview = Preview {
                owner: owner.to_owned(),
                pgid,
                child,
                source: source.clone(),
                data: data.clone(),
            };
            let now = Instant::now();
            loop {
                if let Some(s) = preview.child.try_wait()? {
                    let log_path = preview.data.join("preview.log");
                    let _ =
                        nix::sys::signal::killpg(preview.pgid, nix::sys::signal::Signal::SIGKILL);
                    let _ = fs::remove_dir_all(&preview.source);
                    bail!("preview exited ({s}). See {}", log_path.display())
                }
                if TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_ok() {
                    return Ok((addr, preview));
                }
                if self.shutdown.load(Ordering::Acquire) || now.elapsed() > Duration::from_secs(12)
                {
                    let log_path = preview.data.join("preview.log");
                    let _ =
                        nix::sys::signal::killpg(preview.pgid, nix::sys::signal::Signal::SIGKILL);
                    let _ = preview.child.wait();
                    let _ = fs::remove_dir_all(&preview.source);
                    bail!(
                        "preview did not listen on SWARM_PORT. See {}",
                        log_path.display()
                    )
                }
                thread::sleep(Duration::from_millis(100));
            }
        })();
        let (addr, mut preview) = match start {
            Ok(started) => started,
            Err(err) => {
                let _ = fs::remove_dir_all(&source);
                return Err(err);
            }
        };
        let mut previews = self
            .previews
            .lock()
            .map_err(|_| anyhow::anyhow!("preview lock poisoned"))?;
        match self.engine.store.get(owner) {
            Ok(current) if current["state"] != "stopped" => {}
            _ => {
                stop_preview(&mut preview);
                bail!("run was stopped or deleted before the preview started");
            }
        }
        previews.retain_mut(|existing| {
            if matches!(existing.child.try_wait(), Ok(None)) {
                true
            } else {
                stop_preview(existing);
                false
            }
        });
        while previews.iter().filter(|p| p.owner == owner).count() >= 6 {
            if let Some(pos) = previews.iter().position(|p| p.owner == owner) {
                let mut oldest = previews.remove(pos);
                stop_preview(&mut oldest);
            }
        }
        previews.push(preview);
        Ok(json!({"url":format!("http://{addr}")}))
    }
}
fn ensure_run_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "invalid run or candidate ID"
    );
    Ok(())
}

fn output_folder(run: &Value, root: &Path, candidate: Option<&str>) -> Result<PathBuf> {
    let base = if let Some(id) = candidate {
        find(run, "candidates", id)?;
        ensure_run_id(id)?;
        root.join("candidates").join(id).join("source")
    } else {
        PathBuf::from(
            run["repo"]["Root"]
                .as_str()
                .context("missing project folder")?,
        )
    };
    project::subdir(&base, run["repo"]["Subdir"].as_str().unwrap_or("."))
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
    fn advice(
        &self,
        profile: &Value,
        prompt: &str,
        files: Vec<(String, Vec<u8>)>,
    ) -> Result<String> {
        let job = tempfile::Builder::new()
            .prefix("advice-")
            .tempdir_in(&self.dir)?;
        let work = job.path().join("work");
        fs::create_dir(&work)?;
        let references = if files.is_empty() {
            String::new()
        } else {
            let files = crate::attachments::save(&work, files)?;
            format!(
                "\n\nBrief attachments: {}",
                crate::attachments::context(&work, &files)?
            )
        };
        let spec = json!({"request":{"role":"advisor","dir":work,"profile":profile,"prompt":format!("Suggest a concise editable developer brief. Identify the highest-impact missing decision, if any. Preserve the developer's goal and distinguish assumptions from requirements. Respond in plain text.\n\n{prompt}{references}")}});
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
                let stop_started = Instant::now();
                while stop_started.elapsed() < Duration::from_millis(1500)
                    && crate::supervisor::reception(job.path()).is_none()
                {
                    thread::sleep(Duration::from_millis(25));
                }
                bail!("advisor timed out")
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod output_folder_tests {
    use super::*;

    #[test]
    fn opens_only_the_recorded_project_or_candidate_subdirectory() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let project = dir.path().join("project");
        let root = dir.path().join("run");
        let candidate = root.join("candidates/v1/source");
        for base in [&project, &candidate] {
            fs::create_dir_all(base.join("app"))?;
        }
        let mut run = json!({"repo":{"Root":project,"Subdir":"app"},"candidates":[{"id":"v1"}]});
        assert_eq!(
            output_folder(&run, &root, None)?,
            project.join("app").canonicalize()?
        );
        assert_eq!(
            output_folder(&run, &root, Some("v1"))?,
            candidate.join("app").canonicalize()?
        );
        assert!(output_folder(&run, &root, Some("../other")).is_err());
        run["repo"]["Subdir"] = json!("..");
        assert!(output_folder(&run, &root, None).is_err());
        let escaped = root.join("candidates/v2/source");
        fs::create_dir_all(&escaped)?;
        std::os::unix::fs::symlink(dir.path(), escaped.join("app"))?;
        run["repo"]["Subdir"] = json!("app");
        run["candidates"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"v2"}));
        assert!(output_folder(&run, &root, Some("v2")).is_err());
        assert!(project::subdir(&escaped, "app").is_err());
        Ok(())
    }
}
