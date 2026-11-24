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
    let start = Instant::now();
    let session: Value = loop {
        if let Ok(bytes) = fs::read(&path) {
            break serde_json::from_slice(&bytes)?;
        }
        ensure!(
            daemon.0.try_wait()?.is_none() && start.elapsed() < Duration::from_secs(5),
            "service failed to start"
        );
        thread::sleep(Duration::from_millis(25));
    };
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
