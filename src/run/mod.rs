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
        push(
            &mut r["observations"],
            json!({"id":uid(),"kind":kind,"ref":id,"text":text}),
        )
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
    active(r)
        .iter()
        .any(|t| t["parent"] == id && !matches!(s(t, "state"), "committed" | "abandoned"))
}
fn committed(r: &Value) -> bool {
    !active(r).is_empty()
        && active(r)
            .iter()
            .all(|t| matches!(s(t, "state"), "committed" | "abandoned"))
}
pub fn record_answer(r: &mut Value, q: usize, text: &str) {
    let ids = strings(&r["inbox"][q]["tasks"]);
    let question = s(&r["inbox"][q], "text").to_owned();
    r["inbox"][q]["state"] = json!("answered");
    r["inbox"][q]["answer"] = json!(text);
    for id in ids {
        if let Some(i) = find(&r["tasks"], &id) {
            let t = &mut r["tasks"][i];
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
            "invalid benchmark category"
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
    let p = json!({"revision":r["policy"],"rule":r["input"]["recipe"]["rule"],"at":now()});
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
        self.store.save(r, kind, &[])
    }
    pub fn assign(&self, r: &Value, tasks: &mut [Value]) {
        let catalog = read(self.root(r).join("benchmarks.json"))
            .unwrap_or_else(|_| json!({"note":"The run's benchmark snapshot is unavailable."}));
        let workers = strings(&r["input"]["workers"]);
        for t in tasks {
            let (w, route) = routing::choose(
                s(&r["input"], "routing"),
                s(t, "class"),
                s(t, "worker"),
                &workers,
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
    fn pin_recipe(&self, r: &Value, mut recipe: Value) -> Result<Value> {
        eval::validate(&recipe)?;
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
    pub fn start(&self, input: Value, profiles: Value) -> Result<Value> {
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
            "recipe_approved": approved, "proposed_recipe": {},
            "started": now(), "round_started": now()
        });
        // Verify and pin delivery authority before touching the project or its index.
        release::authorize(&mut r, &self.dir)?;
        let mut repo = project::open(
            s(&input, "path"),
            &strings(&input["initial_files"]),
            !yes(&input["skip_init"]),
        )?;
        if !s(&input, "target").is_empty() {
            repo.target = format!(
                "refs/heads/{}",
                s(&input, "target").trim_start_matches("refs/heads/")
            );
            let head = project::target_head(&repo.root, &repo.target)?;
            ensure!(
                project::is_ancestor(&repo.root, &head, &repo.head),
                "target must be an ancestor of the chosen base"
            )
        }
        r["repo"] = json!(repo);
        r["project"] = json!(self.store.project_id(&repo.common)?);
        if approved {
            r["input"]["recipe"] = self.pin_recipe(&r, input["recipe"].clone())?;
            policy(&mut r)
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
        self.save(&mut r, "started")?;
        Ok(r)
    }
    pub fn dispatch(&self, r: &mut Value, role: &str, task: &str, mut job: Value) -> Result<()> {
        let worker = find(&r["tasks"], task)
            .filter(|_| role == "worker")
            .map(|i| &r["tasks"][i]);
        let msg_count = worker.map(|t| a(&t["notes"]).len()).unwrap_or_default();
        let prompt = s(&job["request"], "prompt");
        if !prompt.is_empty() {
            job["request"]["prompt"] =
                json!(format!("Run context: {}\n{}", context(r, worker), prompt));
        }
        let id = uid();
        let dir = self.jobdir(r, &id);
        fs::create_dir_all(&dir)?;
        supervisor::write_json(&dir.join("job.json"), &job)?;
        let turn = json!({"id":id,"task":task,"role":role,"generation":r["generation"],"round":r["round"],"policy":r["policy"],"state":"running","messages":msg_count});
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
}
