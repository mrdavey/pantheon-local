//! Codex exec reports child IDs in collaboration events. Copy public child
//! output from those rollouts while the supervising turn owns the process.
use super::{Recorder, key, label};
use anyhow::Result;
use chrono::{DateTime, Local, Utc};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

pub(super) struct Archive {
    readers: BTreeMap<String, BufReader<File>>,
    results: HashMap<String, String>,
    root: Option<PathBuf>,
    parent: Option<(String, BufReader<File>)>,
    since: DateTime<Utc>,
}

impl Archive {
    pub(super) fn new(request: &Value) -> Self {
        let watches = request["profile"]["kind"] == "codex"
            && request["role"] == "worker"
            && request["subagents"] != false
            && request["subagent_limit"].is_null();
        let root = watches
            .then(|| {
                std::env::var_os("CODEX_HOME")
                    .map(PathBuf::from)
                    .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".codex")))
            })
            .flatten();
        let parent = root
            .as_deref()
            .zip(key(&request["session"]))
            .and_then(|(root, id)| {
                let mut reader = parent_rollout(root, id).ok()??;
                reader.seek(SeekFrom::End(0)).ok()?;
                Some((id.into(), reader))
            });
        Self {
            readers: BTreeMap::new(),
            results: HashMap::new(),
            root,
            parent,
            since: Utc::now(),
        }
    }
}

fn parent_rollout(root: &Path, id: &str) -> Result<Option<BufReader<File>>> {
    let Some(file) = open_rollout(root, id) else {
        return Ok(None);
    };
    let mut reader = BufReader::new(file);
    let valid = read_record(&mut reader)?
        .is_some_and(|meta| meta["type"] == "session_meta" && meta["payload"]["id"] == id);
    Ok(valid.then_some(reader))
}

pub(crate) fn open_rollout(root: &Path, session: &str) -> Option<File> {
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
        if let Some(entry) = entries.into_iter().flatten().flatten().find(|e| {
            e.file_type().is_ok_and(|kind| kind.is_file())
                && e.file_name()
                    .to_str()
                    .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(&suffix))
        }) {
            return File::open(entry.path()).ok();
        }
    }
    None
}

fn state(status: &Value) -> Option<&'static str> {
    Some(match status.as_str()? {
        "pending_init" | "pendingInit" => "pending",
        "running" => "running",
        "completed" => "received",
        "errored" | "failed" => "failed",
        "interrupted" => "interrupted",
        "shutdown" => "stopped",
        "not_found" | "notFound" => "unknown",
        _ => return None,
    })
}

impl Recorder<'_> {
    pub(super) fn codex_event(&mut self, event: &Value) -> Result<()> {
        if event["type"] == "thread.started" {
            self.codex_session = key(&event["session"]).map(str::to_owned);
        }

        let item = &event["raw"]["item"];
        if item["type"] != "collab_tool_call" {
            return Ok(());
        }
        let Some(sender) = key(&item["sender_thread_id"]) else {
            return Ok(());
        };
        let parent = if self.codex_session.as_deref() == Some(sender) {
            None
        } else if self.children.contains_key(sender) {
            Some(sender)
        } else {
            return Ok(());
        };
        if item["tool"] == "spawn_agent" {
            for receiver in item["receiver_thread_ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(key)
            {
                if receiver != sender {
                    self.association(
                        receiver,
                        parent,
                        None,
                        label(&item["prompt"]),
                        Some("running"),
                    )?;
                }
            }
        }
        for (id, info) in item["agents_states"].as_object().into_iter().flatten() {
            if self.children.contains_key(id) {
                self.update(id, None, None, None, state(&info["status"]))?;
                if let Some(result) = info["message"].as_str().filter(|s| !s.is_empty())
                    && self.codex.results.get(id).is_none_or(|old| old != result)
                {
                    self.append(id, &json!({"type":"subagent_result","content":result,"logged_at":event["logged_at"]}), false)?;
                    self.codex.results.insert(id.clone(), result.into());
                }
            }
        }
        Ok(())
    }

    pub fn poll_codex(&mut self, finishing: bool) -> Result<()> {
        let Some(root) = self.codex.root.clone() else {
            return Ok(());
        };
        self.poll_parent(&root, finishing)?;
        let mut visited = std::collections::BTreeSet::new();
        loop {
            let next = self
                .children
                .keys()
                .find(|id| !visited.contains(*id))
                .cloned();
            let Some(id) = next else { break };
            visited.insert(id.clone());
            let mut reader = if let Some(reader) = self.codex.readers.remove(&id) {
                reader
            } else {
                let Some(file) = open_rollout(&root, &id) else {
                    continue;
                };
                let mut reader = BufReader::new(file);
                let Some(meta) = read_record(&mut reader)? else {
                    continue;
                };
                let payload = &meta["payload"];
                let spawn = &payload["source"]["subagent"]["thread_spawn"];
                let parent = self.children[&id]
                    .parent
                    .as_ref()
                    .or(self.codex_session.as_ref());
                // The harness must attest to the owner in the child file as well.
                if meta["type"] != "session_meta"
                    || payload["id"] != id
                    || key(&spawn["parent_thread_id"]) != parent.map(String::as_str)
                    || parent.is_none()
                {
                    continue;
                }
                self.update(
                    &id,
                    None,
                    label(&spawn["agent_nickname"]).or_else(|| label(&spawn["agent_role"])),
                    None,
                    None,
                )?;
                reader
            };
            let start = reader.stream_position()?;
            while let Some(record) = read_record(&mut reader)? {
                self.codex_record(&id, &record)?;
                if !finishing && reader.stream_position()?.saturating_sub(start) >= 256 * 1024 {
                    break;
                }
            }
            self.codex.readers.insert(id, reader);
        }
        Ok(())
    }

    fn poll_parent(&mut self, root: &Path, finishing: bool) -> Result<()> {
        let Some(session) = self.codex_session.clone() else {
            return Ok(());
        };
        let mut reader = match self.codex.parent.take() {
            Some((id, reader)) if id == session => reader,
            _ => {
                let Some(reader) = parent_rollout(root, &session)? else {
                    return Ok(());
                };
                reader
            }
        };
        let start = reader.stream_position()?;
        while let Some(record) = read_record(&mut reader)? {
            if record["type"] == "event_msg"
                && record["payload"]["type"] == "collab_agent_spawn_end"
                && record["timestamp"]
                    .as_str()
                    .and_then(|stamp| stamp.parse::<DateTime<Utc>>().ok())
                    .is_some_and(|stamp| stamp >= self.codex.since)
            {
                self.codex_spawn(&session, None, &record["payload"])?;
            }
            if !finishing && reader.stream_position()?.saturating_sub(start) >= 256 * 1024 {
                break;
            }
        }
        self.codex.parent = Some((session, reader));
        Ok(())
    }

    fn codex_spawn(&mut self, sender: &str, parent: Option<&str>, payload: &Value) -> Result<()> {
        if payload["sender_thread_id"] == sender
            && let Some(child) = key(&payload["new_thread_id"]).filter(|child| *child != sender)
        {
            let state = (!self.children.contains_key(child))
                .then(|| state(&payload["status"]).unwrap_or("running"));
            self.association(
                child,
                parent,
                label(&payload["new_agent_nickname"]),
                label(&payload["prompt"]),
                state,
            )?;
        }
        Ok(())
    }

    fn codex_record(&mut self, id: &str, record: &Value) -> Result<()> {
        let payload = &record["payload"];
        let time = &record["timestamp"];
        if time
            .as_str()
            .and_then(|stamp| stamp.parse::<DateTime<Utc>>().ok())
            .is_some_and(|stamp| stamp < self.codex.since)
        {
            return Ok(());
        }
        let output = match (record["type"].as_str(), payload["type"].as_str()) {
            (Some("event_msg"), Some("collab_agent_spawn_end")) => {
                return self.codex_spawn(id, Some(id), payload);
            }
            (Some("event_msg"), Some("agent_message")) => {
                json!({"type":"agent_message","text":payload["message"],"raw":payload})
            }
            (Some("event_msg"), Some("exec_command_begin" | "exec_command_end")) => {
                json!({"type":"command_execution","raw":payload})
            }
            (Some("event_msg"), Some("exec_command_output_delta")) => {
                json!({"type":"command_output","raw":payload})
            }
            (Some("event_msg"), Some("task_complete" | "turn_complete")) => {
                self.update(id, None, None, None, Some("received"))?;
                json!({"type":"subagent_result","content":payload["last_agent_message"]})
            }
            (Some("event_msg"), Some("turn_aborted" | "error")) => {
                self.update(
                    id,
                    None,
                    None,
                    None,
                    Some(if payload["type"] == "error" {
                        "failed"
                    } else {
                        "interrupted"
                    }),
                )?;
                json!({"type":"error","text":payload["message"],"raw":payload})
            }
            (Some("event_msg"), Some("token_count")) => {
                let mut usage = payload["info"]["total_token_usage"].clone();
                if !usage.is_object() {
                    return Ok(());
                }
                usage["type"] = "usage".into();
                usage
            }
            (Some("token_usage_record"), _) if payload["thread_id"] == id => {
                let mut usage = payload["thread_token_usage"].clone();
                if !usage.is_object() {
                    return Ok(());
                }
                usage["type"] = "usage".into();
                usage
            }
            (Some("response_item"), Some("reasoning")) => {
                let summary = payload["summary"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|entry| entry["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                if summary.is_empty() {
                    return Ok(());
                }
                json!({"type":"reasoning_summary","text":summary})
            }
            _ => return Ok(()),
        };
        let mut output = output;
        output["timestamp"] = time.clone();
        self.append(
            id,
            &output,
            !matches!(output["type"].as_str(), Some("usage" | "subagent_result")),
        )
    }
}

fn read_record(input: &mut BufReader<File>) -> Result<Option<Value>> {
    const LIMIT: u64 = 8 * 1024 * 1024;
    let mut line = Vec::new();
    let start = input.stream_position()?;
    if input.take(LIMIT).read_until(b'\n', &mut line)? == 0 {
        return Ok(None);
    }
    if line.last() != Some(&b'\n') {
        if (line.len() as u64) < LIMIT || input.skip_until(b'\n')? == 0 {
            input.seek(SeekFrom::Start(start))?;
            return Ok(None);
        }
        return Ok(Some(Value::Null));
    }
    Ok(Some(serde_json::from_slice(&line).unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn archives_only_reported_child_output_and_verifies_rollout_ancestry() -> Result<()> {
        let home = tempfile::tempdir()?;
        let job = tempfile::tempdir()?;
        let request = json!({"profile":{"kind":"codex"}});
        let mut capture = Recorder::new(job.path(), &request);
        capture.event(&mut json!({"type":"thread.started","session":"main"}))?;
        capture.codex.root = Some(home.path().into());
        let time = Utc::now();
        let hex = format!("{:012x}", time.timestamp_millis());
        let session = |n| format!("{}-{}-7000-8000-{:012x}", &hex[..8], &hex[8..], n);
        let child = session(1);
        let nested = session(2);
        let foreign = session(3);
        let dir = home
            .path()
            .join("sessions")
            .join(time.format("%Y/%m/%d").to_string());
        fs::create_dir_all(&dir)?;
        let rollout = |id: &str, parent: &str| -> Result<File> {
            let mut file = File::create(dir.join(format!("rollout-fixture-{id}.jsonl")))?;
            writeln!(
                file,
                "{}",
                json!({"type":"session_meta","payload":{"id":id,"source":
                {"subagent":{"thread_spawn":{"parent_thread_id":parent,"agent_nickname":"Reviewer"}}}}})
            )?;
            Ok(file)
        };
        let mut child_file = rollout(&child, "main")?;
        writeln!(
            child_file,
            "{}",
            json!({"type":"event_msg","timestamp":time,"payload":{
            "type":"collab_agent_spawn_end","sender_thread_id":child,"new_thread_id":nested,"status":"running"}})
        )?;
        writeln!(
            child_file,
            "{}",
            json!({"type":"response_item","timestamp":time,"payload":{
            "type":"reasoning","summary":[{"text":"Visible summary"}],"content":[{"text":"Private raw content"}],
            "encrypted_content":"opaque encrypted data"}})
        )?;
        let mut nested_file = rollout(&nested, &child)?;
        write!(
            nested_file,
            "{}",
            json!({"type":"event_msg","timestamp":time,"payload":{
            "type":"agent_message","message":"Nested output"}})
        )?;
        let mut wrong_file = rollout(&foreign, "some-other-run")?;
        writeln!(
            wrong_file,
            "{}",
            json!({"type":"event_msg","payload":{
            "type":"agent_message","message":"Unrelated output"}})
        )?;

        capture.event(&mut json!({"type":"item.completed","raw":{"item":{
            "type":"collab_tool_call","tool":"spawn_agent","sender_thread_id":"main",
            "receiver_thread_ids":[child,foreign]}}}))?;
        capture.poll_codex(false)?;
        let first = super::super::stored(job.path())?;
        assert!(first[&child].transcript);
        assert!(!first[&nested].transcript);
        assert!(!first[&foreign].transcript);
        writeln!(nested_file)?;
        writeln!(
            nested_file,
            "{}",
            json!({"type":"event_msg","timestamp":time,"payload":{
            "type":"token_count","info":{"total_token_usage":{"reasoning_output_tokens":17}}}})
        )?;
        capture.finish()?;

        let dir = super::super::child_dir(job.path(), &child)?;
        let copied = fs::read_to_string(dir.join("events.log"))?;
        assert!(copied.contains("Visible summary"));
        assert!(!copied.contains("Private raw content"));
        assert!(!copied.contains("opaque encrypted data"));
        let nested_dir = super::super::child_dir(job.path(), &nested)?;
        assert_eq!(super::super::reasoning_tokens(&nested_dir), Some(17));
        assert!(fs::read_to_string(nested_dir.join("events.log"))?.contains("Nested output"));
        Ok(())
    }
}
