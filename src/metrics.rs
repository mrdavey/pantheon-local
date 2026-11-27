//! Cumulative, turn-local usage receipts. Keep telemetry separate from run revisions.
mod antigravity;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    fs, io,
    path::Path,
};

#[derive(Clone, Default, Debug, Serialize, Deserialize, PartialEq)]
pub struct Reading {
    pub tokens: Option<u64>,
    pub usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub breakdown: Option<Breakdown>,
}

/// Disjoint billing categories. Input excludes cached reads and cache writes.
#[derive(Clone, Default, Debug, Serialize, Deserialize, PartialEq)]
pub struct Breakdown {
    pub input: u64,
    pub cached: u64,
    pub output: u64,
    pub cache_write: u64,
}

impl Breakdown {
    fn native(usage: &Value) -> Option<Self> {
        let cached = usage["cached_input_tokens"].as_u64().unwrap_or(0);
        let cache_write = usage["cache_write_input_tokens"].as_u64().unwrap_or(0);
        Some(Self {
            input: usage["input_tokens"]
                .as_u64()?
                .checked_sub(cached)?
                .checked_sub(cache_write)?,
            cached,
            cache_write,
            output: usage["output_tokens"].as_u64()?,
        })
    }

    fn claude(m: &Value) -> Option<Self> {
        Some(Self {
            input: m["inputTokens"].as_u64()?,
            output: m["outputTokens"].as_u64()?,
            cached: m["cacheReadInputTokens"].as_u64().unwrap_or(0),
            cache_write: m["cacheCreationInputTokens"].as_u64().unwrap_or(0),
        })
    }
}

fn usd(value: &Value) -> Option<f64> {
    value.as_f64().filter(|n| n.is_finite() && *n >= 0.0)
}

fn count(v: &Value, keys: &[&str]) -> Option<u64> {
    let values = keys
        .iter()
        .filter_map(|key| v[key].as_u64())
        .collect::<Vec<_>>();
    if values.is_empty() {
        None
    } else {
        values.into_iter().try_fold(0u64, u64::checked_add)
    }
}

pub fn model(profile: &Value) -> &str {
    [
        &profile["model"],
        &profile["detected"]["model"],
        &profile["detected"]["label"],
    ]
    .into_iter()
    .filter_map(Value::as_str)
    .find(|s| !s.is_empty())
    .unwrap_or("Unknown model")
}

// Standard short-context text API rates per million tokens on 2026-11-25.
// https://developers.openai.com/api/docs/pricing
fn codex_cost(model: &str, input: u64, cached: u64, output: u64) -> Option<f64> {
    let (i, c, o) = match model {
        "gpt-6.2-astra" => (8., 0.4, 40.),
        "gpt-6.2-sol" | "gpt-6.1-sol" => (2., 0.1, 10.),
        "gpt-6.1-terra" => (1., 0.05, 5.),
        "gpt-6-luna" => (0.1, 0.01, 0.5),
        "gpt-6-sol" => (2., 0.2, 10.),
        "gpt-5.4" => (2.5, 0.25, 15.),
        "gpt-5.4-mini" => (0.75, 0.075, 4.5),
        "gpt-6-astra" => (10., 1., 50.),
        _ => return None,
    };
    Some(
        (input.checked_sub(cached)? as f64 * i + cached as f64 * c + output as f64 * o)
            / 1_000_000.,
    )
}

#[derive(Serialize, Deserialize, Clone)]
struct Sample {
    at: i64,
    models: BTreeMap<String, Reading>,
}

pub struct Recorder {
    kind: String,
    model: String,
    history: Vec<Sample>,
    seen_messages: HashSet<String>,
    claude_inputs: BTreeMap<String, Reading>,
}
impl Recorder {
    pub fn new(request: &Value) -> Self {
        Self {
            kind: request["profile"]["kind"]
                .as_str()
                .unwrap_or_default()
                .into(),
            model: model(&request["profile"]).into(),
            history: vec![],
            seen_messages: HashSet::new(),
            claude_inputs: BTreeMap::new(),
        }
    }
    fn single(
        &self,
        name: Option<&str>,
        tokens: Option<u64>,
        usd: Option<f64>,
    ) -> BTreeMap<String, Reading> {
        BTreeMap::from([(
            name.filter(|s| !s.is_empty())
                .unwrap_or(&self.model)
                .to_owned(),
            Reading {
                tokens,
                usd,
                breakdown: None,
            },
        )])
    }
    fn save(&mut self, dir: &Path, models: BTreeMap<String, Reading>) -> Result<()> {
        if !models
            .values()
            .any(|r| r.tokens.is_some() || r.usd.is_some())
            || self.history.last().is_some_and(|s| s.models == models)
        {
            return Ok(());
        }
        let at = Utc::now().timestamp_millis();
        // Coalesce bursts and bound storage while retaining the final reading of each interval.
        if self
            .history
            .last()
            .is_some_and(|s| s.at / 1000 == at / 1000)
        {
            self.history.pop();
        }
        if self.history.len() >= 512 {
            self.history = self.history.drain(..).skip(1).step_by(2).collect();
        }
        self.history.push(Sample { at, models });
        crate::supervisor::write_json(&dir.join("usage.json"), &json!(self.history))
    }

    fn readings(&mut self, event: &Value) -> Option<BTreeMap<String, Reading>> {
        let ty = event["type"].as_str().unwrap_or("");
        let raw = &event["raw"];
        let usage = &raw["usage"];
        let models = match (self.kind.as_str(), ty) {
            ("codex", "turn.completed") | ("codex", "usage") => {
                if let Some(name) = event["model"].as_str() {
                    self.model = name.into();
                }
                let u = if ty == "usage" {
                    event
                } else {
                    event.get("turn_usage").unwrap_or(usage)
                };
                let tokens = u["tokens"]
                    .as_u64()
                    .or_else(|| count(u, &["input_tokens", "output_tokens"]));
                let cost = u["input_tokens"]
                    .as_u64()
                    .zip(u["output_tokens"].as_u64())
                    .and_then(|(i, o)| {
                        codex_cost(
                            &self.model,
                            i,
                            u["cached_input_tokens"].as_u64().unwrap_or(0),
                            o,
                        )
                    });
                let mut rows = self.single(None, tokens, cost);
                rows.get_mut(&self.model)?.breakdown = Breakdown::native(u);
                rows
            }
            ("claude", "system") if raw["subtype"] == "init" => {
                if let Some(name) = raw["model"].as_str() {
                    self.model = name.into();
                }
                return None;
            }
            ("claude", "assistant") => {
                let m = &raw["message"];
                let (Some(id), Some(name), Some(n)) = (
                    m["id"].as_str(),
                    m["model"].as_str(),
                    count(
                        &m["usage"],
                        &[
                            "input_tokens",
                            "cache_read_input_tokens",
                            "cache_creation_input_tokens",
                        ],
                    ),
                ) else {
                    return None;
                };
                if !self.seen_messages.insert(id.into()) {
                    return None;
                }
                let r = self.claude_inputs.entry(name.into()).or_default();
                r.tokens = Some(r.tokens.unwrap_or(0).saturating_add(n));
                self.claude_inputs.clone()
            }
            ("claude", "result") => {
                let mut rows = BTreeMap::new();
                if let Some(per_model) = raw["modelUsage"].as_object() {
                    for (name, m) in per_model {
                        rows.insert(
                            name.clone(),
                            Reading {
                                tokens: count(
                                    m,
                                    &[
                                        "inputTokens",
                                        "outputTokens",
                                        "cacheReadInputTokens",
                                        "cacheCreationInputTokens",
                                    ],
                                ),
                                usd: usd(&m["costUSD"]),
                                breakdown: Breakdown::claude(m),
                            },
                        );
                    }
                }
                if rows.is_empty() {
                    self.single(
                        None,
                        count(
                            usage,
                            &[
                                "input_tokens",
                                "output_tokens",
                                "cache_read_input_tokens",
                                "cache_creation_input_tokens",
                            ],
                        ),
                        usd(&raw["total_cost_usd"]),
                    )
                } else {
                    rows
                }
            }
            ("custom" | "hermes" | "antigravity", "usage") => {
                if let Some(name) = event["model"].as_str().filter(|s| !s.is_empty()) {
                    self.model = name.into();
                }
                let prior = self.history.last().and_then(|s| s.models.get(&self.model));
                let mut rows = self.single(
                    None,
                    event["tokens"]
                        .as_u64()
                        .or_else(|| prior.and_then(|r| r.tokens)),
                    usd(&event["cost_usd"]).or_else(|| prior.and_then(|r| r.usd)),
                );
                rows.get_mut(&self.model)?.breakdown = Breakdown::native(event).or_else(|| {
                    prior
                        .filter(|r| r.tokens == rows[&self.model].tokens)
                        .and_then(|r| r.breakdown.clone())
                });
                rows
            }
            _ => return None,
        };
        Some(models)
    }

    pub fn event(&mut self, dir: &Path, event: &Value) -> Result<()> {
        if let Some(models) = self.readings(event) {
            self.save(dir, models)?;
        }
        Ok(())
    }

    pub fn finish(&mut self, dir: &Path, result: &mut Value) -> Result<()> {
        // A total-only final receipt can complete a partial live sample; do not add it twice.
        let last = self.history.last().map(|s| &s.models);
        if last.is_none() || self.kind == "custom" {
            let prior = last.and_then(|m| m.first_key_value());
            let name = result["model"]
                .as_str()
                .or_else(|| prior.map(|(name, _)| name.as_str()));
            let tokens = result["tokens"]
                .as_u64()
                .or_else(|| prior.and_then(|(_, r)| r.tokens));
            let cost = usd(&result["cost_usd"]).or_else(|| prior.and_then(|(_, r)| r.usd));
            let mut rows = self.single(name, tokens, cost);
            if let Some((model, reading)) = prior
                && reading.tokens == tokens
                && let Some(row) = rows.get_mut(model)
            {
                row.breakdown.clone_from(&reading.breakdown);
            }
            self.save(dir, rows)?;
        }
        if let Some(sample) = self.history.last()
            && let Some(total) = sample
                .models
                .values()
                .try_fold(0u64, |acc, r| acc.checked_add(r.tokens?))
        {
            result["tokens"] = total.into();
        }
        Ok(())
    }
}

fn optional(path: &Path) -> Result<Option<fs::File>> {
    match fs::File::open(path) {
        Ok(f) => Ok(Some(f)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn codex_receipt(dir: &Path) -> Option<Value> {
    let tail = crate::supervisor::log_tail(&dir.join("events.log"), 128 * 1024).ok()?;
    tail.lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|e| e["type"] == "turn.completed" && e["raw"]["usage"].is_object())
}

// Codex exec returns the thread's lifetime counters, including on resume.
pub fn prior_usage(turns: &[Value], root: &Path, session: &str) -> Option<Value> {
    for t in turns
        .iter()
        .rev()
        .filter(|t| !matches!(t["role"].as_str(), Some("chat" | "eval" | "release")))
    {
        if let Some(id) = t["usage_session"].as_str() {
            if id == session {
                return Some(t["codex_usage"].clone());
            }
            continue;
        }
        let dir = root.join("turns").join(t["id"].as_str()?);
        let Some(receipt) = fs::File::open(dir.join("result.json"))
            .ok()
            .and_then(|f| serde_json::from_reader::<_, Value>(f).ok())
        else {
            continue;
        };
        if receipt["session"] == session {
            return receipt
                .get("codex_usage")
                .cloned()
                .or_else(|| codex_receipt(&dir).map(|e| e["raw"]["usage"].clone()));
        }
    }
    None
}

pub fn turn_usage(usage: &Value, baseline: Option<&Value>) -> Value {
    let Some(prior) = baseline else {
        return usage.clone();
    };
    if !prior.is_object() {
        return Value::Null;
    }
    let mut delta = json!({});
    for key in [
        "input_tokens",
        "output_tokens",
        "cached_input_tokens",
        "cache_write_input_tokens",
    ] {
        if let Some(current) = usage[key].as_u64() {
            let Some(value) = current.checked_sub(prior[key].as_u64().unwrap_or(0)) else {
                return Value::Null;
            };
            delta[key] = value.into();
        }
    }
    if let Some(reasoning) = usage["reasoning_output_tokens"]
        .as_u64()
        .zip(prior["reasoning_output_tokens"].as_u64())
        .and_then(|(now, before)| now.checked_sub(before))
    {
        delta["reasoning_output_tokens"] = reasoning.into();
    }
    delta
}

fn legacy_sample(
    dir: &Path,
    profile: &Value,
    file: fs::File,
    turns: &[Value],
    root: &Path,
) -> Result<Vec<Sample>> {
    let at: chrono::DateTime<Utc> = file.metadata()?.modified()?.into();
    let result: Value = serde_json::from_reader(file)?;
    let mut recorder = Recorder::new(&json!({"profile":profile}));
    let mut rows = recorder.single(
        result["model"].as_str(),
        result["tokens"].as_u64(),
        usd(&result["cost_usd"]),
    );
    if recorder.kind == "codex"
        && let Some(mut event) = codex_receipt(dir)
    {
        let session = result["session"].as_str().unwrap_or_default();
        let raw = event["raw"]["usage"].clone();
        let prior = (!session.is_empty())
            .then(|| prior_usage(turns, root, session))
            .flatten();
        event["turn_usage"] = turn_usage(&raw, prior.as_ref());
        if let Some(native) = recorder.readings(&event) {
            rows = native;
        }
    } else if recorder.kind == "claude"
        && let Ok(tail) = crate::supervisor::log_tail(&dir.join("events.log"), 128 * 1024)
    {
        for line in tail.lines().rev() {
            let Ok(event) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if event["type"] == "result"
                && let Some(native) = recorder.readings(&event)
                && native.values().any(|r| r.tokens.is_some())
            {
                rows = native;
                break;
            }
        }
    }
    Ok(if rows.values().all(|r| *r == Reading::default()) {
        vec![]
    } else {
        vec![Sample {
            at: at.timestamp_millis(),
            models: rows,
        }]
    })
}

fn modified_millis(dir: &Path, file: &str) -> Option<i64> {
    let modified = fs::metadata(dir.join(file)).ok()?.modified().ok()?;
    Some(chrono::DateTime::<Utc>::from(modified).timestamp_millis())
}

/// Older runs stored completion only on the worker turn or its receipt file.
pub fn milestone_completions(run: &Value, root: &Path) -> Value {
    let workers: BTreeMap<_, _> = run["turns"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|turn| turn["role"] == "worker")
        .filter_map(|turn| Some((turn["task"].as_str()?, turn)))
        .collect();
    let times: BTreeMap<_, _> = run["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|task| task["state"] == "committed")
        .filter_map(|task| {
            let id = task["id"].as_str()?;
            let at = task["completed_at"].as_i64().or_else(|| {
                let turn = workers.get(id)?;
                turn["finished"].as_i64().or_else(|| {
                    modified_millis(
                        &root.join("turns").join(turn["id"].as_str()?),
                        "result.json",
                    )
                })
            })?;
            Some((id, at))
        })
        .collect();
    json!(times)
}

/// Elapsed wall time begins at dispatch, including waits between attempts.
/// Read only the first and last receipts when an older run lacks timestamps.
pub fn milestone_timing(run: &Value, root: &Path, id: &str) -> Result<Value> {
    let task = run["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["id"] == id)
        .context("milestone not found")?;
    let mut turns = run["turns"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| t["role"] == "worker" && t["task"] == id);
    let Some(first) = turns.next() else {
        return Ok(json!({"started":null,"ended":null,"running":false}));
    };
    let last = turns.next_back().unwrap_or(first);
    let dir = |turn: &Value| {
        root.join("turns")
            .join(turn["id"].as_str().unwrap_or_default())
    };
    let started = first["started"]
        .as_i64()
        .or_else(|| modified_millis(&dir(first), "job.json"));
    let running =
        task["state"] == "running" && last["state"] == "running" && run["state"] != "stopped";
    let ended = if running {
        None
    } else {
        task["completed_at"]
            .as_i64()
            .filter(|_| task["state"] == "committed")
            .or_else(|| last["finished"].as_i64())
            .or_else(|| modified_millis(&dir(last), "result.json"))
            .filter(|end| started.is_some_and(|start| *end >= start))
    };
    Ok(json!({"started":started,"ended":ended,"running":running}))
}

pub fn milestone_usage(run: &Value, root: &Path, task: &str) -> Result<Value> {
    anyhow::ensure!(
        run["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|t| t["id"] == task),
        "milestone not found"
    );
    let report = filtered_report(run, root, 0, Some(task))?;
    let turns = report["turns"].as_array().context("missing usage report")?;
    let mut tokens: Option<u64> = None;
    let mut cost = None;
    let mut partial_tokens = false;
    let mut partial_cost = false;
    for turn in turns {
        let models = turn["samples"]
            .as_array()
            .and_then(|v| v.last())
            .and_then(|v| v["models"].as_object());
        if let Some(models) = models {
            for receipt in models.values() {
                if let Some(n) = receipt["tokens"].as_u64() {
                    tokens = Some(tokens.unwrap_or(0).saturating_add(n))
                } else {
                    partial_tokens = true;
                }
                if let Some(n) = receipt["usd"].as_f64() {
                    *cost.get_or_insert(0.) += n
                } else {
                    partial_cost = true;
                }
            }
        } else {
            partial_tokens = true;
            partial_cost = true;
        }
    }
    Ok(
        json!({"tokens":tokens,"usd":cost,"partial_tokens":partial_tokens,"partial_cost":partial_cost,"turns":turns.len()}),
    )
}
pub fn report(run: &Value, root: &Path, from: usize) -> Result<Value> {
    filtered_report(run, root, from, None)
}
fn filtered_report(run: &Value, root: &Path, from: usize, task: Option<&str>) -> Result<Value> {
    let turns = run["turns"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut rows = Vec::new();
    let mut next = turns.len();
    for (index, t) in turns.iter().enumerate().skip(from) {
        if task.is_some_and(|id| t["task"] != id || t["role"] != "worker")
            || matches!(t["role"].as_str(), Some("eval" | "release"))
        {
            continue;
        }
        if t["state"] == "running" {
            next = next.min(index);
        }
        let dir = root.join("turns").join(t["id"].as_str().unwrap_or(""));
        let agent = t["agent"]
            .as_str()
            .or_else(|| {
                if t["role"] == "worker" {
                    run["tasks"]
                        .as_array()?
                        .iter()
                        .find(|task| task["id"] == t["task"])?["worker"]
                        .as_str()
                } else {
                    run["input"]["coordinator"].as_str()
                }
            })
            .unwrap_or("Unknown agent");
        let profile = &run["profiles"][agent];
        let name = model(profile);
        let mut history: Vec<Sample> = if let Some(f) = optional(&dir.join("usage.json"))? {
            serde_json::from_reader(f)?
        } else {
            vec![]
        };
        // Older usage files retained totals but dropped the billing breakdown.
        // Recover a matching final reading from the original native receipt.
        let needs_native = history.last().is_some_and(|sample| {
            matches!(profile["kind"].as_str(), Some("codex" | "claude"))
                && sample
                    .models
                    .values()
                    .any(|r| r.usd.is_none() && r.breakdown.is_none())
        });
        if (history.is_empty() || needs_native)
            && let Some(f) = optional(&dir.join("result.json"))?
        {
            let native = legacy_sample(&dir, profile, f, &turns[..index], root)?;
            if history.is_empty() {
                history = native;
            } else if let (Some(last), Some(receipt)) = (history.last_mut(), native.last()) {
                for (model, reading) in &mut last.models {
                    if let Some(raw) = receipt.models.get(model)
                        && raw.tokens == reading.tokens
                    {
                        reading.usd = reading.usd.or(raw.usd);
                        if reading.breakdown.is_none() {
                            reading.breakdown.clone_from(&raw.breakdown);
                        }
                    }
                }
            }
        }
        if profile["kind"] == "antigravity" {
            history = antigravity::history(&dir, t, name, history);
        }
        for sample in &mut history {
            for (name, reading) in &mut sample.models {
                if reading.usd.is_none() {
                    reading.usd = crate::pricing::estimate(&run["model_rates"][name], reading);
                }
            }
        }
        let started = t["started"]
            .as_i64()
            .or_else(|| modified_millis(&dir, "job.json"));
        rows.push(json!({"id":t["id"],"agent":agent,"role":t["role"],"model":name,"state":t["state"],"started":started,"samples":history,"harness":profile["kind"]}));
    }
    Ok(json!({"turns":rows,"next":next}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn report_recovers_antigravity_turns_with_missing_or_incorrect_totals() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let run = json!({"input":{"coordinator":"agy"},"profiles":{"agy":{"kind":"antigravity","model":"gemini-fixture"}},
            "turns":[{"id":"empty","role":"chat","state":"received","started":2000,"finished":5000},
                {"id":"undercount","role":"chat","state":"received","started":6000,"finished":8000,
                "acp_usage":{"totalTokens":170}}]});
        let turns = tmp.path().join("turns");
        fs::create_dir_all(turns.join("empty"))?;
        fs::create_dir_all(turns.join("undercount"))?;
        crate::supervisor::write_json(
            &turns.join("empty/result.json"),
            &json!({"acp_usage":{"totalTokens":223}}),
        )?;
        crate::supervisor::write_json(
            &turns.join("undercount/usage.json"),
            &json!([{"at":7000,"models":{"gemini-fixture":{"tokens":22,"usd":null}}}]),
        )?;
        let metrics = report(&run, tmp.path(), 0)?;
        for (i, expected) in [223, 170].into_iter().enumerate() {
            assert_eq!(
                metrics["turns"][i]["samples"][0]["models"]["gemini-fixture"]["tokens"],
                expected
            );
        }
        Ok(())
    }
    #[test]
    fn milestone_clock_spans_attempts_and_stops_on_completion() -> Result<()> {
        let private = tempfile::tempdir()?;
        let root = private.path();
        let mut run = json!({"state":"working","tasks":[{"id":"build","state":"running"}],"turns":[
            {"id":"one","task":"build","role":"worker","started":1200,"finished":3000,"state":"received"},
            {"id":"two","task":"build","role":"worker","started":4000,"state":"running"},
            {"id":"unrelated","task":"other","role":"worker","started":9999,"state":"running"}]});
        assert_eq!(
            milestone_timing(&run, root, "build")?,
            json!({"started":1200,"ended":null,"running":true})
        );
        run["tasks"][0]["state"] = json!("committed");
        run["tasks"][0]["completed_at"] = json!(7000);
        run["turns"][1]["state"] = json!("received");
        assert_eq!(
            milestone_timing(&run, root, "build")?,
            json!({"started":1200,"ended":7000,"running":false})
        );
        assert!(milestone_timing(&run, root, "missing").is_err());
        // Legacy runs retain dispatch and receipt times on disk.
        run["turns"][0]["started"] = Value::Null;
        run["tasks"][0]["completed_at"] = Value::Null;
        for (turn, file) in [("one", "job.json"), ("two", "result.json")] {
            let path = root.join("turns").join(turn);
            fs::create_dir_all(&path)?;
            fs::write(path.join(file), "{}")?;
        }
        let legacy = milestone_timing(&run, root, "build")?;
        assert!(legacy["started"].is_i64() && legacy["ended"].is_i64());
        assert_eq!(milestone_completions(&run, root)["build"], legacy["ended"]);
        run["tasks"][0]["state"] = json!("running");
        assert_eq!(milestone_completions(&run, root), json!({}));
        assert_eq!(legacy["running"], false);
        Ok(())
    }

    #[test]
    fn receipts_replace_live_readings_and_keep_real_zeroes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut writer = Recorder::new(&json!({"profile":{"kind":"custom"}}));
        for tokens in [4, 4, 9] {
            writer.event(
                dir.path(),
                &json!({"type":"usage","model":"mini","tokens":tokens,"cost_usd":0}),
            )?;
        }
        let mut result = json!({"tokens":12});
        writer.finish(dir.path(), &mut result)?;
        assert_eq!(
            writer.history.last().unwrap().models["mini"],
            Reading {
                tokens: Some(12),
                breakdown: None,
                usd: Some(0.)
            }
        );
        Ok(())
    }

    #[test]
    fn native_costs_and_legacy_receipts_preserve_model_and_token_boundaries() -> Result<()> {
        let root = tempfile::tempdir()?;
        let dir = root.path().join("turns/a");
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("result.json"), r#"{"tokens":11}"#)?;
        fs::write(dir.join("events.log"),json!({"type":"turn.completed","raw":{"usage":{"input_tokens":10,"cached_input_tokens":4,"output_tokens":1}}}).to_string())?;
        let run = json!({"turns":[{"id":"eval","role":"eval"},{"id":"a","role":"worker","task":"legacy-task","state":"received"},
            {"id":"b","role":"chat","state":"running"}],"input":{"coordinator":"leader"},"tasks":[{"id":"legacy-task","worker":"runner"}],"profiles":{"runner":{"kind":"codex","model":"gpt-6.2-sol"}}});
        let r = report(&run, root.path(), 0)?;
        assert_eq!(r["turns"][0]["agent"], "runner");
        assert_eq!(r["turns"][1]["agent"], "leader");
        assert_eq!(r["next"], 2);
        assert_eq!(r["turns"].as_array().unwrap().len(), 2);
        let reading = &r["turns"][0]["samples"][0]["models"]["gpt-6.2-sol"];
        assert_eq!(reading["tokens"], 11);
        let individual = milestone_usage(&run, root.path(), "legacy-task")?;
        assert_eq!(individual["tokens"], 11);
        assert_eq!(individual["usd"], reading["usd"]);
        assert_eq!(individual["turns"], 1);
        assert!(milestone_usage(&run, root.path(), "missing").is_err());
        assert!((reading["usd"].as_f64().unwrap() - 0.0000224).abs() < 1e-12);
        assert_eq!(
            report(&run, root.path(), 2)?["turns"][0]["samples"],
            json!([])
        );
        let mut writer = Recorder::new(&json!({"profile":{"kind":"claude"}}));
        let message = json!({"type":"assistant","raw":{"message":{"id":"x","model":"one","usage":{"input_tokens":5,"output_tokens":1}}}});
        writer.event(&dir, &message)?;
        writer.event(&dir, &message)?;
        assert_eq!(writer.history.last().unwrap().models["one"].tokens, Some(5));
        writer.event(&dir,&json!({"type":"result","raw":{"modelUsage":{"one":{"inputTokens":5,"cacheReadInputTokens":2,"outputTokens":4,"thinkingTokens":2,"costUSD":0.03},"two":{"inputTokens":10,"outputTokens":2,"costUSD":0.01}}}}))?;
        let mut result = json!({});
        writer.finish(&dir, &mut result)?;
        assert_eq!(result["tokens"], 23);
        assert_eq!(writer.history.last().unwrap().models["two"].usd, Some(0.01));
        Ok(())
    }
    #[test]
    fn resumed_exec_charges_only_the_increment_including_legacy_pages() -> Result<()> {
        let root = tempfile::tempdir()?;
        let run = json!({"turns":[{"id":"a","role":"worker","agent":"w","state":"received"},{"id":"b","role":"worker","agent":"w","state":"received"}],
          "profiles":{"w":{"kind":"codex","model":"gpt-6.2-sol"}}});
        for (id, input) in [("a", 100), ("b", 180)] {
            let dir = root.path().join("turns").join(id);
            fs::create_dir_all(&dir)?;
            fs::write(
                dir.join("result.json"),
                json!({"session":"same-thread","tokens":input+10}).to_string(),
            )?;
            fs::write(dir.join("events.log"),json!({"type":"turn.completed","raw":{"usage":{"input_tokens":input,"output_tokens":10}}}).to_string())?;
        }
        let prior = prior_usage(
            &run["turns"].as_array().unwrap()[..1],
            root.path(),
            "same-thread",
        )
        .unwrap();
        let delta = turn_usage(
            &json!({"input_tokens":180,"output_tokens":10}),
            Some(&prior),
        );
        assert_eq!(delta, json!({"input_tokens":80,"output_tokens":0}));
        assert!(turn_usage(&prior, Some(&Value::Null)).is_null());
        for from in [0, 1] {
            assert_eq!(
                report(&run, root.path(), from)?["turns"][1 - from]["samples"][0]["models"]["gpt-6.2-sol"]
                    ["tokens"],
                80
            );
        }
        Ok(())
    }
}
