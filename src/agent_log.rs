//! Run-owned transcripts and metadata for children reported by native adapters.
mod codex;
pub(crate) use codex::open_rollout as open_codex_rollout;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

const INDEX: &str = "agents.json";

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
struct Child {
    parent: Option<String>,
    title: String,
    detail: String,
    state: String,
    transcript: bool,
}

pub struct Recorder<'a> {
    dir: &'a Path,
    kind: &'a str,
    children: BTreeMap<String, Child>,
    claude_tasks: HashMap<String, String>,
    reasoning: HashMap<String, u64>,
    codex_session: Option<String>,
    codex: codex::Archive,
}

fn key(value: &Value) -> Option<&str> {
    value.as_str().filter(|s| !s.is_empty() && s.len() <= 100)
}

fn label(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().chars().take(240).collect())
}

fn log_dir(dir: &Path, id: &str) -> PathBuf {
    dir.join("agents").join(hex::encode(id))
}

fn stored(dir: &Path) -> Result<BTreeMap<String, Child>> {
    match File::open(dir.join(INDEX)) {
        Ok(file) => Ok(serde_json::from_reader(file)?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(e.into()),
    }
}

pub fn roster(run: &Value, root: &Path) -> Result<Value> {
    let mut rows = Vec::new();
    for turn in run["turns"].as_array().into_iter().flatten() {
        let Some(id) = turn["id"].as_str() else {
            continue;
        };
        for (child_id, child) in stored(&root.join("turns").join(id))? {
            let state = if turn["state"] != "running" && child.state == "running" {
                "unknown"
            } else {
                &child.state
            };
            rows.push(
                json!({"id":child_id,"turn":id,"parent":child.parent,"title":child.title,
                "detail":child.detail,"state":state,"transcript":child.transcript}),
            );
        }
    }
    Ok(rows.into())
}

pub fn child_dir(dir: &Path, id: &str) -> Result<PathBuf> {
    ensure!(stored(dir)?.contains_key(id), "unknown subagent");
    Ok(log_dir(dir, id))
}

pub fn reasoning_tokens(dir: &Path) -> Option<u64> {
    serde_json::from_reader(File::open(dir.join("reasoning.json")).ok()?).ok()
}

fn reported_reasoning(event: &Value) -> Option<u64> {
    let raw = &event["raw"];
    let usage = [
        event.get("turn_usage").unwrap_or(&raw["usage"]),
        event,
        &raw["update"]["usage"],
    ];
    usage
        .into_iter()
        .find_map(|usage| {
            [
                "reasoning_output_tokens",
                "reasoning_tokens",
                "thinking_tokens",
                "reasoningOutputTokens",
                "reasoningTokens",
                "thinkingTokens",
            ]
            .iter()
            .find_map(|key| usage[*key].as_u64())
        })
        .or_else(|| {
            raw["modelUsage"]
                .as_object()
                .filter(|models| !models.is_empty())?
                .values()
                .try_fold(0u64, |sum, model| {
                    sum.checked_add(model["thinkingTokens"].as_u64()?)
                })
        })
}

impl<'a> Recorder<'a> {
    pub fn new(dir: &'a Path, request: &'a Value) -> Self {
        Self {
            dir,
            kind: request["profile"]["kind"].as_str().unwrap_or_default(),
            children: BTreeMap::new(),
            claude_tasks: HashMap::new(),
            reasoning: HashMap::new(),
            codex_session: None,
            codex: codex::Archive::new(request),
        }
    }

    fn save(&self) -> Result<()> {
        crate::supervisor::write_json(&self.dir.join(INDEX), &json!(self.children))
    }

    fn update(
        &mut self,
        id: &str,
        parent: Option<&str>,
        title: Option<String>,
        detail: Option<String>,
        state: Option<&str>,
    ) -> Result<()> {
        let child = self.children.entry(id.into()).or_insert_with(|| Child {
            parent: parent.map(str::to_owned),
            state: "unknown".into(),
            ..Child::default()
        });
        let previous = child.clone();
        if let Some(title) = title {
            child.title = title;
        }
        if let Some(detail) = detail {
            child.detail = detail;
        }
        if let Some(state) = state {
            child.state = state.into();
        }
        if *child != previous || previous.title.is_empty() {
            if child.title.is_empty() {
                child.title = "Subagent".into();
            }
            self.save()?;
        }
        Ok(())
    }

    fn record_usage(&mut self, id: &str, dir: &Path, event: &Value) -> Result<()> {
        if let Some(value) = reported_reasoning(event) {
            self.write_usage(id, dir, value)?;
        }
        Ok(())
    }

    fn write_usage(&mut self, id: &str, dir: &Path, value: u64) -> Result<()> {
        if self.reasoning.get(id) != Some(&value) {
            crate::supervisor::write_json(&dir.join("reasoning.json"), &json!(value))?;
            self.reasoning.insert(id.into(), value);
        }
        Ok(())
    }

    fn append(&mut self, id: &str, event: &Value, transcript: bool) -> Result<()> {
        let dir = log_dir(self.dir, id);
        fs::create_dir_all(&dir)?;
        let mut line = serde_json::to_vec(event)?;
        line.push(b'\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(dir.join("events.log"))?;
        file.write_all(&line)?;
        self.record_usage(id, &dir, event)?;
        if transcript
            && let Some(child) = self.children.get_mut(id)
            && !child.transcript
        {
            child.transcript = true;
            self.save()?;
        }
        Ok(())
    }

    fn association(
        &mut self,
        id: &str,
        parent: Option<&str>,
        title: Option<String>,
        detail: Option<String>,
        state: Option<&str>,
    ) -> Result<()> {
        if parent == Some(id) || parent.is_some_and(|p| !self.children.contains_key(p)) {
            return Ok(());
        }
        self.update(id, parent, title, detail, state)
    }

    // The main transcript keeps its original events. Agent output can omit records
    // assigned to a child while the raw view retains the full adapter stream.
    pub fn event(&mut self, event: &mut Value) -> Result<bool> {
        let child = match self.kind {
            "claude" => self.claude(event)?,
            "hermes" | "antigravity" => self.acp(event)?,
            "codex" => {
                self.codex_event(event)?;
                None
            }
            "custom" => self.custom(event)?,
            _ => None,
        };
        if let Some(id) = child {
            self.append(&id, event, true)?;
            event["log_agent"] = id.into();
            Ok(true)
        } else {
            self.record_usage("", self.dir, event)?;
            Ok(false)
        }
    }

    pub fn finish(&mut self) -> Result<()> {
        self.poll_codex(true)?;
        for id in self.children.keys() {
            match File::open(log_dir(self.dir, id).join("events.log")) {
                Ok(file) => file.sync_all()?,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    fn claude(&mut self, event: &Value) -> Result<Option<String>> {
        let raw = &event["raw"];
        let parent = key(&raw["parent_tool_use_id"]);
        if let Some(id) = parent
            && !self.children.contains_key(id)
        {
            self.update(id, None, None, None, Some("running"))?;
        }
        for block in raw["message"]["content"].as_array().into_iter().flatten() {
            if block["type"] == "tool_use"
                && matches!(block["name"].as_str(), Some("Agent" | "Task"))
                && let Some(id) = key(&block["id"])
            {
                let input = &block["input"];
                self.association(
                    id,
                    parent,
                    label(&input["description"]).or_else(|| label(&input["subagent_type"])),
                    label(&input["prompt"]),
                    Some("running"),
                )?;
            }
            if block["type"] == "tool_result"
                && let Some(id) = key(&block["tool_use_id"])
                && self.children.contains_key(id)
            {
                let background = block["content"].as_array().is_some_and(|parts| {
                    parts.iter().any(|p| {
                        p["text"]
                            .as_str()
                            .is_some_and(|s| s.contains("Async agent launched"))
                    })
                });
                if !background {
                    self.update(
                        id,
                        None,
                        None,
                        None,
                        Some(if block["is_error"] == true {
                            "failed"
                        } else {
                            "received"
                        }),
                    )?;
                }
                self.append(id, &json!({"type":"subagent_result","content":block["content"],"logged_at":event["logged_at"]}), false)?;
            }
        }
        if event["type"] == "system" {
            let task = key(&raw["task_id"]);
            let id = key(&raw["tool_use_id"])
                .or_else(|| task.and_then(|t| self.claude_tasks.get(t).map(String::as_str)))
                .map(str::to_owned);
            if let Some(id) = id
                && matches!(
                    raw["subtype"].as_str(),
                    Some("task_started" | "task_progress" | "task_notification")
                )
            {
                if let Some(task) = task {
                    self.claude_tasks.insert(task.into(), id.clone());
                }
                let state = match raw["status"].as_str() {
                    Some("completed") => "received",
                    Some("failed") => "failed",
                    Some("stopped") => "cancelled",
                    _ => "running",
                };
                self.association(&id, parent, label(&raw["description"]), None, Some(state))?;
                if raw["subtype"] == "task_notification" && self.children.contains_key(&id) {
                    self.append(&id, &json!({"type":"subagent_result","content":raw["summary"],"logged_at":event["logged_at"]}), false)?;
                }
            }
        }
        Ok(parent.map(str::to_owned))
    }

    fn acp(&mut self, event: &Value) -> Result<Option<String>> {
        let raw = &event["raw"];
        let update = &raw["update"];
        let session = key(&raw["sessionId"]);
        let child = session.filter(|id| self.children.contains_key(*id));
        if event["type"] == "subagent_update"
            && let Some(id) = key(&update["sessionId"])
            && (session == key(&event["session"]) || child.is_some())
            && session != Some(id)
        {
            let state = match update["state"]["state"].as_str() {
                Some("running") => Some("running"),
                Some("requires_action") => Some("question"),
                Some("idle") => Some(match update["state"]["stopReason"].as_str() {
                    Some("end_turn") => "received",
                    Some("cancelled") => "cancelled",
                    Some("refusal" | "max_tokens" | "max_turn_requests") => "failed",
                    _ => "idle",
                }),
                Some("unknown") => Some("unknown"),
                _ => None,
            };
            self.association(
                id,
                child,
                label(&update["title"]),
                label(&update["description"]),
                state,
            )?;
        }
        Ok(child.map(str::to_owned))
    }

    fn custom(&mut self, event: &Value) -> Result<Option<String>> {
        let child = key(&event["agent"]);
        if event["type"] == "subagent"
            && let Some(id) = child
        {
            let parent = key(&event["parent_agent"]);
            let state = event["state"].as_str().filter(|s| {
                matches!(
                    *s,
                    "pending"
                        | "running"
                        | "question"
                        | "received"
                        | "failed"
                        | "cancelled"
                        | "unknown"
                )
            });
            self.association(
                id,
                parent,
                label(&event["title"]),
                label(&event["detail"]),
                state,
            )?;
            if self.children.contains_key(id) && !event["text"].is_null() {
                self.append(id, &json!({"type":"subagent_result","content":event["text"],"logged_at":event["logged_at"]}), false)?;
            }
            return Ok(None);
        }
        Ok(child
            .filter(|id| self.children.contains_key(*id))
            .map(str::to_owned))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_streams_children_but_keeps_result_only_children_visible() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let request = json!({"profile":{"kind":"claude"}});
        let mut recorder = Recorder::new(dir.path(), &request);
        let tool = |id, description| json!({"type":"tool_use","name":"Agent","id":id,"input":{"description":description}});
        recorder.event(&mut json!({"type":"assistant","raw":{"message":{"content":[tool("first","First review"),tool("second","Second review")]}}}))?;
        let mut output = json!({"type":"assistant","raw":{"parent_tool_use_id":"first","message":{"content":[
            {"type":"text","text":"First child output"}, tool("nested","Nested review")
        ]}}});
        assert!(recorder.event(&mut output)?);
        assert_eq!(output["log_agent"], "first");
        recorder.event(&mut json!({"type":"user","raw":{"message":{"content":[
            {"type":"tool_result","tool_use_id":"second","is_error":true,"content":"Second child failed"}
        ]}}}))?;
        recorder.event(&mut json!({"type":"result","raw":{"modelUsage":{
            "one":{"thinkingTokens":0},"two":{"thinkingTokens":21}
        }}}))?;
        recorder.finish()?;
        let children = stored(dir.path())?;
        assert_eq!(children["nested"].parent.as_deref(), Some("first"));
        assert!(children["first"].transcript);
        assert!(!children["second"].transcript);
        assert_eq!(children["second"].state, "failed");
        assert_eq!(reasoning_tokens(dir.path()), Some(21));
        let child_dir = child_dir(dir.path(), "second")?;
        assert!(fs::read_to_string(child_dir.join("events.log"))?.contains("Second child failed"));
        Ok(())
    }

    #[test]
    fn acp_attributes_output_and_nested_children_to_announced_sessions() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let request = json!({"profile":{"kind":"hermes"}});
        let mut recorder = Recorder::new(dir.path(), &request);
        let announcement = |parent, child| {
            json!({"type":"subagent_update","session":"main",
            "raw":{"sessionId":parent,"update":{"sessionId":child,"title":child,"state":{"state":"running"}}}})
        };
        recorder.event(&mut announcement("unrelated", "lost"))?;
        recorder.event(&mut announcement("main", "first"))?;
        recorder.event(&mut announcement("first", "nested"))?;
        let mut output = json!({"type":"agent_thought_chunk","session":"main","text":"Visible reasoning",
            "raw":{"sessionId":"nested","update":{"content":{"text":"Visible reasoning"}}}});
        assert!(recorder.event(&mut output)?);
        assert_eq!(output["log_agent"], "nested");
        recorder.event(
            &mut json!({"type":"usage_update","session":"main","raw":{"sessionId":"nested",
            "update":{"usage":{"reasoningTokens":0}}}}),
        )?;
        recorder.finish()?;
        let children = stored(dir.path())?;
        assert_eq!(children.len(), 2);
        assert_eq!(children["nested"].parent.as_deref(), Some("first"));
        assert!(children["nested"].transcript);
        assert_eq!(reasoning_tokens(&child_dir(dir.path(), "nested")?), Some(0));
        assert_eq!(reasoning_tokens(dir.path()), None);
        Ok(())
    }
}
