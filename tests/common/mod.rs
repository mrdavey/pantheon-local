//! Shared harness for integration tests against a real local daemon.
#![allow(dead_code)] // Cargo compiles each integration target as its own crate.
use anyhow::{Context, Result, bail, ensure};
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use wait_timeout::ChildExt;

pub const TIMEOUT: Duration = Duration::from_secs(40);
mod example;
pub use example::example;
// A caller may invoke Cargo from a Git hook with its real index exported.
fn isolated_git_environment(command: &mut Command) -> &mut Command {
    for (name, _) in std::env::vars_os() {
        if name.as_encoded_bytes().starts_with(b"GIT_") {
            command.env_remove(name);
        }
    }
    command
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
}
pub fn git(path: &Path, args: &[&str]) -> Result<String> {
    let output = isolated_git_environment(&mut Command::new("git"))
        .current_dir(path)
        .args(args)
        .output()?;
    ensure!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim_end().to_owned())
}
pub fn repo(path: &Path) -> Result<PathBuf> {
    fs::create_dir(path)?;
    git(path, &["init", "-b", "main"])?;
    git(path, &["config", "user.name", "Pantheon Local Test"])?;
    git(path, &["config", "user.email", "test@example.invalid"])?;
    Ok(path.into())
}
pub fn until<T>(timeout: Duration, mut f: impl FnMut() -> Result<Option<T>>) -> Result<T> {
    let start = Instant::now();
    loop {
        if let Some(v) = f()? {
            return Ok(v);
        }
        ensure!(
            start.elapsed() < timeout,
            "condition did not become true within {timeout:?}"
        );
        thread::sleep(Duration::from_millis(120));
    }
}
pub fn rows(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}
pub fn item<'a>(rows: &'a Value, key: &str, id: &Value) -> Result<&'a Value> {
    self::rows(rows)
        .iter()
        .find(|v| v[key] == *id)
        .with_context(|| format!("no {key}={id} in {rows}"))
}
pub fn run_path(run: &Value) -> Result<String> {
    Ok(format!("/runs/{}", run["id"].as_str().context("run ID")?))
}
pub fn passing(r: &Value) -> Result<&Value> {
    item(&r["candidates"], "state", &json!("passed"))
}
pub fn experiment_recipe() -> Result<Value> {
    Ok(json!({
        "checks":[["true"]],
        "cases":[{"id":"search","driver":example("experiment-grader")?,"samples":3,"required":true,"holdback":true}],
        "rule":{"metrics":[{"name":"relevance","unit":"hits","direction":"higher"},{"name":"latency","unit":"ms","direction":"lower"}],
        "objective":"relevance","limits":{"latency":5},"minimum":2,"min_samples":3}
    }))
}
pub struct Service {
    pub state: PathBuf,
    pub info: Value,
    client: Client,
    env: BTreeMap<String, PathBuf>,
    child: Option<Child>,
}
impl Service {
    pub fn new(root: &Path) -> Result<Self> {
        Self::with_env(root, BTreeMap::new())
    }
    pub fn with_env(root: &Path, env: BTreeMap<String, PathBuf>) -> Result<Self> {
        let profile = json!({"kind":"custom","executable":example("fixture-agent")?});
        let profiles = ["first", "second", "writer", "writer-two", "verifier"]
            .into_iter()
            .map(|k| (k, profile.clone()))
            .collect::<BTreeMap<_, _>>();
        Self::with_profiles(root, env, &json!({"profiles":profiles}))
    }
    pub fn with_profiles(
        root: &Path,
        env: BTreeMap<String, PathBuf>,
        profiles: &Value,
    ) -> Result<Self> {
        let state = root.join("state");
        fs::create_dir(&state)?;
        fs::write(state.join("profiles.json"), serde_json::to_vec(profiles)?)?;
        let mut svc = Self {
            state,
            info: Value::Null,
            env,
            child: None,
            client: Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(15))
                .build()?,
        };
        svc.start()?;
        Ok(svc)
    }
    fn start(&mut self) -> Result<()> {
        let test = std::env::current_exe()?;
        let archive = test
            .parent()
            .filter(|d| d.ends_with("tests"))
            .and_then(Path::parent)
            .map(|d| d.join("swarm"))
            .filter(|p| p.is_file());
        let binary = archive
            .or_else(|| std::env::var_os("SWARM_BIN").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_swarm")));
        self.child = Some(
            isolated_git_environment(&mut Command::new(binary))
                .arg("serve")
                .envs(&self.env)
                .env("SWARM_STATE_DIR", &self.state)
                .stdout(Stdio::null())
                .spawn()?,
        );
        self.info = until(Duration::from_secs(10), || {
            let child = self.child.as_mut().context("daemon missing")?;
            ensure!(child.try_wait()?.is_none(), "daemon exited during startup");
            match fs::read(self.state.join("service.json")) {
                Ok(data) => Ok(Some(serde_json::from_slice(&data)?)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e.into()),
            }
        })?;
        Ok(())
    }
    fn stop(&mut self, hard: bool) -> Result<()> {
        if let Some(child) = &mut self.child {
            if child.try_wait()?.is_none() {
                if hard {
                    child.kill()?
                } else {
                    kill(Pid::from_raw(child.id() as i32), Signal::SIGTERM)?;
                }
                if child.wait_timeout(Duration::from_secs(10))?.is_none() {
                    child.kill()?;
                    child.wait()?;
                    bail!("daemon did not stop within deadline")
                }
            }
            self.child = None;
        }
        let meta = self.state.join("service.json");
        if meta.exists() {
            fs::remove_file(meta)?
        }
        Ok(())
    }
    pub fn restart(&mut self, hard: bool) -> Result<()> {
        self.stop(hard)?;
        self.start()
    }
    fn request(&self, path: &str, value: Option<&Value>) -> Result<Value> {
        let url = format!(
            "{}/api{path}",
            self.info["url"].as_str().context("daemon URL")?
        );
        let request = if let Some(data) = value {
            self.client.post(&url).json(data)
        } else {
            self.client.get(&url)
        };
        let response = request
            .bearer_auth(self.info["token"].as_str().context("daemon token")?)
            .send()?;
        let status = response.status();
        let data = response.text()?;
        ensure!(status.is_success(), "{path}: {status}: {data}");
        serde_json::from_str(&data).with_context(|| format!("invalid JSON on {path}"))
    }
    pub fn get(&self, path: &str) -> Result<Value> {
        self.request(path, None)
    }
    pub fn post(&self, path: &str, data: &Value) -> Result<Value> {
        self.request(path, Some(data))
    }
    pub fn wait_state(&self, path: &str, state: &str) -> Result<Value> {
        until(TIMEOUT, || {
            let r = self.get(path)?;
            ensure!(
                r["state"] != "blocked" || state == "blocked",
                "run blocked: {r}"
            );
            Ok((r["state"] == state).then_some(r))
        })
        .with_context(|| format!("waiting for {state}; latest: {:?}", self.get(path)))
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.stop(false);
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
