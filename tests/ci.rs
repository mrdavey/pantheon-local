//! Exercise the Actions entry points with real Git trees and offline run history.
mod common;
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap, env, fs, os::unix::fs::PermissionsExt, path::Path, process::Command,
};
use tempfile::TempDir;

struct Fixture {
    dir: TempDir,
    base: String,
}

impl Fixture {
    fn new() -> Result<Self> {
        let dir = tempfile::tempdir()?;
        let repo = common::repo(&dir.path().join("repo"))?;
        common::git(&repo, &["commit", "--allow-empty", "-m", "base"])?;
        let base = common::git(&repo, &["rev-parse", "HEAD"])?;
        let gh = dir.path().join("gh");
        fs::write(
            &gh,
            r#"#!/bin/sh
case "$2" in
  *head_sha=*) cat "$FIXTURE/base" ;;
  *workflows/ci.yml/runs*) cat "$FIXTURE/runs" ;;
  *runs/1/jobs*|*attempts/1/jobs*) cat "$FIXTURE/jobs" ;;
  *) exit 1 ;;
esac
"#,
        )?;
        fs::set_permissions(gh, fs::Permissions::from_mode(0o755))?;
        fs::write(
            dir.path().join("base"),
            r#"{"workflow_runs":[{"conclusion":"success"}]}"#,
        )?;
        fs::write(dir.path().join("runs"), r#"{"workflow_runs":[]}"#)?;
        Ok(Self { dir, base })
    }

    fn commit(&self, name: &str, data: &str) -> Result<()> {
        let repo = self.dir.path().join("repo");
        let path = repo.join(name);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, data)?;
        common::git(&repo, &["add", "-A"])?;
        common::git(&repo, &["commit", "-m", "change"])?;
        Ok(())
    }

    fn run(&self, args: &[&str], vars: &[(&str, &str)]) -> Result<BTreeMap<String, String>> {
        let root = self.dir.path();
        let output = root.join("outputs");
        fs::write(&output, "")?;
        let path = env::join_paths(
            std::iter::once(root.to_path_buf())
                .chain(env::split_paths(&env::var_os("PATH").unwrap())),
        )?;
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/ci/checks.py");
        let result = Command::new("python3")
            .arg(script)
            .args(args)
            .current_dir(root.join("repo"))
            .env("FIXTURE", root)
            .env("GITHUB_OUTPUT", output.as_path())
            .env("GITHUB_REPOSITORY", "owner/repo")
            .env("GITHUB_RUN_ATTEMPT", "1")
            .env("RELEASE_TAG", "false")
            .env("PATH", path)
            .env("RUNNER_OS", "Linux")
            .env("RUNNER_ARCH", "X64")
            .env("ImageOS", "ubuntu24")
            .env("ImageVersion", "one")
            .env("CHECKS", r#"["browser","fmt","lint","package","tests"]"#)
            .env("HISTORY", "[]")
            .env("TEST_TARGET", "")
            .envs(vars.iter().copied())
            .output()?;
        ensure!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        Ok(fs::read_to_string(output)?
            .lines()
            .map(|line| {
                let (name, value) = line.split_once('=').unwrap();
                (name.to_owned(), value.to_owned())
            })
            .collect())
    }

    fn plan(&self) -> Result<BTreeMap<String, String>> {
        self.run(&["plan", self.base.trim()], &[])
    }
}

#[test]
fn selects_changes_against_a_verified_base_including_deletions() -> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.commit("docs/notes.md", "notes")?;
    assert_eq!(fixture.plan()?["platforms"], "[]");
    fixture.commit("web/src/main.jsx", "code")?;
    assert_eq!(fixture.plan()?["checks"], r#"["browser"]"#);
    assert_eq!(fixture.plan()?["platforms"], r#"["ubuntu-24.04"]"#);
    fixture.base = common::git(&fixture.dir.path().join("repo"), &["rev-parse", "HEAD"])?;
    fs::remove_file(fixture.dir.path().join("repo/web/src/main.jsx"))?;
    fixture.commit("tests/service.rs", "test")?;
    assert_eq!(fixture.plan()?["test_target"], "service");
    assert_eq!(
        fixture.plan()?["checks"],
        r#"["browser", "fmt", "lint", "tests"]"#
    );
    fixture.base = common::git(&fixture.dir.path().join("repo"), &["rev-parse", "HEAD"])?;
    fixture.commit("scripts/package.sh", "package")?;
    assert_eq!(fixture.plan()?["checks"], r#"["package"]"#);
    let release = fixture.run(&["plan", ""], &[("RELEASE_TAG", "true")])?;
    assert_eq!(release["checks"], "[]");
    fs::write(
        fixture.dir.path().join("base"),
        r#"{"workflow_runs":[{"conclusion":"failure"}]}"#,
    )?;
    assert_eq!(
        fixture.plan()?["checks"],
        r#"["browser", "fmt", "lint", "package", "tests"]"#
    );
    let release = fixture.run(&["plan", ""], &[("RELEASE_TAG", "true")])?;
    assert_eq!(release["checks"], r#"["browser", "fmt", "lint", "tests"]"#);
    Ok(())
}

#[test]
fn reuses_only_matching_successful_steps_from_trusted_runs() -> Result<()> {
    let fixture = Fixture::new()?;
    fixture.commit("src/main.rs", "code")?;
    let keys = fixture.run(&["check"], &[])?;
    assert_ne!(
        fixture.run(&["check"], &[("TEST_TARGET", "service")])?["tests"],
        keys["tests"]
    );
    let step = |lane: &str, result: &str, time: &str| json!({"name":keys[lane],"conclusion":result,"completed_at":time});
    let jobs = json!({"jobs":[{"steps":[step("tests","success","1"),step("fmt","success","1"),
        step("tests","failure","2"),step("fmt","skipped","2")]}]});
    let root = fixture.dir.path();
    fs::write(root.join("jobs"), jobs.to_string())?;
    let head = common::git(&root.join("repo"), &["rev-parse", "HEAD"])?;
    let run = |id, repo| {
        json!({"id":id,"status":"completed","event":"pull_request","head_sha":head,
                                "head_repository":{"full_name":repo}})
    };
    fs::write(
        root.join("runs"),
        json!({"workflow_runs":[run(2,"fork/repo"),run(1,"owner/repo")]}).to_string(),
    )?;
    let plan = fixture.plan()?;
    assert_eq!(
        serde_json::from_str::<Value>(&plan["history"])?,
        json!([keys["fmt"]])
    );
    let check = fixture.run(&["check"], &[("HISTORY", &plan["history"])])?;
    assert_eq!(check["run_fmt"], "false");
    assert_eq!(check["run_tests"], "true");
    let check = fixture.run(
        &["check"],
        &[("ImageVersion", "two"), ("HISTORY", &plan["history"])],
    )?;
    assert_eq!(check["run_fmt"], "true");
    fixture.commit("web/src/main.jsx", "web")?;
    let changed = fixture.run(&["check"], &[])?;
    assert_eq!(changed["fmt"], keys["fmt"]);
    for lane in ["assets", "browser", "tests", "package"] {
        assert_ne!(changed[lane], keys[lane]);
    }
    fixture.commit(".github/workflows/ci.yml", "new recipe")?;
    assert_ne!(fixture.run(&["check"], &[])?["fmt"], keys["fmt"]);
    Ok(())
}
