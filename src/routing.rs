use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, NaiveDate, Utc};
use reqwest::{Url, blocking::Client, redirect::Policy};
use serde_json::{Value, json};
use std::{
    cmp::Ordering,
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::Path,
    time::{Duration, Instant},
};

const FEED: &str = "https://api.harborframework.com/functions/v1/api/v1/leaderboards/rows";
const SOURCE: &str =
    "https://hub.harborframework.com/datasets/terminal-bench/terminal-bench/5?tab=leaderboard";
const SUITE: &str = "Terminal-Bench 4.1";
fn field<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or_default()
}
fn score(v: &Value) -> f64 {
    v["score"].as_f64().unwrap_or(-1.)
}
fn cost(v: &Value) -> Option<f64> {
    v["cost_per_trial"].as_f64()
}
pub fn mode_valid(v: &str) -> bool {
    matches!(v, "" | "manual" | "quality" | "balanced" | "economy")
}
pub fn class_valid(v: &str) -> bool {
    matches!(
        v,
        "" | "implementation" | "debugging" | "testing" | "automation" | "documentation" | "design"
    )
}
fn norm(v: &str) -> String {
    v.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
fn matches_profile(p: &Value, row: &Value) -> bool {
    let model = match field(p, "benchmark_model") {
        "" => field(p, "model"),
        m => m,
    };
    if model.is_empty() || norm(model) != norm(field(row, "model")) {
        return false;
    }
    let effort = field(p, "benchmark_effort");
    if !effort.is_empty() && norm(effort) != norm(field(row, "effort")) {
        return false;
    }
    let agent = norm(field(row, "agent"));
    match field(p, "benchmark_agent") {
        "" => matches!(
            (field(p, "kind"), agent.as_str()),
            ("codex", "codex" | "codexcli")
                | ("claude", "claudecode")
                | ("hermes", "hermes" | "hermesagent")
                | ("antigravity", "antigravity" | "antigravitycli")
        ),
        v => norm(v) == agent,
    }
}
pub fn choose(
    mode: &str,
    class: &str,
    fallback: &str,
    workers: &[String],
    profiles: &Value,
    catalog: &Value,
) -> (String, Option<Value>) {
    if matches!(mode, "" | "manual") {
        return (fallback.into(), None);
    }
    let failed = |reason: &str| (fallback.into(), Some(json!({"mode":mode,"reason":reason})));
    if class.is_empty() {
        return failed("The coordinator did not specify a benchmark category.");
    }
    type Group<'a> = BTreeMap<&'a str, &'a Value>;
    let mut suites: BTreeMap<(&str, &str, &str), Group> = BTreeMap::new();
    for row in catalog["rows"].as_array().into_iter().flatten() {
        let category = field(row, "class");
        if category != class
            && !(category == "terminal"
                && matches!(
                    class,
                    "implementation" | "debugging" | "testing" | "automation"
                ))
        {
            continue;
        }
        if mode != "quality" && cost(row).is_none() {
            continue;
        }
        let options = suites
            .entry((field(row, "suite"), field(row, "source"), category))
            .or_default();
        for name in workers {
            if matches_profile(&profiles[name], row)
                && options.get(name.as_str()).is_none_or(|old| {
                    field(row, "date") > field(old, "date")
                        || (field(row, "date") == field(old, "date") && score(row) > score(old))
                })
            {
                options.insert(name, row);
            }
        }
    }
    let mut groups = suites
        .iter()
        .filter(|(_, g)| g.len() >= 2)
        .collect::<Vec<_>>();
    groups.sort_by_key(|(key, g)| {
        (
            std::cmp::Reverse(key.2 == class),
            std::cmp::Reverse(g.values().map(|v| field(v, "date")).max()),
            *key,
        )
    });
    let Some((_, group)) = groups.first() else {
        let note = format!(
            "Fewer than two selected profiles have comparable evidence. {}",
            field(catalog, "note")
        );
        return failed(note.trim_end());
    };
    let best = group.values().map(|r| score(r)).fold(0.0, f64::max);
    let margin = match mode {
        "balanced" => 5.,
        "economy" => 15.,
        _ => 0.,
    };
    let selected = group
        .iter()
        .filter(|(_, r)| score(r) >= best - margin)
        .min_by(|(an, a), (bn, b)| {
            (if mode == "quality" {
                score(b).total_cmp(&score(a))
            } else {
                Ordering::Equal
            })
            .then_with(|| {
                cost(a)
                    .unwrap_or(f64::INFINITY)
                    .total_cmp(&cost(b).unwrap_or(f64::INFINITY))
            })
            .then_with(|| score(b).total_cmp(&score(a)))
            .then_with(|| an.cmp(bn))
        });
    let Some((name, row)) = selected else {
        return failed("The saved snapshot has no usable scores for these profiles.");
    };
    let reason = match mode {
        "balanced" => "Lowest measured cost within five points of the best score.",
        "economy" => "Lowest measured cost within fifteen points of the best score.",
        _ => "Highest measured score.",
    };
    (
        name.to_string(),
        Some(json!({"mode":mode,"reason":reason,"evidence":row,"compared":group.len()})),
    )
}
fn valid(row: &Value, now: DateTime<Utc>) -> bool {
    if ["model", "agent", "suite", "class"]
        .iter()
        .any(|k| field(row, k).is_empty())
        || !(0.0..=100.0).contains(&score(row))
        || !(row["cost_per_trial"].is_null() || cost(row).is_some_and(|c| c >= 0.0))
    {
        return false;
    }
    let Ok(date) = NaiveDate::parse_from_str(field(row, "date"), "%Y-%m-%d") else {
        return false;
    };
    let age = now.signed_duration_since(date.and_time(chrono::NaiveTime::MIN).and_utc());
    let Ok(url) = Url::parse(field(row, "source")) else {
        return false;
    };
    age >= chrono::Duration::days(-1)
        && age < chrono::Duration::days(180)
        && url.scheme() == "https"
        && url.host().is_some()
        && url.username().is_empty()
        && url.password().is_none()
}
fn bounded_json(mut reader: impl Read) -> Result<Value> {
    const LIMIT: u64 = 4 << 20;
    let mut buf = Vec::new();
    (&mut reader).take(LIMIT + 1).read_to_end(&mut buf)?;
    ensure!(
        buf.len() as u64 <= LIMIT,
        "benchmark feed exceeds size limit"
    );
    Ok(serde_json::from_slice(&buf)?)
}
fn fetch(endpoint: &str, now: DateTime<Utc>) -> Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(4);
    let client = Client::builder().redirect(Policy::none()).build()?;
    let mut rows = Vec::new();
    let mut cursor = None;
    for _ in 0..5 {
        let mut url = Url::parse(endpoint)?;
        url.query_pairs_mut()
            .append_pair("package", "terminal-bench/terminal-bench")
            .append_pair("name", "4-1-0")
            .append_pair("page_size", "100");
        if let Some(cursor) = cursor.as_deref() {
            url.query_pairs_mut().append_pair("cursor", cursor);
        }
        let time = deadline
            .checked_duration_since(Instant::now())
            .context("benchmark refresh timed out")?;
        let res = client
            .get(url)
            .timeout(time)
            .header("Harbor-API-Version", "2026-09-10.preview")
            .send()?;
        ensure!(
            res.status().as_u16() == 200,
            "benchmark feed HTTP {}",
            res.status()
        );
        let data = bounded_json(res)?;
        for item in data["items"].as_array().into_iter().flatten() {
            if item["status"] != "display" {
                continue;
            }
            let meta = &item["metadata"];
            let metrics = &item["metrics"];
            let mut row = json!({"model":meta["model_display"]["label"],"agent":meta["agent_display"]["label"],"date":meta["date"],
                "class":"terminal","suite":SUITE,"source":SOURCE,"score":metrics["accuracy"]});
            if let Some(effort) = meta["reasoning_effort"].as_str().filter(|a| !a.is_empty()) {
                row["effort"] = json!(effort)
            }
            if let (Some(n), Some(total)) = (
                item["n_trials"].as_u64(),
                metrics["total_cost_usd"].as_f64(),
            ) && n > 0
                && total > 0.0
            {
                row["cost_per_trial"] = json!(total / n as f64)
            }
            if valid(&row, now) {
                rows.push(row)
            }
        }
        cursor = data["next_cursor"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        if cursor.is_none() {
            ensure!(!rows.is_empty(), "benchmark feed contained no usable rows");
            return Ok(json!({"rows":rows,"fetched":now.to_rfc3339()}));
        }
    }
    bail!("benchmark feed exceeded page limit")
}
pub fn read(cache: &Path, local: &Path) -> Value {
    let now = Utc::now();
    let mut out = File::open(cache)
        .ok()
        .and_then(|f| bounded_json(f).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let age = field(&out, "fetched")
        .parse::<DateTime<Utc>>()
        .ok()
        .map(|t| now.signed_duration_since(t))
        .filter(|t| *t >= chrono::Duration::zero());
    let mut rows = out["rows"].as_array().cloned().unwrap_or_default();
    rows.retain(|r| valid(r, now));
    out["rows"] = json!(rows);
    if age.is_none_or(|t| t >= chrono::Duration::days(1)) || rows.is_empty() {
        match fetch(FEED, now) {
            Ok(fresh) => {
                let _ = crate::supervisor::write_json(cache, &fresh);
                out = fresh
            }
            Err(_) if !rows.is_empty() && age.is_some_and(|t| t < chrono::Duration::days(7)) => {
                out["note"] = json!("Live feed unavailable and recent cached results are in use.")
            }
            Err(_) => out = json!({"rows":[],"note":"Live benchmark feed is unavailable."}),
        }
    }
    match File::open(local) {
        Ok(f) => match bounded_json(f) {
            Ok(data) if data["rows"].is_array() => {
                if let Some(rows) = out["rows"].as_array_mut() {
                    rows.extend(
                        data["rows"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|v| valid(v, now))
                            .cloned(),
                    )
                }
            }
            _ => local_error(&mut out),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(_) => local_error(&mut out),
    }
    out
}
fn local_error(out: &mut Value) {
    out["note"] = json!(
        format!(
            "{} Could not read local benchmark results.",
            field(out, "note")
        )
        .trim()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(model: &str, agent: &str, score: f64, cost: Option<f64>) -> Value {
        json!({"model":model,"agent":agent,"score":score,"cost_per_trial":cost,"suite":"T","source":"https://example.invalid/t","class":"terminal","date":"2026-11-22"})
    }
    #[test]
    fn rank_only_comparable_evidence() {
        let profiles = json!({"premium":{"kind":"codex","model":"GPT-pro"},"efficient":{"kind":"claude","model":"regular"},"budget":{"kind":"hermes","benchmark_model":"Cheap model"}});
        let workers = ["premium", "efficient", "budget"].map(String::from);
        let mut rows = vec![
            row("GPT Pro", "Codex", 91., Some(1.)),
            row("regular", "Claude Code", 87., Some(0.25)),
            row("Cheap model", "Hermes", 76., Some(0.08)),
        ];
        let mut stale = rows[0].clone();
        stale["date"] = json!("2026-11-20");
        stale["score"] = json!(99);
        rows.push(stale);
        for (mode, class, winner, evidence) in [
            ("manual", "testing", "premium", false),
            ("balanced", "testing", "efficient", true),
            ("quality", "debugging", "premium", true),
            ("economy", "implementation", "budget", true),
            ("balanced", "design", "premium", false),
        ] {
            let (name, why) = choose(
                mode,
                class,
                "premium",
                &workers,
                &profiles,
                &json!({"rows":rows}),
            );
            assert_eq!(name, winner);
            assert_eq!(why.is_some_and(|why| !why["evidence"].is_null()), evidence);
        }
        rows[1]["suite"] = json!("other");
        rows[2]["cost_per_trial"] = Value::Null;
        let (_, why) = choose(
            "balanced",
            "testing",
            "premium",
            &workers,
            &profiles,
            &json!({"rows":rows}),
        );
        assert!(why.unwrap()["evidence"].is_null());
        rows[1]["score"] = json!(-2);
        rows[0]["score"] = json!(-3);
        rows[2]["score"] = json!(-4);
        rows.pop();
        let (name, why) = choose(
            "quality",
            "testing",
            "budget",
            &workers,
            &profiles,
            &json!({"rows":rows}),
        );
        assert_eq!(name, "budget");
        assert!(why.unwrap()["evidence"].is_null());
    }
    #[test]
    fn reject_stale_or_malformed_cache_rows() {
        let now = "2026-11-24T00:00:00Z".parse().unwrap();
        let r = row("1", "Codex", 89., None);
        assert!(valid(&r, now));
        for (k, bad) in [
            ("score", json!(-2)),
            ("source", json!("https://user@host/test")),
            ("date", json!("2025-10-01")),
            ("date", json!("2026-11-27")),
            ("cost_per_trial", json!(-1)),
        ] {
            let mut bad_row = r.clone();
            bad_row[k] = bad;
            assert!(!valid(&bad_row, now));
        }
    }
    #[test]
    fn fetch_paginated_public_feed() -> Result<()> {
        use std::{
            io::{BufRead, BufReader, Write},
            net::TcpListener,
            thread,
        };
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let endpoint = format!("http://{}", listener.local_addr()?);
        let serve = thread::spawn(move || -> Result<()> {
            for page in 0..2 {
                let (mut sock, _) = listener.accept()?;
                let mut header = String::new();
                let mut reader = BufReader::new(&sock);
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line)?;
                    if line.trim().is_empty() {
                        break;
                    }
                    header.push_str(&line);
                }
                assert!(header.contains("name=4-1-0"));
                assert!(header.to_lowercase().contains("harbor-api-version"));
                assert_eq!(header.contains("cursor=next"), page == 1);
                let r = json!({"status":"display","n_trials":5,"metadata":{"date":"2026-11-22","agent_display":{"label":"Codex"},"model_display":{"label":"M"}},"metrics":{"accuracy":88.4,"total_cost_usd":2.5}});
                let body =
                    json!({"next_cursor":if page==0{"next"}else{""},"items":[r,{"status":"hide"}]})
                        .to_string();
                write!(
                    sock,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )?;
            }
            Ok(())
        });
        let catalog = fetch(&endpoint, "2026-11-23T00:00:00Z".parse()?)?;
        serve.join().unwrap()?;
        assert_eq!(catalog["rows"].as_array().unwrap().len(), 2);
        assert_eq!(catalog["rows"][0]["cost_per_trial"], 0.5);
        Ok(())
    }
}
