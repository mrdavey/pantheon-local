use crate::{adapter, eval, project, routing, store::Store, supervisor, uid};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

pub mod comparison;
pub mod experiment;
pub mod release;
use scheduler::budget;
mod candidate;
mod commands;
pub(crate) mod failure_log;
mod pool;
mod recovery;
mod scheduler;
#[cfg(test)]
mod tests;
mod worker;

pub struct Engine {
    pub dir: PathBuf,
    pub store: Store,
}
pub fn s<'v>(v: &'v Value, key: &str) -> &'v str {
    v[key].as_str().unwrap_or_default()
}
fn record_title(run: &mut Value, outcome: &Value) {
    if !s(run, "title").is_empty() {
        return;
    }
    let title = s(outcome, "title")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if !title.is_empty() && title.chars().count() <= 80 {
        run["title"] = json!(title);
    }
}
pub fn a(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}
pub fn num(v: &Value) -> i64 {
    v.as_i64().unwrap_or_default()
}
fn yes(v: &Value) -> bool {
    v.as_bool().unwrap_or(false)
}
fn strings(v: &Value) -> Vec<String> {
    a(v).iter()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect()
}
pub fn push(v: &mut Value, entry: Value) {
    if let Some(items) = v.as_array_mut() {
        items.push(entry)
    } else {
        *v = json!([entry])
    }
}
pub fn find(v: &Value, id: &str) -> Option<usize> {
    a(v).iter().position(|x| x["id"] == id)
}
fn index(v: &Value, id: &str) -> Result<usize> {
    find(v, id).with_context(|| format!("missing item {id}"))
}
fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
fn read(path: impl AsRef<Path>) -> Result<Value> {
    Ok(serde_json::from_reader(fs::File::open(path)?)?)
}
fn active(r: &Value) -> &[Value] {
    a(&r["tasks"])
        .get(num(&r["task_start"]) as usize..)
        .unwrap_or_default()
}
fn work_base(r: &Value) -> &str {
    r["round_base"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(s(&r["repo"], "Head"))
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 48
        && !id.starts_with('-')
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
pub fn observe(r: &mut Value, kind: &str, id: &str, text: &str) {
    if !s(r, "project").is_empty() {
        let observation = json!({"id":uid(),"kind":kind,"ref":id,"text":text});
        failure_log::record(r, &observation);
        push(&mut r["observations"], observation)
    }
}
pub fn held(r: &Value, id: &str) -> bool {
    a(&r["inbox"])
        .iter()
        .any(|q| q["state"] == "open" && yes(&q["blocking"]) && a(&q["tasks"]).contains(&json!(id)))
}
fn waiting(r: &Value) -> bool {
    a(&r["inbox"])
        .iter()
        .any(|q| q["state"] == "open" && yes(&q["blocking"]) && a(&q["tasks"]).is_empty())
}
fn children_pending(r: &Value, id: &str) -> bool {
    active(r).iter().any(|t| {
        t["parent"] == id && !matches!(s(t, "state"), "committed" | "abandoned" | "superseded")
    })
}
fn committed(r: &Value) -> bool {
    !active(r).is_empty()
        && active(r)
            .iter()
            .all(|t| matches!(s(t, "state"), "committed" | "abandoned" | "superseded"))
}
pub fn record_answer(r: &mut Value, q: usize, text: &str) {
    let ids = strings(&r["inbox"][q]["tasks"]);
    let question = s(&r["inbox"][q], "text").to_owned();
    r["inbox"][q]["state"] = json!("answered");
    r["inbox"][q]["answer"] = json!(text);
    for id in ids {
        if let Some(i) = find(&r["tasks"], &id) {
            let t = &mut r["tasks"][i];
            if t["state"] == "blocked" {
                t["reported_failure"] = Value::Null;
            }
            match s(t, "state") {
                "question" => {
                    t["state"] = json!("pending");
                    t["question"] = json!("");
                    t["reply"] = json!(text)
                }
                "running" | "pending" | "blocked" | "interrupted" => push(
                    &mut t["notes"],
                    json!(format!("Developer answered {question} — {text}")),
                ),
                _ => {}
            }
        }
    }
}
pub fn validate_tasks(added: &[Value], existing: &[Value], workers: &Value) -> Result<()> {
    ensure!(!added.is_empty(), "supply at least one milestone");
    let mut tasks = HashMap::new();
    for t in existing {
        tasks.insert(s(t, "id"), t);
    }
    for t in added {
        let id = s(t, "id");
        ensure!(
            valid_id(id)
                && !tasks.contains_key(id)
                && a(workers).contains(&t["worker"])
                && !s(t, "goal").trim().is_empty(),
            "invalid milestone {id:?}"
        );
        ensure!(
            routing::class_valid(s(t, "class")),
            "invalid benchmark category; choose implementation, debugging, testing, automation, documentation or design, or omit class"
        );
        let eligible = a(&t["eligible_workers"]);
        ensure!(
            eligible.is_empty()
                || (eligible.contains(&t["worker"])
                    && eligible.iter().all(|name| a(workers).contains(name))),
            "eligible workers must include the assigned worker and belong to this team"
        );
        for k in ["variant", "alternative"] {
            ensure!(
                s(t, k).is_empty() || valid_id(s(t, k)),
                "invalid variant or alternative"
            );
        }
        ensure!(
            s(t, "variant").is_empty() || s(t, "alternative").is_empty(),
            "use a variant or a legacy alternative group"
        );
        for p in a(&t["paths"]) {
            let path = Path::new(p.as_str().context("invalid scope")?);
            ensure!(
                path.components()
                    .any(|c| matches!(c, std::path::Component::Normal(_)))
                    && path.components().all(|c| matches!(
                        c,
                        std::path::Component::Normal(_) | std::path::Component::CurDir
                    )),
                "invalid repo-relative milestone path"
            );
        }
        for k in ["round", "delivered"] {
            ensure!(num(&t[k]) == 0, "service-owned milestone fields");
        }
        for k in [
            "parent",
            "conflict",
            "state",
            "workspace",
            "commit",
            "session",
            "base",
            "question",
            "reply",
            "finding",
            "repair_base",
        ] {
            ensure!(s(t, k).is_empty(), "service-owned milestone fields");
        }
        ensure!(
            a(&t["notes"]).is_empty() && a(&t["donors"]).is_empty() && t["route"].is_null(),
            "service-owned milestone fields"
        );
        tasks.insert(id, t);
    }
    let mut remaining = HashMap::new();
    let mut outgoing: HashMap<&str, Vec<&str>> = HashMap::new();
    for (&id, t) in &tasks {
        let deps = a(&t["depends"]);
        remaining.insert(id, deps.len());
        for d in deps {
            let dep = d.as_str().context("invalid dependency")?;
            let other = tasks.get(dep).context("unavailable dependency")?;
            ensure!(
                s(other, "alternative").is_empty()
                    && (s(other, "variant").is_empty() || other["variant"] == t["variant"]),
                "unavailable dependency {dep}"
            );
            outgoing.entry(dep).or_default().push(id);
        }
    }
    let mut ready: Vec<_> = remaining
        .iter()
        .filter_map(|(&id, &n)| (n == 0).then_some(id))
        .collect();
    let mut seen = 0;
    while let Some(id) = ready.pop() {
        seen += 1;
        for child in outgoing.get(id).into_iter().flatten() {
            if let Some(n) = remaining.get_mut(child) {
                *n -= 1;
                if *n == 0 {
                    ready.push(child)
                }
            }
        }
    }
    ensure!(seen == tasks.len(), "dependency cycle");
    Ok(())
}
fn policy(r: &mut Value) {
    let p = json!({"revision":r["policy"],"rule":r["input"]["recipe"]["rule"],"checks":r["input"]["recipe"]["checks"],"at":now()});
    push(&mut r["policies"], p)
}
fn context(r: &Value, worker: Option<&Value>) -> Value {
    let mut decisions: Vec<_> = a(&r["inbox"])
        .iter()
        .filter(|q| q["source"] == "plan" && q["state"] == "answered")
        .map(|q| json!({"question":q["text"],"answer":q["answer"]}))
        .collect();
    if decisions.is_empty() && !s(r, "answer").is_empty() {
        decisions.push(json!({"answer":r["answer"]}))
    }
    let mut c = json!({"planning_decisions":decisions,"round_goal":r["round_goal"],"checks_approved":r["recipe_approved"]});
    c["minimum_workers"] = json!(r["input"]["min_workers"].as_u64().unwrap_or(1));
    c["team"] = json!(strings(&r["input"]["workers"]).iter().map(|name| {
        let p = &r["profiles"][name];
        let effective = |key: &str| p[key].as_str().filter(|s| !s.is_empty()).or_else(|| p["detected"][key].as_str());
        json!({"profile":name,"harness":p["kind"],"model":effective("model").or(p["detected"]["label"].as_str()),"thinking_effort":effective("effort")})
    }).collect::<Vec<_>>());
    c["delegation"] = json!({"allowed":r["input"]["subagents"] != false,"max_concurrent_children":r["input"]["subagent_limit"]});
    if yes(&r["recipe_approved"]) {
        for (dest, src) in [
            ("checks", "checks"),
            ("selection_rule", "rule"),
            ("review_criteria", "rubric"),
        ] {
            c[dest] = r["input"]["recipe"][src].clone()
        }
    }
    if let Some(w) = worker {
        c["peer_milestones"] = json!(
            active(r)
                .iter()
                .filter(|t| t["id"] != w["id"]
                    && s(t, "variant") == s(w, "variant")
                    && s(t, "alternative").is_empty()
                    && matches!(s(t, "state"), "running" | "pending" | "question"))
                .map(|t| json!({"id":t["id"],"goal":t["goal"],"paths":t["paths"]}))
                .collect::<Vec<_>>()
        );
    }
    c
}
impl Engine {
    pub fn root(&self, r: &Value) -> PathBuf {
        self.dir.join("runs").join(s(r, "id"))
    }
    fn integration(&self, r: &Value) -> String {
        self.root(r)
            .join("integration")
            .to_string_lossy()
            .into_owned()
    }
    fn jobdir(&self, r: &Value, id: &str) -> PathBuf {
        self.root(r).join("turns").join(id)
    }
    fn save(&self, r: &mut Value, kind: &str) -> Result<()> {
        recovery::report_failures(r);
        self.store.save(r, kind, &[])
    }
    pub fn assign(&self, r: &Value, tasks: &mut [Value]) {
        let catalog = read(self.root(r).join("benchmarks.json"))
            .unwrap_or_else(|_| json!({"note":"The run's benchmark snapshot is unavailable."}));
        let workers = strings(&r["input"]["workers"]);
        let assigned_at = now();
        for t in tasks {
            t["assigned_at"] = json!(assigned_at);
            let eligible = strings(&t["eligible_workers"]);
            let pool = if eligible.is_empty() {
                &workers
            } else {
                &eligible
            };
            let (w, route) = routing::choose(
                s(&r["input"], "routing"),
                s(t, "class"),
                s(t, "worker"),
                pool,
                &r["profiles"],
                &catalog,
            );
            t["worker"] = json!(w);
            if let Some(route) = route {
                t["route"] = route
            }
            t["state"] = json!("pending");
        }
    }
    fn activate_initial_recipe(&self, r: &mut Value, recipe: Value) -> Result<()> {
        r["input"]["recipe"] = self.pin_recipe(r, recipe)?;
        r["recipe_approved"] = json!(true);
        r["proposed_recipe"] = Value::Null;
        policy(r);
        Ok(())
    }
    fn pin_recipe(&self, r: &Value, mut recipe: Value) -> Result<Value> {
        eval::validate(&recipe)?;
        if r["input"]["verification_of"].is_object() {
            ensure!(
                !a(&recipe["checks"]).is_empty() || !a(&recipe["cases"]).is_empty(),
                "correctness runs require an executable acceptance check"
            );
        }
        if let Some(cases) = recipe["cases"].as_array_mut() {
            for c in cases {
                let rev = num(&c["revision"]);
                if rev > 0 {
                    let entry = self.store.case_revision(s(r, "project"), s(c, "id"), rev)?;
                    ensure!(entry["state"] != "retired", "case revision is retired");
                    for (k, source) in [("input", "scenario"), ("expected", "expected")] {
                        if c[k].is_null() {
                            c[k] = entry[source].clone()
                        }
                    }
                }
            }
        }
        eval::pin(&recipe, &self.root(r).join("evaluators"))
    }
    pub fn verify_implementation(&self, run_id: &str, candidate_id: &str) -> Result<Value> {
        let source = experiment::public(self.store.get(run_id)?);
        ensure!(
            strings(&source["landed_implementation_ids"])
                .iter()
                .any(|id| id == candidate_id),
            "choose a landed implementation to verify"
        );
        let candidate = &source["candidates"][index(&source["candidates"], candidate_id)?];
        let original = &source["input"];
        let objective = original["verification_of"]["objective"]
            .as_str()
            .unwrap_or(s(original, "prompt"));
        let project = project::subdir(
            Path::new(s(&source["repo"], "Root")),
            s(&source["repo"], "Subdir"),
        )?;
        let branch = s(&source["repo"], "Target");
        ensure!(
            !branch.is_empty(),
            "this implementation has no recorded landing branch"
        );
        let goals = strings(&candidate["tasks"])
            .iter()
            .filter_map(|id| find(&source["tasks"], id))
            .map(|i| s(&source["tasks"][i], "goal"))
            .filter(|goal| !goal.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let brief = format!(
            "Find and fix bugs in the landed implementation.\n\nOriginal objective: {objective}\n\nLanded milestones:\n{goals}\n\nReview the current project against that objective. Divide independent checks across the team. Exercise the real entry points, integration paths, edge cases and existing test suite. Reproduce observed defects, fix their causes, and add regression tests where needed. Run the relevant tests and build on the integrated result. Report fixes, checks and outcomes, and anything you could not verify. If the implementation already works, record the evidence without manufacturing changes."
        );
        // Use the resolved profile IDs and snapshot, including separate instances of the same harness.
        let mut input = json!({
            "prompt":brief, "path":project, "target":branch, "skip_init":true, "base_from_target":true,
            "verification_of":{"run":run_id,"candidate":candidate_id,"commit":candidate["commit"],"objective":objective},
            "recipe":{"checks":[]},
            "delivery":if source["held"] == true || !matches!(s(original, "delivery"), "auto" | "deploy") { "review" } else { "auto" }
        });
        for key in [
            "coordinator",
            "workers",
            "min_workers",
            "routing",
            "token_target",
            "subagents",
            "subagent_limit",
        ] {
            if let Some(value) = original.get(key) {
                input[key] = value.clone();
            }
        }
        self.start(input, source["profiles"].clone())
    }

    pub fn start(&self, mut input: Value, mut profiles: Value) -> Result<Value> {
        pool::configure(&mut input, &mut profiles)?;
        if input["worker_pool"].is_array() {
            adapter::models::discover(
                profiles.as_object_mut().context("invalid profiles")?,
                &self.dir,
            );
        }
        ensure!(
            input["subagent_limit"].is_null()
                || input["subagent_limit"].as_u64().is_some_and(|n| n > 0),
            "subagent limit must be positive"
        );
        let attachments = crate::attachments::unpack(&mut input)?;
        let mode = s(&input, "routing");
        ensure!(routing::mode_valid(mode), "unknown benchmark routing mode");
        ensure!(
            !s(&input, "prompt").trim().is_empty()
                && !s(&input, "path").is_empty()
                && !a(&input["workers"]).is_empty(),
            "choose a project, brief, coordinator, and worker"
        );
        ensure!(
            num(&input["token_target"]) >= 0,
            "token target cannot be negative"
        );
        for name in
            std::iter::once(s(&input, "coordinator").to_owned()).chain(strings(&input["workers"]))
        {
            let p = &profiles[&name];
            ensure!(!p.is_null(), "profile {name:?} is unavailable");
            adapter::validate(p)?;
            let binary = adapter::binary(p);
            ensure!(
                adapter::installed(&binary),
                "profile {name:?}: executable {binary:?} is unavailable"
            );
        }
        let approved = ["checks", "cases"]
            .iter()
            .any(|k| !a(&input["recipe"][k]).is_empty())
            || !s(&input["recipe"], "rubric").is_empty();
        if approved {
            eval::validate(&input["recipe"])?
        }
        let mut r = json!({
            "id": uid(), "input": input, "profiles": profiles,
            "state": "planning", "generation": 1, "round": 1, "policy": 1,
            "tasks": [], "turns": [], "candidates": [], "tokens": 0, "unknown_turns": 0,
            "recipe_approved": approved,
            "started": now(), "round_started": now()
        });
        // Verify and pin delivery authority before touching the project or its index.
        release::authorize(&mut r, &self.dir)?;
        let from_target = yes(&input["base_from_target"]);
        ensure!(
            !from_target || !s(&input, "target").is_empty(),
            "choose the branch to start from"
        );
        let mut repo = project::open(
            s(&input, "path"),
            &strings(&input["initial_files"]),
            !from_target && !yes(&input["skip_init"]),
        )?;
        if !s(&input, "target").is_empty() {
            repo.target = format!(
                "refs/heads/{}",
                s(&input, "target").trim_start_matches("refs/heads/")
            );
            let head = project::target_head(&repo.root, &repo.target)?;
            if from_target {
                if let Some(landed) = input["verification_of"]["commit"].as_str() {
                    ensure!(
                        project::is_ancestor(&repo.root, landed, &head),
                        "the target branch no longer contains this landed implementation"
                    );
                }
                repo.head = head;
            } else {
                ensure!(
                    project::is_ancestor(&repo.root, &head, &repo.head),
                    "target must be an ancestor of the chosen base"
                );
            }
        }
        r["repo"] = json!(repo);
        r["project"] = json!(self.store.project_id(&repo.common)?);
        // Remove partial setup on error. A successful SQLite save transfers ownership to the run.
        let runs = self.dir.join("runs");
        fs::create_dir_all(&runs)?;
        let pending = tempfile::Builder::new()
            .prefix(s(&r, "id"))
            .rand_bytes(0)
            .tempdir_in(runs)?;
        if approved {
            self.activate_initial_recipe(&mut r, input["recipe"].clone())?;
        }
        let integ = self.integration(&r);
        project::clone_repo(&repo.root, &integ, &repo.head, "swarm-integration")?;
        project::apply_commit_policy(&repo.root, &integ)?;
        fs::create_dir_all(self.root(&r).join("coordinator"))?;
        if !matches!(mode, "" | "manual") {
            let cat = routing::read(
                &self.dir.join("benchmark-cache.json"),
                &self.dir.join("benchmarks.json"),
            );
            r["benchmark_note"] = cat["note"].clone();
            r["benchmark_fetched"] = cat["fetched"].clone();
            supervisor::write_json(&self.root(&r).join("benchmarks.json"), &cat)?
        }
        if !attachments.is_empty() {
            r["input"]["attachments"] = crate::attachments::save(&self.root(&r), attachments)?;
        }
        let selected = project::subdir(Path::new(&repo.root), &repo.subdir)?;
        let workspace = tempfile::Builder::new()
            .prefix(&format!("Pantheon work {}", s(&r, "id")))
            .rand_bytes(0)
            .tempdir_in(&selected)?;
        // Each milestone is an independent checkout. Keep even the marker out
        // of the target index and leave these user-visible files on run deletion.
        fs::write(workspace.path().join(".gitignore"), "*\n")?;
        r["worker_folder"] = json!(workspace.path());
        r["working_folder"] = json!(selected);
        self.save(&mut r, "started")?;
        let _ = workspace.keep();
        let _ = pending.keep();
        Ok(r)
    }
    pub fn fetch_prices(&self, r: &mut Value, from: usize, automatic: bool) -> Result<()> {
        if a(&r["turns"])
            .iter()
            .any(|t| t["role"] == "pricing" && t["state"] == "running")
            || (automatic && budget(r))
        {
            return Ok(());
        }
        ensure!(!budget(r), "the run has reached its token target");
        let report = crate::metrics::report(r, &self.root(r), from)?;
        let mut models = crate::pricing::missing(&report);
        models.retain(|model, usage| {
            r["model_rates"].get(model).is_none()
                && (!automatic
                    || (r["pricing_attempted"].get(model).is_none()
                        && (usage["breakdown"].is_object() || model.ends_with(":free"))))
        });
        if models.is_empty() {
            if !automatic {
                r["pricing_status"] =
                    json!("No additional model prices can be fetched from the recorded usage.");
                self.save(r, "pricing-checked")?;
            }
            return Ok(());
        }
        let prompt = format!(
            "{}\n{}",
            crate::pricing::INSTRUCTIONS,
            serde_json::to_string(&models)?
        );
        let dir = self.root(r).join("pricing");
        fs::create_dir_all(&dir)?;
        let request = json!({"role":"pricing","profile":r["profiles"][s(&r["input"],"coordinator")],"dir":dir,"prompt":prompt});
        self.dispatch_with(r, "pricing", "", json!({"request":request}), |run| {
            for model in models.keys() {
                run["pricing_attempted"][model] = json!(true);
            }
            run["pricing_models"] = json!(models);
            run["pricing_status"] = json!("Finding model prices…");
        })
    }
    pub fn dispatch(&self, r: &mut Value, role: &str, task: &str, job: Value) -> Result<()> {
        self.dispatch_with(r, role, task, job, |_| {})
    }
    fn turn_context(
        &self,
        r: &Value,
        worker: Option<&Value>,
        role: &str,
        dir: &Path,
    ) -> Result<Value> {
        let mut context = context(r, worker);
        if matches!(role, "repair" | "plan") && r["run_recovery_context"].is_object() {
            context["prior_failure_and_recovery"] = r["run_recovery_context"].clone();
        }
        if !a(&r["input"]["attachments"]).is_empty() {
            context["brief_attachments"] =
                crate::attachments::context(&self.root(r), &r["input"]["attachments"])?;
        }
        let shared = worker::shared_skills(
            dir,
            Path::new(&self.integration(r)),
            std::env::var_os("HOME").as_deref().map(Path::new),
        );
        if !shared.is_empty() {
            context["shared_skill_files"] = json!(shared);
        }
        Ok(context)
    }
    pub fn dispatch_with(
        &self,
        r: &mut Value,
        role: &str,
        task: &str,
        mut job: Value,
        before_save: impl FnOnce(&mut Value),
    ) -> Result<()> {
        let worker = find(&r["tasks"], task)
            .filter(|_| role == "worker")
            .map(|i| &r["tasks"][i]);
        let msg_count = worker
            .map(|t| {
                if worker::awaiting_consultation(t) {
                    num(&t["delivered"]).max(0) as usize
                } else {
                    a(&t["notes"]).len()
                }
            })
            .unwrap_or_default();
        let prompt = s(&job["request"], "prompt");
        if !prompt.is_empty() && !matches!(role, "chat" | "pricing") {
            let context =
                self.turn_context(r, worker, role, Path::new(s(&job["request"], "dir")))?;
            job["request"]["prompt"] = json!(format!("Run context: {context}\n{prompt}"));
        }
        if job["request"].is_object() {
            for key in ["subagents", "subagent_limit"] {
                job["request"][key] = r["input"][key].clone();
            }
        }
        let request = &mut job["request"];
        if request["profile"]["kind"] == "codex"
            && role != "chat"
            && let Some(session) = request["session"].as_str().filter(|s| !s.is_empty())
        {
            request["codex_baseline"] = json!(crate::metrics::prior_usage(
                a(&r["turns"]),
                &self.root(r),
                session
            ));
        }
        if request["profile"]["kind"] == "hermes"
            && let Some(session) = request["session"].as_str().filter(|s| !s.is_empty())
        {
            request["acp_baseline"] = a(&r["turns"])
                .iter()
                .rev()
                .find(|t| t["acp_session"] == session)
                .map(|t| t["acp_usage"].clone())
                .unwrap_or(Value::Null);
        }
        let id = uid();
        let dir = self.jobdir(r, &id);
        fs::create_dir_all(&dir)?;
        supervisor::write_json(&dir.join("job.json"), &job)?;
        let agent = match role {
            "worker" => worker.map(|t| &t["worker"]),
            "plan" | "repair" | "observe" | "chat" | "pricing" => Some(&r["input"]["coordinator"]),
            _ => None,
        };
        let mut turn = json!({"id":id,"started":chrono::Utc::now().timestamp_millis(),"agent":agent,"task":task,"role":role,"generation":r["generation"],"round":r["round"],"policy":r["policy"],"state":"running","messages":msg_count});
        if role == "observe" {
            let mut ids = std::collections::BTreeSet::new();
            for event in a(&r["observations"]).iter().take(30) {
                let reference = s(event, "ref");
                if find(&r["tasks"], reference).is_some() {
                    ids.insert(reference);
                } else if let Some(i) = find(&r["candidates"], reference) {
                    ids.extend(
                        a(&r["candidates"][i]["tasks"])
                            .iter()
                            .filter_map(Value::as_str),
                    );
                }
            }
            turn["milestones"] = json!(ids);
        } else if role == "repair"
            && let Some(i) = find(&r["candidates"], s(r, "repair_id"))
        {
            turn["milestones"] = r["candidates"][i]["tasks"].clone();
        }
        before_save(r);
        push(&mut r["turns"], turn);
        self.save(r, "dispatched")?;
        if let Err(e) = supervisor::spawn(&dir) {
            supervisor::write_json(&dir.join("result.json"), &json!({"error":e.to_string()}))?
        }
        Ok(())
    }
    fn agent_turn(&self, r: &mut Value, role: &str, task: &str, request: Value) -> Result<()> {
        self.dispatch(r, role, task, json!({"request":request}))
    }
    fn agent_turn_with(
        &self,
        r: &mut Value,
        role: &str,
        task: &str,
        request: Value,
        before_save: impl FnOnce(&mut Value),
    ) -> Result<()> {
        self.dispatch_with(r, role, task, json!({"request":request}), before_save)
    }
}
