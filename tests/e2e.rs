mod common;
use anyhow::{Context, Result, ensure};
use common::*;
use serde_json::{Value, json};
use std::{collections::HashSet, fs, path::Path};

fn start(svc: &Service, path: &Path, prompt: &str, options: Value) -> Result<String> {
    let mut input = json!({"path":path,"prompt":prompt,"coordinator":"first","workers":["writer","writer-two"]});
    input
        .as_object_mut()
        .unwrap()
        .extend(options.as_object().unwrap().clone());
    run_path(&svc.post("/runs", &input)?)
}
#[test]
fn prices_itemized_usage_with_the_coordinator_and_preserves_receipts() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let project = repo(&tmp.path().join("project"))?;
    let svc = Service::new(tmp.path())?;
    let url = start(&svc, &project, "Measure tariff lookup", json!({}))?;
    let run = until(TIMEOUT, || {
        let run = svc.get(&url)?;
        Ok((run["pricing_revision"] == 1).then_some(run))
    })?;
    assert_eq!(
        Path::new(run["working_folder"].as_str().unwrap()),
        project.canonicalize()?.as_path()
    );
    assert_eq!(git(&project, &["status", "--porcelain"])?, "");
    let usage = svc.get(&format!("{url}/metrics"))?;
    let plan = rows(&usage["turns"])
        .iter()
        .find(|t| t["role"] == "plan")
        .context("plan")?;
    let price = &rows(&plan["samples"]).last().unwrap()["models"]["Fixture Tariff"];
    assert_eq!(price["tokens"], 5);
    assert!((price["usd"].as_f64().context("estimated cost")? - 0.000012).abs() < 1e-12);
    svc.post(&format!("{url}/metrics/prices"), &json!({}))?;
    assert_eq!(
        rows(&svc.get(&url)?["turns"])
            .iter()
            .filter(|t| t["role"] == "pricing")
            .count(),
        1
    );
    Ok(())
}

#[test]
fn team_coordinates_product_decisions_and_peer_handoffs() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let project = repo(&tmp.path().join("team"))?;
    let mut svc = Service::new(tmp.path())?;
    let url = start(
        &svc,
        &project,
        "Build the team export",
        json!({"workers":["writer","writer-two","second","verifier"]}),
    )?;
    let first = svc.wait_state(&url, "question")?;
    assert_eq!(first["question"], "Which date format?");
    assert!(rows(&first["tasks"]).is_empty());
    svc.post(&url, &json!({"kind":"answer","text":"ISO dates"}))?;
    let second = svc.wait_state(&url, "question")?;
    assert_eq!(second["question"], "What should the export optimize?");
    svc.restart(false)?;
    svc.post(&url,&json!({"kind":"answer-inbox","task":rows(&second["inbox"]).last().context("inbox")?["id"],"text":"readability"}))?;
    let question = until(TIMEOUT, || {
        let r = svc.get(&url)?;
        ensure!(
            r["observer_error"].as_str().unwrap_or_default().is_empty(),
            "{r}"
        );
        let open: Vec<_> = rows(&r["inbox"])
            .iter()
            .filter(|q| q["state"] == "open")
            .collect();
        if !open.is_empty()
            && rows(&r["tasks"])
                .iter()
                .filter(|t| t["id"] != "team-view")
                .all(|t| t["state"] == "committed")
        {
            assert_eq!(open.len(), 1);
            assert_eq!(open[0]["tasks"], json!(["team-view"]));
            Ok(Some(open[0].clone()))
        } else {
            Ok(None)
        }
    })?;
    svc.restart(false)?;
    let saved = svc.get(&url)?;
    let format = item(&saved["tasks"], "id", &json!("team-format"))?;
    assert_eq!(format["state"], "committed");
    assert!(rows(&format["notes"]).contains(&json!("Worker asked: Which date contract should the exporter use?\n\nCoordinator answered: ISO dates")));
    assert_eq!(
        rows(&saved["inbox"])
            .iter()
            .filter(|q| q["state"] == "open")
            .collect::<Vec<_>>(),
        [&question]
    );
    svc.post(
        &url,
        &json!({"kind":"answer-inbox","task":question["id"],"text":"No quotes"}),
    )?;
    let completed = svc.wait_state(&url, "checks")?;
    assert_eq!(completed["recipe_approved"], false);
    assert!(
        rows(&completed["turns"])
            .iter()
            .all(|t| t["role"] != "eval")
    );
    assert!(
        rows(&completed["tasks"])
            .iter()
            .all(|t| t["state"] == "committed")
    );
    assert_eq!(
        rows(&completed["tasks"])
            .iter()
            .map(|t| t["worker"].as_str().unwrap())
            .collect::<HashSet<_>>()
            .len(),
        4
    );
    svc.post(&url, &json!({"kind":"approve"}))?;
    let ready = svc.wait_state(&url, "review")?;
    let folder = Path::new(ready["worker_folder"].as_str().context("working folder")?);
    for task in rows(&ready["tasks"]) {
        let checkout = Path::new(task["workspace"].as_str().context("milestone workspace")?);
        ensure!(checkout.is_dir() && checkout.parent() == Some(folder));
    }
    let candidate = passing(&ready)?;
    svc.post(&url, &json!({"kind":"land","candidate":candidate["id"]}))?;
    assert_eq!(git(&project, &["rev-parse", "HEAD"])?, candidate["commit"]);
    Ok(())
}
#[test]
fn restart_recovers_workers_and_repairs_failed_candidate() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let project = repo(&tmp.path().join("repo"))?;
    let mut svc = Service::new(tmp.path())?;
    let url = start(&svc, &project, "Build the fixture", json!({}))?;
    until(TIMEOUT, || {
        let run = svc.get(&url)?;
        let turns = svc
            .state
            .join("runs")
            .join(run["id"].as_str().unwrap())
            .join("turns");
        // The daemon persists dispatch intent before spawning the supervisor.
        // Kill it only after the worker whose recovery we are testing has claimed the turn.
        Ok(rows(&run["turns"])
            .iter()
            .any(|t| {
                t["role"] == "worker"
                    && t["state"] == "running"
                    && turns
                        .join(t["id"].as_str().unwrap())
                        .join("claim")
                        .is_file()
            })
            .then_some(()))
    })?;
    svc.restart(true)?;
    svc.wait_state(&url, "checks")?;
    svc.post(&url, &json!({"kind":"approve"}))?;
    let ready = svc.wait_state(&url, "review")?;
    assert_eq!(
        rows(&ready["turns"])
            .iter()
            .filter(|t| t["role"] == "worker")
            .count(),
        4
    );
    assert!(
        rows(&ready["candidates"])
            .iter()
            .any(|c| c["state"] == "failed")
    );
    let c = passing(&ready)?;
    let landed = svc.post(&url, &json!({"kind":"land","candidate":c["id"]}))?;
    assert_eq!(landed["state"], "landed");
    assert_eq!(git(&project, &["rev-parse", "HEAD"])?, c["commit"]);
    assert_eq!(fs::read_to_string(project.join("api.txt"))?, "correct");
    assert!(git(&project, &["log", "--format=%H"])?.lines().count() >= 5);
    Ok(())
}
#[test]
fn preserves_dirty_index_then_rebuilds_when_branch_moves() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let project = repo(&tmp.path().join("repo"))?;
    let svc = Service::new(tmp.path())?;
    let readme = project.join("README.md");
    let scratch = project.join("scratch.txt");
    fs::write(&readme, "before\n")?;
    git(&project, &["add", "README.md"])?;
    git(&project, &["commit", "-m", "Base"])?;
    fs::write(&readme, "staged\n")?;
    git(&project, &["add", "README.md"])?;
    fs::write(&scratch, "private\n")?;
    let index = git(&project, &["ls-files", "--stage"])?;
    let status = git(&project, &["status", "--porcelain"])?;
    let url = start(
        &svc,
        &project,
        "Add quote export",
        json!({"workers":["writer"]}),
    )?;
    svc.wait_state(&url, "question")?;
    svc.post(&url, &json!({"kind":"coordinator","text":"second"}))?;
    svc.post(&url, &json!({"kind":"answer","text":"private"}))?;
    svc.wait_state(&url, "checks")?;
    svc.post(&url, &json!({"kind":"approve"}))?;
    let ready = svc.wait_state(&url, "review")?;
    assert_eq!(ready["input"]["coordinator"], "second");
    let candidate = passing(&ready)?;
    let land = json!({"kind":"land","candidate":candidate["id"]});
    assert!(
        svc.post(&url, &land)
            .unwrap_err()
            .to_string()
            .contains("local changes")
    );
    assert_eq!(git(&project, &["ls-files", "--stage"])?, index);
    assert_eq!(git(&project, &["status", "--porcelain"])?, status);
    assert_eq!(fs::read_to_string(&scratch)?, "private\n");
    git(
        &project,
        &["commit", "-m", "Land developer work separately"],
    )?;
    fs::remove_file(scratch)?;
    let stale = svc.post(&url, &land)?;
    assert_eq!(stale["state"], "evaluating");
    assert_eq!(stale["candidates"][0]["state"], "stale");
    let ready = svc.wait_state(&url, "review")?;
    let passed = passing(&ready)?;
    assert_ne!(passed["id"], candidate["id"]);
    svc.post(&url, &json!({"kind":"land","candidate":passed["id"]}))?;
    assert_eq!(git(&project, &["rev-parse", "HEAD"])?, passed["commit"]);
    assert_eq!(fs::read_to_string(readme)?, "staged\n");
    Ok(())
}
#[test]
fn coordinator_delegates_within_parent_scope() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let project = repo(&tmp.path().join("repo"))?;
    let svc = Service::new(tmp.path())?;
    let url = start(
        &svc,
        &project,
        "Delegate the report",
        json!({"workers":["writer"],"subagents":true,"subagent_limit":1,"delivery":"auto","recipe":{"checks":[[example("fixture-agent")?,"check","docs/report.txt","reviewed both sections"]]}}),
    )?;
    let landed = svc.wait_state(&url, "landed")?;
    let children: Vec<_> = rows(&landed["tasks"])
        .iter()
        .filter(|t| t["parent"] == "report")
        .collect();
    assert_eq!(children.len(), 2);
    assert_eq!(
        rows(&landed["turns"])
            .iter()
            .filter(|t| t["role"] == "worker" && t["task"] == "report")
            .count(),
        2
    );
    for child in children {
        let id = child["id"].as_str().unwrap();
        assert_eq!(child["paths"], json!(["docs"]));
        assert_eq!(
            fs::read_to_string(project.join("docs").join(format!("{id}.txt")))?,
            id
        );
    }
    Ok(())
}
#[test]
fn token_budget_pauses_before_workers_and_only_passing_alternative_lands() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let project = repo(&tmp.path().join("repo"))?;
    let svc = Service::new(tmp.path())?;
    let url = start(
        &svc,
        &project,
        "Compare approaches",
        json!({"coordinator":"second","token_target":5}),
    )?;
    let paused = svc.wait_state(&url, "budget")?;
    assert!(rows(&paused["turns"]).iter().all(|t| t["role"] != "worker"));
    svc.post(&url, &json!({"kind":"resume","tokens":0}))?;
    svc.wait_state(&url, "checks")?;
    svc.post(&url, &json!({"kind":"approve"}))?;
    let ready = svc.wait_state(&url, "review")?;
    assert_eq!(rows(&ready["candidates"]).len(), 2);
    for c in rows(&ready["candidates"]) {
        assert_eq!(rows(&c["tasks"]).len(), 1);
        assert!(c["commit"].as_str().is_some_and(|s| !s.is_empty()));
    }
    let bad = item(&ready["candidates"], "state", &json!("failed"))?;
    assert!(
        svc.post(&url, &json!({"kind":"land","candidate":bad["id"]}))
            .is_err()
    );
    svc.post(
        &url,
        &json!({"kind":"land","candidate":passing(&ready)?["id"]}),
    )?;
    assert_eq!(fs::read_to_string(project.join("result.txt"))?, "correct");
    Ok(())
}

#[test]
fn coordinator_recovers_blocked_workers_without_operator_input() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let project = repo(&tmp.path().join("recovery"))?;
    let svc = Service::new(tmp.path())?;
    let url = start(
        &svc,
        &project,
        "Recover two failed milestones automatically",
        json!({"delivery":"review"}),
    )?;
    let run = svc.wait_state(&url, "checks")?;
    svc.post(
        &url,
        &json!({"kind":"approve","recipe":run["proposed_recipe"]}),
    )?;
    let run = svc.wait_state(&url, "review")?;
    let task = |id: &str| item(&run["tasks"], "id", &json!(id));
    assert_eq!(task("retry-work")?["automatic_retries"], 1);
    assert_eq!(task("bad-base")?["state"], "superseded");
    assert_eq!(
        task("bad-base")?["failed_receipt"]["commit"],
        task("bad-base")?["base"]
    );
    assert!(
        task("bad-base")?["previous_failure"]
            .as_str()
            .unwrap()
            .contains("descendant of its assigned base")
    );
    assert_eq!(task("bad-base")?["superseded_by"], "replacement");
    assert_eq!(task("final-check")?["depends"], json!(["replacement"]));
    for id in ["retry-work", "replacement", "final-check"] {
        assert_eq!(task(id)?["state"], "committed");
    }
    let page = svc.get(&format!("{url}/activity?milestone=retry-work"))?;
    let activity = rows(&page["items"]);
    let failures = svc.get(&format!(
        "{url}/activity?role=recovery&milestone=retry-work&q=rebuild"
    ))?;
    assert_eq!(failures["total"], 1);
    let report = item(&run["failure_log"], "id", &failures["items"][0]["id"])?;
    assert_eq!(report["state"], "reviewed");
    assert_eq!(report["evidence"]["worker"], "writer");
    assert!(report["source"].is_string() && report["review_turn"].is_string());
    assert_eq!(activity.iter().filter(|r| r["role"] == "worker").count(), 2);
    assert!(activity.iter().any(|r| r["role"] == "observe"));
    assert!(activity.iter().any(|r| r["role"] == "eval"));
    Ok(())
}
