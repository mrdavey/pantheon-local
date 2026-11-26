//! Coordinator-sourced standard API estimates, kept separately from provider receipts.
use crate::{adapter, metrics::Reading};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub fn estimate(rate: &Value, reading: &Reading) -> Option<f64> {
    let price = |key: &str| rate[key].as_f64().filter(|r| r.is_finite() && *r >= 0.);
    let amount = |count: u64, key| {
        if count == 0 {
            Some(0.)
        } else {
            Some(count as f64 * price(key)?)
        }
    };
    let b = reading.breakdown.as_ref();
    let cost = if let Some(b) = b {
        amount(b.input, "input")?
            + amount(b.cached, "cached")?
            + amount(b.output, "output")?
            + amount(b.cache_write, "cache_write")?
    } else {
        // A total-only receipt is priceable only when every category costs the same.
        let flat = price("input")?;
        if ["output", "cached", "cache_write"]
            .iter()
            .any(|key| price(key) != Some(flat))
        {
            return None;
        }
        reading.tokens? as f64 * flat
    };
    (rate.is_object() && cost.is_finite()).then_some(cost / 1_000_000.)
}

pub fn missing(report: &Value) -> BTreeMap<String, Value> {
    let mut missing = BTreeMap::new();
    for turn in report["turns"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| t["role"] != "pricing")
    {
        let mut collect = |model: &str, receipt: &Value| {
            if model.is_empty() || model == "Unknown model" || !receipt["usd"].is_null() {
                return;
            }
            let row = missing
                .entry(model.to_owned())
                .or_insert_with(|| json!({"agents":{}, "turns":0, "itemized_turns":0}));
            row["agents"][turn["agent"].as_str().unwrap_or("Unknown agent")] =
                turn["harness"].clone();
            row["turns"] = json!(row["turns"].as_u64().unwrap_or(0) + 1);
            if let Some(n) = receipt["tokens"].as_u64() {
                row["reported_tokens"] = json!(
                    row["reported_tokens"]
                        .as_u64()
                        .unwrap_or(0)
                        .saturating_add(n)
                );
            }
            if let Some(parts) = receipt["breakdown"].as_object() {
                row["itemized_turns"] = json!(row["itemized_turns"].as_u64().unwrap_or(0) + 1);
                for (key, count) in parts {
                    let old = row["breakdown"][key].as_u64().unwrap_or(0);
                    row["breakdown"][key] = json!(old.saturating_add(count.as_u64().unwrap_or(0)));
                }
            }
        };
        if let Some(models) = turn["samples"]
            .as_array()
            .and_then(|s| s.last())
            .and_then(|s| s["models"].as_object())
        {
            for (model, receipt) in models {
                collect(model, receipt);
            }
        } else if let Some(model) = turn["model"].as_str() {
            collect(model, &Value::Null);
        }
    }
    missing
}

pub fn finish(run: &mut Value, result: &Value) {
    let reply = adapter::decode_outcome(result["text"].as_str().unwrap_or(""));
    let mut count = 0;
    let error = result["error"].as_str().filter(|text| !text.is_empty());
    if error.is_none()
        && let Ok(reply) = reply
        && let Some(rates) = reply["rates"].as_object()
    {
        for (model, rate) in rates {
            if run["pricing_models"].get(model).is_none() {
                continue;
            }
            let Some(source) = rate["source"]
                .as_str()
                .and_then(|s| url::Url::parse(s).ok())
            else {
                continue;
            };
            if source.scheme() != "https"
                || source.host_str().is_none()
                || !source.username().is_empty()
                || source.password().is_some()
            {
                continue;
            }
            let valid = |key: &str| rate[key].as_f64().is_some_and(|n| n.is_finite() && n >= 0.);
            if !valid("input")
                || !valid("output")
                || ["cached", "cache_write"]
                    .iter()
                    .any(|key| !rate[key].is_null() && !valid(key))
            {
                continue;
            }
            run["model_rates"][model] = json!({"input":rate["input"],"output":rate["output"],"cached":rate["cached"],"cache_write":rate["cache_write"],"source":source.as_str(),"fetched":chrono::Utc::now().to_rfc3339()});
            count += 1;
        }
    }
    run["pricing_status"] = json!(if let Some(error) = error {
        format!("Price lookup failed. {error}")
    } else if count == 0 {
        "No published prices found for these models.".into()
    } else {
        format!("Updated pricing for {count} model(s).")
    });
    run["pricing_revision"] = json!(run["pricing_revision"].as_u64().unwrap_or(0) + 1);
}

pub const INSTRUCTIONS: &str = "Find current, published standard text API prices for the exact models in the following usage receipts. Use web search and the provider's official pricing page (or OpenRouter for an OpenRouter model). Use USD per million tokens. Return JSON only: {\"rates\":{\"exact model key\":{\"input\":0.0,\"cached\":null,\"cache_write\":null,\"output\":0.0,\"source\":\"https://...\"}}}. Record zero only for an explicitly free category. Return null for an unspecified cache rate. Omit any model whose identity, tier, context price, or source you cannot verify. Never infer a price or split a total token count.";
