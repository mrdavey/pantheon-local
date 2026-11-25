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
  *status=success*) cat "$FIXTURE/green" ;;
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
        fs::write(dir.path().join("green"), r#"{"workflow_runs":[]}"#)?;
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
            .env("SCOPE", r#"{"browser":true,"fmt":true,"lint":["*"],"package":["ubuntu-24.04","macos-latest"],"tests":["*"]}"#)
            .env("HISTORY", "[]")
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
    assert_eq!(fixture.plan()?["checks"], r#"["web"]"#);
    assert_eq!(fixture.plan()?["platforms"], "[]");
    fixture.base = common::git(&fixture.dir.path().join("repo"), &["rev-parse", "HEAD"])?;
    fs::remove_file(fixture.dir.path().join("repo/web/src/main.jsx"))?;
    fixture.commit("tests/service.rs", "test")?;
    assert_eq!(
        serde_json::from_str::<Value>(&fixture.plan()?["scope"])?["tests"],
        json!(["test:service"])
    );
    assert_eq!(
        fixture.plan()?["checks"],
        r#"["fmt", "lint", "tests", "web"]"#
    );
    fixture.base = common::git(&fixture.dir.path().join("repo"), &["rev-parse", "HEAD"])?;
    fixture.commit("scripts/package.sh", "package")?;
    assert_eq!(fixture.plan()?["checks"], r#"["package"]"#);
    let release = fixture.run(&["plan", ""], &[("RELEASE_TAG", "true")])?;
    assert_eq!(release["checks"], r#"["browser"]"#);
    fs::write(
        fixture.dir.path().join("base"),
        r#"{"workflow_runs":[{"conclusion":"failure"}]}"#,
    )?;
    assert_eq!(
        fixture.plan()?["checks"],
        r#"["browser", "build", "fmt", "lint", "package", "tests", "web"]"#
    );
    let release = fixture.run(&["plan", ""], &[("RELEASE_TAG", "true")])?;
    assert_eq!(
        release["checks"],
        r#"["browser", "build", "fmt", "lint", "tests"]"#
    );
    Ok(())
}

#[test]
fn web_changes_need_only_node_and_chrome_even_across_pending_pushes() -> Result<()> {
    let fixture = Fixture::new()?;
    let root = fixture.dir.path();
    fixture.commit("web/src/main.jsx", "React")?;
    let pending = common::git(&root.join("repo"), &["rev-parse", "HEAD"])?;
    fixture.commit("web/src/style.css", "CSS")?;
    fs::write(
        root.join("base"),
        r#"{"workflow_runs":[{"status":"in_progress","conclusion":null}]}"#,
    )?;
    fs::write(
        root.join("green"),
        json!({"workflow_runs":[{"head_sha":fixture.base}]}).to_string(),
    )?;
    let plan = fixture.run(&["plan", &pending], &[])?;
    assert_eq!(plan["platforms"], "[]");
    assert_eq!(plan["checks"], r#"["web"]"#);
    for tool in ["rustc", "cargo", "google-chrome"] {
        let path = root.join(tool);
        let command = if tool == "google-chrome" {
            "echo Chrome"
        } else {
            "exit 99"
        };
        fs::write(&path, format!("#!/bin/sh\n{command}\n"))?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    let check = fixture.run(
        &["web"],
        &[("SCOPE", &plan["scope"]), ("CHROME_BIN", "google-chrome")],
    )?;
    assert_eq!(check["run_web"], "true");
    assert_eq!(check.len(), 2);
    fixture.commit("src/service.rs", "Rust")?;
    let mixed = fixture.run(&["plan", &pending], &[])?;
    assert_eq!(
        mixed["checks"],
        r#"["build", "fmt", "lint", "tests", "web"]"#
    );
    assert_eq!(mixed["platforms"], r#"["ubuntu-24.04", "macos-latest"]"#);
    Ok(())
}

#[test]
fn reuses_only_matching_successful_steps_from_trusted_runs() -> Result<()> {
    let fixture = Fixture::new()?;
    fixture.commit("src/main.rs", "code")?;
    let keys = fixture.run(&["check"], &[])?;
    assert_ne!(
        fixture.run(&["check"], &[("SCOPE", r#"{"tests":["test:service"]}"#)])?["tests"],
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

#[test]
fn routes_multiple_targets_shared_sources_and_platforms() -> Result<()> {
    let mut fixture = Fixture::new()?;
    for name in ["e2e", "experiments", "service", "workbench", "ci"] {
        fixture.commit(&format!("tests/{name}.rs"), "mod common;")?;
    }
    for name in [
        "fixture-agent",
        "fixture-release",
        "experiment-grader",
        "hello-check",
        "quotes-check",
    ] {
        fixture.commit(&format!("examples/{name}.rs"), "fn main() {}")?;
    }
    let repo = fixture.dir.path().join("repo");
    fixture.base = common::git(&repo, &["rev-parse", "HEAD"])?;
    fixture.commit("tests/e2e.rs", "// changed")?;
    fixture.commit("tests/service.rs", "// changed")?;
    let plan = fixture.plan()?;
    let scope: Value = serde_json::from_str(&plan["scope"])?;
    let pair = json!(["test:e2e", "test:service"]);
    assert_eq!(scope, json!({"fmt": true, "lint": pair, "tests": pair}));
    assert_eq!(plan["node_platforms"], "[]");
    let checks = fixture.run(&["check"], &[("SCOPE", &plan["scope"])])?;
    assert_eq!(checks["need_assets"], "false");
    assert_eq!(checks["build_targets"], r#"["example:fixture-agent"]"#);

    // A later edit to an unrelated test must leave this pair's receipt valid.
    fixture.commit("tests/ci.rs", "// policy regression")?;
    let unchanged = fixture.run(&["check"], &[("SCOPE", &plan["scope"])])?;
    assert_eq!(unchanged["tests"], checks["tests"]);
    fixture.base = common::git(&repo, &["rev-parse", "HEAD"])?;
    fixture.commit("examples/fixture-release.rs", "// release fixture")?;
    let scope: Value = serde_json::from_str(&fixture.plan()?["scope"])?;
    assert_eq!(
        scope["tests"],
        json!([
            "example:fixture-release",
            "test:experiments",
            "test:workbench"
        ])
    );
    assert_eq!(scope["lint"], json!(["example:fixture-release"]));
    assert_eq!(scope["browser"], true);

    fixture.base = common::git(&repo, &["rev-parse", "HEAD"])?;
    fixture.commit("tests/browser/mod.rs", "// shared ChromeDriver client")?;
    let scope: Value = serde_json::from_str(&fixture.plan()?["scope"])?;
    assert_eq!(
        scope["lint"],
        json!(["example:quotes-check", "test:workbench"])
    );
    assert_eq!(scope["build"], json!(["example:quotes-check"]));

    fixture.base = common::git(&repo, &["rev-parse", "HEAD"])?;
    fixture.commit("src/run/tests.rs", "// daemon unit tests")?;
    let scope: Value = serde_json::from_str(&fixture.plan()?["scope"])?;
    assert_eq!(
        scope,
        json!({"fmt":true,"lint":["bin:swarm"],"tests":["bin:swarm"]})
    );
    fixture.base = common::git(&repo, &["rev-parse", "HEAD"])?;
    fixture.commit("scripts/open-pantheon-local.command", "#!/bin/sh")?;
    let plan = fixture.plan()?;
    assert_eq!(plan["platforms"], r#"["macos-latest"]"#);
    assert_eq!(plan["checks"], r#"["package"]"#);
    fixture.commit("tests/service.rs", "// mixed")?;
    let plan = fixture.plan()?;
    let linux = fixture.run(&["check"], &[("SCOPE", &plan["scope"])])?;
    assert!(!linux.contains_key("run_package"));
    assert_eq!(linux["run_tests"], "true");
    Ok(())
}

#[test]
fn discovers_new_cargo_targets_and_handles_removals_and_unknown_inputs() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let repo = fixture.dir.path().join("repo");
    fixture.commit("examples/new-check/main.rs", "fn main() {}")?;
    fixture.base = common::git(&repo, &["rev-parse", "HEAD"])?;
    fixture.commit("examples/new-check/helper.rs", "// helper")?;
    let scope: Value = serde_json::from_str(&fixture.plan()?["scope"])?;
    assert_eq!(
        scope,
        json!({"fmt":true,"lint":["example:new-check"],"build":["example:new-check"],"tests":["example:new-check"]})
    );
    fixture.base = common::git(&repo, &["rev-parse", "HEAD"])?;
    fixture.commit("examples/recipe.json", "{}")?;
    assert_eq!(fixture.plan()?["platforms"], "[]");
    fixture.commit("examples/recipe.json", "{")?;
    assert!(fixture.plan().is_err());
    fs::remove_file(repo.join("examples/recipe.json"))?;
    fs::remove_dir_all(repo.join("examples/new-check"))?;
    fixture.commit("docs/note.md", "removed example")?;
    let scope: Value = serde_json::from_str(&fixture.plan()?["scope"])?;
    assert_eq!(scope["build"], json!(["*"]));
    fixture.base = common::git(&repo, &["rev-parse", "HEAD"])?;
    fixture.commit("future-tool.config", "unknown")?;
    let scope: Value = serde_json::from_str(&fixture.plan()?["scope"])?;
    assert_eq!(scope["lint"], json!(["*"]));
    Ok(())
}
