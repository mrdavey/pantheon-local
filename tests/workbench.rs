//! Desktop and touch-sized Chromium flows, backed by the real Rust service.
mod browser;
mod common;
#[path = "workbench/proxy.rs"]
mod proxy;
use anyhow::{Context, Result, ensure};
use browser::{Browser, Locator, until};
use common::{Service, example, git, repo};
use proxy::{Once, Proxy};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, os::unix::fs::symlink};
use tempfile::TempDir;

struct Harness {
    b: Browser,
    proxy: Proxy,
    svc: Service,
    dir: TempDir,
    mobile: bool,
}
impl Harness {
    fn new(mobile: bool) -> Result<Self> {
        let dir = tempfile::tempdir()?;
        let native = dir.path().join("fake-codex");
        symlink(example("fixture-agent")?, &native)?;
        let fixture = json!({"kind":"custom","executable":example("fixture-agent")?});
        let worker = |model: &str| json!({"kind":"custom","executable":fixture["executable"],"benchmark_model":model,"benchmark_agent":"Fixture Agent"});
        let hermes_home = dir.path().join("hermes-home");
        let desktop_profile = hermes_home.join("profiles/desktop-work");
        fs::create_dir_all(&desktop_profile)?;
        fs::write(desktop_profile.join("config.yaml"), "")?;
        let profiles = json!({"profiles":{"hermes":{"kind":"hermes","executable":fixture["executable"],"hermes_profile":"default"},"first":fixture,"second":worker("Efficient model"),"writer":worker("Frontier model"),"writer-two":worker("Frontier model"),"codex":{"kind":"codex","executable":native},"claude":{"kind":"claude","executable":dir.path().join("missing-claude")}}});
        let env = BTreeMap::from([
            ("HERMES_HOME".into(), hermes_home),
            ("SWARM_TEST_SIGNED_IN".into(), dir.path().join("signed-in")),
            ("SWARM_TEST_LIVE".into(), dir.path().join("live.json")),
        ]);
        let svc = Service::with_profiles(dir.path(), env, &profiles)?;
        let now = chrono::Utc::now();
        let row = |model, effort, score, cost| json!({"model":model,"effort":effort,"agent":"Fixture Agent","class":"terminal","suite":"Fixture Bench","source":"https://example.invalid/benchmark","date":now.date_naive(),"score":score,"cost_per_trial":cost});
        fs::write(
            svc.state.join("benchmark-cache.json"),
            serde_json::to_vec(
                &json!({"fetched":now,"rows":[row("Frontier model","",91,1.),row("Efficient model","high",88,0.1)]}),
            )?,
        )?;
        let proxy = Proxy::new(&svc.info)?;
        let b = Browser::launch(None, mobile)?;
        Ok(Self {
            b,
            proxy,
            svc,
            dir,
            mobile,
        })
    }
    fn run(&self, scenario: fn(&Self) -> Result<()>) -> Result<()> {
        let result = scenario(self);
        if result.is_err() {
            let name = std::thread::current()
                .name()
                .unwrap_or("workbench")
                .to_owned();
            let artifacts = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/workbench")
                .join(name);
            if fs::create_dir_all(&artifacts).is_ok() {
                let _ = self.b.screenshot(&artifacts.join("page.png"));
                let _ = self.b.trace(&artifacts.join("browser.json"));
            }
            if let Ok(text) = self.b.eval("return document.body.innerText", vec![]) {
                eprintln!("Page at failure: {text}");
            }
            if let Ok(runs) = self.svc.get("/runs") {
                for r in common::rows(&runs) {
                    for c in common::rows(&r["candidates"]) {
                        if c["state"] == "inconclusive"
                            && let Some(log) = c["log"].as_str()
                        {
                            eprintln!(
                                "Candidate check log: {}",
                                fs::read_to_string(log).unwrap_or_default()
                            );
                        }
                    }
                }
            }
        }
        result
    }
    fn goto(&self) -> Result<()> {
        self.b.goto(&format!(
            "{}/#{}",
            self.proxy.url,
            self.svc.info["token"].as_str().unwrap()
        ))
    }
    fn button(&self, name: &str) -> Locator<'_> {
        self.b.role("button", name)
    }
    fn click(&self, name: &str) -> Result<()> {
        self.button(name).click()
    }
    fn preview(&self, link: Locator, filename: &str) -> Result<()> {
        let old = self.b.windows()?;
        link.click()?;
        let window = until("preview tab", || {
            Ok(self.b.windows()?.into_iter().find(|w| !old.contains(w)))
        })?;
        self.b.switch(&window)?;
        self.b.role("link", filename).visible()?;
        let url = url::Url::parse(&self.b.url()?)?;
        ensure!(
            url.origin() != url::Url::parse(&self.proxy.url)?.origin()
                && url.origin()
                    != url::Url::parse(self.svc.info["url"].as_str().unwrap())?.origin(),
            "preview must have its own origin"
        );
        self.b.close_window()?;
        self.b.switch(&old[0])
    }
    fn no_overflow(&self) -> Result<()> {
        ensure!(
            self.b.eval(
                "return document.documentElement.scrollWidth <= innerWidth",
                vec![]
            )? == true,
            "horizontal overflow"
        );
        Ok(())
    }
    fn run_button(&self, brief: &str) -> Locator<'_> {
        self.b.xpath(&format!(
            "//nav[@aria-label='Runs']//button[contains(.,{})]",
            browser::literal(brief)
        ))
    }
    fn repo(&self, name: &str) -> Result<std::path::PathBuf> {
        repo(&self.dir.path().join(name))
    }
}
fn requires_link(h: &Harness) -> Result<()> {
    h.b.goto(&h.proxy.url)?;
    h.b.role("alert", "").contains("link printed in Terminal")
}
fn color_and_keyboard(h: &Harness) -> Result<()> {
    h.goto()?;
    h.b.role("heading", "Agent setup").visible()?;
    ensure!(
        h.button("Agent setup").attr("aria-current")? == "page",
        "setup navigation not active"
    );
    for scheme in ["dark", "light"] {
        h.b.cdp("Emulation.setEmulatedMedia",json!({"features":[{"name":"prefers-color-scheme","value":scheme},{"name":"prefers-reduced-motion","value":"reduce"}]}))?;
        until("requested color scheme", || {
            Ok((h.b.eval(
                "return getComputedStyle(document.documentElement).colorScheme",
                vec![],
            )? == scheme)
                .then_some(()))
        })?;
        h.no_overflow()?;
    }
    h.b.key("\u{e004}")?;
    h.b.role("link", "Skip to content").focused()
}
fn failures(h: &Harness) -> Result<()> {
    h.proxy.inject(Once::SnapshotFailure);
    h.goto()?;
    h.b.role("alert", "")
        .contains("Couldn’t load Pantheon Local")?;
    h.click("Try again")?;
    h.b.role("navigation", "Workspace").visible()?;
    h.b.role("heading", "Agent setup").visible()?;
    h.proxy.inject(Once::MissingGit);
    h.click("Check again")?;
    h.b.role("alert", "").contains("Git is required")?;
    h.b.text("Sign-in unverified").visible()?;
    h.button("Use first").enabled(false)?;
    if h.mobile {
        h.button("Check again").tap()?;
    } else {
        h.button("Check again").focus()?;
        h.b.key("\u{e007}")?;
    }
    h.button("Use first").enabled(true)?;
    if !h.mobile {
        h.button("Check again").focused()?;
    }
    h.click("Use first")?;
    h.click("Advanced settings")?;
    h.proxy.inject(Once::DestinationsFailure);
    h.b.role("combobox", "Delivery").select("deploy")?;
    h.b.role("alert", "")
        .contains("Couldn’t load destinations")?;
    h.proxy.inject(Once::DestinationsSuccess);
    h.click("Reload destinations")?;
    h.b.role("combobox", "Destination").select("test-staging")?;
    h.b.text("The tested commit can land and deploy to this destination automatically.")
        .visible()?;
    h.no_overflow()
}
fn desktop_profile_roles(h: &Harness) -> Result<()> {
    h.goto()?;
    let profile = h.button("Use Hermes · desktop-work");
    if h.mobile {
        profile.tap()?;
    } else {
        profile.focus()?;
        h.b.key("\u{e007}")?;
    }
    h.b.label("Coordinator").value("hermes-desktop-work")?;
    let worker = h.b.role("checkbox", "Hermes · desktop-work");
    ensure!(worker.property("checked")? == true);
    h.b.label("Coordinator").select("first")?;
    ensure!(worker.property("checked")? == true);
    h.no_overflow()
}

fn run_form(h: &Harness) -> Result<()> {
    let folder = h.dir.path().join("new #project");
    fs::create_dir(&folder)?;
    fs::write(folder.join("example.txt"), "hello")?;
    let encoded = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("cwd", folder.to_str().context("fixture path")?)
        .finish();
    h.b.goto(&format!(
        "{}/#{}&{encoded}",
        h.proxy.url,
        h.svc.info["token"].as_str().unwrap()
    ))?;
    h.click("New run")?;
    h.b.label("Project folder")
        .value(folder.to_str().unwrap())?;
    h.b.role("checkbox", "first").visible()?;
    h.b.role("checkbox", "Codex").absent()?;
    h.b.role("checkbox", "Claude Code").absent()?;
    h.b.xpath("//select[@aria-label='Coordinator']/option[@value='claude' or @value='codex']")
        .absent()?;
    h.b.label("Coordinator").select("hermes-desktop-work")?;
    h.click("Agent default · Model settings")?;
    h.b.text("Choose the model and reasoning level in your Hermes profile.")
        .visible()?;
    h.b.key("\u{e00c}")?;
    h.click("Advanced settings")?;
    h.b.label("Delivery").value("auto")?;
    h.b.label("Worker routing").value("manual")?;
    h.click("Choose files…")?;
    h.b.role("checkbox", "example.txt").checked(true)?;
    h.click("Use selected files")?;
    h.button("Choose files (1)").visible()?;
    h.click("Done")?;
    h.b.label("Project folder").fill("/tmp/another-project")?;
    h.click("Advanced settings")?;
    h.button("Choose files…").visible()?;
    h.no_overflow()
}

fn export(h: &Harness) -> Result<()> {
    let project = h.repo("quotes")?;
    let p = project.to_str().unwrap();
    let b = &h.b;
    h.goto()?;
    b.role("heading", "Agent setup").visible()?;
    b.role("heading", "Your configured agents").visible()?;
    h.click("Set up new agent")?;
    b.text("Sign in required").visible()?;
    b.role("link", "Install Claude Code").visible()?;
    fs::write(h.dir.path().join("signed-in"), "")?;
    h.click("Check again")?;
    b.role("link", "Install Claude Code").visible()?;
    h.click("Done")?;
    b.text("Signed in").visible()?;
    h.click("Settings for second")?;
    b.label("Benchmark effort").fill("high")?;
    h.click("Save model settings")?;
    b.text("Saved for new runs.").visible()?;
    h.click("Close dialog")?;
    h.click("Use first")?;
    b.role("heading", "Start a run").focused()?;
    b.label("Coordinator").value("first")?;
    ensure!(b.role("checkbox", "first").property("checked")? == true);
    h.button("Needs you").focus()?;
    b.key("\u{e004}")?;
    b.label("What do you want to build?").focused()?;
    h.proxy.inject(Once::FolderSelected);
    h.click("Browse…")?;
    b.label("Project folder")
        .value("/tmp/Project With Spaces")?;
    h.proxy.inject(Once::FolderCancelled);
    h.click("Browse…")?;
    h.button("Browse…").enabled(true)?;
    b.label("Project folder")
        .value("/tmp/Project With Spaces")?;
    b.label("Project folder").fill(p)?;
    b.label("What do you want to build?")
        .fill("Add quote export")?;
    h.click("Agent setup")?;
    h.click("New run")?;
    b.label("Project folder").value(p)?;
    b.label("What do you want to build?")
        .value("Add quote export")?;
    b.label("Coordinator").select("first")?;
    for (role, checked) in [("first", false), ("writer", true), ("second", true)] {
        b.role("checkbox", role).checked(checked)?;
    }
    h.click("Advanced settings")?;
    b.label("Delivery").value("auto")?;
    b.label("Delivery").select("review")?;
    let routing = b.xpath("//label[contains(.,'Worker routing')]/select");
    routing.value("manual")?;
    routing.select("balanced")?;
    h.click("Done")?;
    h.click("Improve brief")?;
    until("advice", || {
        Ok(b.label("Suggested brief")
            .property("value")?
            .as_str()
            .unwrap_or_default()
            .contains("Keep quotes private")
            .then_some(()))
    })?;
    h.click("Use suggestion")?;
    b.text("Original brief").click()?;
    h.click("Restore original")?;
    h.click("Start run")?;
    b.role("heading", "Quote export").visible()?;
    b.text("Full brief").focus()?;
    b.key("\u{e007}")?;
    b.xpath("//details[@open]//p[.='Add quote export']")
        .visible()?;
    b.key("\u{e007}")?;
    h.button("Coordinator chat").visible()?;
    b.label("Who can view quotes?").fill("private")?;
    h.click("Send answer")?;
    b.role("heading", "Review acceptance checks").visible()?;
    let fixture = example("fixture-agent")?;
    let check = json!([fixture, "check", "export.txt", "open quotes"]);
    b.xpath(r#"//code[contains(., "export.txt 'open quotes'")]"#)
        .visible()?;
    b.text("View or edit JSON").click()?;
    b.label("Acceptance recipe").fill("")?;
    h.click("Approve checks")?;
    b.role("alert", "").contains("Enter a valid JSON object")?;
    b.label("Acceptance recipe").value("")?;
    b.label("Acceptance recipe")
        .fill(&json!({"checks":[check],"preview":[fixture,"serve-preview"]}).to_string())?;
    h.click("Approve checks")?;
    h.click("Work")?;
    h.button("Land on main").visible()?;
    b.xpath("//section[header[contains(@class,'run-heading')]]//select[@aria-label='Coordinator' or parent::label[contains(.,'Coordinator')]]").absent()?;
    h.button("Open project folder").visible()?;
    let milestone = b.xpath("//button[@data-task='export']");
    milestone
        .xpath(".//strong[.='Write the open quotes export.']")
        .visible()?;
    milestone.click()?;
    b.xpath("//dialog[@open]//code[.='open quotes']")
        .visible()?;
    b.text("Why this model?").click()?;
    b.xpath("//*[contains(text(),'Efficient model') and contains(.,'88.0%')]")
        .visible()?;
    h.click("Close dialog")?;
    let launcher = h.button("Coordinator chat");
    if h.mobile {
        launcher.tap()?;
    } else {
        launcher.focus()?;
        b.key("\u{e007}")?;
        b.label("Message").focused()?;
    }
    b.role("dialog", "Coordinator chat").visible()?;
    h.no_overflow()?;
    let question = "Why did the coordinator split the **work**?";
    b.label("Message").fill(question)?;
    b.key("\u{e00c}")?;
    launcher.focused()?;
    h.click("Coordinator chat")?;
    b.label("Message").value(question)?;
    h.click("Send")?;
    b.xpath("//article//strong[.='work']").visible()?;
    b.text("The export milestone owns the generated CSV.")
        .visible()?;
    b.label("Message").fill("Add a timing milestone")?;
    h.click("Send")?;
    b.role("heading", "Proposed milestones").visible()?;
    h.click("Add milestones")?;
    b.text("Milestones added").visible()?;
    h.click("Minimize chat")?;
    h.click("Work")?;
    h.button("Land on main").visible()?;
    h.proxy.inject(Once::SnapshotRace);
    b.reload()?;
    until("events poll after snapshot cursor", || {
        let v = h.proxy.cursor_checked();
        ensure!(v != Some(false), "stale event cursor");
        Ok(v.map(|_| ()))
    })?;
    h.run_button("Quote export").click()?;
    h.click("Coordinator chat")?;
    b.text("Milestones added").visible()?;
    b.text("Explain the result after reopening.").visible()?;
    h.click("Minimize chat")?;
    h.click("Work")?;
    let candidate = b.xpath("//article[.//button[normalize-space(.)='Land on main']]");
    candidate.xpath(".//button[.='View diff']").click()?;
    b.label("Candidate diff").contains("timing.txt")?;
    b.label("Changed file").select("1")?;
    b.label("Changes to timing.txt").contains("+")?;
    h.no_overflow()?;
    b.key("\u{e00c}")?;
    candidate.xpath(".//button[.='View checks']").click()?;
    b.label("Acceptance checks").contains("open quotes")?;
    b.xpath("//dialog[@open]//code[contains(@class,'shell-command')]")
        .visible()?;
    h.click("Close dialog")?;
    candidate
        .xpath(".//button[.='Open output folder']")
        .visible()?;
    candidate.xpath(".//button[.='Start preview']").click()?;
    h.preview(b.xpath("//a[contains(.,'Open preview')]"), "export.txt")?;
    h.click("Land on main")?;
    b.xpath("//main//*[text()='Landed']").visible()?;
    ensure!(b.label("Landed commit").text_content()? == git(&project, &["rev-parse", "HEAD"])?);
    ensure!(fs::read_to_string(project.join("export.txt"))?.contains("open quotes"));
    Ok(())
}
fn valid_commit(b: &Browser) -> Result<()> {
    let text = b.label("Landed commit").text_content()?;
    ensure!(
        text.len() == 40 && text.bytes().all(|c| c.is_ascii_hexdigit()),
        "invalid landed commit"
    );
    Ok(())
}
fn compare(h: &Harness) -> Result<()> {
    let mut recipe = common::experiment_recipe()?;
    recipe["cases"][0]["samples"] = json!(2);
    recipe["rule"]["min_samples"] = json!(2);
    h.svc.post("/runs",&json!({"path":h.repo("experiment")?,"prompt":"Measure approaches","coordinator":"first","workers":["writer","writer-two"],"compare":true,"delivery":"auto","recipe":recipe}))?;
    h.goto()?;
    h.run_button("Measure approaches").click()?;
    h.click("Experiment")?;
    let b = &h.b;
    let outcome = b.xpath("//*[contains(concat(' ',@class,' '),' outcome ')]");
    outcome.contains("Landed")?;
    outcome.contains("balanced")?;
    b.text("relevance (hits) by latency (ms)").visible()?;
    valid_commit(b)?;
    h.click("Work")?;
    valid_commit(b)?;
    h.click("Experiment")?;
    b.text("Eval suite").click()?;
    b.text("latency must be at most 5 ms").visible()?;
    h.click("Close dialog")?;
    let balanced = h.button("balanced, 7 relevance (hits)");
    balanced.focus()?;
    balanced.focused()?;
    let desc = balanced.attr("aria-describedby")?;
    b.xpath(&format!(
        "//*[@id={}]",
        browser::literal(desc.as_str().context("plot description")?)
    ))
    .contains("3.05 latency (ms)")?;
    ensure!(
        balanced.rect()?["y"].as_f64().unwrap()
            < h.button("Baseline, 3 relevance (hits)").rect()?["y"]
                .as_f64()
                .unwrap()
    );
    h.proxy.inject(Once::EvidenceFailure);
    h.click("Compare case results")?;
    b.role("rowheader", "search").visible()?;
    b.text("Couldn’t load case results for 1 implementation.")
        .visible()?;
    h.click("Reload case results")?;
    h.button("Reload case results").absent()?;
    b.role("combobox", "Show").select("Regressions")?;
    b.text("No regressions in the loaded results.").visible()?;
    b.label("Feedback for the coordinator")
        .fill("Exact title lookup should survive.")?;
    h.click("Send feedback")?;
    b.xpath("//button[contains(.,'Needs you') and contains(.,'1')]")
        .click()?;
    b.role("heading", "Should this behavior be required?")
        .visible()?;
    h.click("Dismiss")?;
    b.text("No open questions or blockers in your inbox.")
        .visible()?;
    h.click("Show history")?;
    h.click("Restore to inbox")?;
    h.click("Yes")?;
    h.click("Send answer")?;
    b.text("No open questions or blockers in your inbox.")
        .visible()?;
    h.run_button("Measure approaches").click()?;
    b.text("Project eval cases").click()?;
    b.text("Exact title lookup should survive.").visible()?;
    h.click("Close dialog")?;
    h.click("Improve this version")?;
    b.role("checkbox", "fast").checked(true)?;
    b.label("What should carry over?")
        .fill("Keep the faster formatter.")?;
    h.proxy.inject(Once::CombinationRace);
    h.click("Build combination")?;
    let round = b.role("combobox", "Round");
    round.value("2")?;
    b.text("From balanced + fast").visible()?;
    h.button("Resume auto-land").visible()?;
    round.select("1")?;
    outcome.contains("Landed")?;
    outcome
        .xpath(".//*[self::h2 or self::h3]")
        .contains("balanced")?;
    round.select("2")?;
    h.click("Keep current version")?;
    outcome.contains("Keeping the baseline")?;
    outcome.contains("Baseline")?;
    h.button("Land selected on main").absent()?;
    b.xpath("//tr[contains(.,'From balanced + fast')]//button[.='Select']")
        .click()?;
    outcome.contains("Selected")?;
    h.button("Land selected on main").visible()
}
fn blind(h: &Harness) -> Result<()> {
    let b = &h.b;
    fs::write(
        h.dir.path().join("live.json"),
        br#"{"digest":"sha256:previous","healthy":true}"#,
    )?;
    fs::write(
        h.svc.state.join("destinations.json"),
        serde_json::to_vec(
            &json!({"staging":{"executable":example("fixture-release")?,"environment":["SWARM_TEST_LIVE"]}}),
        )?,
    )?;
    h.svc.post("/runs",&json!({"path":h.repo("blind")?,"prompt":"Measure approaches with blind review","coordinator":"first","workers":["writer","writer-two"],"compare":true,"delivery":"deploy","destination":"staging","recipe":{"checks":[["true"]],"preview":[example("fixture-agent")?,"serve-preview"],"rubric":"Which preview best serves the intended search?"}}))?;
    h.goto()?;
    h.run_button("Measure approaches with blind review")
        .click()?;
    h.click("Experiment")?;
    h.click("Blind review")?;
    h.click("Start review")?;
    for side in ["A", "B"] {
        h.click(&format!("Load {side}"))?;
        h.preview(b.role("link", &format!("Open {side}")), "result.txt")?;
    }
    h.click("Tie")?;
    b.text("tie").visible()?;
    b.xpath("//tr[contains(.,'balanced')]//button[.='Select']")
        .click()?;
    valid_commit(b)?;
    b.text("Live and healthy").visible()?;
    let live: Value = serde_json::from_slice(&fs::read(h.dir.path().join("live.json"))?)?;
    ensure!(live["healthy"] == true);
    b.text(live["digest"].as_str().unwrap()).visible()
}
macro_rules! scenario {
    ($desktop:ident,$mobile:ident,$body:ident) => {
        #[test]
        #[ignore = "requires Chrome and ChromeDriver"]
        fn $desktop() -> Result<()> {
            Harness::new(false)?.run($body)
        }
        #[test]
        #[ignore = "requires Chrome and ChromeDriver"]
        fn $mobile() -> Result<()> {
            Harness::new(true)?.run($body)
        }
    };
}
scenario!(desktop_session_link, mobile_session_link, requires_link);
scenario!(
    desktop_setup_themes,
    mobile_setup_themes,
    color_and_keyboard
);
scenario!(desktop_setup_recovery, mobile_setup_recovery, failures);
fn blocked_milestone(h: &Harness) -> Result<()> {
    let brief =
        "Goal\nRecover the fixture milestone\n\nRequirements\nUse the reviewed worker files.";
    let run = h.svc.post("/runs",&json!({"path":h.repo("recovery")?,"prompt":brief,"coordinator":"first","workers":["writer"],"recipe":{"checks":[["true"]]}}))?;
    let path = common::run_path(&run)?;
    h.goto()?;
    h.b.xpath("//button[contains(.,'Needs you') and contains(.,'1')]")
        .click()?;
    h.b.role("heading", "recover-export").visible()?;
    h.b.text("Cannot write Git metadata.").visible()?;
    h.click("Dismiss")?;
    h.b.text("No open questions or blockers in your inbox.")
        .visible()?;
    h.click("Show history")?;
    h.click("Restore to inbox")?;
    let state = h.svc.get(&path)?;
    let workspace = std::path::Path::new(
        state["tasks"][0]["workspace"]
            .as_str()
            .context("worker checkout")?,
    );
    git(workspace, &["add", "export.txt"])?;
    git(workspace, &["commit", "-m", "Record reviewed export"])?;
    h.click("Retry milestone")?;
    h.run_button("Recover the fixture milestone").click()?;
    h.b.role("heading", "Recover the fixture milestone")
        .visible()?;
    h.b.xpath("//button[@data-task='recover-export']").click()?;
    h.b.text("Recovered the verified commit from the worker checkout.")
        .visible()?;
    h.no_overflow()
}
scenario!(
    desktop_blocked_milestone,
    mobile_blocked_milestone,
    blocked_milestone
);
fn turn_activity(h: &Harness) -> Result<()> {
    let folder = h.repo("formatted-activity")?.join("packages/cli");
    fs::create_dir_all(&folder)?;
    fs::write(folder.join("README.md"), "Expense CLI")?;
    git(&folder, &["add", "README.md"])?;
    git(&folder, &["commit", "-m", "Create the nested project"])?;
    let run = h.svc.post(
        "/runs",
        &json!({
            "path":folder, "prompt":"Add quote export",
            "coordinator":"first", "workers":["writer"]
        }),
    )?;
    let path = common::run_path(&run)?;
    let saved = h.svc.wait_state(&path, "question")?;
    let turn = saved["turns"][0]["id"].as_str().context("plan turn")?;
    let log = h
        .svc
        .state
        .join("runs")
        .join(saved["id"].as_str().unwrap())
        .join("turns")
        .join(turn)
        .join("events.log");
    // The native adapters save JSON lines. The browser must read both contracts.
    let events = [
        json!({"type":"agent_thought_chunk","text":"private-fixture"}),
        json!({"type":"system","raw":{"subtype":"init","session_id":"private-fixture"}}),
        json!({"type":"item.completed","raw":{"item":{"type":"reasoning","text":"private-fixture"}}}),
        json!({"type":"assistant","raw":{"message":{"content":[{"type":"thinking","thinking":"private-fixture"},
            {"type":"redacted_thinking","data":"private-fixture"},{"type":"text","text":"Public response."}]}}}),
        json!({"type":"agent_message_chunk","raw":{"update":{"messageId":"reply"}},"text":"Review **"}),
        json!({"type":"agent_message_chunk","raw":{"update":{"messageId":"reply"}},"text":"storage**."}),
        json!({"type":"item.completed","raw":{"item":{"type":"command_execution",
            "command":"cargo test --locked","aggregated_output":"test restart ... ok","exit_code":0}}}),
        json!({"type":"agent_message_chunk","raw":{"update":{"messageId":"receipt"}},"text":"{\"status\":\"ready\",\"files\":[\"src/main.rs\"]}"}),
    ];
    fs::write(
        log,
        events
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )?;
    h.goto()?;
    h.run_button("Quote export").click()?;
    let folder_button = h.button("Open project folder");
    folder_button.contains("cli")?;
    ensure!(
        folder_button.attr("title")?.as_str() == Some(folder.canonicalize()?.to_str().unwrap())
    );
    h.b.xpath("//button[starts-with(.,'View activity')]")
        .click()?;
    h.click("View log for Planning")?;
    let log = h.b.label("Turn log");
    log.xpath(".//strong[.='storage']").visible()?;
    log.xpath(".//code[.='cargo test --locked']").visible()?;
    log.xpath(".//pre[.='test restart ... ok']").visible()?;
    log.xpath(".//dt[.='Files']").visible()?;
    log.contains("src/main.rs")?;
    log.contains("Public response.")?;
    ensure!(!log.text_content()?.contains("private-fixture"));
    h.no_overflow()?;
    h.click("Back to activity")?;
    h.click("Close dialog")?;
    h.b.xpath("//button[starts-with(.,'View activity')]")
        .focused()
}
scenario!(desktop_turn_activity, mobile_turn_activity, turn_activity);
fn chat_history(h: &Harness) -> Result<()> {
    let b = &h.b;
    let run = h.svc.post(
        "/runs",
        &json!({
            "path": h.repo("history")?, "prompt": "Add quote export",
            "coordinator": "first", "workers": ["writer"]
        }),
    )?;
    let path = common::run_path(&run)?;
    h.svc.wait_state(&path, "question")?;
    let mut message = String::from("Review the `total` command.\n\n");
    for n in 1..=45 {
        message.push_str(&format!("- Check row {n}\n"));
    }
    message.push_str("\n![Screenshot](https://example.invalid/private-image)\n");
    h.svc.post(&path, &json!({"kind":"chat","text":message}))?;
    h.goto()?;
    h.run_button("Quote export").click()?;
    h.click("Coordinator chat")?;
    let history = b.role("log", "Coordinator conversation");
    history.contains("The export milestone owns the generated CSV.")?;
    history.xpath(".//code[.='total']").visible()?;
    history.xpath(".//img").absent()?;
    history.xpath(".//li[.='Check row 45']").visible()?;
    history.focus()?;
    b.key("\u{e011}")?; // Home
    h.button("Latest messages ↓").visible()?;
    b.label("Message").fill("Keep this draft")?;
    h.click("Minimize chat")?;
    h.click("Agent setup")?;
    h.run_button("Quote export").click()?;
    h.click("Coordinator chat")?;
    b.label("Message").value("Keep this draft")?;
    h.click("Latest messages ↓")?;
    h.button("Latest messages ↓").absent()?;
    b.label("Message")
        .fill("Keyboard **send**\u{e009}\u{e007}\u{e000}")?; // Ctrl + Enter
    history.xpath(".//strong[.='send']").visible()?;
    b.label("Message").value("")?;
    h.svc.post(&path, &json!({"kind":"stop"}))?;
    b.text("Resume the run to send a message.").visible()?;
    h.button("Send").enabled(false)?;
    h.no_overflow()
}
scenario!(desktop_chat_history, mobile_chat_history, chat_history);
scenario!(desktop_run_form, mobile_run_form, run_form);
scenario!(desktop_export, mobile_export, export);
scenario!(
    desktop_hermes_roles,
    mobile_hermes_roles,
    desktop_profile_roles
);
scenario!(desktop_comparison, mobile_comparison, compare);
scenario!(desktop_blind_review, mobile_blind_review, blind);

// Reopening keeps the session and uses the current launcher’s directory.
#[test]
fn open_reuses_the_listening_service() -> Result<()> {
    use nix::{
        sys::signal::{Signal, kill},
        unistd::Pid,
    };
    use std::{
        env,
        process::{Command, Stdio},
        time::Duration,
    };
    use wait_timeout::ChildExt;
    let base = tempfile::tempdir()?;
    let state = base.path().join("state");
    let opener = base.path().join("opened");
    for name in ["open", "xdg-open"] {
        symlink(example("fixture-agent")?, base.path().join(name))?;
    }
    let path = env::join_paths(
        std::iter::once(base.path().to_owned()).chain(env::split_paths(
            &env::var_os("PATH").context("PATH missing")?,
        )),
    )?;
    let launch = || {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_swarm"));
        cmd.arg("open")
            .env("PATH", &path)
            .env("SWARM_STATE_DIR", &state)
            .env("SWARM_TEST_OPENED", &opener)
            .stdout(Stdio::null());
        cmd
    };
    let mut child = launch().spawn()?;
    let check = (|| -> Result<()> {
        let url = until("first open", || {
            Ok(fs::read_to_string(&opener)
                .ok()
                .filter(|s| !s.trim().is_empty()))
        })?;
        let info: Value = serde_json::from_slice(&fs::read(state.join("service.json"))?)?;
        let expected = |cwd: &std::path::Path| -> Result<String> {
            let path = fs::canonicalize(cwd)?;
            let query = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("cwd", path.to_str().context("cwd")?)
                .finish();
            Ok(format!(
                "{}/#{}&{query}\n",
                info["url"].as_str().context("URL")?,
                info["token"].as_str().context("token")?
            ))
        };
        ensure!(
            url == expected(&env::current_dir()?)?,
            "incorrect workbench link"
        );
        let second_cwd = base.path().join("new project #2&tests");
        fs::create_dir(&second_cwd)?;
        let mut second = launch().current_dir(&second_cwd).spawn()?;
        let Some(status) = second.wait_timeout(Duration::from_secs(5))? else {
            second.kill()?;
            second.wait()?;
            anyhow::bail!("second open failed to reuse service")
        };
        ensure!(
            status.success() && fs::read_to_string(&opener)? == url + &expected(&second_cwd)?,
            "incorrect reopened link"
        );
        Ok(())
    })();
    let _ = kill(Pid::from_raw(child.id() as i32), Signal::SIGTERM);
    if child.wait_timeout(Duration::from_secs(5))?.is_none() {
        child.kill()?;
        child.wait()?;
    }
    check
}

fn delete_from_settings(h: &Harness) -> Result<()> {
    let repo = h.repo("delete")?;
    let run = h.svc.post("/runs", &json!({"path":repo,"prompt":"Add quote export","coordinator":"first","workers":["writer"]}))?;
    let path = common::run_path(&run)?;
    let waiting = h.svc.wait_state(&path, "question")?;
    h.goto()?;
    h.run_button("Quote export").click()?;
    h.click("Run settings")?;
    h.b.role("heading", "Run settings").visible()?;
    h.click("Pause run")?;
    h.click("Delete run")?;
    h.click("Cancel")?;
    h.b.key("\u{e00c}")?;
    h.button("Run settings").focused()?;
    h.click("Settings for Quote export")?;
    h.click("Delete run")?;
    h.click("Delete permanently")?;
    h.b.role("heading", "Start a run").visible()?;
    h.run_button("Quote export").absent()?;
    ensure!(h.svc.get(&path).is_err());
    ensure!(repo.join(".git").is_dir());
    ensure!(
        !h.svc
            .state
            .join("runs")
            .join(waiting["id"].as_str().unwrap())
            .exists()
    );
    h.no_overflow()
}
scenario!(
    desktop_run_settings,
    mobile_run_settings,
    delete_from_settings
);
