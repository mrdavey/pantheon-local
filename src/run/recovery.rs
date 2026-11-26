//! Durable failure reports and coordinator-directed milestone recovery.
use super::*;

/// Mark each failure episode once, including failures saved by older daemons.
pub(super) fn report_failures(r: &mut Value) -> bool {
    if s(r, "project").is_empty() || r["state"] == "stopped" {
        return false;
    }
    let start = num(&r["task_start"]) as usize;
    let round = r["round"].clone();
    let policy = r["policy"].clone();
    let mut reports = Vec::new();
    for (field, kind, detail) in [
        ("tasks", "worker-blocked", "finding"),
        ("candidates", "candidate-failed", "detail"),
        ("releases", "release-failed", "detail"),
        ("conversation", "chat-failed", "error"),
    ] {
        let offset = if field == "tasks" { start } else { 0 };
        if let Some(items) = r.get_mut(field).and_then(Value::as_array_mut) {
            for item in items.iter_mut().skip(offset) {
                if field == "candidates" && (item["round"] != round || item["policy"] != policy) {
                    continue;
                }
                if (field == "conversation" && !s(item, "error").is_empty())
                    || matches!(
                        s(item, "state"),
                        "blocked" | "failed" | "inconclusive" | "needs-rollback" | "uncertain"
                    )
                {
                    let episode = json!([item["state"], item[detail]]);
                    if item["reported_failure"] != episode {
                        let reason = s(item, detail).to_owned();
                        reports.push((kind, s(item, "id").to_owned(), reason.clone()));
                        item["reported_failure"] = episode;
                        if field == "tasks" {
                            item["previous_failure"] = json!(reason);
                        }
                    }
                } else if let Some(fields) = item.as_object_mut() {
                    fields.remove("reported_failure");
                }
            }
        }
    }
    if r["state"] == "blocked" {
        if r["reported_run_failure"].is_null() || s(r, "reported_run_failure") != s(r, "detail") {
            let detail = s(r, "detail").to_owned();
            reports.push(("run-blocked", s(r, "id").to_owned(), detail.clone()));
            r["reported_run_failure"] = json!(detail);
        }
    } else if let Some(fields) = r.as_object_mut() {
        fields.remove("reported_run_failure");
    }
    let changed = !reports.is_empty();
    if changed && !s(r, "observer_error").is_empty() {
        r["review_feedback"] = r["observer_error"].take();
    }
    for (kind, id, detail) in reports {
        if kind == "candidate-failed"
            && a(&r["observations"]).iter().any(|o| {
                o["ref"] == id
                    && matches!(s(o, "kind"), "eval-failed" | "eval-inconclusive")
                    && o["text"] == detail
            })
        {
            continue;
        }
        observe(r, kind, &id, &detail);
        if kind == "worker-blocked"
            && let Some(i) = find(&r["tasks"], &id)
        {
            r["tasks"][i]["failure_observation"] =
                a(&r["observations"]).last().unwrap()["id"].clone();
        }
    }
    changed
}

pub(super) fn recover(r: &mut Value, action: &Value, batch: &[Value]) -> Result<()> {
    let id = s(action, "ref");
    let i = index(&r["tasks"], id)?;
    let old = &r["tasks"][i];
    ensure!(
        batch
            .iter()
            .any(|o| o["kind"] == "worker-blocked" && o["ref"] == id),
        "recovery needs a current worker failure"
    );
    if old["state"] != "blocked"
        || i < num(&r["task_start"]) as usize
        || (old["failure_observation"].is_string()
            && !batch.iter().any(|o| o["id"] == old["failure_observation"]))
    {
        return Ok(());
    }
    let conflict = s(old, "conflict");
    let candidate = find(&r["candidates"], conflict).filter(|&i| {
        let c = &r["candidates"][i];
        !conflict.is_empty()
            && c["round"] == r["round"]
            && c["policy"] == r["policy"]
            && matches!(s(c, "state"), "conflict" | "failed")
            && !a(&c["pending"]).is_empty()
    });
    let retry_conflict =
        candidate.is_some() && matches!(s(r, "state"), "review" | "evaluating" | "blocked");
    ensure!(
        (r["state"] == "working"
            || retry_conflict
            || (r["state"] == "blocked" && super::commands::resume_phase(r) == "working"))
            && !held(r, id)
            && !waiting(r)
            && !super::scheduler::budget(r),
        "milestone is awaiting a run decision"
    );
    let instruction = s(action, "text").trim();
    ensure!(
        !instruction.is_empty() && instruction.len() <= 6000,
        "give concrete recovery instructions"
    );
    if action["kind"] == "retry_milestone" {
        ensure!(
            num(&old["automatic_retries"]) < 2,
            "retry limit reached; replace the milestone or ask the developer"
        );
        let t = &mut r["tasks"][i];
        t["reported_failure"] = json!(false);
        t["automatic_retries"] = json!(num(&t["automatic_retries"]) + 1);
        push(
            &mut t["notes"],
            json!(format!("Coordinator recovery instructions: {instruction}")),
        );
        t["state"] = json!("pending");
        if let Some(ci) = candidate {
            let c = &mut r["candidates"][ci];
            c["state"] = json!("conflict");
            c["detail"] = json!(strings(&c["conflict_paths"]).join(", "));
            r["state"] = json!("evaluating");
        } else {
            r["state"] = json!("working");
        }
        return Ok(());
    }
    supersede(r, id, s(action, "replacement"), instruction)
}

pub(super) fn supersede(
    r: &mut Value,
    id: &str,
    replacement: &str,
    instruction: &str,
) -> Result<()> {
    let i = index(&r["tasks"], id)?;
    let old = &r["tasks"][i];
    ensure!(
        old["state"] == "blocked"
            && i >= num(&r["task_start"]) as usize
            && (r["state"] == "working"
                || (r["state"] == "blocked" && super::commands::resume_phase(r) == "working"))
            && !held(r, id)
            && !waiting(r),
        "select a blocked milestone in the active round"
    );
    ensure!(
        !instruction.trim().is_empty() && instruction.len() <= 6000,
        "supply the reason for replacing this milestone"
    );
    let j = index(&r["tasks"], replacement)?;
    let other = &r["tasks"][j];
    ensure!(
        j >= num(&r["task_start"]) as usize
            && matches!(
                s(other, "state"),
                "pending" | "running" | "committed" | "question"
            )
            && ["variant", "alternative", "parent"]
                .iter()
                .all(|k| s(old, k) == s(other, k))
            && ["repair_base", "conflict"]
                .iter()
                .all(|k| s(old, k).is_empty() && s(other, k).is_empty())
            && !a(&r["tasks"]).iter().any(|t| t["parent"] == id)
            && a(&old["depends"])
                .iter()
                .all(|d| a(&other["depends"]).contains(d))
            && !experiment::inherits_cycle(a(&r["tasks"]), &[], replacement, id),
        "replacement must cover the same prerequisites and branch without depending on the blocked milestone"
    );
    let handoff = format!(
        "This milestone replaces {id}. Original goal: {}. Failure: {}. Coordinator instructions: {instruction}",
        s(old, "goal"),
        s(old, "previous_failure")
    );
    if other["state"] != "committed" {
        push(&mut r["tasks"][j]["notes"], json!(handoff));
    }
    let start = num(&r["task_start"]) as usize;
    for task in r["tasks"].as_array_mut().unwrap().iter_mut().skip(start) {
        if let Some(deps) = task["depends"].as_array_mut()
            && deps.iter().any(|dep| dep == id)
        {
            deps.retain(|dep| dep != id);
            if !deps.iter().any(|dep| dep == replacement) {
                deps.push(json!(replacement));
            }
        }
    }
    let old = &mut r["tasks"][i];
    old["state"] = json!("superseded");
    old["superseded_by"] = json!(replacement);
    push(&mut old["notes"], json!(instruction));
    r["state"] = json!("working");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_once_per_episode_and_discards_a_stale_replacement() -> Result<()> {
        let mut r = json!({"project":"p","state":"working","tasks":[{"id":"a","state":"blocked","finding":"missing receipt"}]});
        assert!(report_failures(&mut r));
        let first = r["observations"][0]["id"].clone();
        assert!(!report_failures(&mut r));
        r["tasks"][0]["state"] = json!("pending");
        report_failures(&mut r);
        r["tasks"][0]["state"] = json!("blocked");
        assert!(report_failures(&mut r));
        assert_ne!(first, r["tasks"][0]["failure_observation"]);
        let actions = json!({"actions":[{"id":"add","kind":"delegate","tasks":[{"id":"wrong","worker":"w","goal":"stale replacement"}]}]});
        experiment::finish_observation(
            &mut r,
            &json!({"task":first}),
            &json!({"text":actions.to_string()}),
        )?;
        assert_eq!(a(&r["tasks"]).len(), 1);
        assert_eq!(a(&r["observations"]).len(), 1);
        let latest = r["tasks"][0]["failure_observation"].clone();
        assert!(
            experiment::finish_observation(
                &mut r,
                &json!({"task":latest}),
                &json!({"text":"{\"actions\":[]}"})
            )
            .is_err()
        );
        Ok(())
    }
    #[test]
    fn rejects_replacement_cycles_and_changed_variant_or_prerequisites() {
        let base = json!({"state":"working","tasks":[{"id":"old","state":"blocked","depends":["base"]},{"id":"new","state":"pending"}]});
        let batch = [json!({"kind":"worker-blocked","ref":"old"})];
        let action = json!({"kind":"supersede_milestone","ref":"old","replacement":"new","text":"Fresh workspace"});
        for (deps, variant) in [
            (json!([]), ""),
            (json!(["base", "old"]), ""),
            (json!(["base"]), "other"),
        ] {
            let mut r = base.clone();
            r["tasks"][1]["depends"] = deps;
            r["tasks"][1]["variant"] = json!(variant);
            assert!(recover(&mut r, &action, &batch).is_err());
            assert_eq!(r["tasks"][0]["state"], "blocked");
        }
    }
}
