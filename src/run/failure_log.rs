//! Keep failure evidence after the coordinator consumes its observation queue.
use super::*;

pub(super) fn is_failure(kind: &str) -> bool {
    kind.ends_with("-failed")
        || matches!(
            kind,
            "worker-blocked"
                | "run-blocked"
                | "eval-inconclusive"
                | "integration-conflict"
                | "coordinator-finding"
        )
}

pub(super) fn record(r: &mut Value, observation: &Value) {
    let kind = s(observation, "kind");
    if !is_failure(kind) {
        return;
    }
    let target = s(observation, "ref");
    let task = find(&r["tasks"], target).map(|i| &r["tasks"][i]);
    let release = find(&r["releases"], target).map(|i| &r["releases"][i]);
    let candidate_id = release.map_or(target, |rel| s(rel, "candidate"));
    let candidate = find(&r["candidates"], candidate_id).map(|i| &r["candidates"][i]);
    let source_role = match kind {
        "worker-blocked" => "worker",
        "candidate-failed" | "eval-failed" | "eval-inconclusive" => "eval",
        "release-failed" => "release",
        "chat-failed" => "chat",
        "pricing-failed" => "pricing",
        _ => "",
    };
    let source = a(&r["turns"])
        .iter()
        .rev()
        .find(|t| t["role"] == source_role && t["task"] == target);
    let milestones = match (task, candidate) {
        (Some(t), _) => json!([t["id"]]),
        (_, Some(c)) => c["tasks"].clone(),
        _ => json!([]),
    };
    let mut report = json!({"id":observation["id"],"role":"recovery","kind":kind,"task":target,
        "started":chrono::Utc::now().timestamp_millis(),"state":"awaiting-review","detail":observation["text"],
        "agent":r["input"]["coordinator"],"milestones":milestones,"source":source.map(|t| &t["id"]),
        "round":r["round"],"policy":r["policy"]});
    if let Some(t) = task {
        report["evidence"] = json!({"worker":t["worker"],"goal":t["goal"],"assigned_base":t["base"],"receipt":t["failed_receipt"],"previous_retries":num(&t["automatic_retries"])});
    } else if let Some(c) = candidate {
        report["evidence"] = json!({"candidate":c["id"],"commit":c["commit"],"state":c["state"]});
    } else if let Some(t) = a(&r["turns"]).last()
        && kind == "run-blocked"
        && matches!(s(t, "role"), "plan" | "repair")
        && s(observation, "text").starts_with(&format!("{} failed:", s(t, "role")))
        && t["state"] == "received"
    {
        report["source"] = t["id"].clone();
    }
    push(&mut r["failure_log"], report);
}

pub(super) fn decision(r: &mut Value, id: &str, state: &str, reason: &str, turn: &Value) {
    if let Some(i) = find(&r["failure_log"], id) {
        let entry = &mut r["failure_log"][i];
        entry["state"] = json!(state);
        entry["updated"] = json!(chrono::Utc::now().timestamp_millis());
        if state == "needs-attention" {
            entry["last_review_error"] = json!(reason);
        }
        entry["decision"] = json!(reason);
        if turn["id"].is_string() {
            entry["review_turn"] = turn["id"].clone();
        }
    }
}

pub(super) fn reviewed(r: &mut Value, batch: &[Value], actions: &[Value], turn: &Value) {
    for ob in batch {
        let id = s(ob, "id");
        if find(&r["failure_log"], id).is_none() {
            continue;
        }
        let mut notes = Vec::new();
        for action in actions.iter().filter(|action| {
            action["ref"] == ob["id"]
                || action["ref"] == ob["ref"]
                || a(&action["question"]["tasks"]).contains(&ob["ref"])
                || a(&action["tasks"])
                    .iter()
                    .any(|task| a(&task["depends"]).contains(&ob["ref"]))
                || (batch.len() == 1 && action["kind"] == "delegate")
        }) {
            let verb = s(action, "kind").replace('_', " ");
            let text = s(action, "text");
            let note = match s(action, "kind") {
                "ask" | "update_question" => format!("Asked: {}", s(&action["question"], "text")),
                "supersede_milestone" => {
                    format!("Replaced by {}. {text}", s(action, "replacement"))
                }
                "delegate" => format!(
                    "Assigned {}",
                    a(&action["tasks"])
                        .iter()
                        .map(|t| format!("{} ({})", s(t, "goal"), s(t, "id")))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                "assess_failure" => text.to_owned(),
                _ if !text.is_empty() => format!("{verb}: {text}"),
                _ => verb,
            };
            notes.push(note);
        }
        let decision_text = if notes.is_empty() {
            "Reviewed without a separate recovery action.".into()
        } else {
            notes.join("\n")
        };
        decision(r, id, "reviewed", &decision_text, turn);
    }
}

pub(super) fn finish_review(r: &mut Value, turn: &Value, reason: &str) {
    if let Some(id) = r
        .as_object_mut()
        .and_then(|fields| fields.remove("review_failure_id"))
        && let Some(id) = id.as_str()
    {
        decision(r, id, "reviewed", reason, turn);
    }
}

/// Index surviving evidence on completed older runs without scheduling a new review.
pub(crate) fn index_completed(r: &mut Value) -> bool {
    if r["state"] != "landed" || !r["failure_log"].is_null() {
        return false;
    }
    let mut entries = Vec::new();
    for task in a(&r["tasks"]) {
        let reason = s(task, "previous_failure");
        if reason.is_empty() {
            continue;
        }
        let id = s(task, "id");
        let replacement = s(task, "superseded_by");
        let outcome = if !replacement.is_empty() {
            format!("Superseded by {replacement}.")
        } else if task["state"] == "committed" {
            format!(
                "This milestone subsequently committed {}.",
                s(task, "commit")
            )
        } else {
            "No later decision was recorded.".into()
        };
        // A successful retry has a later turn. Only link an unambiguous failed receipt.
        let source = if task["state"] == "superseded" {
            a(&r["turns"])
                .iter()
                .rev()
                .find(|t| t["role"] == "worker" && t["task"] == id)
        } else {
            None
        };
        entries.push((
            json!({"id":uid(),"kind":"worker-blocked","ref":id,"text":reason}),
            outcome,
            source.cloned(),
        ));
    }
    for candidate in a(&r["candidates"])
        .iter()
        .filter(|c| matches!(s(c, "state"), "failed" | "inconclusive"))
    {
        let id = s(candidate, "id");
        let detail = s(candidate, "detail");
        let repair = a(&r["tasks"]).iter().find(|t| {
            !s(candidate, "commit").is_empty() && t["repair_base"] == candidate["commit"]
        });
        let outcome = repair
            .map(|t| format!("Repair milestone {} was {}.", s(t, "id"), s(t, "state")))
            .unwrap_or_else(|| "No linked repair was recorded.".to_owned());
        let source = a(&r["turns"])
            .iter()
            .rev()
            .find(|t| t["role"] == "eval" && t["task"] == id);
        entries.push((json!({"id":uid(),"kind":"candidate-failed","ref":id,"text":if detail.is_empty() {"Candidate checks did not pass."} else { detail }}),outcome,source.cloned()));
    }
    for (ob, outcome, source) in &entries {
        record(r, ob);
        let entry = r["failure_log"].as_array_mut().unwrap().last_mut().unwrap();
        entry["historical"] = json!(true);
        entry["state"] = json!("recorded");
        entry["decision"] = json!(outcome);
        entry["source"] = json!(source.as_ref().map(|t| &t["id"]));
        entry["started"] = json!(
            source
                .as_ref()
                .and_then(|t| t["finished"].as_i64().or_else(|| t["started"].as_i64()))
        );
    }
    !entries.is_empty()
}
