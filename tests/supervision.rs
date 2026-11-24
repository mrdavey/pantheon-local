//! Exercise cancellation at the subprocess boundary, including a worker that ignores TERM.
#[path = "common/example.rs"]
mod example;
use anyhow::{Context, Result, ensure};
use nix::{
    errno::Errno,
    sys::signal::{Signal, kill, killpg},
    unistd::Pid,
};
use serde_json::json;
use std::{
    fs,
    os::unix::process::CommandExt,
    process::Command,
    thread,
    time::{Duration, Instant},
};

#[test]
fn cancellation_reaps_the_worker_group() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let dir = temp.path();
    let worker = example::native_fixture(dir, "mock-resistant")?;
    fs::write(
        dir.join("job.json"),
        serde_json::to_vec(
            &json!({"request":{"profile":{"kind":"custom","executable":worker},"role":"worker","dir":dir,"prompt":"wait"}}),
        )?,
    )?;
    let mut process = Command::new(env!("CARGO_BIN_EXE_swarm"))
        .arg("_supervise")
        .arg(dir)
        .process_group(0)
        .spawn()?;
    let group = Pid::from_raw(process.id().try_into()?);
    let result = (|| -> Result<()> {
        let start = Instant::now();
        let worker = loop {
            if let Ok(raw) = fs::read_to_string(dir.join("worker.pid"))
                && let Ok(id) = raw.trim().parse::<i32>()
            {
                break Pid::from_raw(id);
            }
            ensure!(
                start.elapsed() < Duration::from_secs(5) && process.try_wait()?.is_none(),
                "worker failed to start"
            );
            thread::sleep(Duration::from_millis(20));
        };
        fs::write(dir.join("stop"), "")?;
        killpg(group, Signal::SIGTERM)?;
        let start = Instant::now();
        while process.try_wait()?.is_none() {
            ensure!(
                start.elapsed() < Duration::from_secs(5),
                "supervisor failed to stop"
            );
            thread::sleep(Duration::from_millis(20));
        }
        // A dead child may briefly remain a zombie until its new parent reaps it.
        if kill(worker, None) != Err(Errno::ESRCH) {
            let out = Command::new("ps")
                .args(["-p", &worker.to_string(), "-o", "stat="])
                .output()?;
            ensure!(
                !out.status.success() || String::from_utf8(out.stdout)?.trim().starts_with('Z'),
                "worker survived cancellation"
            );
        }
        Ok(())
    })();
    if process.try_wait()?.is_none() {
        let _ = killpg(group, Signal::SIGKILL);
        process.wait().context("reaping test supervisor")?;
    }
    result
}
