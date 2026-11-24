//! Explicit, signed-in native-agent qualification. These tests consume provider quota.
mod common;
use anyhow::{Context, Result, ensure};
use common::*;
use serde_json::{Value, json};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

fn profiles() -> Result<Value> {
    if let Some(path) = env::var_os("SWARM_NATIVE_PROFILES") {
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    } else {
        Ok(json!({"profiles":{"codex":{"kind":"codex"}}}))
    }
}
fn workers() -> Vec<String> {
    env::var("SWARM_NATIVE_WORKERS")
        .unwrap_or_else(|_| "codex".into())
        .split(',')
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}
fn timeout() -> Result<Duration> {
    Ok(Duration::from_secs(
        env::var("SWARM_NATIVE_TIMEOUT")
            .unwrap_or_else(|_| "240".into())
            .parse()?,
    ))
}
fn rustc() -> Result<PathBuf> {
    if let Some(p) = env::var_os("SWARM_NATIVE_RUSTC") {
        return Ok(fs::canonicalize(p)?);
    }
    let result = Command::new("rustup")
        .args(["which", "rustc"])
        .output()
        .context("set SWARM_NATIVE_RUSTC to an absolute compiler path or install rustup")?;
    ensure!(result.status.success(), "rustup which rustc failed");
    Ok(Path::new(std::str::from_utf8(&result.stdout)?.trim()).canonicalize()?)
}
fn driver() -> Result<PathBuf> {
    if let Some(p) = env::var_os("CHROMEDRIVER") {
        return Ok(fs::canonicalize(p)?);
    }
    env::split_paths(&env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join("chromedriver"))
        .find(|p| p.is_file())
        .context("set CHROMEDRIVER to a driver matching the browser")?
        .canonicalize()
        .map_err(Into::into)
}
struct NativeRun<'a> {
    svc: &'a Service,
    url: String,
}
impl NativeRun<'_> {
    fn start<'a>(
        svc: &'a Service,
        project: &Path,
        prompt: &str,
        mut options: Value,
    ) -> Result<NativeRun<'a>> {
        options["path"] = json!(project);
        options["coordinator"] =
            json!(env::var("SWARM_NATIVE_COORDINATOR").unwrap_or_else(|_| "codex".into()));
        options["workers"] = json!(workers());
        options["prompt"] = json!(prompt);
        let url = run_path(&svc.post("/runs", &options)?)?;
        eprintln!("Started native run {url}");
        Ok(NativeRun { svc, url })
    }
    fn wait(&self, done: impl Fn(&Value) -> bool) -> Result<Value> {
        let timeout = timeout()?;
        until(timeout, || {
            let r = self.svc.get(&self.url)?;
            if r["state"] == "blocked"
                || r["state"] == "question"
                || r["observer_error"].as_str().is_some_and(|s| !s.is_empty())
                || rows(&r["tasks"]).iter().any(|t| t["state"] == "blocked")
            {
                let logs = rows(&r["turns"])
                    .iter()
                    .filter(|t| {
                        matches!(t["role"].as_str(), Some("plan" | "observe"))
                            && t["state"] == "received"
                    })
                    .filter_map(|t| {
                        self.svc
                            .get(&format!("{}/turns/{}", self.url, t["id"].as_str()?))
                            .ok()
                    })
                    .filter_map(|v| {
                        v.as_str().map(|s| {
                            s.chars()
                                .rev()
                                .take(1400)
                                .collect::<Vec<_>>()
                                .into_iter()
                                .rev()
                                .collect::<String>()
                        })
                    })
                    .collect::<Vec<_>>();
                anyhow::bail!("native run cannot proceed: {r}; recent coordinator output: {logs:?}")
            }
            Ok(done(&r).then_some(r))
        })
        .with_context(|| format!("native run status: {:?}", self.svc.get(&self.url)))
    }
}
impl Drop for NativeRun<'_> {
    fn drop(&mut self) {
        if let Ok(r) = self.svc.get(&self.url)
            && rows(&r["turns"]).iter().any(|t| t["state"] == "running")
        {
            let _ = self.svc.post(&self.url, &json!({"kind":"stop"}));
        }
    }
}
#[test]
#[ignore = "uses signed-in coding agents and model quota"]
fn native_experiment() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let svc = Service::with_profiles(tmp.path(), Default::default(), &profiles()?)?;
    let project = repo(&tmp.path().join("project"))?;
    let mut recipe = experiment_recipe()?;
    recipe["cases"][0]["samples"] = json!(1);
    recipe["rule"]["min_samples"] = json!(1);
    let run = NativeRun::start(
        &svc,
        &project,
        "Run a tiny two-variant coding experiment. Use two independent milestones named relevance and throughput, each in its own variant. Relevance creates only result.txt containing correct followed by one newline. Throughput creates only result.txt containing wrong followed by one newline. Commit both. These are fixture labels for the accepted recipe. All product decisions are answered. Afterward I will send feedback for the case library.",
        json!({"compare":true,"delivery":"auto","recipe":recipe}),
    )?;
    let result = run.wait(|r| {
        r["state"] == "landed" || r["state"] == "review" && r["selection"]["state"] == "undecided"
    })?;
    assert_eq!(result["state"], "landed", "{result}");
    assert_eq!(fs::read_to_string(project.join("result.txt"))?, "correct\n");
    eprintln!("Landed {}", git(&project, &["rev-parse", "HEAD"])?);
    svc.post(&run.url,&json!({"kind":"feedback","text":"Please save a draft eval case based on this observation: searching for an exact note title should rank that note first. Ask one focused product question in the inbox about ambiguous titles, with no blocking work."}))?;
    let r = run.wait(|r| !rows(&r["inbox"]).is_empty())?;
    let cases = svc.get(&format!("{}/cases", run.url))?;
    assert!(!rows(&cases).is_empty() && !rows(&r["inbox"]).is_empty());
    Ok(())
}
#[test]
#[ignore = "uses signed-in coding agents and model quota"]
fn native_smoke() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let svc = Service::with_profiles(tmp.path(), Default::default(), &profiles()?)?;
    let project = repo(&tmp.path().join("quotes"))?;
    let (prompt, checks, staged) = if let Some(browser) = env::var_os("SWARM_NATIVE_BROWSER") {
        (
            "Build a tiny private quote tracker. Start with a foundation milestone for README, .gitignore and shared contract. Run independent API and UI milestones after that. Use a standalone Rust standard library HTTP server in app.rs compiled directly with rustc --edition=2024. Bind to 127.0.0.1 and SWARM_PORT. Persist in SWARM_DATA_DIR. UI owns index.html. GET / serves index.html. GET /quotes returns [{id,client,amount,status}]. POST /quotes with {client,amount} stores an open quote. POST /accept with {id} accepts it. Use Client and Amount field labels, an Add quote button, table rows with Accept buttons, Accepted status, and exact text such as Outstanding: $75.00 for open amounts. Commit each milestone.",
            json!([[
                example("quotes-check")?,
                fs::canonicalize(browser)?,
                driver()?,
                rustc()?
            ]]),
            None,
        )
    } else {
        fs::write(project.join("README.md"), "Existing quote tracker\n")?;
        fs::write(
            project.join("quotes.rs"),
            "pub const QUOTES: &[(&str, &str, u32)] = &[(\"North, Inc.\", \"open\", 125), (\"Closed\", \"accepted\", 75)];\n",
        )?;
        git(&project, &["add", "--", "README.md", "quotes.rs"])?;
        git(&project, &["commit", "-m", "Add existing quote tracker"])?;
        fs::write(project.join("README.md"), "Unrelated staged note\n")?;
        git(&project, &["add", "README.md"])?;
        fs::write(project.join("scratch.txt"), "Untracked")?;
        (
            "Add export.rs to this existing Rust project in one milestone. Compile it with rustc directly. Import QUOTES from quotes.rs as a module. Its tuples are (client,status,amount). Print CSV with header client,amount and only open quotes. Escape comma-containing names. Keep the change to export.rs and commit it.",
            json!([
                [rustc()?, "--edition=2024", "export.rs", "-o", "export"],
                [
                    example("fixture-agent")?,
                    "check-output",
                    "./export",
                    "client,amount\n\"North, Inc.\",125\n"
                ]
            ]),
            Some(git(&project, &["ls-files", "--stage"])?),
        )
    };
    let run = NativeRun::start(&svc, &project, prompt, json!({"recipe":{"checks":checks}}))?;
    let r = run.wait(|r| r["state"] == "review")?;
    let c = passing(&r)?;
    if let Some(index) = staged {
        let diff = svc.get(&format!("{}/diff/{}", run.url, c["id"].as_str().unwrap()))?;
        let paths = diff
            .as_str()
            .context("diff text")?
            .lines()
            .filter(|l| l.starts_with("diff --git "))
            .collect::<Vec<_>>();
        assert_eq!(paths, ["diff --git a/export.rs b/export.rs"]);
        assert_eq!(git(&project, &["ls-files", "--stage"])?, index);
        assert_eq!(
            fs::read_to_string(project.join("scratch.txt"))?,
            "Untracked"
        )
    } else {
        let landed = svc.post(&run.url, &json!({"kind":"land","candidate":c["id"]}))?;
        assert_eq!(landed["state"], "landed");
        assert_eq!(git(&project, &["rev-parse", "HEAD"])?, c["commit"])
    }
    eprintln!(
        "Verified {} with milestone commits {}",
        c["commit"], r["tasks"]
    );
    Ok(())
}
