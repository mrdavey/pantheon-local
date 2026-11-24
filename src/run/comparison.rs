//! Compare only measurements taken for the current revision and policy.
use crate::run::{a, find, s};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{fs::File, path::Path};

pub fn evidence(run: &Value, id: &str, root: &Path) -> Result<Value> {
    let i = find(&run["candidates"], id).context("unknown candidate")?;
    let job = s(&run["candidates"][i], "evidence");
    ensure!(!job.is_empty(), "no measured evidence for this candidate");
    Ok(serde_json::from_reader(File::open(
        root.join("turns").join(job).join("measurements.json"),
    )?)?)
}
pub fn record_report(
    candidate: &mut Value,
    recipe: &Value,
    report: &Value,
    job: &str,
) -> Result<()> {
    ensure!(
        report["commit"].is_string()
            && report["inputs"].is_string()
            && report["commit"] == candidate["commit"]
            && report["inputs"] == candidate["inputs"],
        "evaluation receipt does not match the candidate inputs"
    );
    let (state, detail, score) = crate::eval::summarize(recipe, report)?;
    candidate["evidence"] = json!(job);
    candidate["state"] = json!(state);
    candidate["detail"] = json!(detail);
    candidate["score"] = score;
    Ok(())
}
fn direction(rule: &Value, name: &str) -> f64 {
    if a(&rule["metrics"])
        .iter()
        .any(|m| m["name"] == name && m["direction"] == "lower")
    {
        -1.
    } else {
        1.
    }
}
fn score(c: &Value, name: &str) -> Option<f64> {
    c["score"][name]["value"].as_f64()
}
pub fn eligible(rule: &Value, c: &Value) -> bool {
    if c["state"] != "passed" || c["baseline"] == true {
        return false;
    }
    if rule.is_null() {
        return true;
    }
    score(c, s(rule, "objective")).is_some()
        && rule["limits"]
            .as_object()
            .into_iter()
            .flatten()
            .all(|(name, limit)| {
                score(c, name)
                    .zip(limit.as_f64())
                    .is_some_and(|(v, limit)| (v - limit) * direction(rule, name) >= 0.)
            })
}
pub fn recommend(run: &Value) -> Value {
    let recipe = &run["input"]["recipe"];
    let rule = &recipe["rule"];
    let objective = s(rule, "objective");
    let mut result = json!({"round":run["round"],"policy":run["policy"],"at":chrono::Utc::now(),"state":"undecided"});
    let mut options = vec![];
    let mut baseline = None;
    for c in a(&run["candidates"])
        .iter()
        .filter(|c| c["round"] == run["round"] && c["policy"] == run["policy"])
    {
        if c["baseline"] == true {
            baseline = Some(c);
            continue;
        }
        let missing = |m: &str| c["score"][m]["n"].as_u64().unwrap_or(0) == 0;
        if matches!(s(c, "state"), "inconclusive" | "cancelled")
            || (!rule.is_null()
                && c["state"] == "passed"
                && (missing(objective)
                    || rule["limits"]
                        .as_object()
                        .is_some_and(|limits| limits.keys().any(|k| missing(k)))))
        {
            result["reason"] = json!("Some measurements are missing or inconclusive.");
            return result;
        }
        if eligible(rule, c) {
            options.push(c);
        }
    }
    let quality = |c: &Value| score(c, objective).unwrap_or_default() * direction(rule, objective);
    if let Some(base) = baseline.filter(|_| !rule.is_null()) {
        if base["state"] == "inconclusive" || score(base, objective).is_none() {
            result["reason"] = json!("The baseline could not be measured.");
            return result;
        }
        options.retain(|c| {
            let gain = quality(c) - quality(base);
            gain > 0. && gain >= rule["minimum"].as_f64().unwrap_or_default()
        });
    }
    if options.is_empty() {
        result["reason"] = json!("No implementation met the approved rule.");
        if baseline.is_some() {
            result["state"] = json!("baseline");
        }
        return result;
    }
    if options.len() > 1 {
        if rule.is_null() {
            result["reason"] = json!("Choose among the passing implementations.");
            return result;
        }
        options.sort_by(|a, b| quality(b).total_cmp(&quality(a)));
        let gap = quality(options[0]) - quality(options[1]);
        if gap < rule["min_lead"].as_f64().unwrap_or_default() {
            result["reason"] = json!("Leading implementations are too close.");
            return result;
        }
        if gap == 0. {
            let name = s(rule, "tie_break");
            let best = quality(options[0]);
            let leaders = options.iter().take_while(|c| quality(c) == best).count();
            if name.is_empty() || options[..leaders].iter().any(|c| score(c, name).is_none()) {
                result["reason"] = json!("Leading implementations are tied.");
                return result;
            }
            options[..leaders].sort_by(|a, b| {
                let q = |c: &Value| score(c, name).unwrap_or_default() * direction(rule, name);
                q(b).total_cmp(&q(a))
            });
            if score(options[0], name) == score(options[1], name) {
                result["reason"] = json!("Leading implementations are tied.");
                return result;
            }
        }
    }
    result["recommended"] = options[0]["id"].clone();
    if s(recipe, "rubric").is_empty() {
        result["state"] = json!("recommended");
        result["reason"] = json!("All required checks passed and the approved limits were met.");
    } else {
        result["reason"] = json!("Review the implementations against the rubric before selecting.");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn minimum_lead() {
        let r = json!({"input":{"recipe":{"rule":{"objective":"score","min_lead":2,"metrics":[{"name":"score","direction":"higher"}]}}},"candidates":[{"id":"a","state":"passed","score":{"score":{"value":9,"n":3}}},{"id":"b","state":"passed","score":{"score":{"value":8,"n":3}}}]});
        assert_eq!(recommend(&r)["state"], "undecided");
    }
}
