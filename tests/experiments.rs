mod common;
use anyhow::{Context, Result, ensure};
use common::*;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path, time::Duration};
fn start(svc: &Service, path: &Path, options: Value) -> Result<String> {
    let mut input = json!({"path":path,"prompt":"Measure approaches","coordinator":"first","workers":["writer","writer-two"],"compare":true,"recipe":experiment_recipe()?});
    input
        .as_object_mut()
        .unwrap()
        .extend(options.as_object().unwrap().clone());
    run_path(&svc.post("/runs", &input)?)
}
fn reports(run: &Value) -> BTreeMap<String, Value> {
    rows(&run["candidates"])
        .iter()
        .filter(|c| c["policy"] == run["policy"] && c["round"] == run["round"])
        .map(|c| {
            (
                c["name"].as_str().unwrap().to_owned(),
                c["evidence"].clone(),
            )
        })
        .collect()
}
#[test]
fn measured_selection_durable_feedback_combination_and_evidence_reuse() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let project = repo(&tmp.path().join("notes"))?;
    let mut svc = Service::new(tmp.path())?;
    let url = start(&svc, &project, json!({"delivery":"auto"}))?;
    let landed = svc.wait_state(&url, "landed")?;
    let chosen = &landed["selection"]["candidate"];
    let best = item(&landed["candidates"], "id", chosen)?;
    let loser = item(&landed["candidates"], "name", &json!("fast"))?;
    assert_eq!(best["name"], "balanced");
    assert_eq!(best["eligible"], true);
    assert_eq!(loser["state"], "passed");
    assert_eq!(loser["eligible"], false);
    assert_eq!(git(&project, &["rev-parse", "HEAD"])?, best["commit"]);
    let evidence = svc.get(&format!("{url}/evidence/{}", chosen.as_str().unwrap()))?;
    assert_eq!(rows(&evidence["samples"]).len(), 3);
    assert_eq!(evidence["commit"], best["commit"]);
    let feedback = json!({"kind":"feedback","id":"record-case","candidate":loser["id"],"text":"Search by title should keep its exact match."});
    svc.post(&url, &feedback)?;
    svc.post(&url, &feedback)?;
    let question = until(TIMEOUT, || {
        Ok(rows(&svc.get(&url)?["inbox"])
            .iter()
            .find(|q| q["state"] == "open")
            .cloned())
    })?;
    svc.restart(true)?;
    assert_eq!(rows(&svc.get(&format!("{url}/cases"))?).len(), 1);
    svc.post(
        &url,
        &json!({"kind":"answer-inbox","task":question["id"],"text":"Yes"}),
    )?;
    svc.post(&url,&json!({"kind":"combine","candidate":chosen,"donors":[loser["id"]],"text":"Add the faster formatter only."}))?;
    let combined = until(TIMEOUT, || {
        let r = svc.get(&url)?;
        ensure!(r["state"] != "blocked", "{r}");
        Ok((r["round"] == 2 && r["state"] == "review").then_some(r))
    })?;
    let mixed = rows(&combined["candidates"])
        .iter()
        .find(|c| c["round"] == 2 && c["baseline"] != true)
        .context("combined candidate")?;
    assert_eq!(mixed["parents"], json!([chosen, loser["id"]]));
    assert_eq!(mixed["score"]["relevance"]["value"].as_f64(), Some(8.0));
    assert_eq!(
        rows(&combined["past_selections"]).last().unwrap()["candidate"],
        *chosen
    );
    assert_eq!(combined["held"], true);
    assert_eq!(git(&project, &["rev-parse", "HEAD"])?, best["commit"]);
    let old = reports(&combined);
    assert!(!old.is_empty());
    let mut changed = experiment_recipe()?;
    changed["rule"]["minimum"] = json!(0.5);
    svc.post(&url, &json!({"kind":"update-suite","recipe":changed}))?;
    assert_eq!(reports(&svc.wait_state(&url, "review")?), old);
    changed["cases"][0]["input"] = json!({"query":"different"});
    svc.post(&url, &json!({"kind":"update-suite","recipe":changed}))?;
    let fresh = reports(&svc.wait_state(&url, "review")?);
    assert_eq!(fresh.len(), old.len());
    for (name, report) in fresh {
        assert_ne!(report, old[&name])
    }
    Ok(())
}
#[test]
fn baseline_is_retained_and_overlapping_work_can_be_repaired() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let svc = Service::new(tmp.path())?;
    let mut recipe = experiment_recipe()?;
    recipe["rule"]["minimum"] = json!(101);
    let url = start(
        &svc,
        &repo(&tmp.path().join("baseline"))?,
        json!({"delivery":"auto","recipe":recipe}),
    )?;
    let review = svc.wait_state(&url, "review")?;
    assert_measured(&svc, &url, &review)?;
    assert_eq!(review["selection"]["state"], "baseline", "{review}");
    assert!(
        rows(&review["past_selections"])
            .iter()
            .all(|s| s["landed_at"].is_null())
    );
    let project = repo(&tmp.path().join("conflict"))?;
    fs::write(project.join("overlap.txt"), "before")?;
    git(&project, &["add", "overlap.txt"])?;
    git(&project, &["commit", "-m", "Shared starting point"])?;
    let url = start(
        &svc,
        &project,
        json!({"prompt":"Resolve overlap","delivery":"auto","compare":false,"recipe":{"checks":[[example("fixture-agent")?,"check","overlap.txt","both"]]}}),
    )?;
    svc.wait_state(&url, "landed")?;
    assert_eq!(fs::read_to_string(project.join("overlap.txt"))?, "both");
    assert_eq!(
        fs::read_to_string(project.join("clean-merge.txt"))?,
        "from right"
    );
    Ok(())
}
fn assert_measured(svc: &Service, url: &str, r: &Value) -> Result<()> {
    for c in rows(&r["candidates"]) {
        if c["state"] == "inconclusive" {
            let evidence = svc.get(&format!("{url}/evidence/{}", c["id"].as_str().unwrap()));
            anyhow::bail!("inconclusive {:?}: {evidence:?}", c["name"]);
        }
    }
    Ok(())
}
fn release_state(svc: &Service, path: &str, wanted: &str) -> Result<Value> {
    until(TIMEOUT, || {
        let r = svc.get(path)?;
        ensure!(r["state"] != "blocked", "{r}");
        assert_measured(svc, path, &r)?;
        let release = rows(&r["releases"]).last();
        if let Some(release) = release {
            if matches!(release["state"].as_str(), Some("uncertain" | "failed")) {
                let logs = rows(&r["turns"])
                    .iter()
                    .filter(|t| t["role"] == "release")
                    .map(|t| {
                        let dir = svc
                            .state
                            .join("runs")
                            .join(r["id"].as_str().unwrap())
                            .join("turns")
                            .join(t["id"].as_str().unwrap());
                        let outcome =
                            fs::read_to_string(dir.join("result.json")).unwrap_or_default();
                        let log = fs::read_to_string(dir.join("events.log")).unwrap_or_default();
                        (outcome, log)
                    })
                    .collect::<Vec<_>>();
                anyhow::bail!("release stopped: {release}; receipts: {logs:?}");
            }
            if release["state"] == wanted {
                return Ok(Some(release.clone()));
            }
        }
        Ok(None)
    })
    .with_context(|| format!("waiting for release {wanted}: {:?}", svc.get(path)))
}
#[test]
fn release_recovers_after_restart_reuses_identical_artifact_and_rolls_back() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let root = tmp.path();
    let live = root.join("live.json");
    let previous = json!({"digest":"sha256:previous","healthy":true});
    fs::write(&live, serde_json::to_vec(&previous)?)?;
    let mut svc = Service::with_env(
        root,
        BTreeMap::from([("SWARM_TEST_LIVE".into(), live.clone())]),
    )?;
    fs::write(
        svc.state.join("destinations.json"),
        serde_json::to_vec(
            &json!({"demo":{"executable":example("fixture-release")?,"environment":["SWARM_TEST_LIVE"],"rollback":true}}),
        )?,
    )?;
    assert_eq!(svc.get("/destinations")?, json!(["demo"]));
    let release = repo(&root.join("release"))?;
    let deploy = json!({"delivery":"deploy","destination":"demo"});
    fs::write(root.join("live.json.interrupt"), "")?;
    let url = start(&svc, &release, deploy.clone())?;
    until(Duration::from_secs(40), || {
        assert_measured(&svc, &url, &svc.get(&url)?)?;
        let val: Value = serde_json::from_slice(&fs::read(&live)?)?;
        Ok((val["digest"] != "sha256:previous").then_some(()))
    })
    .with_context(|| format!("waiting for promotion: {:?}", svc.get(&url)))?;
    svc.restart(true)?;
    let got = release_state(&svc, &url, "deployed")?;
    assert_eq!(got["digest"], got["observed"]);
    assert_eq!(got["commit"], git(&release, &["rev-parse", "HEAD"])?);
    fs::remove_file(root.join("live.json.interrupt"))?;
    fs::write(root.join("live.json.forbid-promotion"), "")?;
    let same = repo(&root.join("identical-artifact"))?;
    let url = start(&svc, &same, deploy.clone())?;
    svc.wait_state(&url, "landed")?;
    let got = release_state(&svc, &url, "deployed")?;
    assert_eq!(got["digest"], got["observed"]);
    assert_eq!(got["commit"], git(&same, &["rev-parse", "HEAD"])?);
    assert!(!root.join("live.json.unexpected-promotion").exists());
    fs::remove_file(root.join("live.json.forbid-promotion"))?;
    fs::write(&live, serde_json::to_vec(&previous)?)?;
    fs::write(root.join("live.json.unhealthy"), "")?;
    let url = start(&svc, &repo(&root.join("rollback"))?, deploy)?;
    assert_eq!(
        release_state(&svc, &url, "rolled-back")?["observed"],
        "sha256:previous"
    );
    Ok(())
}
