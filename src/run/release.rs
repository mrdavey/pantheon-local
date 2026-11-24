//! A release runs only the executable authorized when the run started.
use crate::run::{a, find, s};
use crate::{eval, project};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    env,
    fs::{self, File},
    io::ErrorKind,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Serialize, Deserialize, Clone)]
struct Destination {
    executable: PathBuf,
    #[serde(default)]
    environment: Vec<String>,
    #[serde(default)]
    rollback: bool,
    #[serde(default)]
    digest: String,
}
#[derive(Deserialize, Serialize)]
struct Receipt {
    digest: String,
    healthy: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Phase {
    Build,
    Readback,
    Deploy,
    Rollback,
}

impl Phase {
    fn name(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Readback => "readback",
            Self::Deploy => "deploy",
            Self::Rollback => "rollback",
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ReleaseJob {
    repo: String,
    target: String,
    config: Destination,
    phase: Phase,
    operation: String,
    destination: String,
    commit: String,
    #[serde(default)]
    source: String,
    artifact: PathBuf,
    #[serde(default)]
    digest: String,
    #[serde(default)]
    previous: String,
}
fn config(dir: &Path) -> Result<BTreeMap<String, Destination>> {
    let file = match File::open(dir.join("destinations.json")) {
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(BTreeMap::new()),
        f => f?,
    };
    Ok(serde_json::from_reader(file)?)
}
fn valid_id(id: &str) -> bool {
    (1..=48).contains(&id.len())
        && !id.starts_with('-')
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
pub fn destinations(state: &Path) -> Result<Vec<String>> {
    Ok(config(state)?.into_keys().filter(|k| valid_id(k)).collect())
}
pub fn scrub_worker(cmd: &mut Command, state: &Path) -> Result<()> {
    for d in config(state)?.values() {
        for name in &d.environment {
            ensure!(
                !name.is_empty() && !name.contains(['=', '\0']),
                "invalid release environment name"
            );
            cmd.env_remove(name);
        }
    }
    Ok(())
}
fn artifact_digest(p: &Path) -> Result<String> {
    ensure!(
        p.symlink_metadata()?.is_file(),
        "expected a regular artifact"
    );
    Ok(format!("sha256:{}", eval::hash_file(p)?))
}
pub fn authorize(run: &mut Value, state: &Path) -> Result<()> {
    match s(&run["input"], "delivery") {
        "" | "review" | "auto" => return Ok(()),
        "deploy" => (),
        _ => bail!("choose review, auto-land, or auto-deploy"),
    }
    let name = s(&run["input"], "destination");
    ensure!(valid_id(name), "choose a configured destination");
    let mut d = config(state)?.remove(name).context("unknown destination")?;
    ensure!(
        d.executable.is_absolute(),
        "release adapter path must be absolute"
    );
    d.executable = d.executable.canonicalize()?;
    ensure!(
        d.executable.metadata()?.permissions().mode() & 0o111 != 0,
        "release adapter must be executable"
    );
    d.digest = artifact_digest(&d.executable)?;
    run["deploy_config"] = serde_json::to_value(d)?;
    Ok(())
}
impl ReleaseJob {
    fn invoke(&self, phase: &str, deadline: Instant, log: &File) -> Result<Vec<u8>> {
        let mut cmd = Command::new(&self.config.executable);
        cmd.arg(phase)
            .env_clear()
            .stderr(log.try_clone()?)
            .stdout(Stdio::null());
        for key in ["PATH", "LANG"] {
            if let Some(v) = env::var_os(key) {
                cmd.env(key, v);
            }
        }
        if !matches!(phase, "build" | "verify") {
            for key in &self.config.environment {
                ensure!(
                    !key.contains(['=', '\0']) && !key.is_empty(),
                    "invalid release environment name"
                );
                if let Some(v) = env::var_os(key) {
                    cmd.env(key, v);
                }
            }
        }
        if !self.source.is_empty() {
            cmd.current_dir(&self.source);
        }
        let input = json!({"operation":self.operation,"destination":self.destination,"commit":self.commit,"source":self.source,"artifact":self.artifact,"digest":self.digest,"previous":self.previous});
        eval::execute(
            &mut cmd,
            Some(&serde_json::to_vec(&input)?),
            phase == "status",
            deadline,
        )
    }
    fn status(&self, deadline: Instant, log: &File) -> Result<Receipt> {
        Ok(serde_json::from_slice(
            &self.invoke("status", deadline, log)?,
        )?)
    }
}
pub fn supervise(job: &Value, log: &mut File) -> Result<Value> {
    let mut j: ReleaseJob = serde_json::from_value(job.clone())?;
    let deadline = Instant::now() + Duration::from_secs(20 * 60);
    ensure!(
        artifact_digest(&j.config.executable)? == j.config.digest,
        "authorized release adapter changed"
    );
    let text = match j.phase {
        Phase::Build => {
            project::check_snapshot(&j.source, &j.commit)?;
            j.invoke("build", deadline, log)?;
            project::check_snapshot(&j.source, &j.commit)?;
            j.digest = artifact_digest(&j.artifact)?;
            fs::set_permissions(&j.artifact, fs::Permissions::from_mode(0o400))?;
            j.invoke("verify", deadline, log)?;
            ensure!(
                artifact_digest(&j.artifact)? == j.digest,
                "artifact changed during verification"
            );
            j.digest
        }
        Phase::Readback => serde_json::to_string(&j.status(deadline, log)?)?,
        Phase::Deploy | Phase::Rollback => {
            let live = j.status(deadline, log)?;
            ensure!(
                live.digest
                    == if j.phase == Phase::Rollback {
                        &j.digest
                    } else {
                        &j.previous
                    }
                    .as_str(),
                "live artifact moved before release"
            );
            if j.phase == Phase::Deploy {
                ensure!(
                    project::target_head(&j.repo, &j.target)? == j.commit,
                    "target moved before deployment"
                );
                ensure!(
                    artifact_digest(&j.artifact)? == j.digest,
                    "release artifact changed"
                );
            }
            j.invoke(j.phase.name(), deadline, log)?;
            String::new()
        }
    };
    Ok(json!({"text":text}))
}
pub fn next_job(
    run: &mut Value,
    all_runs: impl FnOnce() -> Result<Vec<Value>>,
    root: &Path,
) -> Result<Option<(String, Value)>> {
    if run["state"] == "stopped" || run["deploy_config"].is_null() {
        return Ok(None);
    }
    for i in 0..a(&run["releases"]).len() {
        let entry = &run["releases"][i];
        if entry["held"] == true {
            continue;
        }
        let (phase, next) = match s(entry, "state") {
            "queued" => (Phase::Build, "building"),
            "built" => (Phase::Readback, "reading-baseline"),
            "staged" => (Phase::Deploy, "deploying"),
            "verify" => (Phase::Readback, "verifying"),
            "needs-rollback" => (Phase::Rollback, "rolling-back"),
            "verify-rollback" => (Phase::Readback, "checking-rollback"),
            "reconcile" => (Phase::Readback, "reconciling"),
            _ => continue,
        };
        if all_runs()?.iter().flat_map(|r| a(&r["releases"])).any(|e| {
            e["id"] != entry["id"]
                && e["destination"] == entry["destination"]
                && (matches!(
                    s(e, "state"),
                    "reading-baseline"
                        | "deploying"
                        | "verify"
                        | "verifying"
                        | "needs-rollback"
                        | "rolling-back"
                        | "verify-rollback"
                        | "checking-rollback"
                        | "uncertain"
                        | "reconcile"
                        | "reconciling"
                ) || (e["state"] == "staged" && entry["state"] == "built"))
        }) {
            return Ok(None);
        }
        let repo = &run["repo"];
        let repo_root = s(repo, "Root");
        let target = s(repo, "Target");
        let commit = s(entry, "commit");
        if matches!(phase, Phase::Deploy | Phase::Build)
            && project::target_head(repo_root, target)? != commit
        {
            run["releases"][i]["state"] = json!("superseded");
            run["releases"][i]["detail"] = json!("The target moved before promotion.");
            return Ok(None);
        }
        let id = s(entry, "id").to_owned();
        let dir = root.join("releases").join(&id);
        let mut source = String::new();
        if phase == Phase::Build {
            fs::create_dir_all(&dir)?;
            let checkout = dir.join("source");
            if checkout.exists() {
                fs::remove_dir_all(&checkout)?;
            }
            project::clone_repo(
                root.join("integration")
                    .to_str()
                    .context("invalid integration path")?,
                checkout.to_str().context("invalid release path")?,
                commit,
                "swarm-release",
            )?;
            source = checkout
                .join(s(repo, "Subdir"))
                .to_str()
                .context("invalid source path")?
                .to_owned();
        }
        let j = ReleaseJob {
            repo: repo_root.into(),
            target: target.into(),
            config: serde_json::from_value(run["deploy_config"].clone())?,
            phase,
            operation: format!("{id}/{}", phase.name()),
            destination: s(entry, "destination").into(),
            commit: commit.into(),
            source,
            artifact: dir.join("artifact"),
            digest: s(entry, "digest").into(),
            previous: s(entry, "previous").into(),
        };
        if matches!(phase, Phase::Deploy | Phase::Rollback) {
            run["releases"][i]["last_effect"] = json!(phase);
        }
        run["releases"][i]["state"] = json!(next);
        return Ok(Some((id, json!({"release":j}))));
    }
    Ok(None)
}
pub fn finish(run: &mut Value, id: &str, result: &Value) -> Result<()> {
    let i = find(&run["releases"], id).context("release intent missing")?;
    let can_rollback = run["deploy_config"]["rollback"] == true;
    let e = &mut run["releases"][i];
    let err = s(result, "error");
    let text = s(result, "text");
    let receipt = serde_json::from_str::<Receipt>(text)
        .ok()
        .filter(|_| err.is_empty());
    e["detail"] = json!(err);
    let state = match s(e, "state") {
        "building" => {
            e["digest"] = json!(text);
            if err.is_empty() && !text.is_empty() {
                "built"
            } else {
                "failed"
            }
        }
        "reading-baseline" => match receipt {
            None => "failed",
            Some(r) if !r.digest.is_empty() && !r.healthy => {
                e["detail"] = json!("Current live artifact is unhealthy.");
                "failed"
            }
            Some(r) => {
                e["previous"] = json!(r.digest);
                if r.healthy && r.digest == s(e, "digest") {
                    e["observed"] = json!(r.digest);
                    e["at"] = json!(chrono::Utc::now());
                    "deployed"
                } else {
                    "staged"
                }
            }
        },
        "deploying" => "verify",
        "rolling-back" => "verify-rollback",
        "verifying" | "checking-rollback" | "reconciling" => {
            let rollback = e["last_effect"] == "rollback" || e["state"] == "checking-rollback";
            if let Some(r) = receipt {
                e["observed"] = json!(r.digest);
                if r.digest == s(e, "digest") && r.healthy && !rollback {
                    e["at"] = json!(chrono::Utc::now());
                    "deployed"
                } else if r.digest == s(e, "previous") && (r.digest.is_empty() || r.healthy) {
                    e["at"] = json!(chrono::Utc::now());
                    e["detail"] = json!("The previous artifact is active.");
                    if rollback { "rolled-back" } else { "failed" }
                } else if r.digest == s(e, "digest") && !r.healthy && !rollback {
                    e["detail"] = json!("The promoted artifact failed its health check.");
                    if can_rollback && !s(e, "previous").is_empty() {
                        "needs-rollback"
                    } else {
                        "failed"
                    }
                } else {
                    e["detail"] =
                        json!("Destination reports an unexpected artifact or unhealthy rollback.");
                    "uncertain"
                }
            } else {
                e["detail"] =
                    json!("Live artifact could not be verified. Reconcile before another release.");
                "uncertain"
            }
        }
        other => bail!("unexpected release state {other}"),
    };
    e["state"] = json!(state);
    Ok(())
}
pub fn command(run: &mut Value, action: &Value) -> Result<()> {
    let i = find(&run["releases"], s(action, "task")).context("unknown release")?;
    let e = &mut run["releases"][i];
    match s(action, "kind") {
        "hold-release" | "resume-release" => {
            ensure!(
                matches!(
                    s(e, "state"),
                    "queued" | "building" | "built" | "reading-baseline" | "staged"
                ),
                "promotion already started or ended"
            );
            e["held"] = json!(action["kind"] == "hold-release");
        }
        "reconcile-release" => {
            ensure!(
                e["state"] == "uncertain",
                "this release does not need reconciliation"
            );
            e["state"] = json!("reconcile");
        }
        _ => bail!("unknown release command"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn release_history_is_read_only_for_an_actionable_phase() -> Result<()> {
        let mut run = json!({"deploy_config":{}, "releases":[
            {"id":"held","state":"staged","held":true,"destination":"test"},
            {"id":"done","state":"deployed","destination":"test"}
        ]});
        let root = Path::new(".");
        assert!(next_job(&mut run, || panic!("idle release read other runs"), root)?.is_none());
        run["releases"][0]["held"] = json!(false);
        let other = json!({"releases":[{"id":"pending","state":"uncertain","destination":"test"}]});
        assert!(next_job(&mut run, || Ok(vec![other]), root)?.is_none());
        assert_eq!(run["releases"][0]["state"], "staged");
        Ok(())
    }

    #[test]
    fn receipt_requires_both_fields() {
        for raw in [
            r#"{"healthy":true}"#,
            r#"{"digest":null,"healthy":true}"#,
            r#"{"digest":"old"}"#,
        ] {
            assert!(serde_json::from_str::<Receipt>(raw).is_err());
        }
        assert!(serde_json::from_str::<Receipt>(r#"{"digest":"","healthy":false}"#).is_ok());
    }
    #[test]
    fn reconciles_already_live() {
        let mut r = json!({"releases":[{"id":"r","state":"reconciling","previous":"same","digest":"same"}]});
        finish(
            &mut r,
            "r",
            &json!({"text":"{\"digest\":\"same\",\"healthy\":true}"}),
        )
        .unwrap();
        assert_eq!(r["releases"][0]["state"], "deployed");
    }
}
