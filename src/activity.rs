//! Filter turn metadata before paging. Event bodies are read only when a turn is opened.
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    path::Path,
};

fn references(ids: &Value, milestone: &str) -> bool {
    ids.as_array()
        .is_some_and(|ids| ids.iter().any(|id| id == milestone))
}

fn lookup<'a>(run: &'a Value, field: &str, id: &Value) -> Option<&'a Value> {
    run[field].as_array()?.iter().find(|item| &item["id"] == id)
}

fn candidate_contains(run: &Value, candidate: &Value, milestone: &str) -> bool {
    lookup(run, "candidates", candidate).is_some_and(|c| references(&c["tasks"], milestone))
}

// Older coordinator turns saved their structured inputs and replies on disk.
// Only consult those receipts when the caller opens a scoped history.
fn legacy_related(run: &Value, turn: &Value, root: &Path, milestone: &str) -> bool {
    let Some(id) = turn["id"].as_str() else {
        return false;
    };
    if Path::new(id).file_name().and_then(|s| s.to_str()) != Some(id) {
        return false;
    }
    let dir = root.join("turns").join(id);
    let read =
        |name| -> Option<Value> { serde_json::from_reader(File::open(dir.join(name)).ok()?).ok() };
    let relevant = |id: &Value| id == milestone || candidate_contains(run, id, milestone);
    if turn["role"] == "observe"
        && let Some(job) = read("job.json")
        && let Some((_, state)) = job["request"]["prompt"]
            .as_str()
            .and_then(|p| p.rsplit_once("\nState: "))
        && let Ok(state) = serde_json::from_str::<Value>(state)
        && state["Observations"]
            .as_array()
            .is_some_and(|events| events.iter().any(|e| relevant(&e["ref"])))
    {
        return true;
    }
    let Some(reply) = read("result.json")
        .and_then(|res| crate::adapter::decode_outcome(res["text"].as_str()?).ok())
    else {
        return false;
    };
    let assigns = |data: &Value| {
        data["tasks"]
            .as_array()
            .is_some_and(|tasks| tasks.iter().any(|task| task["id"] == milestone))
    };
    assigns(&reply)
        || reply["task"] == milestone
        || reply["actions"].as_array().is_some_and(|actions| {
            actions.iter().any(|a| {
                relevant(&a["ref"])
                    || a["replacement"] == milestone
                    || assigns(a)
                    || references(&a["question"]["tasks"], milestone)
            })
        })
}

fn related(
    run: &Value,
    turn: &Value,
    root: &Path,
    milestone: &str,
    related_turns: &HashSet<&str>,
) -> bool {
    if references(&turn["milestones"], milestone) {
        return true;
    }
    let task = &turn["task"];
    match turn["role"].as_str() {
        Some("worker") => task == milestone,
        Some("eval") => candidate_contains(run, task, milestone),
        Some("release") => lookup(run, "releases", task)
            .is_some_and(|r| candidate_contains(run, &r["candidate"], milestone)),
        Some("chat") => lookup(run, "conversation", task).is_some_and(|m| {
            let ctx = &m["context"];
            match ctx["kind"].as_str() {
                Some("milestone") => ctx["id"] == milestone,
                Some("implementation") => candidate_contains(run, &ctx["id"], milestone),
                Some("activity") => ctx["id"]
                    .as_str()
                    .is_some_and(|id| related_turns.contains(id)),
                _ => false,
            }
        }),
        Some("observe" | "plan" | "repair") if turn["milestones"].is_null() => {
            legacy_related(run, turn, root, milestone)
        }
        _ => false,
    }
}

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
    let mut related_turns = HashSet::new();
    for t in turns {
        let role = t["role"].as_str().unwrap_or("");
        let task = t["task"].as_str().unwrap_or("");
        let milestone = get("milestone");
        if !milestone.is_empty() {
            if !related(run, t, root, milestone, &related_turns) {
                continue;
            }
            if let Some(id) = t["id"].as_str() {
                related_turns.insert(id);
            }
        }
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
    fn recovers_legacy_observer_links_and_follows_activity_conversations() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let old = dir.path().join("turns/review");
        std::fs::create_dir_all(&old)?;
        let state = json!({"Tasks":[{"id":"a"},{"id":"unrelated"}],"Observations":[{"ref":"a"}]});
        std::fs::write(
            old.join("job.json"),
            json!({"request":{"prompt":format!("Review\nState: {state}")}}).to_string(),
        )?;
        let run = json!({"turns":[{"id":"review","role":"observe"},{"id":"chat1","role":"chat","task":"question"},{"id":"ship","role":"release","task":"release"}],
            "releases":[{"id":"release","candidate":"candidate"}],"candidates":[{"id":"candidate","tasks":["a"]}],
            "conversation":[{"id":"question","context":{"kind":"activity","id":"review"}}]});
        assert_eq!(page(&run, dir.path(), "milestone=a")?["total"], 3);
        assert_eq!(page(&run, dir.path(), "milestone=unrelated")?["total"], 0);
        Ok(())
    }

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
