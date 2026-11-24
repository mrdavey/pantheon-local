use super::*;

pub(super) fn resume_phase(r: &Value) -> &str {
    if r["state"] == "stopped" {
        match s(r, "stopped_from") {
            p @ ("planning" | "question" | "checks" | "working" | "evaluating" | "review"
            | "repairing") => return p,
            "" => {
                if !s(r, "question").is_empty() {
                    return "question";
                }
                if !s(r, "repair_id").is_empty() {
                    return "repairing";
                }
                if a(&r["candidates"]).iter().any(|c| {
                    c["round"] == r["round"]
                        && c["policy"] == r["policy"]
                        && matches!(s(c, "state"), "queued" | "conflict" | "cancelled")
                }) {
                    return "evaluating";
                }
                if !r["selection"].is_null() {
                    return "review";
                }
            }
            _ => {}
        }
    }
    if active(r).is_empty() {
        "planning"
    } else if !yes(&r["recipe_approved"]) {
        "checks"
    } else {
        "working"
    }
}

impl Engine {
    pub(super) fn chat(&self, r: &mut Value, i: usize) -> Result<()> {
        let msg = &r["conversation"][i];
        let id = s(msg, "id").to_owned();
        let summary=a(&r["tasks"]).iter().map(|t|json!({"id":t["id"],"goal":t["goal"],"worker":t["worker"],"state":t["state"],"finding":t["finding"],"commit":t["commit"],"depends":t["depends"],"route":t["route"]})).collect::<Vec<_>>();
        let candidates = a(&r["candidates"])
            .iter()
            .map(|c| json!({"commit":c["commit"],"state":c["state"],"detail":c["detail"]}))
            .collect::<Vec<_>>();
        let history = a(&r["conversation"])
            .iter()
            .rev()
            .take(12)
            .filter(|m| m["id"] != id)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|m| format!("{}: {}", s(m, "role"), s(m, "text")))
            .collect::<Vec<_>>()
            .join("\n");
        let prompt = format!(
            "You are the Pantheon Local coordinator answering the developer. Explain the run from the milestone and candidate evidence. Use no commands or tools. Only if the developer asks to add work, propose new milestones with distinct IDs, goals, fallback workers from {}, classes, dependencies and path scopes, and additional argv acceptance checks. The developer decides whether to add them. Return only JSON {{\"reply\":\"answer\",\"tasks\":[],\"checks\":[]}}.\nRun state: {}\nOriginal brief: {}\nMilestones: {}\nCandidates: {}\nRecent conversation: {history}\nDeveloper message: {}",
            r["input"]["workers"],
            s(r, "state"),
            s(&r["input"], "prompt"),
            json!(summary),
            json!(candidates),
            s(msg, "text")
        );
        let req = json!({"profile":r["profiles"][s(&r["input"],"coordinator")],"role":"chat","dir":self.root(r).join("coordinator"),"prompt":prompt,"session":s(r,"chat_session")});
        r["conversation"][i]["state"] = json!("running");
        self.agent_turn(r, "chat", &id, req)
    }
    pub(super) fn finish_chat(&self, r: &mut Value, id: &str, res: &Value) {
        let Some(i) = find(&r["conversation"], id) else {
            return;
        };
        let result = adapter::decode_typed_outcome("chat", s(res, "text"));
        if !s(res, "error").is_empty()
            || result.as_ref().is_err()
            || result
                .as_ref()
                .is_ok_and(|v| s(v, "reply").trim().is_empty() || s(v, "reply").len() > 12000)
        {
            r["conversation"][i]["state"] = json!("failed");
            r["conversation"][i]["error"] = json!(if s(res, "error").is_empty() {
                "The coordinator did not return a usable reply."
            } else {
                s(res, "error")
            });
            return;
        }
        let Ok(data) = result else { return };
        r["conversation"][i]["state"] = json!("answered");
        r["chat_session"] = json!(s(res, "session"));
        let mut reply = json!({"id":uid(),"role":"coordinator","text":data["reply"]});
        if !a(&data["tasks"]).is_empty() {
            let validation =
                validate_tasks(a(&data["tasks"]), a(&r["tasks"]), &r["input"]["workers"]).and_then(
                    |_| {
                        if a(&data["checks"]).is_empty() {
                            Ok(())
                        } else {
                            eval::validate(&json!({"checks":data["checks"]}))
                        }
                    },
                );
            match validation {
                Ok(()) => {
                    reply["tasks"] = data["tasks"].clone();
                    reply["checks"] = data["checks"].clone();
                    reply["proposal_state"] = json!("pending")
                }
                Err(e) => {
                    reply["error"] =
                        json!(format!("The proposed milestones could not be added: {e}"))
                }
            }
        }
        push(&mut r["conversation"], reply)
    }
    fn accept_proposal(&self, r: &mut Value, id: &str) -> Result<()> {
        let i = index(&r["conversation"], id)?;
        let msg = r["conversation"][i].clone();
        ensure!(
            msg["proposal_state"] == "pending",
            "proposal is no longer open"
        );
        ensure!(
            matches!(s(r, "state"), "working" | "review") && s(r, "landing").is_empty(),
            "add milestones when the run is working or ready for review"
        );
        let mut tasks = a(&msg["tasks"]).to_vec();
        if let Some(repair) = a(&r["tasks"])
            .iter()
            .rev()
            .find(|t| !s(t, "repair_base").is_empty())
        {
            for t in &mut tasks {
                if !a(&t["depends"]).contains(&repair["id"]) {
                    push(&mut t["depends"], repair["id"].clone())
                }
            }
        }
        validate_tasks(&tasks, a(&r["tasks"]), &r["input"]["workers"])?;
        self.assign(r, &mut tasks);
        for task in tasks {
            push(&mut r["tasks"], task)
        }
        for check in a(&msg["checks"]) {
            push(&mut r["input"]["recipe"]["checks"], check.clone())
        }
        if !a(&msg["checks"]).is_empty() {
            r["policy"] = json!(num(&r["policy"]) + 1);
            policy(r)
        }
        experiment::archive_selection(r);
        if let Some(cs) = r["candidates"].as_array_mut() {
            for c in cs {
                if c["state"] == "passed" {
                    c["state"] = json!("superseded")
                }
            }
        }
        r["state"] = json!("working");
        r["conversation"][i]["proposal_state"] = json!("added");
        Ok(())
    }
    pub fn command(&self, id: &str, action: Value) -> Result<Value> {
        let mut r = self.store.get(id)?;
        let kind = s(&action, "kind");
        let cmd_id = s(&action, "id");
        let text = s(&action, "text");
        let task = s(&action, "task");
        if !cmd_id.is_empty() {
            ensure!(valid_id(cmd_id), "invalid command ID");
            let digest = crate::compat::command_digest(&action)?;
            if let Some(previous) = r["commands"][cmd_id].as_str() {
                ensure!(
                    previous == digest || previous == kind,
                    "command ID already used"
                );
                return Ok(r);
            }
            r["commands"][cmd_id] = json!(digest)
        }
        if let Some(expected) = action["expected_revision"].as_i64() {
            ensure!(
                num(&r["revision"]) == expected,
                "the run changed; review its current state and retry"
            )
        }
        if matches!(
            kind,
            "answer"
                | "answer-inbox"
                | "select"
                | "keep-baseline"
                | "combine"
                | "new-round"
                | "feedback"
                | "vote"
                | "change"
                | "approve"
                | "update-suite"
                | "land"
        ) {
            r["interventions"] = json!(num(&r["interventions"]) + 1)
        }
        if matches!(kind, "combine" | "feedback") && s(&r, "project").is_empty() {
            r["project"] = json!(self.store.project_id(s(&r["repo"], "Common"))?)
        }
        if kind == "new-round" {
            self.new_round(&mut r, text)?
        } else if kind == "update-suite" {
            self.update_suite(&mut r, &action)?
        } else if !experiment::command(&mut r, &action)? {
            ensure!(
                r["state"] != "landed" || matches!(kind, "land" | "chat" | "dismiss-proposal"),
                "run is already landed"
            );
            match kind {
                "chat" => {
                    let text = text.trim();
                    ensure!(
                        !text.is_empty() && text.len() <= 6000,
                        "write a message of 1 to 6,000 bytes"
                    );
                    ensure!(
                        r["state"] != "stopped",
                        "resume the run before sending a message"
                    );
                    ensure!(!budget(&r), "the run reached its token target");
                    ensure!(
                        a(&r["conversation"]).len() < 200,
                        "this run's conversation is full"
                    );
                    ensure!(
                        !a(&r["conversation"])
                            .iter()
                            .any(|m| matches!(s(m, "state"), "queued" | "running")),
                        "the coordinator is still answering"
                    );
                    push(
                        &mut r["conversation"],
                        json!({"id":uid(),"role":"user","text":text,"state":"queued"}),
                    );
                }
                "accept-proposal" => self.accept_proposal(&mut r, task)?,
                "dismiss-proposal" => {
                    let i = index(&r["conversation"], task)?;
                    ensure!(
                        r["conversation"][i]["proposal_state"] == "pending",
                        "proposal is no longer open"
                    );
                    r["conversation"][i]["proposal_state"] = json!("dismissed")
                }
                "approve" => {
                    ensure!(r["state"] == "checks", "no recipe waiting for approval");
                    let recipe = action
                        .get("recipe")
                        .filter(|v| !v.is_null())
                        .unwrap_or(&r["proposed_recipe"]);
                    r["input"]["recipe"] = self.pin_recipe(&r, recipe.clone())?;
                    r["recipe_approved"] = json!(true);
                    r["state"] = json!("working");
                    policy(&mut r)
                }
                "answer" => {
                    ensure!(!text.trim().is_empty(), "answer is required");
                    if task.is_empty() && r["state"] == "question" {
                        r["state"] = json!("planning");
                        r["question"] = json!("");
                        r["answer"] = json!(text);
                        for i in 0..a(&r["inbox"]).len() {
                            if r["inbox"][i]["state"] == "open" && r["inbox"][i]["source"] == "plan"
                            {
                                record_answer(&mut r, i, text)
                            }
                        }
                    } else {
                        let j = index(&r["tasks"], task)?;
                        ensure!(
                            r["tasks"][j]["state"] == "question",
                            "question is no longer open"
                        );
                        for i in 0..a(&r["inbox"]).len() {
                            if r["inbox"][i]["state"] == "open"
                                && a(&r["inbox"][i]["tasks"]).contains(&json!(task))
                            {
                                record_answer(&mut r, i, text)
                            }
                        }
                        r["tasks"][j]["state"] = json!("pending");
                        r["tasks"][j]["question"] = json!("");
                        r["tasks"][j]["reply"] = json!(text);
                    }
                }
                "coordinator" => {
                    if r["input"]["coordinator"] == text {
                        return Ok(r);
                    }
                    ensure!(!r["profiles"][text].is_null(), "profile unavailable");
                    r["generation"] = json!(num(&r["generation"]) + 1);
                    for key in [
                        "chat_session",
                        "observer_session",
                        "observer_error",
                        "session",
                    ] {
                        r[key] = json!("")
                    }
                    r["input"]["coordinator"] = json!(text);
                    if let Some(ms) = r["conversation"].as_array_mut() {
                        for m in ms {
                            if m["state"] == "running" {
                                m["state"] = json!("queued")
                            }
                        }
                    }
                    if r["state"] == "blocked" && a(&r["tasks"]).is_empty() {
                        r["state"] = json!("planning");
                        r["detail"] = json!("")
                    }
                }
                "stop" => {
                    if r["state"] != "stopped" {
                        r["stopped_from"] = r["state"].clone()
                    }
                    r["state"] = json!("stopped");
                    self.save(&mut r, "stop-requested")?;
                    for turn in a(&r["turns"]) {
                        if turn["state"] == "running" {
                            supervisor::stop_job(&self.jobdir(&r, s(turn, "id")))?
                        }
                    }
                }
                "resume" => {
                    ensure!(
                        matches!(s(&r, "state"), "stopped" | "blocked" | "budget" | "working"),
                        "run is already active or awaiting another action"
                    );
                    ensure!(
                        r["state"] != "stopped"
                            || !a(&r["turns"]).iter().any(|t| t["state"] == "running"),
                        "previous turns are still stopping"
                    );
                    let target = num(&action["tokens"]);
                    ensure!(target >= 0, "token target cannot be negative");
                    r["input"]["token_target"] = json!(target);
                    r["observer_error"] = json!("");
                    r["state"] = json!(resume_phase(&r));
                    r["stopped_from"] = json!("");
                    if r["state"] == "evaluating" {
                        for i in 0..a(&r["candidates"]).len() {
                            let c = &r["candidates"][i];
                            if c["round"] == r["round"]
                                && c["policy"] == r["policy"]
                                && c["state"] == "cancelled"
                            {
                                let id = uid();
                                let next = json!({"id":id,"base":c["base"],"tasks":c["tasks"],"state":"queued","round":c["round"],"policy":c["policy"],"name":c["name"],"baseline":c["baseline"],"parents":c["parents"]});
                                r["candidates"][i]["state"] = json!("superseded");
                                r["candidates"][i]["detail"] =
                                    json!(format!("Verification stopped. Retrying as {id}"));
                                push(&mut r["candidates"], next)
                            }
                        }
                    }
                    for i in num(&r["task_start"]) as usize..a(&r["tasks"]).len() {
                        let t = &mut r["tasks"][i];
                        if (task.is_empty() || t["id"] == task)
                            && matches!(s(t, "state"), "blocked" | "interrupted")
                        {
                            t["state"] = json!("pending")
                        }
                    }
                }
                "change" => {
                    ensure!(
                        r["state"] == "review" && !text.trim().is_empty(),
                        "select a reviewed candidate and describe the change"
                    );
                    let j = index(&r["candidates"], s(&action, "candidate"))?;
                    ensure!(
                        r["candidates"][j]["state"] == "passed",
                        "select a passing candidate"
                    );
                    if let Some(cs) = r["candidates"].as_array_mut() {
                        for c in cs {
                            if c["state"] == "passed" {
                                c["state"] = json!("superseded")
                            }
                        }
                    }
                    r["state"] = json!("repairing");
                    r["repair_id"] = action["candidate"].clone();
                    r["revision_goal"] = json!(text)
                }
                "land" => {
                    if s(&r["repo"], "Target").is_empty()
                        && r["state"] == "review"
                        && !text.is_empty()
                    {
                        let target =
                            format!("refs/heads/{}", text.trim_start_matches("refs/heads/"));
                        project::target_head(s(&r["repo"], "Root"), &target)?;
                        r["repo"]["Target"] = json!(target)
                    }
                    self.land(&mut r, s(&action, "candidate"))?;
                    return Ok(r);
                }
                _ => bail!("unknown action"),
            }
        }
        self.save(&mut r, kind)?;
        Ok(r)
    }
    fn update_suite(&self, r: &mut Value, action: &Value) -> Result<()> {
        ensure!(
            s(r, "landing").is_empty()
                && !matches!(s(r, "state"), "landed" | "planning" | "question" | "checks")
                && r["combination"].is_null(),
            "the eval suite cannot change during this stage"
        );
        ensure!(
            !a(&r["turns"])
                .iter()
                .any(|t| t["state"] == "running" && t["role"] == "eval"),
            "wait for the current evaluation to finish"
        );
        let recipe = action
            .get("recipe")
            .filter(|v| !v.is_null())
            .unwrap_or(&r["suite_proposal"]);
        ensure!(!recipe.is_null(), "supply a suite to approve");
        let pinned = self.pin_recipe(r, recipe.clone())?;
        experiment::archive_selection(r);
        r["policy"] = json!(num(&r["policy"]) + 1);
        r["input"]["recipe"] = pinned;
        r["suite_proposal"] = Value::Null;
        policy(r);
        if matches!(s(r, "state"), "evaluating" | "review")
            || r["state"] == "blocked" && committed(r)
        {
            self.candidates(r)?
        }
        Ok(())
    }
    fn new_round(&self, r: &mut Value, goal: &str) -> Result<()> {
        ensure!(
            s(r, "landing").is_empty()
                && r["combination"].is_null()
                && matches!(s(r, "state"), "review" | "landed" | "blocked")
                && !goal.trim().is_empty()
                && goal.len() <= 6000,
            "describe a next experiment after the current attempts finish"
        );
        ensure!(
            !a(&r["turns"])
                .iter()
                .any(|t| t["state"] == "running" && matches!(s(t, "role"), "worker" | "eval")),
            "wait for active workers and evaluators"
        );
        let repo = &r["repo"];
        let mut base = work_base(r).to_owned();
        if !s(repo, "Target").is_empty() {
            base = project::target_head(s(repo, "Root"), s(repo, "Target"))?;
            project::fetch(&self.integration(r), s(repo, "Root"), &base)?
        }
        experiment::archive_selection(r);
        r["round_started"] = json!(now());
        r["round"] = json!(num(&r["round"]) + 1);
        r["round_base"] = json!(base);
        r["round_goal"] = json!(goal);
        r["task_start"] = json!(a(&r["tasks"]).len());
        r["state"] = json!("planning");
        r["generation"] = json!(num(&r["generation"]) + 1);
        for key in [
            "detail",
            "question",
            "answer",
            "observer_error",
            "observer_session",
            "session",
            "chat_session",
        ] {
            r[key] = json!("")
        }
        r["observations"] = json!([]);
        if let Some(ms) = r["conversation"].as_array_mut() {
            for m in ms {
                if matches!(s(m, "state"), "queued" | "running") {
                    m["state"] = json!("failed");
                    m["error"] = json!("A new round started before the coordinator answered.")
                }
                if m["proposal_state"] == "pending" {
                    m["proposal_state"] = json!("dismissed")
                }
            }
        }
        if let Some(qs) = r["inbox"].as_array_mut() {
            for q in qs {
                if q["state"] == "open" {
                    q["state"] = json!("superseded")
                }
            }
        }
        Ok(())
    }
}
