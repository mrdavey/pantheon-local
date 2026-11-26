//! Filter turn metadata before paging. Event bodies are read only when a turn is opened.
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{collections::HashMap, path::Path};

pub fn page(run: &Value, root: &Path, query: &str) -> Result<Value> {
    let params: HashMap<_, _> = url::form_urlencoded::parse(query.as_bytes()).collect();
    let get = |key: &str| params.get(key).map(|v| v.as_ref()).unwrap_or("");
    let page = get("page").parse::<usize>().unwrap_or(1).max(1);
    let size = get("size").parse::<usize>().unwrap_or(20).clamp(1, 100);
    let search = get("q").trim().to_lowercase();
    let turns = run["turns"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let tasks: HashMap<_, _> = run["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| Some((t["id"].as_str()?, t)))
        .collect();
    let mut descriptions = HashMap::new();
    for (role, collection, field) in [
        ("worker", "tasks", "goal"),
        ("chat", "conversation", "text"),
        ("eval", "candidates", "name"),
        ("observe", "observations", "text"),
        ("release", "releases", "destination"),
    ] {
        for item in run[collection].as_array().into_iter().flatten() {
            if let (Some(id), Some(text)) = (item["id"].as_str(), item[field].as_str())
                && !text.trim().is_empty()
            {
                descriptions.insert((role, id), text);
            }
        }
    }
    let mut rows = Vec::new();
    for t in turns {
        let role = t["role"].as_str().unwrap_or("");
        let task = t["task"].as_str().unwrap_or("");
        let agent = t["agent"]
            .as_str()
            .or_else(|| {
                if role == "worker" {
                    tasks.get(task)?["worker"].as_str()
                } else if matches!(role, "chat" | "repair" | "plan" | "observe" | "pricing") {
                    run["input"]["coordinator"].as_str()
                } else {
                    None
                }
            })
            .unwrap_or("");
        let label = match role {
            "plan" => "Planning",
            "observe" => "Coordinator review",
            "chat" => "Conversation",
            "eval" => "Evaluation",
            "repair" => "Repair planning",
            "pricing" => "Model pricing",
            _ => role,
        };
        let role_matches = match get("role") {
            "" => true,
            "coordinator" => matches!(role, "plan" | "observe" | "repair" | "chat" | "pricing"),
            specific => role == specific,
        };
        let detail = descriptions
            .get(&(role, task))
            .copied()
            .unwrap_or(match role {
                "plan" => "Create the run plan",
                "observe" => "Review milestone progress",
                "repair" => "Plan the next repair",
                "pricing" => "Look up model prices",
                "eval" => "Check the implementation",
                "chat" => "Respond to the developer",
                "release" => "Deliver the selected implementation",
                "worker" => "Work on the assigned milestone",
                _ => label,
            });
        if !role_matches
            || (!get("state").is_empty() && t["state"] != get("state"))
            || (!get("agent").is_empty() && agent != get("agent"))
            || (!search.is_empty()
                && ![
                    role,
                    label,
                    task,
                    detail,
                    agent,
                    t["state"].as_str().unwrap_or(""),
                    t["id"].as_str().unwrap_or(""),
                ]
                .iter()
                .any(|s| s.to_lowercase().contains(&search)))
        {
            continue;
        }
        rows.push((t, agent, detail));
    }
    // Dispatch appends turns in chronological order, even in older runs without timestamps.
    if get("sort") != "oldest" {
        rows.reverse()
    }
    let total = rows.len();
    let page = page.min(total.div_ceil(size).max(1));
    let mut items = Vec::new();
    for (turn, agent, detail) in rows.into_iter().skip((page - 1) * size).take(size) {
        let id = turn["id"].as_str().context("turn missing ID")?;
        let started = turn["started"].as_i64().or_else(|| {
            let time = root
                .join("turns")
                .join(id)
                .join("job.json")
                .metadata()
                .ok()?
                .modified()
                .ok()?;
            Some(chrono::DateTime::<chrono::Utc>::from(time).timestamp_millis())
        });
        items.push(json!({"id":id,"role":turn["role"],"state":turn["state"],"started":started,"detail":detail,"agent":agent}));
    }
    Ok(json!({"items":items,"total":total,"page":page,"pages":total.div_ceil(size).max(1)}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn searches_before_paging_and_includes_legacy_agent_names() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let run = json!({"input":{"coordinator":"lead"},"tasks":[{"id":"csv","goal":"Export receipts","worker":"local"}],
            "conversation":[{"id":"msg","text":"Why this model?"}],
            "turns":[{"id":"one","role":"plan","state":"received","started":1},
                {"id":"two","role":"worker","task":"csv","state":"received"},
                {"id":"three","role":"chat","task":"msg","state":"running"}]});
        let get = |q| page(&run, dir.path(), q);
        assert_eq!(get("size=1&page=2")?["items"][0]["id"], "two");
        assert_eq!(get("size=1&sort=oldest")?["items"][0]["id"], "one");
        assert_eq!(get("role=coordinator")?["total"], 2);
        assert_eq!(get("q=planning")?["total"], 1);
        assert_eq!(
            get("q=receipts&state=received&agent=local")?["items"][0]["id"],
            "two"
        );
        assert_eq!(
            get("agent=lead&role=chat&q=model&size=1&page=999")?["items"][0]["id"],
            "three"
        );
        assert_eq!(get("agent=local&state=running")?["total"], 0);
        Ok(())
    }
}
