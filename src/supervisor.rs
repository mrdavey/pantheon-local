use anyhow::{Context, Result, bail, ensure};
use nix::{
    errno::Errno,
    sys::signal::{Signal, killpg},
    unistd::{Pid, getpgid},
};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    os::unix::{fs::OpenOptionsExt, process::CommandExt},
    path::Path,
    process::{Command, Stdio},
    sync::Mutex,
    thread,
    time::Duration,
};

pub fn write_json(path: &Path, value: &Value) -> Result<()> {
    let mut f = tempfile::NamedTempFile::new_in(path.parent().context("missing job directory")?)?;
    serde_json::to_writer(f.as_file_mut(), value)?;
    f.write_all(b"\n")?;
    f.as_file().sync_all()?;
    f.persist(path)?;
    Ok(())
}
fn read_json(path: &Path) -> Result<Value> {
    Ok(serde_json::from_reader(File::open(path)?)?)
}
pub fn spawn(dir: &Path) -> Result<()> {
    let state = dir.ancestors().nth(4).context("invalid job directory")?;
    spawn_with_state(dir, state)
}
pub fn spawn_with_state(dir: &Path, state: &Path) -> Result<()> {
    let job = read_json(&dir.join("job.json"))?;
    let mut cmd = Command::new(std::env::current_exe()?);
    cmd.arg("_supervise")
        .arg(dir)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if job["release"].is_null() {
        crate::run::release::scrub_worker(&mut cmd, state)?;
    }
    let mut child = cmd.spawn()?;
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
pub fn supervise(dir: &Path) -> Result<()> {
    // Keep ownership of the process-group ID until every child has received
    // the stop signal, even when an adapter ignores graceful cancellation.
    let own_group = getpgid(None)? == Pid::this();
    let mut signals = signal_hook::iterator::Signals::new([signal_hook::consts::SIGTERM])?;
    let signal_handle = signals.handle();
    let watchdog = thread::spawn(move || {
        if signals.forever().next().is_some() && own_group {
            thread::sleep(Duration::from_millis(750));
            let _ = killpg(Pid::this(), Signal::SIGKILL);
        }
    });
    let mut claim = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(dir.join("claim"))?;
    writeln!(claim, "{}", std::process::id())?;
    claim.sync_all()?;
    drop(claim);
    let result = match execute(dir) {
        Ok(v) => v,
        Err(e) => json!({"error":e.to_string()}),
    };
    let written = write_json(&dir.join("result.json"), &result);
    signal_handle.close();
    // A cancelled job can finish while ignored children remain in its group.
    if own_group && dir.join("stop").is_file() {
        let _ = killpg(Pid::this(), Signal::SIGKILL);
    }
    let _ = watchdog.join();
    written
}
fn execute(dir: &Path) -> Result<Value> {
    let spec = read_json(&dir.join("job.json"))?;
    ensure!(!dir.join("stop").try_exists()?, "stopped");
    let mut log = OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(dir.join("events.log"))?;
    let result = if !spec["release"].is_null() {
        crate::run::release::supervise(&spec["release"], &mut log)?
    } else if !spec["recipe"].is_null() {
        crate::eval::supervise(dir, &spec, &mut log)?;
        json!({})
    } else {
        let logger = Mutex::new((BufWriter::new(&mut log), None::<io::Error>));
        let result = crate::adapter::run(&spec["request"], None, |event| {
            if let Ok(mut guard) = logger.lock() {
                let (writer, error) = &mut *guard;
                if error.is_none() {
                    *error = (|| -> io::Result<()> {
                        serde_json::to_writer(&mut *writer, &event)?;
                        writer.write_all(b"\n")?;
                        writer.flush()
                    })()
                    .err();
                }
            }
        });
        let (mut writer, error) = logger
            .into_inner()
            .map_err(|_| anyhow::anyhow!("event writer failed"))?;
        writer.flush()?;
        if let Some(e) = error {
            return Err(e.into());
        }
        result
    };
    log.sync_all()?;
    Ok(result)
}
fn identity(dir: &Path) -> Result<Option<Pid>> {
    let raw = match fs::read_to_string(dir.join("claim")) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let raw: i32 = raw.trim().parse().context("invalid process identity")?;
    ensure!(raw > 1, "invalid process identity");
    let pid = Pid::from_raw(raw);
    if getpgid(Some(pid)) != Ok(pid) {
        return Ok(None);
    }
    let output = Command::new("ps")
        .args(["-ww", "-p", &raw.to_string(), "-o", "command="])
        .output()?;
    let suffix = format!(" _supervise {}", dir.display());
    Ok((output.status.success()
        && String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .ends_with(&suffix))
    .then_some(pid))
}
pub fn reception(dir: &Path) -> Option<Value> {
    if let Ok(v) = read_json(&dir.join("result.json")) {
        return Some(v);
    }
    let Ok(modified) = fs::metadata(dir.join("job.json")).and_then(|s| s.modified()) else {
        return Some(json!({"error":"dispatch intent missing"}));
    };
    if modified.elapsed().unwrap_or_default() < Duration::from_secs(4)
        || identity(dir).ok().flatten().is_some()
    {
        return None;
    }
    let msg = if dir.join("stop").exists() {
        "stopped"
    } else {
        "turn outcome uncertain; inspect the checkout and retry explicitly"
    };
    Some(json!({"error":msg}))
}
pub fn stop_job(dir: &Path) -> Result<()> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(dir.join("stop"))?;
    if dir.join("result.json").try_exists()? {
        return Ok(());
    }
    let Some(pid) = identity(dir)? else {
        return Ok(());
    };
    match killpg(pid, Signal::SIGTERM) {
        Ok(()) | Err(Errno::ESRCH) => Ok(()),
        Err(e) => bail!("cannot stop job: {e}"),
    }
}
