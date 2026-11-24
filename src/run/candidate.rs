use super::*;
fn selections(tasks: &[Value]) -> Result<Vec<Vec<String>>> {
    let tasks = &tasks[tasks
        .iter()
        .rposition(|t| !s(t, "repair_base").is_empty() && s(t, "conflict").is_empty())
        .unwrap_or(0)..];
    let mut required = vec![];
    let mut variants: Vec<(&str, Vec<String>)> = vec![];
    let mut groups: Vec<(&str, Vec<String>)> = vec![];
    let mut children: HashMap<&str, Vec<String>> = HashMap::new();
    for t in tasks {
        if t["state"] == "abandoned" || !s(t, "conflict").is_empty() {
            continue;
        }
        let id = s(t, "id").to_owned();
        if !s(t, "parent").is_empty() {
            children.entry(s(t, "parent")).or_default().push(id);
            continue;
        }
        let (g, k) = if !s(t, "variant").is_empty() {
            (&mut variants, "variant")
        } else if !s(t, "alternative").is_empty() {
            (&mut groups, "alternative")
        } else {
            required.push(id);
            continue;
        };
        let key = s(t, k);
        if let Some((_, items)) = g.iter_mut().find(|(name, _)| *name == key) {
            items.push(id)
        } else {
            g.push((key, vec![id]))
        }
    }
    ensure!(
        groups.is_empty() || variants.is_empty(),
        "use variant memberships consistently"
    );
    let mut sets = vec![required.clone()];
    if !variants.is_empty() {
        sets = variants
            .into_iter()
            .map(|(_, ids)| required.iter().cloned().chain(ids).collect())
            .collect()
    }
    for (_, choices) in groups {
        let mut next = vec![];
        for base in sets {
            for id in &choices {
                let mut set = base.clone();
                set.push(id.clone());
                next.push(set);
                ensure!(next.len() <= 12, "too many candidate combinations")
            }
        }
        sets = next
    }
    for set in &mut sets {
        let mut i = 0;
        while i < set.len() {
            if let Some(c) = children.get(set[i].as_str()) {
                set.extend(c.iter().cloned())
            }
            i += 1;
        }
    }
    Ok(sets)
}
impl Engine {
    pub(super) fn candidates(&self, r: &mut Value) -> Result<()> {
        let mut base = work_base(r).to_owned();
        let repo = &r["repo"];
        if !s(repo, "Target").is_empty() {
            base = project::target_head(s(repo, "Root"), s(repo, "Target"))?;
            project::fetch(&self.integration(r), s(repo, "Root"), &base)?
        }
        let sets = selections(active(r))?;
        let mut rows = vec![];
        if yes(&r["input"]["compare"]) {
            rows.push(json!({"id":uid(),"round":r["round"],"policy":r["policy"],"state":"queued","base":base,"baseline":true,"name":"Baseline","tasks":[]}));
        }
        for ids in sets {
            let mut names = vec![];
            let mut parents = vec![];
            for id in &ids {
                let t = &r["tasks"][index(&r["tasks"], id)?];
                if !s(t, "variant").is_empty() {
                    names.push(s(t, "variant").to_owned());
                } else if !s(t, "alternative").is_empty() || !s(t, "repair_base").is_empty() {
                    names.push(s(t, "goal").to_owned())
                }
                parents.extend(strings(&t["donors"]));
            }
            if names.is_empty() {
                names = ids.clone()
            }
            names.dedup();
            rows.push(json!({"id":uid(),"base":base,"tasks":ids,"state":"queued","round":r["round"],"policy":r["policy"],"parents":parents,"name":names.join(" · ")}));
        }
        for row in rows {
            push(&mut r["candidates"], row)
        }
        r["state"] = json!("evaluating");
        self.save(r, "candidates-queued")
    }
    pub(super) fn eval_result(&self, r: &mut Value, turn: &Value, result: &Value) -> Result<()> {
        let i = index(&r["candidates"], s(turn, "task"))?;
        let dir = self.jobdir(r, s(turn, "id"));
        let recipe = r["input"]["recipe"].clone();
        let c = &mut r["candidates"][i];
        c["state"] = json!("passed");
        c["log"] = json!(dir.join("events.log"));
        c["detail"] = json!("");
        if !s(c, "inputs").is_empty() {
            let res = read(dir.join("measurements.json"))
                .and_then(|report| comparison::record_report(c, &recipe, &report, s(turn, "id")));
            if let Err(e) = res {
                c["state"] = json!("inconclusive");
                c["detail"] = json!(e.to_string())
            }
        }
        if !s(result, "error").is_empty() && c["state"] != "inconclusive" {
            c["state"] = json!("failed");
            c["detail"] = result["error"].clone()
        }
        let c = c.clone();
        if yes(&r["input"]["compare"])
            || !s(result, "error").is_empty()
            || !s(&c, "inputs").is_empty()
        {
            observe(
                r,
                &format!("eval-{}", s(&c, "state")),
                s(&c, "id"),
                s(&c, "detail"),
            )
        }
        Ok(())
    }
    fn evaluate(&self, r: &mut Value, i: usize, dir: &str) -> Result<()> {
        let recipe = r["input"]["recipe"].clone();
        let mut c = r["candidates"][i].clone();
        if !a(&recipe["cases"]).is_empty() {
            let inputs = match eval::inputs(&recipe, s(&c, "commit")) {
                Ok(v) => v,
                Err(e) => {
                    r["candidates"][i]["state"] = json!("inconclusive");
                    r["candidates"][i]["detail"] = json!(e.to_string());
                    return self.save(r, "eval-inputs-unavailable");
                }
            };
            c["inputs"] = json!(inputs);
            for p in a(&r["candidates"]) {
                if p["id"] != c["id"]
                    && p["inputs"] == inputs
                    && p["state"] == "passed"
                    && !s(p, "evidence").is_empty()
                    && let Ok(report) =
                        read(self.jobdir(r, s(p, "evidence")).join("measurements.json"))
                {
                    let mut reused = c.clone();
                    if comparison::record_report(&mut reused, &recipe, &report, s(p, "evidence"))
                        .is_ok()
                        && reused["state"] == "passed"
                    {
                        reused["log"] = p["log"].clone();
                        r["candidates"][i] = reused;
                        observe(
                            r,
                            "eval-complete",
                            s(&c, "id"),
                            "Reused matching measurements",
                        );
                        return self.save(r, "eval-reused");
                    }
                }
            }
        }
        c["state"] = json!("evaluating");
        let job = json!({"request":{"dir":Path::new(dir).join(s(&r["repo"],"Subdir"))},"recipe":recipe,"expected_commit":c["commit"],"inputs":s(&c,"inputs"),"scope":{"repository":r["repo"]["Root"],"state":self.dir}});
        r["candidates"][i] = c.clone();
        self.dispatch(r, "eval", s(&c, "id"), job)
    }
    pub(super) fn candidate_step(&self, r: &mut Value) -> Result<()> {
        for i in 0..a(&r["candidates"]).len() {
            let c = r["candidates"][i].clone();
            if c["round"] != r["round"] || c["policy"] != r["policy"] {
                continue;
            }
            match s(&c, "state") {
                "evaluating" => return Ok(()),
                "conflict" => return self.advance_conflict(r, i),
                "queued" => {}
                _ => continue,
            }
            if !a(&r["input"]["recipe"]["cases"]).is_empty()
                && self.store.scheduled_runs()?.iter().any(|other| {
                    other["id"] != r["id"]
                        && !a(&other["input"]["recipe"]["cases"]).is_empty()
                        && a(&other["turns"])
                            .iter()
                            .any(|t| t["role"] == "eval" && t["state"] == "running")
                })
            {
                return Ok(());
            }
            let path = self
                .root(r)
                .join("candidates")
                .join(s(&c, "id"))
                .join("source");
            if path.exists() {
                fs::remove_dir_all(&path)?
            }
            let dir = path.to_string_lossy();
            let integ = self.integration(r);
            project::clone_repo(&integ, &dir, s(&c, "base"), "swarm-candidate")?;
            if !yes(&c["baseline"]) {
                project::apply_commit_policy(s(&r["repo"], "Root"), &dir)?;
            }
            for (j, id) in strings(&c["tasks"]).iter().enumerate() {
                let t = &r["tasks"][index(&r["tasks"], id)?];
                project::fetch(&dir, &integ, s(t, "commit"))?;
                if project::merge(&dir, s(t, "commit")).is_err() {
                    let paths = project::conflicted_paths(&dir)?;
                    if paths.is_empty() {
                        r["candidates"][i]["state"] = json!("failed");
                        r["candidates"][i]["detail"] =
                            json!("Git integration failed without a resolvable merge.");
                        return self.save(r, "assembly-failed");
                    }
                    let head = project::head(&dir)?;
                    ensure!(
                        !a(&r["tasks"])
                            .iter()
                            .any(|t| t["round"] == c["round"] && t["conflict"] == c["id"]),
                        "integration repair already attempted"
                    );
                    project::import_commit(
                        &integ,
                        &dir,
                        &head,
                        &format!("refs/swarm/conflicts/{}", s(&c, "id")),
                    )?;
                    let item = &mut r["candidates"][i];
                    item["state"] = json!("conflict");
                    item["detail"] = json!(paths.join(", "));
                    item["conflict_paths"] = json!(paths);
                    item["conflict_head"] = json!(head);
                    item["pending"] = json!(&a(&c["tasks"])[j..]);
                    observe(r, "integration-conflict", s(&c, "id"), &paths.join(", "));
                    return self.save(r, "assembly-conflict");
                }
            }
            let sha = project::head(&dir)?;
            project::import_commit(
                &integ,
                &dir,
                &sha,
                &format!("refs/swarm/candidates/{}", s(&c, "id")),
            )?;
            r["candidates"][i]["commit"] = json!(sha);
            r["candidates"][i]["preview"] = r["input"]["recipe"]["preview"].clone();
            return self.evaluate(r, i, &dir);
        }
        let inp = &r["input"];
        let current = |c: &&Value| c["round"] == r["round"] && c["policy"] == r["policy"];
        let passed = a(&r["candidates"])
            .iter()
            .filter(current)
            .any(|c| c["state"] == "passed" && !yes(&c["baseline"]));
        if passed
            || yes(&inp["compare"])
            || !inp["recipe"]["rule"].is_null()
            || !s(&inp["recipe"], "rubric").is_empty()
        {
            let rule = inp["recipe"]["rule"].clone();
            let round = r["round"].clone();
            let pol = r["policy"].clone();
            if let Some(candidates) = r["candidates"].as_array_mut() {
                for c in candidates {
                    if c["round"] == round && c["policy"] == pol {
                        c["eligible"] = json!(comparison::eligible(&rule, c))
                    }
                }
            }
            let sel = comparison::recommend(r);
            r["selection"] = sel.clone();
            r["state"] = json!("review");
            r["detail"] = json!("");
            if yes(&r["input"]["compare"])
                || !matches!(s(&r["input"], "delivery"), "" | "review")
                || !a(&r["input"]["recipe"]["cases"]).is_empty()
            {
                observe(r, "comparison-ready", "", s(&sel, "reason"))
            }
            return self.save(r, "review");
        }
        if let Some(c) =
            a(&r["candidates"]).iter().rev().filter(current).find(|c| {
                !yes(&c["baseline"]) && !s(c, "commit").is_empty() && c["state"] == "failed"
            })
            && !active(r).iter().any(|t| !s(t, "repair_base").is_empty())
        {
            let id = c["id"].clone();
            r["state"] = json!("repairing");
            r["repair_id"] = id;
            return self.save(r, "repair-needed");
        }
        r["state"] = json!("blocked");
        r["detail"] = json!("No candidate passed. Inspect the eval evidence.");
        self.save(r, "checks-failed")
    }
    fn advance_conflict(&self, r: &mut Value, i: usize) -> Result<()> {
        let c = r["candidates"][i].clone();
        if let Some(j) = a(&r["tasks"]).iter().position(|t| t["conflict"] == c["id"]) {
            let t = &r["tasks"][j];
            match s(t, "state") {
                "committed" => {
                    let mut ids = vec![t["id"].clone()];
                    ids.extend(a(&c["pending"]).iter().skip(1).cloned());
                    let next = json!({"id":uid(),"round":r["round"],"policy":r["policy"],"base":c["base"],"name":c["name"],"parents":[c["id"]],"tasks":ids,"state":"queued"});
                    r["candidates"][i]["state"] = json!("superseded");
                    push(&mut r["candidates"], next);
                    return self.save(r, "integration-repaired");
                }
                "abandoned" | "blocked" => {}
                "pending" if !held(r, s(t, "id")) && !budget(r) => return self.worker(r, j),
                _ => return Ok(()),
            }
        } else if !a(&r["observations"]).is_empty()
            || !s(r, "observer_error").is_empty()
            || waiting(r)
        {
            return Ok(());
        }
        r["candidates"][i]["state"] = json!("failed");
        r["candidates"][i]["detail"] = json!("The integration repair could not finish.");
        self.save(r, "integration-blocked")
    }
    pub(super) fn land(&self, r: &mut Value, id: &str) -> Result<()> {
        let c = &r["candidates"][index(&r["candidates"], id)?];
        if r["state"] == "landed" && r["detail"] == c["commit"] {
            return Ok(());
        }
        ensure!(
            matches!(s(&r["input"], "delivery"), "" | "review")
                || s(r, "observer_error").is_empty(),
            "retry the coordinator or switch to manual review"
        );
        ensure!(
            r["state"] == "review"
                && comparison::eligible(&r["input"]["recipe"]["rule"], c)
                && c["round"] == r["round"]
                && c["policy"] == r["policy"]
                && !s(&r["repo"], "Target").is_empty()
                && !waiting(r)
                && !strings(&c["tasks"]).iter().any(|id| held(r, id))
                && r["combination"].is_null(),
            "select a passing candidate and a local target branch"
        );
        let c = c.clone();
        let repo = r["repo"].clone();
        let root = s(&repo, "Root");
        let target = s(&repo, "Target");
        let actual = project::target_head(root, target)?;
        let base = s(&c, "base");
        let sha = s(&c, "commit");
        if actual != base && actual != sha {
            let round = c["round"].clone();
            let policy = c["policy"].clone();
            if let Some(cs) = r["candidates"].as_array_mut() {
                for c in cs {
                    if c["round"] == round
                        && c["policy"] == policy
                        && c["base"] != actual
                        && matches!(s(c, "state"), "passed" | "inconclusive" | "cancelled")
                    {
                        c["state"] = json!("stale");
                        c["detail"] =
                            json!("the target moved; rebuilding against its current head");
                    }
                }
            }
            return self.candidates(r);
        }
        if r["selection"]["candidate"] != id {
            let mut sel = comparison::recommend(r);
            sel["candidate"] = json!(id);
            sel["state"] = json!("selected");
            sel["override"] = json!(true);
            sel["reason"] = json!("You selected this implementation.");
            r["selection"] = sel;
        }
        r["landing"] = json!(id);
        self.save(r, "landing")?;
        project::import_commit(
            root,
            &self.integration(r),
            sha,
            &format!("refs/swarm/runs/{}/{id}", s(r, "id")),
        )?;
        if let Err(e) = project::land(root, target, base, sha) {
            if project::target_head(root, target).is_ok_and(|head| head == base) {
                r["landing"] = json!("");
                r["held"] = json!(true);
                r["detail"] = json!(e.to_string());
                self.save(r, "landing-wait")?
            }
            return Err(e);
        }
        mark_landed(r, &c);
        self.save(r, "landed")
    }
}
pub(super) fn mark_landed(r: &mut Value, c: &Value) {
    r["state"] = json!("landed");
    r["detail"] = c["commit"].clone();
    r["landing"] = json!("");
    if r["selection"]["candidate"] != c["id"] {
        r["selection"] =
            json!({"candidate":c["id"],"round":r["round"],"policy":r["policy"],"at":now()})
    }
    r["selection"]["state"] = json!("landed");
    r["selection"]["landed_at"] = json!(now());
    if r["input"]["delivery"] == "deploy" {
        let entry = json!({"id":uid(),"candidate":c["id"],"commit":c["commit"],"destination":r["input"]["destination"],"state":"queued"});
        push(&mut r["releases"], entry);
    }
}
