use anyhow::{Context, Result, bail, ensure};
use nix::{
    errno::Errno,
    sys::signal::{Signal, killpg},
    unistd::{Pid, getpgid},
};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Read, Seek, SeekFrom, Write},
    os::unix::{
        fs::{OpenOptionsExt, PermissionsExt},
        process::CommandExt,
    },
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicI32, Ordering},
    },
    thread,
    time::Duration,
};

static ACTIVE_CHILD_PGID: AtomicI32 = AtomicI32::new(0);

pub(crate) fn set_active_child_group(pgid: i32) {
    ACTIVE_CHILD_PGID.store(pgid, Ordering::Release);
}

pub fn log_tail(path: &Path, limit: u64) -> io::Result<String> {
    let mut file = File::open(path)?;
    let start = file.metadata()?.len().saturating_sub(limit);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

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
    // A running daemon must dispatch its own supervisor protocol after an upgrade.
    #[cfg(target_os = "linux")]
    let mut cmd = Command::new("/proc/self/exe");
    #[cfg(not(target_os = "linux"))]
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
    let done = Arc::new((Mutex::new(false), Condvar::new()));
    let done_wait = done.clone();
    let watchdog = thread::spawn(move || {
        if signals.forever().next().is_some() {
            let child = ACTIVE_CHILD_PGID.load(Ordering::Acquire);
            if child > 1 {
                let _ = killpg(Pid::from_raw(child), Signal::SIGTERM);
            }
            if own_group {
                let (lock, cvar) = &*done_wait;
                let guard = lock.lock().unwrap();
                let (guard, _) = cvar
                    .wait_timeout_while(guard, Duration::from_millis(750), |finished| !*finished)
                    .unwrap();
                if !*guard {
                    let child = ACTIVE_CHILD_PGID.load(Ordering::Acquire);
                    if child > 1 {
                        let _ = killpg(Pid::from_raw(child), Signal::SIGKILL);
                    }
                    let _ = killpg(Pid::this(), Signal::SIGKILL);
                }
            }
        }
    });
    let mut pending_claim = tempfile::NamedTempFile::new_in(dir)?;
    pending_claim
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    writeln!(pending_claim, "{}", std::process::id())?;
    pending_claim.as_file().sync_all()?;
    fs2::FileExt::try_lock_exclusive(pending_claim.as_file()).context("claim already held")?;
    let claim = pending_claim
        .persist_noclobber(dir.join("claim"))
        .map_err(|e| e.error)?;
    let result = match execute(dir) {
        Ok(v) => v,
        Err(e) => json!({"error":e.to_string()}),
    };
    let written = write_json(&dir.join("result.json"), &result);
    {
        let (lock, cvar) = &*done;
        *lock.lock().unwrap() = true;
        cvar.notify_all();
    }
    signal_handle.close();
    let _ = watchdog.join();
    drop(claim);
    // A cancelled job can finish while ignored children remain in its group.
    if own_group && dir.join("stop").is_file() {
        let _ = killpg(Pid::this(), Signal::SIGKILL);
    }
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
        crate::turn_log::observe_output(dir, || {
            crate::run::release::supervise(&spec["release"], &mut log)
        })?
    } else if !spec["recipe"].is_null() {
        crate::turn_log::observe_output(dir, || crate::eval::supervise(dir, &spec, &mut log))?;
        json!({})
    } else {
        let chat = (spec["request"]["role"] == "chat")
            .then(|| crate::chat_stream::Writer::create(dir))
            .transpose()?;
        let mut metrics = crate::metrics::Recorder::new(&spec["request"]);
        let logger = Mutex::new((
            BufWriter::new(&mut log),
            None::<io::Error>,
            chat,
            &mut metrics,
        ));
        let mut result = crate::adapter::run(&spec["request"], None, |mut event| {
            if let Ok(mut guard) = logger.lock() {
                let (writer, error, chat, metrics) = &mut *guard;
                if let Err(e) = metrics.event(dir, &event) {
                    eprintln!("record usage: {e}");
                }
                if error.is_none() {
                    *error = (|| -> io::Result<()> {
                        if let Some(chat) = chat {
                            chat.event(&event)?;
                        }
                        if let Some(obj) = event.as_object_mut() {
                            obj.insert(
                                "logged_at".into(),
                                chrono::Utc::now().timestamp_millis().into(),
                            );
                        }
                        serde_json::to_writer(&mut *writer, &event)?;
                        writer.write_all(b"\n")?;
                        writer.flush()
                    })()
                    .err();
                }
            }
        });
        let (mut writer, error, _, metrics) = logger
            .into_inner()
            .map_err(|_| anyhow::anyhow!("event writer failed"))?;
        writer.flush()?;
        if let Some(e) = error {
            return Err(e.into());
        }
        if let Err(e) = metrics.finish(dir, &mut result) {
            eprintln!("finalize usage: {e}");
        }
        result
    };
    log.sync_all()?;
    Ok(result)
}
fn identity(dir: &Path) -> Result<Option<Pid>> {
    let mut file = match File::open(dir.join("claim")) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let active = match fs2::FileExt::try_lock_shared(&file) {
        Ok(()) => {
            let _ = fs2::FileExt::unlock(&file);
            false
        }
        Err(e)
            if e.kind() == io::ErrorKind::WouldBlock
                || e.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
        {
            true
        }
        Err(e) => return Err(e.into()),
    };
    if !active {
        return Ok(None);
    }
    let mut raw = String::new();
    file.read_to_string(&mut raw)?;
    let raw: i32 = raw.trim().parse().context("invalid process identity")?;
    ensure!(raw > 1, "invalid process identity");
    let pid = Pid::from_raw(raw);
    Ok((getpgid(Some(pid)) == Ok(pid)).then_some(pid))
}
pub fn reception(dir: &Path) -> Option<Value> {
    receive(dir, || identity(dir).ok().flatten().is_some())
}

fn receive(dir: &Path, running: impl FnOnce() -> bool) -> Option<Value> {
    if let Ok(v) = read_json(&dir.join("result.json")) {
        return Some(v);
    }
    let Ok(modified) = fs::metadata(dir.join("job.json")).and_then(|s| s.modified()) else {
        return Some(json!({"error":"dispatch intent missing"}));
    };
    if modified.elapsed().unwrap_or_default() < Duration::from_secs(4) || running() {
        return None;
    }
    // The process may have published its receipt and exited during the identity check.
    if let Ok(result) = read_json(&dir.join("result.json")) {
        return Some(result);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receives_a_result_published_during_the_process_check() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let job = File::create(dir.path().join("job.json"))?;
        job.set_modified(std::time::SystemTime::now() - Duration::from_secs(10))?;
        let result = json!({"text":"done"});
        assert_eq!(
            receive(dir.path(), || {
                write_json(&dir.path().join("result.json"), &result).unwrap();
                false
            }),
            Some(result)
        );
        Ok(())
    }

    #[test]
    fn reads_requested_log_tail_even_with_partial_utf8() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("events.log");
        fs::write(&path, "prefix…ok")?;
        assert_eq!(log_tail(&path, 100)?, "prefix…ok");
        assert_eq!(log_tail(&path, 3)?, "�ok");
        assert_eq!(log_tail(&path, 0)?, "");
        assert_eq!(identity(dir.path())?, None);
        fs::write(dir.path().join("claim"), "")?;
        assert_eq!(identity(dir.path())?, None);
        Ok(())
    }
}
