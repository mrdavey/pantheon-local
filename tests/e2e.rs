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
    let plan = svc.wait_state(&url, "checks")?;
    assert!(rows(&plan["turns"]).iter().all(|t| t["role"] != "worker"));
    assert_eq!(
        rows(&plan["tasks"])
            .iter()
            .map(|t| t["worker"].as_str().unwrap())
            .collect::<HashSet<_>>()
            .len(),
        4
    );
    svc.post(&url, &json!({"kind":"approve"}))?;
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
    let ready = svc.wait_state(&url, "review")?;
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
    svc.wait_state(&url, "checks")?;
    svc.post(&url, &json!({"kind":"approve"}))?;
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
        json!({"workers":["writer"],"delivery":"auto","recipe":{"checks":[[example("fixture-agent")?,"check","docs/report.txt","reviewed both sections"]]}}),
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
    svc.wait_state(&url, "checks")?;
    svc.post(&url, &json!({"kind":"approve"}))?;
    let paused = svc.wait_state(&url, "budget")?;
    assert!(rows(&paused["turns"]).iter().all(|t| t["role"] != "worker"));
    svc.post(&url, &json!({"kind":"resume","tokens":0}))?;
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
