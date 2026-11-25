//! Codex exec publishes its final thread counters on stdout. Between tool calls,
//! recent versions also persist model-call receipts in the active thread rollout.
use super::*;
use chrono::{DateTime, Local, Utc};
use std::fs::File;

pub(super) struct Rollout {
    since: DateTime<Utc>,
    file: Option<File>,
    model: Option<String>,
}

impl Rollout {
    pub fn new() -> Self {
        Self {
            since: Utc::now(),
            file: None,
            model: None,
        }
    }

    fn open(&self, root: &Path, session: &str) -> Option<File> {
        let bytes = session.as_bytes();
        if bytes.len() != 36
            || bytes[14] != b'7'
            || !bytes.iter().enumerate().all(|(i, c)| {
                if matches!(i, 8 | 13 | 18 | 23) {
                    *c == b'-'
                } else {
                    c.is_ascii_hexdigit()
                }
            })
        {
            return None;
        }
        let millis = i64::from_str_radix(&session[..13].replace('-', ""), 16).ok()?;
        let created = DateTime::from_timestamp_millis(millis)?;
        let suffix = format!("-{session}.jsonl");
        let local = created.with_timezone(&Local).format("%Y/%m/%d").to_string();
        let utc = created.format("%Y/%m/%d").to_string();
        for day in [local, utc] {
            let entries = fs::read_dir(root.join("sessions").join(day)).ok();
            if let Some(path) = entries.into_iter().flatten().flatten().find(|e| {
                let name = e.file_name();
                name.to_str()
                    .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(&suffix))
            }) {
                return File::open(path.path()).ok();
            }
        }
        None
    }

    pub fn poll(
        &mut self,
        root: &Path,
        session: &str,
        baseline: Option<&Value>,
        emit: &impl Fn(Value),
    ) -> io::Result<()> {
        if self.file.is_none() {
            self.file = self.open(root, session);
        }
        let Some(file) = &mut self.file else {
            return Ok(());
        };
        // Seek over old history and large tool outputs. Lifetime counters allow
        // the next model receipt to catch up even when an intermediate one is skipped.
        let end = file.metadata()?.len();
        let pos = file.stream_position()?;
        if end <= pos {
            return Ok(());
        }
        let start = pos.max(end.saturating_sub(256 * 1024));
        file.seek(io::SeekFrom::Start(start))?;
        let mut chunk = Vec::new();
        file.take(end - start).read_to_end(&mut chunk)?;
        let complete = chunk.iter().rposition(|&c| c == b'\n').map_or(0, |p| p + 1);
        file.seek(io::SeekFrom::Start(start + complete as u64))?;
        for line in chunk[..complete].split(|&c| c == b'\n') {
            let Ok(line) = std::str::from_utf8(line) else {
                continue;
            };
            if !line.contains("\"token_count\"")
                && !line.contains("\"token_usage_record\"")
                && !line.contains("\"turn_context\"")
            {
                continue;
            }
            let Ok(record) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if record["timestamp"]
                .as_str()
                .and_then(|s| s.parse::<DateTime<Utc>>().ok())
                .is_none_or(|t| t < self.since)
            {
                continue;
            }
            let payload = &record["payload"];
            if record["type"] == "turn_context" {
                self.model = payload["model"].as_str().map(str::to_owned);
            } else {
                let delta =
                    if record["type"] == "token_usage_record" && payload["thread_id"] == session {
                        payload["turn_token_usage"].clone()
                    } else if record["type"] == "event_msg" && payload["type"] == "token_count" {
                        crate::metrics::turn_usage(&payload["info"]["total_token_usage"], baseline)
                    } else {
                        continue;
                    };

                if let Some(mut usage) = delta.as_object().cloned() {
                    usage.insert("type".into(), json!("usage"));
                    usage.insert("model".into(), json!(self.model));
                    emit(usage.into());
                }
            }
        }
        Ok(())
    }
}

/// Observe receipts even while stdout is idle. Drain the watcher before forwarding
/// the final CLI receipt so a delayed partial reading cannot replace final usage.
pub(super) fn stream_with_usage(
    request: &Value,
    input: ChildStdin,
    output: ChildStdout,
    emit: &(impl Fn(Value) + Sync),
) -> Result<Value> {
    use std::sync::mpsc::{RecvTimeoutError, channel};
    let Some(root) = env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|p| PathBuf::from(p).join(".codex")))
    else {
        return super::stream(request, input, output, emit);
    };
    let mut rollout = Rollout::new();
    thread::scope(|scope| {
        let (send, receive) = channel::<Option<String>>();
        let (finished, drained) = channel();
        scope.spawn(move || {
            if let Ok(Some(session)) = receive.recv() {
                loop {
                    let stopping = !matches!(
                        receive.recv_timeout(Duration::from_millis(250)),
                        Err(RecvTimeoutError::Timeout)
                    );
                    let _ = rollout.poll(&root, &session, request.get("codex_baseline"), emit);
                    if stopping {
                        break;
                    }
                }
            }
            let _ = finished.send(());
        });
        let result = super::stream(request, input, output, &|event| {
            match event["type"].as_str() {
                Some("thread.started") => {
                    let _ = send.send(Some(field(&event, "session").into()));
                }
                Some("turn.completed" | "turn.failed") if send.send(None).is_ok() => {
                    let _ = drained.recv();
                }
                _ => (),
            }
            emit(event);
        });
        drop(send);
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn live_rollout_limits_reads_to_the_active_session_and_complete_receipts() -> Result<()> {
        let home = tempfile::tempdir()?;
        let mut tail = Rollout::new();
        let time = Utc::now();
        let hex = format!("{:012x}", time.timestamp_millis());
        let session = format!("{}-{}-7000-8000-000000000000", &hex[..8], &hex[8..]);
        let dir = home
            .path()
            .join("sessions")
            .join(time.format("%Y/%m/%d").to_string());
        fs::create_dir_all(&dir)?;
        let file = dir.join(format!("rollout-fixture-{session}.jsonl"));
        let mut content = File::create(file)?;
        writeln!(
            content,
            "{}",
            json!({"timestamp":time,"type":"turn_context","payload":{"model":"fixture"}})
        )?;
        let receipt = json!({"timestamp":time,"type":"token_usage_record","payload":{"thread_id":session,
            "turn_token_usage":{"input_tokens":400,"output_tokens":20},"thread_token_usage":{"input_tokens":4000,"output_tokens":200}}});
        write!(content, "{receipt}")?;
        let events = RefCell::new(Vec::new());
        let emit = |event| events.borrow_mut().push(event);
        tail.poll(home.path(), &session, None, &emit)?;
        assert!(events.borrow().is_empty());
        writeln!(content)?;
        tail.poll(home.path(), &session, None, &emit)?;
        tail.poll(home.path(), &session, None, &emit)?;
        assert_eq!(
            *events.borrow(),
            vec![json!({"type":"usage","model":"fixture","input_tokens":400,"output_tokens":20})]
        );
        assert!(tail.open(home.path(), "../another-session").is_none());
        Ok(())
    }
}
