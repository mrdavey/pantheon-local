mod common;
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
        c.env("SWARM_STATE_DIR", dir.path())
            .env("SWARM_UPDATE_URL", "off")
            .arg("serve");
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
            .env("SWARM_UPDATE_URL", "off")
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

#[test]
fn run_starts_after_launch_directory_disappears() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let start = tmp.path().join("launcher");
    let state = tmp.path().join("state");
    fs::create_dir(&start)?;
    fs::create_dir(&state)?;
    let project = common::repo(&tmp.path().join("project"))?;
    common::git(&project, &["commit", "--allow-empty", "-m", "Base"])?;
    fs::write(
        state.join("profiles.json"),
        r#"{"profiles":{"fixture":{"kind":"custom","executable":"true"}}}"#,
    )?;
    let mut daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_swarm"))
            .current_dir(&start)
            .env("SWARM_STATE_DIR", &state)
            .env("SWARM_UPDATE_URL", "off")
            .arg("serve")
            .stdout(Stdio::null())
            .spawn()?,
    );
    let info = metadata(&mut daemon, &state.join("service.json"))?;
    fs::remove_dir(start)?;
    let response=Client::builder().no_proxy().timeout(Duration::from_secs(10)).build()?
        .post(format!("{}/api/runs",info["url"].as_str().unwrap()))
        .bearer_auth(info["token"].as_str().unwrap())
        .json(&serde_json::json!({"path":project,"prompt":"Test project access","coordinator":"fixture","workers":["fixture"]})).send()?;
    let status = response.status();
    let run = response.json::<Value>()?;
    ensure!(status.is_success(), "{run}");
    assert_eq!(run["state"], "planning");
    Ok(())
}

#[test]
fn desktop_profile_settings_follow_the_discovered_agent() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let home = tmp.path().join("hermes-home");
    let profile = home.join("profiles/work");
    fs::create_dir_all(&profile)?;
    fs::write(profile.join("config.yaml"), "")?;
    let svc = common::Service::with_profiles(
        tmp.path(),
        std::collections::BTreeMap::from([("HERMES_HOME".into(), home.clone())]),
        &serde_json::json!({"profiles":{"hermes":{"kind":"hermes","executable":"true"}}}),
    )?;
    let current = svc.get("/setup")?;
    assert_eq!(current["profiles"]["hermes-work"]["hermes_profile"], "work");
    svc.post(
        "/setup/model",
        &serde_json::json!({"profile":"hermes-work","benchmark_model":"fixture"}),
    )?;
    assert_eq!(
        svc.get("/setup")?["profiles"]["hermes-work"]["benchmark_model"],
        "fixture"
    );
    fs::create_dir_all(home.join("profiles/.deleted/work"))?;
    assert!(svc.get("/setup")?["profiles"]["hermes-work"].is_null());
    Ok(())
}

#[test]
fn rejected_parent_leaves_child_free_to_start_its_own_project() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let home = tmp.path().join("home");
    fs::create_dir(&home)?;
    fs::write(
        home.join(".gitconfig"),
        "[user]\nname = Test\nemail = test@example.invalid\n",
    )?;
    let svc = common::Service::with_env(tmp.path(), [("HOME".into(), home)].into())?;
    let parent = tmp.path().canonicalize()?.join("Downloads");
    let child = parent.join("test");
    fs::create_dir_all(child.join("src"))?;
    fs::write(parent.join("download.txt"), "private")?;
    let mut input = serde_json::json!({"path":parent,"prompt":"Plan an expense CLI","coordinator":"first","workers":["writer"]});
    let err = svc.post("/runs", &input).unwrap_err().to_string();
    assert!(
        err.contains(parent.to_str().unwrap()) && err.contains("Initial import files"),
        "{err}"
    );
    assert!(!parent.join(".git").exists());

    input["path"] = serde_json::json!(child);
    let run = svc.post("/runs", &input)?;
    assert_eq!(run["repo"]["Root"], child.to_str().unwrap());
    assert_eq!(run["repo"]["Subdir"], ".");
    assert_eq!(run["state"], "planning");
    assert!(common::git(&child, &["ls-tree", "-r", "HEAD"])?.is_empty());
    assert!(!parent.join(".git").exists());
    Ok(())
}

#[test]
fn new_project_import_commits_only_the_validated_literal_selection() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let home = tmp.path().join("home");
    fs::create_dir(&home)?;
    fs::write(
        home.join(".gitconfig"),
        "[user]\nname = Test\nemail = test@example.invalid\n",
    )?;
    let svc = common::Service::with_env(tmp.path(), [("HOME".into(), home)].into())?;
    let folder = tmp.path().join("project");
    fs::create_dir(&folder)?;
    fs::write(folder.join("notes[1]"), "wanted")?;
    fs::write(folder.join("notes1"), "unselected")?;
    let input = serde_json::json!({"path":folder,"prompt":"Review notes","coordinator":"first","workers":["writer"],"initial_files":["notes[1]"]});
    svc.post("/runs", &input)?;
    assert_eq!(common::git(&folder, &["ls-files"])?, "notes[1]");
    assert_eq!(
        common::git(&folder, &["status", "--porcelain"])?,
        "?? notes1"
    );
    Ok(())
}

#[test]
fn notifies_cli_and_workbench_when_a_newer_release_is_available() -> Result<()> {
    use std::{
        io::{BufRead, BufReader, Write},
        net::{TcpListener, TcpStream},
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };
    let tmp = tempfile::tempdir()?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let endpoint = format!("http://{addr}/releases/latest");
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = stop.clone();
    let server = thread::spawn(move || {
        for incoming in listener.incoming() {
            if stop_flag.load(Ordering::Acquire) {
                break;
            }
            let Ok(mut sock) = incoming else {
                break;
            };
            let mut reader = BufReader::new(&sock);
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                    break;
                }
            }
            let body = serde_json::json!({
                "tag_name": "v9.1.0",
                "draft": false,
                "prerelease": false,
                "html_url": "https://github.com/mrdavey/pantheon-local/releases/tag/v9.1.0"
            })
            .to_string();
            let _ = write!(
                sock,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
        }
    });

    let svc = common::Service::with_env(
        tmp.path(),
        [("SWARM_UPDATE_URL".into(), PathBuf::from(&endpoint))].into(),
    )?;
    let expected = serde_json::json!({
        "current": env!("CARGO_PKG_VERSION"),
        "latest": "9.1.0",
        "url": "https://github.com/mrdavey/pantheon-local/releases/tag/v9.1.0"
    });
    let report = svc.get("/setup/status")?;
    assert_eq!(report["setup"]["update"], expected);
    assert_eq!(svc.get("/setup")?["update"], expected);

    let status_out = Command::new(env!("CARGO_BIN_EXE_swarm"))
        .arg("status")
        .env("SWARM_STATE_DIR", &svc.state)
        .env("SWARM_UPDATE_URL", &endpoint)
        .output()?;
    assert!(status_out.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&status_out.stdout)?,
        serde_json::json!([])
    );
    let status_err = String::from_utf8(status_out.stderr)?;
    assert!(
        status_err.contains("Update available: Pantheon Local v9.1.0")
            && status_err.contains("https://github.com/mrdavey/pantheon-local/releases/tag/v9.1.0"),
        "{status_err}"
    );

    let doctor_out = Command::new(env!("CARGO_BIN_EXE_swarm"))
        .arg("doctor")
        .env("SWARM_STATE_DIR", &svc.state)
        .env("SWARM_UPDATE_URL", &endpoint)
        .output()?;
    assert!(doctor_out.status.success());
    let doctor_err = String::from_utf8(doctor_out.stderr)?;
    assert!(
        doctor_err.contains("Update available: Pantheon Local v9.1.0"),
        "{doctor_err}"
    );

    stop.store(true, Ordering::Release);
    let _ = TcpStream::connect(addr);
    let _ = server.join();
    Ok(())
}

#[test]
fn foreground_launch_reveals_a_working_link_and_restores_the_terminal() -> Result<()> {
    use std::{io::Write, os::unix::fs::symlink};

    let tmp = tempfile::tempdir()?;
    let state = tmp.path().join("state");
    let transcript = tmp.path().join("terminal");
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    symlink("/usr/bin/true", tmp.path().join(opener))?;
    let path = std::env::join_paths(std::iter::once(tmp.path().to_owned()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))?;
    // BSD and util-linux script both give the launcher a controlling terminal.
    let command = r#"stty ixon rows 31 cols 84; exec "$PANTHEON_TEST_BIN""#;
    let mut script = Command::new("script");
    if cfg!(target_os = "macos") {
        script.args(["-q", "/dev/null", "/bin/sh", "-c", command]);
    } else {
        script.args(["-q", "-e", "-c", command, "/dev/null"]);
    }
    let mut daemon = Daemon(
        script
            .env("PANTHEON_TEST_BIN", env!("CARGO_BIN_EXE_swarm"))
            .env("PATH", &path)
            .env("TERM", "xterm-256color")
            .env_remove("SWARM_PLAIN")
            .env_remove("REDUCE_MOTION")
            .env("SWARM_STATE_DIR", &state)
            .env("SWARM_UPDATE_URL", "off")
            .stdin(Stdio::piped())
            .stdout(fs::File::create(&transcript)?)
            .spawn()?,
    );
    let session = metadata(&mut daemon, &state.join("service.json"))?;
    let wait_text = |needle: &str| -> Result<String> {
        let start = Instant::now();
        loop {
            let data = fs::read(&transcript)?;
            let text = String::from_utf8_lossy(&data);
            if text.contains(needle) {
                return Ok(text.into_owned());
            }
            ensure!(
                start.elapsed() < Duration::from_secs(5),
                "terminal did not render {needle:?}"
            );
            thread::sleep(Duration::from_millis(30));
        }
    };
    let first = wait_text("Show browser link")?;
    assert!(first.contains("Workbench online"));
    assert!(first.contains("\x1b[?1049h"));
    let token = session["token"].as_str().context("session token")?;
    assert!(!first.contains(token));
    let origin = session["url"].as_str().context("workbench URL")?;
    let session_check = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()?
        .get(format!("{origin}/api/session"))
        .bearer_auth(token);
    // XOFF pauses terminal output while the browser keeps using the daemon.
    daemon.0.stdin.as_mut().unwrap().write_all(b"\x13")?;
    thread::sleep(Duration::from_millis(350));
    let during_pause = session_check.try_clone().unwrap().send();
    daemon.0.stdin.as_mut().unwrap().write_all(b"\x11\n")?;
    assert_eq!(during_pause?.status(), 204);
    let plain = wait_text(token)?;
    assert!(plain.contains("\x1b[?25h\x1b[?1049l"));
    assert_eq!(session_check.send()?.status(), 204);
    daemon.0.stdin.as_mut().unwrap().write_all(b"\x03")?;
    let start = Instant::now();
    while daemon.0.try_wait()?.is_none() {
        ensure!(
            start.elapsed() < Duration::from_secs(5),
            "Ctrl+C did not return to the shell"
        );
        thread::sleep(Duration::from_millis(30));
    }
    assert!(!state.join("service.json").exists());
    Ok(())
}
