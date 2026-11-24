use anyhow::{Context, Result, ensure};
use reqwest::blocking::Client;
use serde_json::Value;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn metadata(daemon: &mut Daemon, path: &std::path::Path) -> Result<Value> {
    let start = Instant::now();
    loop {
        if let Ok(bytes) = fs::read(path) {
            return Ok(serde_json::from_slice(&bytes)?);
        }
        ensure!(
            daemon.0.try_wait()?.is_none() && start.elapsed() < Duration::from_secs(5),
            "service failed to start"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn loopback_session_protects_reads_and_writes() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let launch = || {
        let mut c = Command::new(env!("CARGO_BIN_EXE_swarm"));
        c.env("SWARM_STATE_DIR", dir.path()).arg("serve");
        c
    };
    let mut daemon = Daemon(
        launch()
            .args(["--listen", "localhost:0"])
            .stdout(Stdio::null())
            .spawn()?,
    );
    let path = dir.path().join("service.json");
    let session = metadata(&mut daemon, &path)?;
    assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o077, 0);
    assert!(!launch().output()?.status.success());
    let url = session["url"].as_str().context("missing URL")?;
    let token = session["token"].as_str().context("missing token")?;
    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()?;
    let endpoint = format!("{url}/api/runs");
    assert_eq!(client.get(&endpoint).send()?.status(), 401);
    assert_eq!(
        client
            .get(&endpoint)
            .bearer_auth(token)
            .header("Host", "wrong.invalid")
            .send()?
            .status(),
        400
    );
    assert_eq!(
        client
            .post(&endpoint)
            .bearer_auth(token)
            .header("Origin", "https://example.invalid")
            .send()?
            .status(),
        403
    );
    assert_eq!(
        client
            .post(&endpoint)
            .bearer_auth(token)
            .json(&serde_json::json!({"unknown_field":true}))
            .send()?
            .status(),
        400
    );
    let ok = client.get(endpoint).bearer_auth(token).send()?;
    assert_eq!(ok.status(), 200);
    assert_eq!(ok.headers()["cache-control"], "no-store");
    assert_eq!(ok.json::<Value>()?, serde_json::json!([]));
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn daemon_can_spawn_its_own_supervisor_after_binary_replacement() -> Result<()> {
    use std::{os::unix::fs::symlink, path::PathBuf};
    let tmp = tempfile::tempdir()?;
    let installed = tmp.path().join("swarm");
    let state = tmp.path().join("state");
    fs::create_dir(&state)?;
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_swarm"));
    let fixture = binary
        .parent()
        .context("missing profile directory")?
        .join("examples/fixture-agent");
    fs::write(
        state.join("profiles.json"),
        serde_json::to_vec(
            &serde_json::json!({"profiles":{"fixture":{"kind":"custom","executable":fixture}}}),
        )?,
    )?;
    fs::copy(&binary, &installed)?;
    let mut daemon = Daemon(
        Command::new(&installed)
            .arg("serve")
            .env("SWARM_STATE_DIR", &state)
            .stdout(Stdio::null())
            .spawn()?,
    );
    let session = metadata(&mut daemon, &state.join("service.json"))?;
    let replacement = tmp.path().join("replacement");
    symlink("/bin/false", &replacement)?;
    fs::rename(replacement, installed)?;
    let response = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()?
        .post(format!(
            "{}/api/advice",
            session["url"].as_str().context("missing service URL")?
        ))
        .bearer_auth(session["token"].as_str().context("missing service token")?)
        .json(&serde_json::json!({"profile":"fixture","prompt":"Suggest a brief."}))
        .send()?;
    let status = response.status();
    let body = response.text()?;
    assert!(status.is_success(), "{body}");
    assert!(body.contains("Keep quotes private"), "{body}");
    Ok(())
}
