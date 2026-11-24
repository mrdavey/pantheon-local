use super::*;
fn render(template: &str, fields: &[(&str, String)]) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some((before, after)) = rest.split_once('{') {
        out.push_str(before);
        if let Some((name, tail)) = after.split_once('}')
            && let Some((_, v)) = fields.iter().find(|(k, _)| *k == name)
        {
            out.push_str(v);
            rest = tail
        } else {
            out.push('{');
            rest = after
        }
    }
    out.push_str(rest);
    out
}
impl Engine {
    pub(super) fn plan(&self, r: &mut Value) -> Result<()> {
        let dir = self.integration(r);
        let base = work_base(r);
        let subdir = s(&r["repo"], "Subdir");
        let files = project::files(&dir, base)
            .unwrap_or_default()
            .chars()
            .take(12000)
            .collect();
        let mut guidance = String::new();
        for name in [
            "AGENTS.md",
            "README.md",
            "package.json",
            "Makefile",
            "Cargo.toml",
            "go.mod",
            "pyproject.toml",
            ".swarm.json",
        ] {
            let path = if subdir == "." {
                name.to_owned()
            } else {
                format!("{subdir}/{name}")
            };
            if let Ok(text) = project::read_blob(&dir, base, &path) {
                guidance += &format!(
                    "\n[{path}]\n{}\n",
                    text.chars().take(2000).collect::<String>()
                );
            }
        }
        let inp = &r["input"];
        let profiles = strings(&inp["workers"])
            .iter()
            .map(|id| {
                format!(
                    "\n- {id} ({}, {})",
                    s(&r["profiles"][id], "kind"),
                    s(&r["profiles"][id], "model")
                )
            })
            .collect::<String>();
        let prior = a(&r["tasks"])
            .iter()
            .take(num(&r["task_start"]) as usize)
            .map(|t| s(t, "id"))
            .collect::<Vec<_>>()
            .join(", ");
        let prompt = render(
            include_str!("plan.txt"),
            &[
                ("compare", yes(&inp["compare"]).to_string()),
                ("profiles", profiles),
                ("subdir", subdir.into()),
                ("files", files),
                ("guidance", guidance),
                ("brief", s(inp, "prompt").into()),
                ("round_goal", s(r, "round_goal").into()),
                ("prior_ids", prior),
            ],
        );
        let req = json!({"role":"coordinator","profile":r["profiles"][s(inp,"coordinator")],"dir":dir,"prompt":prompt,"session":s(r,"session")});
        self.agent_turn(r, "plan", "", req)
    }
    pub(super) fn worker(&self, r: &mut Value, i: usize) -> Result<()> {
        let mut t = r["tasks"][i].clone();
        let id = s(&t, "id").to_owned();
        let integ = self.integration(r);
        let mut dir = s(&t, "workspace").to_owned();
        if dir.is_empty() {
            if !a(&t["donors"]).is_empty()
                && !s(&r["repo"], "Target").is_empty()
                && a(&r["past_selections"])
                    .last()
                    .is_some_and(|p| !p["landed_at"].is_null())
            {
                let base = project::target_head(s(&r["repo"], "Root"), s(&r["repo"], "Target"))?;
                project::fetch(&integ, s(&r["repo"], "Root"), &base)?;
                t["repair_base"] = json!(base);
            }
            let base = if s(&t, "repair_base").is_empty() {
                work_base(r)
            } else {
                s(&t, "repair_base")
            };
            let mut path = self.root(r).join("work").join(&id);
            if path.exists() {
                path.set_file_name(format!("{id}-{}", uid()));
            }
            dir = path.to_string_lossy().into_owned();
            project::clone_repo(&integ, &dir, base, &format!("swarm-{id}"))?;
            project::apply_commit_policy(s(&r["repo"], "Root"), &dir)?;
            for donor in strings(&t["donors"]) {
                let c = &r["candidates"][index(&r["candidates"], &donor)?];
                let sha = s(c, "commit");
                project::fetch(&dir, &integ, sha)?;
                project::pin(&dir, &format!("refs/swarm/donors/{donor}"), sha)?;
            }
            if !s(&t, "conflict").is_empty() {
                let c = &r["candidates"][index(&r["candidates"], s(&t, "conflict"))?];
                let incoming = c["pending"][0]
                    .as_str()
                    .context("missing conflict inputs")?;
                let sha = s(&r["tasks"][index(&r["tasks"], incoming)?], "commit");
                project::fetch(&dir, &integ, sha)?;
                ensure!(
                    project::merge(&dir, sha).is_err()
                        && !project::conflicted_paths(&dir)?.is_empty(),
                    "could not reproduce integration conflict"
                );
            }
            for d in strings(&t["depends"]) {
                let sha = s(&r["tasks"][index(&r["tasks"], &d)?], "commit");
                project::fetch(&dir, &integ, sha)?;
                project::merge(&dir, sha)?;
            }
            t["base"] = json!(project::head(&dir)?);
            t["workspace"] = json!(dir);
        } else if num(&t["handoff_retries"]) == 0
            && let Ok(sha) = project::milestone(&dir, s(&t, "base"))
            && s(&t, "reply").is_empty()
            && a(&t["notes"]).len() == num(&t["delivered"]) as usize
        {
            r["tasks"][i]["finding"] =
                json!("Recovered the verified commit from the worker checkout.");
            self.save_commit(r, i, &sha)?;
            return self.save(r, "commit-recovered");
        }
        let mut delegated = vec![];
        for child in a(&r["tasks"]) {
            if child["parent"] == id && !s(child, "commit").is_empty() {
                let sha = s(child, "commit");
                let name = format!("refs/swarm/delegated/{}", s(child, "id"));
                project::fetch(&dir, &integ, sha)?;
                project::pin(&dir, &name, sha)?;
                delegated.push(name)
            }
        }
        let cwd = Path::new(&dir).join(s(&r["repo"], "Subdir"));
        fs::create_dir_all(&cwd)?;
        let notes = strings(&t["notes"])
            .into_iter()
            .skip(num(&t["delivered"]) as usize)
            .collect::<Vec<_>>()
            .join("\n");
        let mut prompt = render(
            include_str!("worker.txt"),
            &[
                ("brief", s(&r["input"], "prompt").into()),
                ("id", id.clone()),
                ("goal", s(&t, "goal").into()),
                ("paths", strings(&t["paths"]).join(", ")),
                ("answer", s(&t, "reply").into()),
                ("notes", notes),
            ],
        );
        if !delegated.is_empty() {
            prompt += &format!(
                "\nReview and integrate delegated results: {}",
                delegated.join(", ")
            );
        }
        for donor in strings(&t["donors"]) {
            prompt += &format!(
                "\nInspect local ref refs/swarm/donors/{donor} for the requested aspects."
            );
        }
        if !s(&t, "conflict").is_empty() {
            prompt +=
                "\nResolve the merge in progress. Preserve both requirements and commit the merge.";
        }
        if num(&t["handoff_retries"]) > 0 {
            prompt += "\nContinue from your current checkout and session. Your last turn ended without a readable milestone receipt. Collect any pending review, check the existing changes, and return the worker JSON above. If finished, report the existing commit or the reviewed files. If you need help, use question or blocked.";
        }
        let req = json!({"role":"worker","profile":r["profiles"][s(&t,"worker")],"dir":cwd,"prompt":prompt,"session":s(&t,"session")});
        t["state"] = json!("running");
        r["tasks"][i] = t;
        self.agent_turn(r, "worker", &id, req)
    }
    pub(super) fn save_commit(&self, r: &mut Value, i: usize, reported: &str) -> Result<()> {
        let t = &r["tasks"][i];
        let dir = s(t, "workspace");
        let base = s(t, "base");
        let conflict = s(t, "conflict");
        let sha = if conflict.is_empty() {
            project::reported_milestone(dir, base)?
        } else {
            let c = &r["candidates"][index(&r["candidates"], conflict)?];
            let incoming = &r["tasks"][index(
                &r["tasks"],
                c["pending"][0].as_str().context("missing merge inputs")?,
            )?];
            let sha = project::head(dir)?;
            ensure!(
                project::is_ancestor(dir, s(incoming, "commit"), &sha)
                    && project::is_ancestor(dir, base, &sha),
                "repair commit does not contain both merge parents"
            );
            sha
        };
        ensure!(
            sha == reported,
            "reported commit does not match worker HEAD"
        );
        let scopes = strings(&t["paths"]);
        if !scopes.is_empty() {
            let paths = if conflict.is_empty() {
                project::changed_paths(dir, base, &sha)?
            } else {
                project::repair_changes(
                    &self
                        .root(r)
                        .join("candidates")
                        .join(conflict)
                        .join("source")
                        .to_string_lossy(),
                    dir,
                    &sha,
                )?
            };
            for path in paths {
                ensure!(
                    scopes
                        .iter()
                        .any(|scope| Path::new(&path).starts_with(scope)),
                    "milestone changed {path:?} outside its assigned scope"
                );
            }
        }
        let id = s(t, "id").to_owned();
        let parent = s(t, "parent").to_owned();
        let mut finding = s(t, "finding").to_owned();
        project::import_commit(
            &self.integration(r),
            dir,
            &sha,
            &format!("refs/swarm/milestones/{id}/{sha}"),
        )?;
        let dirty = project::status(dir).is_ok_and(|x| !x.is_empty());
        let t = &mut r["tasks"][i];
        t["state"] = json!("committed");
        t["commit"] = json!(sha);
        t["question"] = json!("");
        t["reply"] = json!("");
        if let Some(p) = find(&r["tasks"], &parent) {
            let t = &mut r["tasks"][p];
            push(
                &mut t["notes"],
                json!(format!(
                    "Delegated milestone {id} committed {sha}. {finding}"
                )),
            );
            if t["state"] == "question" {
                t["state"] = json!("pending");
                t["question"] = json!("")
            }
        }
        observe(r, "worker-completed", &id, &finding);
        if dirty {
            finding += "\nUncommitted files remain in this worker's checkout. The candidate uses the reported commit.";
            r["tasks"][i]["finding"] = json!(finding)
        }
        Ok(())
    }
    pub(super) fn accept_worker(&self, r: &mut Value, turn: &Value, res: &Value) -> Result<()> {
        let id = s(turn, "task");
        let i = index(&r["tasks"], id)?;
        r["tasks"][i]["session"] = json!(s(res, "session"));
        let result = (|| -> Result<()> {
            if !s(res, "error").is_empty() {
                let t = &r["tasks"][i];
                if !children_pending(r, id)
                    && !held(r, id)
                    && num(&t["delivered"]) as usize == a(&t["notes"]).len()
                    && let Ok(sha) = project::milestone(s(t, "workspace"), s(t, "base"))
                {
                    return self.save_commit(r, i, &sha);
                }
                bail!("{}", s(res, "error"));
            }
            let outcome = match adapter::decode_typed_outcome("worker", s(res, "text")) {
                Ok(outcome) => outcome,
                Err(err) => {
                    let task = &mut r["tasks"][i];
                    let retries = num(&task["handoff_retries"]);
                    if retries >= 2 || s(task, "session").is_empty() {
                        return Err(
                            err.context("worker did not return a readable milestone receipt")
                        );
                    }
                    task["handoff_retries"] = json!(retries + 1);
                    task["state"] = json!("pending");
                    task["finding"] = json!("Waiting for the worker to finish its handoff.");
                    return Ok(());
                }
            };
            if let Some(fields) = r["tasks"][i].as_object_mut() {
                fields.remove("handoff_retries");
            }
            let message = s(&outcome, "message");
            r["tasks"][i]["finding"] = json!(message);
            match s(&outcome, "status") {
                "committed" | "ready" => {
                    if children_pending(r, id)
                        || held(r, id)
                        || (num(&r["tasks"][i]["delivered"]) as usize)
                            < a(&r["tasks"][i]["notes"]).len()
                    {
                        r["tasks"][i]["state"] = json!("pending");
                    } else if outcome["status"] == "ready" {
                        let t = &r["tasks"][i];
                        ensure!(
                            s(t, "conflict").is_empty(),
                            "a merge repair must commit both parents"
                        );
                        let sha = project::commit_handoff(
                            s(t, "workspace"),
                            s(t, "base"),
                            id,
                            s(turn, "id"),
                            &strings(&outcome["files"]),
                            &strings(&t["paths"]),
                            message,
                        )?;
                        self.save_commit(r, i, &sha)?
                    } else {
                        self.save_commit(r, i, s(&outcome, "commit"))?
                    }
                }
                "question" => {
                    ensure!(!message.trim().is_empty(), "empty question");
                    r["tasks"][i]["state"] = json!("question");
                    r["tasks"][i]["question"] = json!(message);
                    observe(r, "worker-question", id, message);
                }
                "finding" => {
                    let j = index(&r["tasks"], s(&outcome, "recipient"))?;
                    ensure!(i != j && !message.is_empty(), "invalid finding recipient");
                    push(
                        &mut r["tasks"][j]["notes"],
                        json!(format!("{id}: {message}")),
                    );
                    r["tasks"][i]["state"] = json!("pending");
                    observe(
                        r,
                        "worker-finding",
                        id,
                        &format!("For {}: {message}", s(&outcome, "recipient")),
                    );
                }
                "blocked" => r["tasks"][i]["state"] = json!("blocked"),
                _ => bail!("unknown worker outcome"),
            };
            Ok(())
        })();
        if let Err(e) = result {
            r["tasks"][i]["state"] = json!("blocked");
            r["tasks"][i]["finding"] = json!(e.to_string())
        }
        Ok(())
    }
}
