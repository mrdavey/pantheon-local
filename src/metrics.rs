//! Cumulative, turn-local usage receipts. Keep telemetry separate from run revisions.
use anyhow::Result;
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
            Reading { tokens, usd },
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
                self.single(None, tokens, cost)
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
                self.single(
                    None,
                    event["tokens"]
                        .as_u64()
                        .or_else(|| prior.and_then(|r| r.tokens)),
                    usd(&event["cost_usd"]).or_else(|| prior.and_then(|r| r.usd)),
                )
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
            self.save(dir, self.single(name, tokens, cost))?;
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
    delta
}

fn legacy_sample(
    dir: &Path,
    profile: &Value,
    file: fs::File,
    turns: &[Value],
    root: &Path,
    baselines: &mut BTreeMap<String, Value>,
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
        let prior = baselines
            .get(session)
            .cloned()
            .or_else(|| prior_usage(turns, root, session));
        event["turn_usage"] = turn_usage(&raw, prior.as_ref());
        if !session.is_empty() {
            baselines.insert(session.into(), raw);
        }
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

pub fn report(run: &Value, root: &Path, from: usize) -> Result<Value> {
    let turns = run["turns"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut rows = Vec::new();
    let mut next = turns.len();
    let mut legacy_baselines = BTreeMap::new();
    for (index, t) in turns.iter().enumerate().skip(from) {
        if matches!(t["role"].as_str(), Some("eval" | "release")) {
            continue;
        }
        if t["state"] == "running" {
            next = next.min(index);
        }
        let dir = root.join("turns").join(t["id"].as_str().unwrap_or(""));
        let profile = &run["profiles"][t["agent"].as_str().unwrap_or("")];
        let name = model(profile);
        let history: Vec<Sample> = if let Some(f) = optional(&dir.join("usage.json"))? {
            serde_json::from_reader(f)?
        } else if let Some(f) = optional(&dir.join("result.json"))? {
            legacy_sample(
                &dir,
                profile,
                f,
                &turns[..from.min(turns.len())],
                root,
                &mut legacy_baselines,
            )?
        } else {
            vec![]
        };
        rows.push(json!({"id":t["id"],"agent":t["agent"],"role":t["role"],"model":name,"state":t["state"],"started":t["started"],"samples":history,"harness":profile["kind"]}));
    }
    Ok(json!({"turns":rows,"next":next}))
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let run = json!({"turns":[{"id":"eval","role":"eval"},{"id":"a","role":"worker","agent":"runner","state":"received"},
            {"id":"b","role":"chat","agent":"runner","state":"running"}],"profiles":{"runner":{"kind":"codex","model":"gpt-6.2-sol"}}});
        let r = report(&run, root.path(), 0)?;
        assert_eq!(r["next"], 2);
        assert_eq!(r["turns"].as_array().unwrap().len(), 2);
        let reading = &r["turns"][0]["samples"][0]["models"]["gpt-6.2-sol"];
        assert_eq!(reading["tokens"], 11);
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
