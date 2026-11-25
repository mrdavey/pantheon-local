use super::*;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum Phase {
    Planning,
    Question,
    Checks,
    Working,
    Evaluating,
    Repairing,
    Review,
    Budget,
    Blocked,
    Stopped,
    Landed,
}
impl Phase {
    fn read(run: &Value) -> Result<Self> {
        use serde::Deserialize;
        Phase::deserialize(&run["state"])
            .with_context(|| format!("unsupported run phase {}", run["state"]))
    }
}

// Index dependencies once per scheduling pass. Each selected task gets its own
// checkout; tasks that depend on a recovered commit wake on the next pass.
pub(super) fn ready_workers(run: &Value) -> Vec<usize> {
    let tasks = a(&run["tasks"]);
    let committed: HashSet<_> = tasks
        .iter()
        .filter(|task| task["state"] == "committed")
        .map(|task| s(task, "id"))
        .collect();
    let mut held: HashSet<&str> = a(&run["inbox"])
        .iter()
        .filter(|question| question["state"] == "open" && yes(&question["blocking"]))
        .flat_map(|question| a(&question["tasks"]).iter().filter_map(Value::as_str))
        .collect();
    held.extend(
        active(run)
            .iter()
            .filter(|task| !matches!(s(task, "state"), "committed" | "abandoned"))
            .filter_map(|task| task["parent"].as_str()),
    );
    let running_children = active(run)
        .iter()
        .filter(|t| t["state"] == "running" && !s(t, "parent").is_empty())
        .count();
    let mut slots = run["input"]["subagent_limit"]
        .as_u64()
        .unwrap_or(u64::MAX)
        .saturating_sub(running_children as u64);
    tasks
        .iter()
        .enumerate()
        .skip(num(&run["task_start"]) as usize)
        .filter(|(_, task)| {
            task["state"] == "pending"
                && !held.contains(s(task, "id"))
                && a(&task["depends"])
                    .iter()
                    .all(|dep| dep.as_str().is_some_and(|id| committed.contains(id)))
        })
        .filter(|(_, t)| {
            if s(t, "parent").is_empty() {
                return true;
            }
            if slots == 0 {
                return false;
            }
            slots -= 1;
            true
        })
        .map(|(i, _)| i)
        .collect()
}
pub(super) fn budget(r: &Value) -> bool {
    let limit = num(&r["input"]["token_target"]);
    limit > 0 && num(&r["tokens"]) >= limit
}
impl Engine {
    pub fn tick(&self) -> Result<()> {
        for mut r in self.store.scheduled_runs()? {
            if let Err(error) = Phase::read(&r) {
                // Preserve a newer runtime's phase. A downgrade must never launch
                // work or reconcile a side effect it cannot interpret.
                let message = error.to_string();
                if r["detail"] != message {
                    r["detail"] = json!(message);
                    self.save(&mut r, "unrecognized-run-phase")?;
                }
                continue;
            }
            let snapshot = r.clone();
            if let Err(e) = self.step(&mut r) {
                eprintln!("run {}: {e:#}", s(&r, "id"));
                if r["revision"] == snapshot["revision"] {
                    r = snapshot;
                }
                let message = e.to_string();
                match s(&r, "state") {
                    "stopped" => {
                        if r["detail"] != message {
                            r["detail"] = json!(message);
                            self.save(&mut r, "stop-error")?;
                        }
                    }
                    "landed" => {
                        let mut changed = false;
                        if let Some(releases) = r["releases"].as_array_mut() {
                            for rel in releases {
                                if matches!(
                                    s(rel, "state"),
                                    "queued" | "built" | "staged" | "needs-rollback" | "reconcile"
                                ) && rel["held"] != true
                                {
                                    rel["state"] = json!("failed");
                                    rel["detail"] = json!(message);
                                    changed = true;
                                    break;
                                }
                            }
                        }
                        if !changed && r["observer_error"] != message {
                            r["observer_error"] = json!(message);
                            changed = true;
                        }
                        if changed {
                            self.save(&mut r, "post-land-error")?;
                        }
                    }
                    "blocked" => {
                        if r["detail"] != message {
                            r["detail"] = json!(message);
                            self.save(&mut r, "blocked")?;
                        }
                    }
                    _ => {
                        r["state"] = json!("blocked");
                        r["detail"] = json!(message);
                        self.save(&mut r, "blocked")?;
                    }
                }
            }
        }
        Ok(())
    }
    fn step(&self, r: &mut Value) -> Result<()> {
        let landing = s(r, "landing");
        if !landing.is_empty() {
            let c = r["candidates"][index(&r["candidates"], landing)?].clone();
            let head = match project::target_head(s(&r["repo"], "Root"), s(&r["repo"], "Target")) {
                Ok(head) => head,
                Err(error) => return self.landing_wait(r, error),
            };
            r["landing"] = json!("");
            if c["commit"] == head {
                candidate::mark_landed(r, &c);
                return self.save(r, "landing-recovered");
            }
            r["held"] = json!(true);
            if r["state"] == "blocked" {
                r["state"] = json!("review");
            }
            r["detail"] = json!("Review the target branch before landing.");
            return self.save(r, "landing-needs-review");
        }
        let (mut workers, mut control, mut chat) = (false, false, false);
        for i in 0..a(&r["turns"]).len() {
            if r["turns"][i]["state"] != "running" {
                continue;
            }
            let turn = r["turns"][i].clone();
            let role = s(&turn, "role");
            let task = s(&turn, "task");
            let dir = self.jobdir(r, s(&turn, "id"));
            if r["state"] == "stopped" {
                supervisor::stop_job(&dir)?
            }
            let current = turn["generation"] == r["generation"];
            let Some(res) = supervisor::reception(&dir) else {
                if role == "worker" {
                    workers = true
                } else if current {
                    if role == "chat" {
                        chat = true
                    } else {
                        control = true
                    }
                }
                continue;
            };
            r["turns"][i]["state"] = json!("received");
            if role == "release" {
                release::finish(r, task, &res)?;
                return self.save(r, "release-finished");
            }
            if role != "eval" {
                if let Some(n) = res["tokens"].as_i64() {
                    r["tokens"] = json!(num(&r["tokens"]).saturating_add(n))
                } else {
                    r["unknown_turns"] = json!(num(&r["unknown_turns"]) + 1)
                }
            }
            if role == "worker"
                && s(&res, "error").is_empty()
                && let Some(j) = find(&r["tasks"], task)
            {
                let t = &mut r["tasks"][j];
                let count = num(&turn["messages"]);
                if count >= 0 && count as usize <= a(&t["notes"]).len() {
                    t["delivered"] = json!(num(&t["delivered"]).max(count))
                }
            }
            if r["state"] != "stopped" && (matches!(role, "worker" | "eval") || current) {
                match role {
                    "observe" => {
                        let mut next = r.clone();
                        let n = a(&r["tasks"]).len();
                        let result = experiment::finish_observation(&mut next, &turn, &res)
                            .and_then(|cases| {
                                let indices = (0..a(&next["tasks"]).len())
                                    .filter(|&i| {
                                        i >= n
                                            || [
                                                "worker",
                                                "class",
                                                "eligible_workers",
                                                "goal",
                                                "paths",
                                                "depends",
                                                "consult",
                                            ]
                                            .iter()
                                            .any(|key| r["tasks"][i][key] != next["tasks"][i][key])
                                    })
                                    .collect::<Vec<_>>();
                                let mut assignments = indices
                                    .iter()
                                    .map(|&i| next["tasks"][i].clone())
                                    .collect::<Vec<_>>();
                                self.assign(&next, &mut assignments);
                                for (i, task) in indices.into_iter().zip(assignments) {
                                    next["tasks"][i] = task;
                                }
                                self.store.save(&mut next, "coordinator-observed", &cases)
                            });
                        if let Err(e) = result {
                            r["observer_error"] = json!(e.to_string());
                            return self.save(r, "observer-error");
                        }
                        *r = next;
                        return Ok(());
                    }
                    "worker" => self.accept_worker(r, &turn, &res)?,
                    "eval" => self.eval_result(r, &turn, &res)?,
                    "chat" => self.finish_chat(r, task, &res),
                    _ => self.accept_control(r, role, &res)?,
                }
            } else if role == "worker"
                && let Some(j) = find(&r["tasks"], task)
            {
                r["tasks"][j]["state"] = json!("interrupted");
                if !s(&res, "session").is_empty() {
                    r["tasks"][j]["session"] = res["session"].clone()
                }
            } else if role == "eval"
                && let Some(j) = find(&r["candidates"], task)
            {
                r["candidates"][j]["state"] = json!("cancelled");
                r["candidates"][j]["detail"] = json!("verification stopped")
            } else if role == "chat" && current {
                self.finish_chat(
                    r,
                    task,
                    &json!({"error":"Conversation paused when the run stopped."}),
                )
            }
            self.save(r, "received")?
        }
        if !chat
            && !control
            && !matches!(s(r, "state"), "stopped" | "planning" | "repairing")
            && let Some(i) = a(&r["conversation"])
                .iter()
                .position(|m| m["state"] == "queued")
        {
            if budget(r) {
                r["conversation"][i]["state"] = json!("failed");
                r["conversation"][i]["error"] = json!("The run reached its token target.");
                return self.save(r, "conversation-paused");
            }
            return self.chat(r, i);
        }
        let obs = a(&r["observations"]);
        let ready = !obs.is_empty()
            && (!workers
                || r["state"] != "working"
                || obs.len() >= 30
                || obs.iter().any(|o| o["kind"] != "worker-completed"));
        if !chat
            && !control
            && ready
            && s(r, "observer_error").is_empty()
            && !budget(r)
            && !matches!(
                s(r, "state"),
                "stopped" | "planning" | "checks" | "repairing"
            )
        {
            let cases = self.store.cases(s(r, "project"), 0, false)?;
            let text = experiment::observer_prompt(r, &cases, &self.root(r));
            let id = s(&obs[obs.len().min(30) - 1], "id").to_owned();
            let req = json!({"profile":r["profiles"][s(&r["input"],"coordinator")],"role":"observer","dir":self.root(r).join("coordinator"),"prompt":text,"session":s(r,"observer_session")});
            return self.agent_turn(r, "observe", &id, req);
        }
        if !a(&r["releases"]).is_empty() {
            let mut staged = r.clone();
            let root = self.root(&staged);
            if let Some((id, job)) = release::next_job(&mut staged, || self.store.list(), &root)? {
                let updated = staged["releases"].clone();
                return self.dispatch_with(r, "release", &id, job, |r| {
                    r["releases"] = updated;
                });
            }
            if staged["releases"] != r["releases"] {
                r["releases"] = staged["releases"].clone();
                self.save(r, "release-updated")?
            }
        }
        let phase = Phase::read(r)?;
        match phase {
            Phase::Review => self.maybe_land(r),
            Phase::Evaluating => {
                if !control {
                    self.promote_worker_questions(r)?;
                }
                self.candidate_step(r)
            }
            Phase::Stopped | Phase::Landed | Phase::Blocked | Phase::Question | Phase::Checks => {
                Ok(())
            }
            Phase::Working if !workers && committed(r) => {
                // The last handoff review may still assign a corrective milestone.
                if control || chat {
                    return Ok(());
                }
                self.candidates(r)
            }
            Phase::Planning | Phase::Working | Phase::Repairing if budget(r) => {
                r["state"] = json!("budget");
                self.save(r, "token-target")
            }
            Phase::Planning if !control => self.plan(r),
            Phase::Repairing if !control => self.repair(r),
            Phase::Working => {
                if !control {
                    self.promote_worker_questions(r)?;
                }
                for i in ready_workers(r) {
                    self.worker(r, i)?;
                }
                Ok(())
            }
            Phase::Planning | Phase::Repairing | Phase::Budget => Ok(()),
        }
    }
    fn promote_worker_questions(&self, r: &mut Value) -> Result<()> {
        if s(r, "project").is_empty() {
            return Ok(());
        }
        let mut additions = vec![];
        for t in active(r) {
            let id = s(t, "id");
            if t["state"] == "question"
                && !s(t, "question").trim().is_empty()
                && !children_pending(r, id)
                && !a(&r["observations"])
                    .iter()
                    .any(|o| o["kind"] == "worker-question" && o["ref"] == id)
                && !a(&r["inbox"])
                    .iter()
                    .any(|q| q["state"] == "open" && a(&q["tasks"]).contains(&t["id"]))
            {
                additions.push(json!({"id":uid(),"text":t["question"],"source":"worker","tasks":[id],"blocking":true,"revision":1,"state":"open"}))
            }
        }
        if !additions.is_empty() {
            for q in additions {
                push(&mut r["inbox"], q)
            }
            self.save(r, "worker-needs-answer")?;
        }
        Ok(())
    }
    fn accept_control(&self, r: &mut Value, role: &str, res: &Value) -> Result<()> {
        ensure!(s(res, "error").is_empty(), "{}", s(res, "error"));
        let out = adapter::decode_typed_outcome(role, s(res, "text"))?;
        r["session"] = json!(s(res, "session"));
        match role {
            "plan" => {
                record_title(r, &out);
                let q = s(&out, "question").trim();
                if !q.is_empty() {
                    r["state"] = json!("question");
                    r["question"] = json!(q);
                    push(
                        &mut r["inbox"],
                        json!({"id":uid(),"text":q,"source":"plan","state":"open","revision":1,"blocking":true}),
                    );
                    return Ok(());
                }
                let mut tasks = a(&out["tasks"]).to_vec();
                let n = num(&r["task_start"]) as usize;
                let mut prev = a(&r["tasks"])
                    .get(..n)
                    .context("invalid round task offset")?
                    .to_vec();
                validate_tasks(&tasks, &prev, &r["input"]["workers"])?;
                if yes(&r["input"]["compare"]) {
                    for t in &mut tasks {
                        t["round"] = r["round"].clone()
                    }
                }
                self.assign(r, &mut tasks);
                prev.extend(tasks);
                r["tasks"] = json!(prev);
                r["state"] = json!("working");
                if !yes(&r["recipe_approved"]) {
                    eval::validate(&out["recipe"])?;
                    r["proposed_recipe"] = out["recipe"].clone();
                    r["state"] = json!("checks");
                }
            }
            "repair" => {
                let old = &r["tasks"][index(&r["tasks"], s(&out, "task"))?];
                let c = &r["candidates"][index(&r["candidates"], s(r, "repair_id"))?];
                ensure!(
                    !s(c, "commit").is_empty() && !s(&out, "instruction").trim().is_empty(),
                    "invalid repair assignment"
                );
                let mut tasks = [
                    json!({"id":format!("repair-{}",uid()),"worker":old["worker"],"goal":out["instruction"],"class":"debugging","repair_base":c["commit"]}),
                ];
                self.assign(r, &mut tasks);
                for t in tasks {
                    push(&mut r["tasks"], t)
                }
                r["state"] = json!("working");
                r["repair_id"] = json!("");
                r["revision_goal"] = json!("");
            }
            _ => bail!("unknown turn role {role}"),
        }
        Ok(())
    }
    fn landing_wait(&self, r: &mut Value, error: anyhow::Error) -> Result<()> {
        let message = error.to_string();
        if r["detail"] != message {
            r["detail"] = json!(message);
            self.save(r, "landing-wait")?;
        }
        Ok(())
    }
    fn maybe_land(&self, r: &mut Value) -> Result<()> {
        let pending_observations = !a(&r["observations"]).is_empty()
            && (!budget(r)
                || a(&r["observations"]).iter().any(|o| {
                    !matches!(
                        s(o, "kind"),
                        "worker-completed" | "comparison-ready" | "eval-passed" | "eval-complete"
                    )
                }));
        if matches!(s(&r["input"], "delivery"), "" | "review")
            || yes(&r["held"])
            || waiting(r)
            || !r["combination"].is_null()
            || !s(r, "observer_error").is_empty()
            || pending_observations
            || a(&r["conversation"]).iter().any(|m| {
                matches!(s(m, "state"), "running" | "queued") || m["proposal_state"] == "pending"
            })
        {
            return Ok(());
        }
        let sel = &r["selection"];
        match s(sel, "state") {
            "recommended" => {
                r["selection"]["candidate"] = sel["recommended"].clone();
                r["selection"]["state"] = json!("selected");
                self.save(r, "selected")
            }
            "selected"
                if sel["round"] == r["round"]
                    && sel["policy"] == r["policy"]
                    && !s(&r["repo"], "Target").is_empty() =>
            {
                let id = s(sel, "candidate").to_owned();
                let candidate = &r["candidates"][index(&r["candidates"], &id)?];
                if a(&candidate["tasks"])
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|task| held(r, task))
                {
                    return Ok(());
                }
                if let Err(e) = self.land(r, &id, None) {
                    return self.landing_wait(r, e);
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    fn repair(&self, r: &mut Value) -> Result<()> {
        let c = &r["candidates"][index(&r["candidates"], s(r, "repair_id"))?];
        let log = supervisor::log_tail(Path::new(s(c, "log")), 12_000).unwrap_or_default();
        let prompt = format!(
            "Revise the integrated candidate {}. Select a worker by milestone ID and return only JSON {{\"task\":\"milestone-id\",\"instruction\":\"specific implementation goal\"}}. Make a normal commit on this candidate.\nBrief: {}\nDeveloper request: {}\nMilestones: {}\nEval error: {}\nEval log:\n{}",
            s(c, "commit"),
            s(&r["input"], "prompt"),
            s(r, "revision_goal"),
            r["tasks"],
            s(c, "detail"),
            log
        );
        let req = json!({"profile":r["profiles"][s(&r["input"],"coordinator")],"role":"coordinator","dir":self.integration(r),"prompt":prompt,"session":s(r,"session")});
        self.agent_turn(r, "repair", "", req)
    }
}
