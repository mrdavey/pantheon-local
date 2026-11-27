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
        fs::write(
            desktop_profile.join("provider_models_cache.json"),
            r#"{"local":{"models":["local/model-one","local/model-two","other/fast"]}}"#,
        )?;
        let profiles = json!({"profiles":{"hermes":{"kind":"hermes","executable":fixture["executable"],"hermes_profile":"default"},"first":fixture,"second":worker("Efficient model"),"writer":worker("Frontier model"),"writer-two":worker("Frontier model"),"codex":{"kind":"codex","executable":native},"claude":{"kind":"claude","executable":dir.path().join("missing-claude")}}});
        let env = BTreeMap::from([
            ("HERMES_HOME".into(), hermes_home),
            ("SWARM_TEST_SIGNED_IN".into(), dir.path().join("signed-in")),
            ("SWARM_TEST_LIVE".into(), dir.path().join("live.json")),
            (
                "SWARM_TEST_MILESTONE_GATE".into(),
                dir.path().join("milestone-gate"),
            ),
            ("SWARM_TEST_CHAT_GATE".into(), dir.path().join("chat-gate")),
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
    fn loading_bar(&self) -> Locator<'_> {
        self.b
            .xpath("//*[@class='loading-status window-status' and @data-loading='true']")
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
    h.b.role("alert", "")
        .contains("Open the workbench link in Terminal")?;
    h.goto()?;
    h.b.role("heading", "Agent setup").visible()
}
fn color_and_keyboard(h: &Harness) -> Result<()> {
    h.goto()?;
    h.b.role("heading", "Agent setup").visible()?;
    ensure!(
        h.button("Agent setup").attr("aria-current")? == "page",
        "setup navigation not active"
    );
    ensure!(h.b.eval("return document.documentElement.dataset.appearance", vec![])? == "dark");
    if !h.mobile {
        ensure!(h.b.eval("const centers = ['.local-indicator','.appearance select'].map(s => {const r=document.querySelector(s).getBoundingClientRect();return r.y+r.height/2}); return Math.max(...centers)-Math.min(...centers) <= 1",vec![])? == true, "header controls have different centerlines");
    }
    h.b.label("Appearance").select("system")?;
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
    h.goto()?;
    h.b.role("heading", "Agent setup").visible()?;
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
    h.click("Team")?;
    h.b.label("Coordinator").value("hermes-desktop-work")?;
    let worker = h.b.text("1. Hermes · desktop-work");
    worker.visible()?;
    h.b.label("Coordinator").select("first")?;
    worker.visible()?;
    h.b.label("Minimum agents").absent()?;
    h.b.text("3. Hermes · desktop-work").visible()?;
    h.click("Add agent")?;
    h.b.label("Harness").select("hermes-desktop-work")?;
    let model = h.b.role("combobox", "Model ID");
    model.fill("MoDeL-t")?;
    h.b.role("option", "local/model-two").visible()?;
    h.b.role("option", "other/fast").absent()?;
    h.b.key("\u{e015}")?;
    h.b.key("\u{e007}")?;
    model.value("local/model-two")?;
    model.fill("local-model-two")?;
    h.b.key("\u{e00c}")?;
    h.b.role("dialog", "Add agent").visible()?;
    h.b.label("Thinking effort").select("high")?;
    h.click("Add to pool")?;
    h.click("Settings for worker 4")?;
    h.b.label("Model ID").value("local-model-two")?;
    h.b.label("Thinking effort").select("low")?;
    h.click("Apply to this agent")?;
    h.click("Settings for worker 1")?;
    h.b.label("Model ID").value("")?;
    h.b.label("Thinking effort").value("")?;
    h.click("Close dialog")?;
    h.click("Settings for worker 4")?;
    h.b.label("Thinking effort").value("low")?;
    h.click("Close dialog")?;
    ensure!(h.b.eval("const list=document.querySelector('.worker-pool ol');return getComputedStyle(list).overflowY === 'auto' && list.scrollHeight > list.clientHeight",vec![])? == true);
    h.click("Remove worker 2")?;
    h.click("Remove worker 2")?;
    h.no_overflow()?;
    h.click("Done")?;
    h.button("Team").contains("3 agents")?;
    let project = h.repo("worker-pool")?;
    h.b.label("Project folder")
        .fill(project.to_str().unwrap())?;
    h.b.label("What do you want to build?")
        .fill("Add quote export")?;
    h.click("Start run")?;
    let run = until("worker pool saved", || {
        Ok(h.svc
            .get("/runs")?
            .as_array()
            .and_then(|r| r.first())
            .cloned())
    })?;
    let workers = common::rows(&run["input"]["workers"]);
    ensure!(workers.len() == 2 && workers[0] != workers[1] && run["input"]["min_workers"] == 2);
    let profile = &run["profiles"][workers[1].as_str().unwrap()];
    ensure!(
        profile["kind"] == "hermes"
            && profile["model"] == "local-model-two"
            && profile["effort"] == "low"
    );
    ensure!(h.svc.get("/setup")?["profiles"]["hermes-desktop-work"]["model"] != "local-model-two");
    Ok(())
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
    h.click("Runs")?;
    h.b.label("Project folder")
        .value(folder.to_str().unwrap())?;
    h.click("Team")?;
    h.click("Add agent")?;
    h.b.label("Harness").visible()?;
    h.b.key("\u{e00c}")?;
    h.b.xpath("//select[@aria-label='Coordinator']/option[@value='claude' or @value='codex']")
        .absent()?;
    h.b.text("1. first").visible()?;
    h.b.role("checkbox", "Unlimited subagents").checked(false)?;
    h.b.label("Concurrent subagents").fill("3")?;
    h.b.role("checkbox", "Allow subagents").checked(false)?;
    h.b.label("Concurrent subagents").absent()?;
    h.b.role("checkbox", "Allow subagents").checked(true)?;
    h.b.label("Concurrent subagents").value("3")?;
    h.b.label("Coordinator").select("hermes-desktop-work")?;
    h.click("Agent default · Model settings")?;
    h.b.text("Choose the model and reasoning level in your Hermes profile.")
        .visible()?;
    h.b.key("\u{e00c}")?;
    h.b.role("heading", "Team").visible()?;
    h.button("Agent default · Model settings").focused()?;
    h.click("Done")?;
    h.click("Advanced settings")?;
    h.b.label("Delivery").value("auto")?;
    h.b.label("Worker routing").value("manual")?;
    h.proxy.inject(Once::InitialSelection);
    h.click("Choose files…")?;
    h.button("Remove example.txt").visible()?;
    h.proxy.inject(Once::MoreInitialFiles);
    h.click("Add more files…")?;
    h.button("Remove nested/notes.md").visible()?;
    h.click("Remove example.txt")?;
    h.button("Remove example.txt").absent()?;
    h.click("Done")?;
    h.b.label("Project folder").fill("/tmp/another-project")?;
    h.click("Advanced settings")?;
    h.button("Choose files…").visible()?;
    h.no_overflow()
}

fn export(h: &Harness) -> Result<()> {
    let project = h.repo("quotes")?;
    fs::create_dir(project.join("docs"))?;
    fs::write(project.join("docs/guide.md"), "# Committed guide\n")?;
    git(&project, &["add", "docs"])?;
    git(&project, &["commit", "-m", "Add project guide"])?;
    let p = project.to_str().unwrap();
    let b = &h.b;
    h.goto()?;
    b.role("heading", "Agent setup").visible()?;
    b.role("heading", "Your configured agents").visible()?;
    h.click("Set up new agent")?;
    b.text("Sign in required").visible()?;
    let request = b.role("link", "Request new agent integration ↗");
    ensure!(
        request.attr("href")?.as_str()
            == Some(
                "https://github.com/mrdavey/pantheon-local/issues/new?template=agent_integration.yml"
            )
    );
    ensure!(request.attr("target")?.as_str() == Some("_blank"));
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
    h.click("Team")?;
    b.label("Coordinator").value("first")?;
    b.text("1. first").visible()?;
    b.text("3. first").visible()?;
    h.click("Done")?;
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
    h.click("Runs")?;
    b.label("Project folder").value(p)?;
    b.label("What do you want to build?")
        .value("Add quote export")?;
    h.click("Team")?;
    b.label("Coordinator").select("first")?;
    for _ in 0..3 {
        h.click("Remove worker 1")?;
    }
    for (index, worker) in ["writer", "second"].into_iter().enumerate() {
        h.click("Add agent")?;
        let harness = b.role("combobox", "Harness");
        harness.select(worker)?;
        harness.value(worker)?;
        h.click("Add to pool")?;
        b.text(&format!("{}. {worker}", index + 1)).visible()?;
    }
    h.click("Done")?;
    h.click("Advanced settings")?;
    b.label("Delivery").value("auto")?;
    b.label("Delivery").select("review")?;
    let routing = b.xpath("//label[contains(.,'Worker routing')]//select");
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
    b.eval(r#"const dt=new DataTransfer(); dt.items.add(new File(["brief evidence"],"scope.md",{type:"text/plain"}));
        document.querySelector('.goal-composer').dispatchEvent(new DragEvent('drop',{dataTransfer:dt,bubbles:true,cancelable:true}));"#, vec![])?;
    h.click("Remove scope.md")?;
    b.eval(r#"const dt=new DataTransfer(); dt.items.add(new File(["brief evidence"],"scope.md",{type:"text/plain"}));
        const input=document.querySelector('input[type=file]'); input.files=dt.files; input.dispatchEvent(new Event('change',{bubbles:true}));"#, vec![])?;
    h.button("Remove scope.md").visible()?;
    b.text("Original brief").click()?;
    h.click("Restore original")?;
    h.click("Start run")?;
    b.role("heading", "Quote export").visible()?;
    let saved = h.svc.get("/runs")?[0].clone();
    ensure!(saved["input"]["attachments"][0]["name"] == "scope.md");
    let job = h
        .svc
        .state
        .join("runs")
        .join(saved["id"].as_str().unwrap())
        .join("turns")
        .join(saved["turns"][0]["id"].as_str().unwrap())
        .join("job.json");
    ensure!(fs::read_to_string(job)?.contains("brief evidence"));
    ensure!(!project.join("scope.md").exists());
    h.click("Run settings")?;
    b.text("View brief").focus()?;
    b.key("\u{e007}")?;
    b.role("dialog", "Run brief").contains("Add quote export")?;
    b.role("dialog", "Run brief").contains("brief evidence")?;
    b.key("\u{e00c}")?;
    h.button("View brief").focused()?;
    h.click("Close dialog")?;
    b.role("dialog", "Coordinator chat").visible()?;
    h.click("Coordinator chat")?;
    b.label("Who can view quotes?").fill("private")?;
    h.click("Send answer")?;
    let completed = h.svc.wait_state(&common::run_path(&saved)?, "review")?;
    ensure!(
        completed["recipe_approved"] == true
            && common::rows(&completed["turns"])
                .iter()
                .any(|turn| turn["role"] == "eval"),
        "the daemon must check the implementation without a workbench approval"
    );
    b.role("heading", "Review acceptance checks").absent()?;
    h.click("Map")?;
    h.click("Zoom in")?;
    h.b.label("Map zoom").contains("125%")?;
    h.click("Filter second assignments")?;
    ensure!(h.button("Filter second assignments").attr("aria-pressed")? == "true");
    h.click("Fit")?;
    h.no_overflow()?;
    h.click("Table")?;
    h.button("Implementations").visible()?;
    b.xpath("//section[header[contains(@class,'run-heading')]]//select[@aria-label='Coordinator' or parent::label[contains(.,'Coordinator')]]").absent()?;
    h.button("Open project folder").visible()?;
    let milestone = b.xpath("//button[@data-task='export']");
    milestone
        .xpath(".//strong[.='Write the open quotes export.']")
        .visible()?;
    milestone.click()?;
    b.xpath("//button[normalize-space(.)='Details']").click()?;
    b.xpath("//*[@data-slot='dialog-content']//code[.='open quotes']")
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
    h.click("Add to run")?;
    b.text("Milestones added").visible()?;
    h.click("Coordinator chat")?;
    h.click("Milestones")?;
    h.button("Implementations").visible()?;
    b.xpath("//button[@data-task='timing']//*[@aria-label='Milestone 3']")
        .visible()?;
    h.click("Implementations")?;
    b.xpath("//section[@aria-label='Successful implementations']//span[.='Superseded']")
        .absent()?;
    h.click("Milestones")?;
    h.proxy.inject(Once::SnapshotRace);
    b.reload()?;
    until("events poll after snapshot cursor", || {
        let v = h.proxy.cursor_checked();
        ensure!(v != Some(false), "stale event cursor");
        Ok(v.map(|_| ()))
    })?;
    h.run_button("Quote export").click()?;
    b.role("dialog", "Coordinator chat").visible()?;
    b.text("Milestones added").visible()?;
    b.text("Explain the result after reopening.").visible()?;
    h.click("Coordinator chat")?;
    h.click("Milestones")?;
    h.click("Implementations")?;
    let candidate = b.xpath("//article[.//button[normalize-space(.)='Land on main']]");
    ensure!(b.eval("return [...document.querySelectorAll('.work-result .actions button')].every(el => parseFloat(getComputedStyle(el).paddingInlineStart) >= 16 && el.getBoundingClientRect().height >= 44)",vec![])? == true,"implementation action padding");
    candidate.xpath(".//button[.='View diff']").click()?;
    b.label("Candidate diff").contains("timing.txt")?;
    b.label("Changed file").select("timing.txt")?;
    b.label("Changes to timing.txt").contains("+")?;
    h.no_overflow()?;
    b.key("\u{e00c}")?;
    candidate.xpath(".//button[.='View checks']").click()?;
    b.label("Acceptance checks").contains("open quotes")?;
    b.xpath("//*[@data-slot='dialog-content']//code[contains(@class,'shell-command')]")
        .visible()?;
    h.click("Close dialog")?;
    candidate
        .xpath(".//button[.='View final milestone']")
        .click()?;
    b.label("Milestone activity").visible()?;
    h.click("Close dialog")?;
    h.click("Implementations")?;
    candidate.xpath(".//button[.='View details']").click()?;
    ensure!(b.eval("const dialog = document.querySelector('.snapshot-dialog');const close = dialog.querySelector('[aria-label=\"Close dialog\"]').getBoundingClientRect();return close.width >= 44 && close.height >= 44 && parseFloat(getComputedStyle(dialog.querySelector('.snapshot-path button')).paddingInlineStart) >= 10", vec![])? == true, "file dialog action padding");
    h.click("export.txt")?;
    b.role("dialog", "Implementation files")
        .contains("open quotes")?;
    h.click("Project")?;
    h.click("docs")?;
    h.click("guide.md")?;
    b.role("heading", "Committed guide").visible()?;
    h.no_overflow()?;
    b.key("\u{e00c}")?;
    h.click("Land on main")?;
    b.xpath("//main//*[text()='Landed']").visible()?;
    let latest = b.xpath("//section[@aria-label='Successful implementations']/article[1]/header");
    latest.xpath(".//span[.='Passed']").visible()?;
    latest.xpath(".//span[.='Landed']").visible()?;
    b.xpath("//strong[.='Implementation landed']").absent()?;
    let run = h.svc.get("/runs")?;
    ensure!(run[0]["detail"] == git(&project, &["rev-parse", "HEAD"])?);
    ensure!(fs::read_to_string(project.join("export.txt"))?.contains("open quotes"));
    let landed_head = git(&project, &["rev-parse", "HEAD"])?;
    git(&project, &["switch", "-c", "unrelated"])?;
    git(
        &project,
        &["commit", "--allow-empty", "-m", "Unrelated local work"],
    )?;
    let source_url = b.url()?;
    h.click("Start new run")?;
    b.reload()?;
    b.label("What do you want to build?").value("")?;
    b.label("Project folder")
        .value(run[0]["working_folder"].as_str().unwrap())?;
    b.label("Target branch").value("main")?;
    h.no_overflow()?;
    b.label("What do you want to build?")
        .fill("Expand the export")?;
    h.click("Start run")?;
    let next = until("feature follow-up", || {
        Ok(h.svc
            .get("/runs")?
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["input"]["prompt"] == "Expand the export")
            .cloned())
    })?;
    ensure!(next["repo"]["Head"] == landed_head && next["repo"]["Target"] == "refs/heads/main");
    // Exercise a real fix on desktop and a no-edit verification on mobile.
    if !h.mobile {
        git(&project, &["switch", "main"])?;
        fs::write(project.join("export.txt"), "broken export")?;
        git(&project, &["commit", "-am", "Regress the export"])?;
        git(&project, &["switch", "unrelated"])?;
    }
    let verification_base = git(&project, &["rev-parse", "main"])?;
    b.goto(&source_url)?;
    h.click("Find and fix bugs")?;
    let verify = until("correctness run", || {
        Ok(h.svc
            .get("/runs")?
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["input"]["verification_of"].is_object())
            .cloned())
    })?;
    ensure!(
        verify["repo"]["Head"] == verification_base
            && verify["input"]["workers"] == run[0]["input"]["workers"]
            && verify["profiles"] == run[0]["profiles"]
    );
    let path = common::run_path(&verify)?;
    let checked = h.svc.wait_state(&path, "review")?;
    ensure!(checked["candidates"][0]["state"] == "passed");
    ensure!((checked["candidates"][0]["commit"] == verification_base) == h.mobile);
    h.svc.post(
        &path,
        &json!({"kind":"land","candidate":checked["candidates"][0]["id"]}),
    )?;
    ensure!(git(&project, &["show", "main:export.txt"])? == "open quotes");
    ensure!(git(&project, &["branch", "--show-current"])? == "unrelated");
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
    h.svc.post("/runs",&json!({"path":h.repo("experiment")?,"prompt":"Measure approaches","coordinator":"first","worker_pool":[{"profile":"writer"},{"profile":"writer-two"}],"compare":true,"delivery":"auto","recipe":recipe}))?;
    h.goto()?;
    h.run_button("Measure approaches").click()?;
    h.click("Experiment")?;
    let b = &h.b;
    let outcome = b.role("group", "Experiment decision");
    outcome.contains("Landed")?;
    outcome.contains("balanced")?;
    h.click("View decision details")?;
    b.role("heading", "Why this decision?").visible()?;
    valid_commit(b)?;
    b.role("heading", "Included milestones").visible()?;
    h.button("Explain this").visible()?;
    h.click("Close dialog")?;
    b.role("link", "balanced · View implementation").click()?;
    b.role("dialog", "Implementation files").visible()?;
    h.no_overflow()?;
    h.click("Close dialog")?;
    h.click("Experiment")?;
    h.click("Help with experiment actions")?;
    b.xpath("//*[@data-slot='dialog-content']//dt[.='Feedback']")
        .visible()?;
    b.key("\u{e00c}")?;
    h.button("Help with experiment actions").focused()?;
    b.text("relevance (hits) by latency (ms)").visible()?;
    b.label("Landed commit").absent()?;
    h.click("Milestones")?;
    b.label("Landed commit").absent()?;
    b.xpath("//strong[.='Implementation landed']").absent()?;
    h.click("Experiment")?;
    h.click("Evaluation tools")?;
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
    h.click("Give feedback")?;
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
    h.click("Experiment")?;
    h.click("Evaluation tools")?;
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
    h.button("Resume auto-land").visible()?;
    round.select("1")?;
    outcome.contains("Landed")?;
    outcome.contains("balanced")?;
    round.select("2")?;
    h.click("Keep current version")?;
    outcome.contains("Keeping the baseline")?;
    outcome.contains("Baseline")?;
    h.button("Land selected on main").absent()?;
    let choice = b
        .xpath("//label[contains(.,'Selected implementation')]//option[last()]")
        .attr("value")?;
    b.label("Selected implementation")
        .select(choice.as_str().context("candidate ID")?)?;
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
    b.label("Selected implementation").select("balanced")?;
    b.role("group", "Experiment decision").contains("Landed")?;
    h.click("View decision details")?;
    valid_commit(b)?;
    h.click("Close dialog")?;
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
    h.click("Collapse sidebar")?;
    h.run_button("Recover the fixture milestone")
        .xpath(".//span[contains(@class,'state working')]/*[local-name()='svg']")
        .visible()?;
    h.click("Expand sidebar")?;
    h.run_button("Recover the fixture milestone").click()?;
    h.b.xpath("//button[@data-task='recover-export']").click()?;
    let clock = h.b.xpath("//time[@aria-label='Elapsed time']");
    let initial = clock.attr("datetime")?;
    until("milestone clock advances", || {
        Ok((clock.attr("datetime")? != initial).then_some(()))
    })?;
    let activity = h.b.label("Milestone activity");
    activity.xpath(".//tbody/tr[1]").click()?;
    h.b.label("Turn log").visible()?;
    h.b.key("\u{e00c}")?;
    activity.visible()?;
    fs::write(h.dir.path().join("milestone-gate"), "")?;
    h.b.xpath("//button[normalize-space(.)='Details']")
        .click()?;
    h.b.label("View recovery for Cannot write Git metadata.")
        .click()?;
    let details = h.b.label("Recovery details");
    details.contains("Cannot write Git metadata.")?;
    details
        .xpath(".//button[normalize-space(.)='Open source log']")
        .click()?;
    h.b.label("Turn log").visible()?;
    h.b.key("\u{e00c}")?;
    h.click("Close dialog")?;
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
    h.b.xpath("//button[normalize-space(.)='Details']")
        .click()?;
    h.b.text("Recovered the verified commit from the worker checkout.")
        .visible()?;
    let duration =
        h.b.xpath("//time[@aria-label='Completed in']")
            .attr("datetime")?;
    h.click("Close dialog")?;
    h.click("Map")?;
    h.b.xpath("//button[contains(@class,'map-select')]")
        .click()?;
    ensure!(
        h.b.xpath("//time[@aria-label='Completed in']")
            .attr("datetime")?
            == duration
    );
    h.click("Close dialog")?;
    h.click("Activity Log")?;
    h.b.label("Activity type").select("recovery")?;
    h.b.label("View recovery for Cannot write Git metadata.")
        .click()?;
    h.b.label("Recovery details")
        .contains("The operator retried")?;
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
    let stored = h.svc.get(&format!("{path}/turns/{turn}/log"))?;
    ensure!(
        stored["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["at"].is_i64()),
        "native output was not timestamped"
    );
    let two_minutes_ago = chrono::Utc::now().timestamp_millis() - 120_000;
    // The native adapters save JSON lines. The browser must read both contracts.
    let events = [
        json!({"type":"agent_thought_chunk","text":"private-fixture"}),
        json!({"type":"tool_call","raw":{"update":{"title":format!("/fixture/{}", "long-directory-name/".repeat(30))}}}),
        json!({"type":"system","raw":{"subtype":"init","session_id":"private-fixture"}}),
        json!({"type":"item.completed","raw":{"item":{"type":"reasoning","text":"private-fixture"}}}),
        json!({"type":"assistant","raw":{"message":{"content":[{"type":"thinking","thinking":"private-fixture"},
            {"type":"redacted_thinking","data":"private-fixture"},{"type":"text","text":"Public response."}]}}}),
        json!({"type":"agent_message_chunk","raw":{"update":{"messageId":"reply"}},"text":"Review **"}),
        json!({"type":"agent_message_chunk","raw":{"update":{"messageId":"reply"}},"text":"storage**."}),
        json!({"type":"item.completed","raw":{"item":{"type":"command_execution",
            "command":"cargo test --locked","aggregated_output":"test restart ... ok","exit_code":0}}}),
        json!({"type":"agent_message_chunk","raw":{"update":{"messageId":"receipt"}},"logged_at":two_minutes_ago,"text":"{\"status\":\"ready\",\"files\":[\"src/main.rs\"]}"}),
    ];
    fs::write(
        &log,
        (0..51)
            .map(|i| {
                if i == 50 {
                    json!({"type":"usage_update","raw":{"usage":{"tokens":12}}}).to_string()
                } else {
                    format!("Earlier message {i}")
                }
            })
            .chain(events.iter().map(Value::to_string))
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
    h.click("Activity Log")?;
    h.proxy.inject(Once::LogFailure);
    h.b.label("View log for Create the run plan").click()?;
    h.click("Try again")?;
    let log_file = log;
    let log = h.b.label("Turn log");
    let first = log.xpath(".//article[1]");
    first.contains("src/main.rs")?;
    ensure!(h.b.eval("const title = document.querySelector('.log-event strong[title^=\"/fixture/\"]'); return !!title && title.scrollWidth > title.clientWidth && getComputedStyle(title).textOverflow === 'ellipsis';", vec![])? == true, "long activity titles must truncate");
    first.contains("2 minutes ago")?;
    first.xpath(".//time[@datetime and @title]").visible()?;
    log.xpath(".//strong[.='storage']").visible()?;
    log.xpath(".//code[.='cargo test --locked']").visible()?;
    log.xpath(".//pre[.='test restart ... ok']").visible()?;
    log.xpath(".//dt[.='Files']").visible()?;
    log.contains("src/main.rs")?;
    log.contains("Public response.")?;
    let usage = log.xpath(".//*[@data-slot='collapsible'][button/strong[.='Usage update']]");
    ensure!(usage.attr("data-state")? == "closed");
    usage.xpath(".//button[@aria-expanded]").click()?;
    usage.xpath(".//dt[.='Tokens']").visible()?;
    ensure!(!log.text_content()?.contains("private-fixture"));
    log.contains("Time unavailable")?;
    if let Some(dir) = std::env::var_os("SWARM_TEST_SCREENSHOTS") {
        h.b.screenshot(&std::path::PathBuf::from(dir).join(if h.mobile {
            "log-mobile.png"
        } else {
            "log-desktop.png"
        }))?;
    }
    h.button("Older").focus()?;
    h.b.key("\u{e007}")?;
    first.contains("Earlier message 30")?;
    h.click("Older")?;
    first.contains("Earlier message 5")?;
    log.xpath(".//button[normalize-space(.)='Older' and @disabled]")
        .visible()?;
    h.click("Newer")?;
    first.contains("Earlier message 30")?;
    h.click("Latest")?;
    first.contains("src/main.rs")?;
    h.no_overflow()?;
    h.click("Close dialog")?;
    h.b.label("View log for Create the run plan").focused()?;
    fs::write(log_file, "")?;
    h.b.label("View log for Create the run plan").click()?;
    log.contains("This turn has no log output yet.")?;
    h.click("Close dialog")?;
    h.b.label("View log for Create the run plan").focused()
}
scenario!(desktop_turn_activity, mobile_turn_activity, turn_activity);
fn live_turn_log(h: &Harness) -> Result<()> {
    use std::io::Write;
    let run = h.svc.post("/runs", &json!({"path":h.repo("live-log")?,"prompt":"Add quote export","coordinator":"first","workers":["writer"]}))?;
    let path = common::run_path(&run)?;
    h.svc.wait_state(&path, "question")?;
    h.goto()?;
    h.run_button("Quote export").click()?;
    h.svc
        .post(&path, &json!({"kind":"chat","text":"Stream this answer"}))?;
    h.click("Activity Log")?;
    h.b.label("View log for Stream this answer").click()?;
    let log = h.b.label("Turn log");
    log.contains("The live answer")?;
    let saved = h.svc.get(&path)?;
    let turn = saved["turns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["role"] == "chat")
        .context("chat turn")?;
    let file = h
        .svc
        .state
        .join("runs")
        .join(saved["id"].as_str().unwrap())
        .join("turns")
        .join(turn["id"].as_str().unwrap())
        .join("events.log");
    let mut writer = fs::OpenOptions::new().append(true).open(file)?;
    for i in 0..28 {
        writeln!(
            writer,
            "{}",
            json!({"type":"stdout","text":format!("Live output {i}"),"logged_at":chrono::Utc::now().timestamp_millis()})
        )?;
    }
    let first = log.xpath(".//article[1]");
    first.contains("Live output 27")?;
    let region = h.b.role("region", "Log messages");
    region.focus()?;
    h.b.key("\u{e00f}")?; // Page Down
    h.button("Go live").visible()?;
    ensure!(
        h.b.eval(
            "return document.querySelector('.log-messages').scrollTop",
            vec![]
        )?
        .as_f64()
        .unwrap_or(0.)
            > 80.,
        "live updates reset the reading position"
    );
    h.click("Older")?;
    first.contains("Live output 2")?;
    writeln!(
        writer,
        "{}",
        json!({"type":"stdout","text":"Another update","logged_at":chrono::Utc::now().timestamp_millis()})
    )?;
    h.click("Newer")?;
    first.contains("Live output 27")?;
    h.click("Go live")?;
    first.contains("Another update")?;
    h.click("Pause updates")?;
    fs::write(h.dir.path().join("chat-gate"), "")?;
    h.button("Go live").absent()?;
    first.contains("Another update")?;
    h.click("Latest")?;
    log.contains("is complete.")?;
    h.no_overflow()
}
scenario!(desktop_live_turn_log, mobile_live_turn_log, live_turn_log);
fn run_agents(h: &Harness) -> Result<()> {
    let folder = h.repo("inspect-agents")?;
    let run = h.svc.post("/runs", &json!({
        "path":folder,"prompt":"Delegate the report. Native agent log fixture",
        "coordinator":"first","workers":["writer"],"delivery":"auto",
        "recipe":{"checks":[[example("fixture-agent")?,"check","docs/report.txt","reviewed both sections"]]}
    }))?;
    let path = common::run_path(&run)?;
    h.goto()?;
    h.run_button("Delegate the report").click()?;
    h.click("Agents")?;
    let roster = h.b.role("navigation", "Agent roster");
    roster.contains("Coordinator")?;
    roster.contains("writer")?;
    let review = roster.xpath(".//button[contains(.,'Review delegate')]");
    if h.mobile {
        review.visible()?;
        h.b.eval("document.querySelector('.agent-row[data-agent-key$=\":review\"]').scrollIntoView({block:'center'})", vec![])?;
        review.tap()?;
    } else {
        review.focus()?;
        h.b.key("\u{e007}")?;
        h.b.role("heading", "Review delegate").focused()?;
    }
    let log = h.b.label("Turn log");
    log.contains("Native review stderr.")?;
    log.contains("37 reasoning tokens")?;
    log.contains("Live")?;
    ensure!(!log.text_content()?.contains("Native research stdout."));
    log.xpath(".//button[strong[.='Reasoning summary']]")
        .click()?;
    log.contains("Compare against the report criteria.")?;
    ensure!(h.b.eval(r#"
        const title = [...document.querySelectorAll(".paged-log button strong")]
            .find(label => label.textContent === "Reasoning summary");
        const button = title.closest("button");
        return button.getBoundingClientRect().bottom >= button.querySelector("time").getBoundingClientRect().bottom;
    "#, vec![])? == true, "the timestamp extends outside its log header");
    if let Some(dir) = std::env::var_os("SWARM_TEST_SCREENSHOTS") {
        h.b.screenshot(&std::path::PathBuf::from(dir).join(if h.mobile {
            "agents-mobile.png"
        } else {
            "agents-desktop.png"
        }))?;
    }
    h.proxy.inject(Once::LogFailure);
    h.click("Raw log")?;
    h.b.role("alert", "").contains("Temporarily unavailable")?;
    h.click("Try again")?;
    log.contains(r#""reasoning_tokens":37"#)?;
    log.contains(r#""type":"stderr""#)?;
    let raw_url = h.b.url()?;
    h.b.reload()?;
    log.contains("Native review stderr.")?;
    ensure!(h.b.url()? == raw_url);
    h.click("Pause updates")?;
    h.button("Go live").visible()?;
    if h.mobile {
        h.click("Back to agents")?;
        review.focused()?;
    } else {
        h.click("Agents")?;
    }
    roster
        .xpath(".//button[contains(.,'Result-only delegate')]")
        .click()?;
    h.b.text("Transcript unavailable.").visible()?;
    log.contains("Only the result was exposed.")?;
    h.svc
        .post(&path, &json!({"kind":"coordinator","text":"second"}))?;
    fs::write(h.dir.path().join("chat-gate"), "")?;
    let landed = h.svc.wait_state(&path, "landed")?;
    h.b.reload()?;
    log.contains("Only the result was exposed.")?;
    if h.mobile {
        h.click("Back to agents")?;
    }
    roster
        .xpath(".//button[contains(.,'Write the first section')]")
        .click()?;
    h.b.role("heading", "Write the first section.").visible()?;
    log.contains("section-one")?;
    if h.mobile {
        h.click("Back to agents")?;
    }
    roster
        .xpath(".//button[.//strong[.='Assemble the final report.']]")
        .click()?;
    h.b.role("combobox", "Turn").visible()?;
    let picker = h.b.role("combobox", "Turn");
    let first = common::rows(&landed["turns"])
        .iter()
        .find(|turn| turn["task"] == "report")
        .and_then(|turn| turn["id"].as_str())
        .context("first report attempt")?;
    picker.select(first)?;
    log.contains("13 reasoning tokens")?;
    ensure!(!log.text_content()?.contains("Native research stdout."));
    h.click("Raw log")?;
    log.contains("Native research stdout.")?;
    h.b.eval("history.back()", vec![])?;
    until("output mode restored by Back", || {
        Ok((h.button("Output").attr("aria-pressed")? == "true").then_some(()))
    })?;
    if h.mobile {
        h.click("Back to agents")?;
    }
    roster.xpath(".//button[.//strong[.='first']]").click()?;
    h.b.role("heading", "first").visible()?;
    log.contains("Page 1")?;
    h.no_overflow()
}

scenario!(desktop_run_agents, mobile_run_agents, run_agents);

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
    b.role("dialog", "Coordinator chat").visible()?;
    let history = b.role("log", "Coordinator conversation");
    history.contains("The export milestone owns the generated CSV.")?;
    history.xpath(".//code[.='total']").visible()?;
    history.xpath(".//img").absent()?;
    history.xpath(".//li[.='Check row 45']").visible()?;
    history.focus()?;
    b.key("\u{e011}")?; // Home
    h.button("Latest messages ↓").visible()?;
    b.label("Message").fill("Keep this draft")?;
    h.click("Coordinator chat")?;
    h.click("Agent setup")?;
    h.run_button("Quote export").click()?;
    b.role("dialog", "Coordinator chat").visible()?;
    b.label("Message").value("Keep this draft")?;
    h.click("Latest messages ↓")?;
    h.button("Latest messages ↓").absent()?;
    b.label("Message")
        .fill("Line one\u{e008}\u{e007}\u{e000}Line two")?;
    b.label("Message").value("Line one\nLine two")?;
    ensure!(b.eval(r#"const event = new KeyboardEvent("keydown", {key:"Enter",isComposing:true,bubbles:true,cancelable:true});
        return document.activeElement.dispatchEvent(event)"#, vec![])? == true);
    b.label("Message").fill("Stream this answer\u{e007}")?;
    history.contains("The live answer")?;
    b.text("Writing…").visible()?;
    h.loading_bar().contains("Waiting for the coordinator…")?;
    ensure!(
        h.svc.get(&path)?["conversation"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["state"]
            == "running"
    );
    b.reload()?;
    h.run_button("Quote export").click()?;
    history.contains("The live answer")?;
    b.text("Writing…").visible()?;
    h.click("Minimize chat")?;
    h.click("Agent setup")?;
    h.loading_bar().contains("Waiting for the coordinator…")?;
    fs::write(h.dir.path().join("chat-gate"), "")?;
    h.loading_bar().absent()?;
    h.run_button("Quote export").click()?;
    h.button("Coordinator chat").visible()?;
    if h.button("Coordinator chat").attr("aria-expanded")? == "false" {
        h.click("Coordinator chat")?;
    }
    history.contains("The live answer is complete.")?;
    b.text("Writing…").absent()?;
    let state = h.svc.get(&path)?;
    let messages = state["conversation"].as_array().unwrap();
    ensure!(
        messages
            .iter()
            .filter(|m| m["text"] == "Stream this answer")
            .count()
            == 1
    );
    ensure!(
        messages
            .iter()
            .filter(|m| m["text"] == "The live answer is complete.")
            .count()
            == 1
    );
    b.label("Message").value("")?;
    h.svc.post(&path, &json!({"kind":"stop"}))?;
    b.text("Resume the run to send a message.").visible()?;
    h.button("Send").enabled(false)?;
    h.no_overflow()
}
fn explain_context(h: &Harness) -> Result<()> {
    let b = &h.b;
    let run = h.svc.post(
        "/runs",
        &json!({
            "path":h.repo("explain")?, "prompt":"Add quote export", "coordinator":"first",
            "workers":["writer"], "delivery":"review"
        }),
    )?;
    let path = common::run_path(&run)?;
    h.svc.wait_state(&path, "question")?;
    h.svc
        .post(&path, &json!({"kind":"answer","text":"private"}))?;
    h.svc.wait_state(&path, "review")?;
    h.goto()?;
    h.run_button("Quote export").click()?;
    b.role("dialog", "Coordinator chat").visible()?;
    b.label("Message").fill("An unfinished follow-up")?;
    h.click("Coordinator chat")?;
    h.click("Milestones")?;
    h.click("Map")?;
    h.click("Full screen")?;
    b.xpath("//button[contains(@class,'map-select')]").click()?;
    h.click("Explain this")?;
    b.role("log", "Coordinator conversation")
        .contains("This milestone writes the open quotes export.")?;
    b.label("Message").value("An unfinished follow-up")?;
    h.click("Coordinator chat")?;
    h.click("Implementations")?;
    h.click("View details")?;
    h.click("Explain this")?;
    b.role("log", "Coordinator conversation")
        .contains("This implementation adds the checked export file.")?;
    b.label("Message").value("An unfinished follow-up")?;
    let state = h.svc.get(&path)?;
    let messages = common::rows(&state["conversation"]);
    ensure!(messages.len() == 4);
    ensure!(
        messages[0]["text"] == "Explain this milestone."
            && messages[0]["context"]["id"] == "export"
    );
    ensure!(
        messages[2]["text"] == "Explain this implementation."
            && messages[2]["context"]["kind"] == "implementation"
    );
    ensure!(
        h.svc
            .post(
                &path,
                &json!({"kind":"chat","text":"Explain","task":"missing"})
            )
            .is_err()
    );
    Ok(())
}
scenario!(
    desktop_explain_context,
    mobile_explain_context,
    explain_context
);
scenario!(desktop_chat_history, mobile_chat_history, chat_history);
fn discovered_model(h: &Harness) -> Result<()> {
    let config = h
        .dir
        .path()
        .join("hermes-home/profiles/desktop-work/config.yaml");
    fs::write(
        &config,
        "model:\n  default: local/writer-one\nagent:\n  reasoning_effort: medium\n",
    )?;
    h.goto()?;
    h.click("Settings for Hermes · desktop-work")?;
    h.b.label("Model ID").value("local/writer-one")?;
    h.b.label("Model name on the benchmark")
        .value("local/writer-one")?;
    h.b.label("Agent name on the benchmark").value("Hermes")?;
    h.b.label("Benchmark effort").value("medium")?;
    h.click("Save model settings")?;
    h.b.text("Saved for new runs.").visible()?;
    h.click("Close dialog")?;
    fs::write(
        &config,
        "model: 'local/writer-two'\nagent:\n  reasoning_effort: high\n",
    )?;
    h.click("Check again")?;
    h.button("Use Hermes · desktop-work").enabled(true)?;
    h.click("Settings for Hermes · desktop-work")?;
    h.b.label("Model name on the benchmark")
        .value("local/writer-two")?;
    h.b.label("Benchmark effort").value("high")?;
    h.b.label("Model name on the benchmark")
        .fill("Explicit match")?;
    h.click("Save model settings")?;
    h.b.text("Saved for new runs.").visible()?;
    h.click("Use detected settings")?;
    h.b.label("Model name on the benchmark")
        .value("local/writer-two")?;
    h.no_overflow()
}
scenario!(
    desktop_detected_models,
    mobile_detected_models,
    discovered_model
);

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
            .env("SWARM_UPDATE_URL", "off")
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

fn runs_navigation(h: &Harness) -> Result<()> {
    let seed = |repo, prompt| {
        h.svc.post(
            "/runs",
            &json!({"path":repo,"prompt":prompt,"coordinator":"first","workers":["writer"]}),
        )
    };
    let quote = seed(h.repo("paper-trail")?, "Add quote export")?;
    h.svc.wait_state(&common::run_path(&quote)?, "question")?;
    let team = seed(h.repo("team")?, "Build the team export")?;
    let team_path = common::run_path(&team)?;
    h.svc.wait_state(&team_path, "question")?;
    h.svc.post(&team_path, &json!({"kind":"stop"}))?;
    h.svc.wait_state(&team_path, "stopped")?;
    h.goto()?;
    let newest = h.run_button("Build the team export");
    newest.click()?;
    ensure!(newest.attr("aria-current")? == "page");
    ensure!(
        h.b.eval(
            r#"
        const runs = Array.from(document.querySelectorAll('.run-nav button'));
        if (runs.length !== 2 || runs[0].getAttribute('aria-current') !== 'page') return false;
        return ['.run-name', '.state'].every(selector => {
            const inset = button => button.querySelector(selector).getBoundingClientRect().left
                - button.getBoundingClientRect().left;
            return Math.abs(inset(runs[0]) - inset(runs[1])) < 1;
        });
    "#,
            vec![]
        )? == true,
        "Recent run names and statuses should have the same left inset"
    );
    h.click("Runs")?;
    h.b.label("All runs").visible()?;
    h.b.xpath("//nav[contains(@class,'run-nav')]//button[contains(@class,'run-settings-button')]")
        .absent()?;
    if let Some(dir) = std::env::var_os("SWARM_TEST_SCREENSHOTS") {
        h.b.screenshot(&std::path::PathBuf::from(dir).join(if h.mobile {
            "runs-index-mobile.png"
        } else {
            "runs-index-desktop.png"
        }))?;
    }
    h.button("New run").visible()?;
    h.click("New run")?;
    h.b.role("heading", "Start a run").visible()?;
    h.b.eval("history.back()", vec![])?;
    h.b.label("All runs").visible()?;
    let titles = || {
        h.b.eval("return Array.from(document.querySelectorAll('.runs-index tbody a'), el=>el.textContent)",vec![])
    };
    ensure!(titles()? == json!(["Build the team export", "Quote export"]));
    h.click("Run")?;
    ensure!(titles()? == json!(["Build the team export", "Quote export"]));
    h.click("Run")?;
    ensure!(titles()? == json!(["Quote export", "Build the team export"]));
    h.click("Run")?;
    ensure!(
        h.b.xpath("//th[button[contains(.,'Run')]]")
            .attr("aria-sort")?
            == "ascending"
    );
    let on_divider = r#"
        const rail = document.querySelector('.sidebar').getBoundingClientRect();
        const toggle = document.querySelector('.rail-toggle').getBoundingClientRect();
        const header = document.querySelector('.workbench-header').getBoundingClientRect();
        if (Math.abs(header.bottom - rail.top) > 1) return false;
        return innerWidth <= 720
            ? Math.abs(rail.bottom - (toggle.top + toggle.height / 2)) <= 1
            : Math.abs(rail.right - (toggle.left + toggle.width / 2)) <= 1;
    "#;
    ensure!(h.b.eval(on_divider, vec![])? == true);
    h.click("Collapse sidebar")?;
    h.button("Expand sidebar").visible()?;
    ensure!(h.b.eval(on_divider, vec![])? == true);
    h.run_button("Quote export")
        .xpath(".//span[@class='recent-initials' and .='QE']")
        .visible()?;
    let quote_shortcut = h.run_button("Quote export");
    let indicator = quote_shortcut.xpath(".//span[contains(@class,'state question')]");
    let dot = indicator.rect()?;
    ensure!(dot["width"].as_f64().unwrap_or_default() >= 6.);
    indicator.xpath(".//*[local-name()='svg']").absent()?;
    let tooltip = h.b.xpath("//*[@data-slot='tooltip-content']");
    if !h.mobile {
        h.run_button("Build the team export").hover()?;
        tooltip.contains("Build the team export")?;
        tooltip.contains("Stopped")?;
        h.b.key("\u{e00c}")?;
        tooltip.absent()?;
    }
    quote_shortcut.focus()?;
    tooltip.contains("Quote export")?;
    tooltip.contains("Needs an answer")?;
    h.b.key("\u{e00c}")?;
    tooltip.absent()?;
    let search = h.b.label("Search runs by name or project");
    let status = h.b.label("Run status");
    search.fill("quote EXPORT")?;
    ensure!(titles()? == json!(["Quote export"]));
    search.fill("paper-TRAIL")?;
    ensure!(titles()? == json!(["Quote export"]));
    status.select("stopped")?;
    h.b.text("No matching runs.").visible()?;
    h.button("Remove status filter Stopped").focus()?;
    h.b.key("\u{e007}")?;
    search.value("paper-TRAIL")?;
    ensure!(titles()? == json!(["Quote export"]));
    status.select("question")?;
    let filtered = h.b.url()?;
    let params = url::Url::parse(&filtered)?;
    ensure!(
        params
            .query_pairs()
            .any(|(key, value)| key == "q" && value == "paper-TRAIL")
    );
    ensure!(
        params
            .query_pairs()
            .any(|(key, value)| key == "status" && value == "question")
    );
    h.no_overflow()?;
    h.b.role("link", "Quote export").click()?;
    ensure!(h.b.url()?.contains(quote["id"].as_str().unwrap()));
    h.b.eval("history.back()", vec![])?;
    h.b.label("All runs").visible()?;
    ensure!(h.b.url()? == filtered);
    h.b.reload()?;
    h.button("Remove search filter paper-TRAIL").visible()?;
    h.button("Remove status filter Needs an answer").visible()?;
    ensure!(titles()? == json!(["Quote export"]));
    h.b.eval("history.forward()", vec![])?;
    h.b.role("heading", "Quote export").visible()?;
    h.click("Expand sidebar")?;
    let tabs = h.b.eval(
        "return Array.from(document.querySelectorAll('.run-tabs button'), el=>el.textContent)",
        vec![],
    )?;
    ensure!(
        tabs == json!([
            "Milestones",
            "Agents",
            "Coordinator chat",
            "Activity Log",
            "Metrics",
            "Experiment"
        ]),
        "Unexpected run tabs: {tabs}"
    );
    h.b.eval("history.back()", vec![])?;
    h.click("Remove search filter paper-TRAIL")?;
    search.value("")?;
    status.value("question")?;
    ensure!(titles()? == json!(["Quote export"]));
    h.click("Remove status filter Needs an answer")?;
    ensure!(titles()? == json!(["Build the team export", "Quote export"]));
    let direct = format!(
        "{}/?page=run&run={}&tab=chat",
        h.proxy.url,
        team["id"].as_str().unwrap()
    );
    h.b.goto(&direct)?;
    h.b.role("heading", "Build the team export").visible()?;
    h.b.role("region", "Coordinator chat").visible()?;
    ensure!(h.b.url()? == direct);
    h.no_overflow()
}
scenario!(
    desktop_runs_navigation,
    mobile_runs_navigation,
    runs_navigation
);

fn delete_from_settings(h: &Harness) -> Result<()> {
    let repo = h.repo("delete")?;
    let run = h.svc.post("/runs", &json!({
        "path":repo,"prompt":"Add quote export","coordinator":"first","workers":["writer"],
        "attachments":[
            {"name":"screenshot.png","data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGNg+M8AAAICAQB7CYF4AAAAAElFTkSuQmCC"},
            {"name":"scope.md","data":"IyMgQXR0YWNobWVudCBub3Rlcw=="},
            {"name":"bytes.bin","data":"AP8="}
        ]
    }))?;
    let path = common::run_path(&run)?;
    let waiting = h.svc.wait_state(&path, "question")?;
    h.goto()?;
    h.run_button("Quote export").click()?;
    h.click("Run settings")?;
    h.b.role("heading", "Run settings").visible()?;
    h.click("View brief")?;
    h.b.reload()?;
    h.b.role("dialog", "Run brief")
        .contains("Add quote export")?;
    h.b.role("heading", "Attachment notes").visible()?;
    let image = h.b.xpath("//img[@alt='screenshot.png']");
    image.visible()?;
    until("brief image preview to decode", || {
        Ok((h.b.eval(
            "return document.querySelector('.brief-attachment img')?.naturalWidth",
            vec![],
        )? == 1)
            .then_some(()))
    })?;
    h.b.role("link", "Download bytes.bin").visible()?;
    ensure!(h.b.role("link", "Download bytes.bin").attr("download")? == "bytes.bin");
    ensure!(h.svc.get(&format!("{path}/brief-attachments/2"))?["data"] == "AP8=");
    h.b.text("Preview unavailable.").visible()?;
    ensure!(h.b.eval("return document.querySelector('.run-brief-dialog').clientWidth > document.querySelector('.run-settings-dialog').clientWidth", vec![])? == true);
    h.no_overflow()?;
    if let Some(dir) = std::env::var_os("SWARM_TEST_SCREENSHOTS") {
        h.b.screenshot(&std::path::PathBuf::from(dir).join(if h.mobile {
            "run-settings-mobile.png"
        } else {
            "run-settings-desktop.png"
        }))?;
    }
    h.b.key("\u{e00c}")?;
    h.b.role("dialog", "Run settings").visible()?;
    h.click("Close dialog")?;
    h.click("Run settings")?;
    h.click("Pause run")?;
    h.click("Delete run")?;
    h.click("Cancel")?;
    h.b.key("\u{e00c}")?;
    h.button("Run settings").focused()?;
    h.click("Run settings")?;
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

fn assignment_map(h: &Harness) -> Result<()> {
    let goal = "Inspect the assignment map";
    let run = h.svc.post("/runs", &json!({"path":h.repo("map")?,"prompt":goal,"coordinator":"first","workers":["writer","writer-two"],"delivery":"review"}))?;
    let path = common::run_path(&run)?;
    h.svc.wait_state(&path, "review")?;
    h.goto()?;
    h.run_button(goal).click()?;
    h.click("Milestones")?;
    let b = &h.b;
    let bounds = b.eval("return [document.querySelector('.milestone-table').offsetWidth,document.querySelector('.run-workspace').getBoundingClientRect().x, document.querySelector('.app').clientHeight]", vec![])?;
    h.click("Coordinator chat")?;
    ensure!(bounds == b.eval("return [document.querySelector('.milestone-table').offsetWidth,document.querySelector('.run-workspace').getBoundingClientRect().x, document.querySelector('.app').clientHeight]", vec![])?);
    for column in ["Owner", "Status", "Milestone", "Completed"] {
        let head = b.xpath(&format!(
            "//table[@aria-label='Milestones']//th[button[contains(.,'{column}')]]"
        ));
        let button = b.xpath(&format!(
            "//table[@aria-label='Milestones']//th/button[contains(.,'{column}')]"
        ));
        button.click()?;
        ensure!(
            head.attr("aria-sort")?
                == if column == "Completed" {
                    "descending"
                } else {
                    "ascending"
                }
        );
        button.click()?;
        ensure!(
            head.attr("aria-sort")?
                == if column == "Completed" {
                    "ascending"
                } else {
                    "descending"
                }
        );
        if column == "Milestone" {
            ensure!(b.eval("return Array.from(document.querySelectorAll('.milestone-table tbody [data-task]'), e=>e.dataset.task).join(',')",vec![])?=="ui,api,foundation");
        }
    }
    b.xpath("//tr[td/button[@data-task='api']]/td[2]").click()?;
    b.role("dialog", "Add persistence.").visible()?;
    h.click("Open milestone foundation")?;
    b.role("dialog", "Create a project explanation.")
        .visible()?;
    b.role("heading", "Create a project explanation.")
        .focused()?;
    b.key("\u{e00c}")?;
    b.xpath("//button[@data-task='api']").focused()?;
    h.click("Map")?;
    b.label("Selected milestone").absent()?;
    let view = b.label("Assignment map, scroll to pan");
    view.visible()?;
    ensure!(
        b.eval(
            "return document.querySelectorAll('.map-world > svg > path').length",
            vec![]
        )? == 2
    );
    h.click("Filter writer-two assignments")?;
    ensure!(b.eval("return document.querySelectorAll('.map-node').length === 3 && document.querySelectorAll('.map-node[data-faded=true]').length === 2",vec![])?==true);
    h.click("All")?;
    h.click("Full screen")?;
    b.role("heading", "Full-screen agent map").visible()?;
    b.key("\u{e00c}")?;
    for _ in 0..6 {
        h.click("Zoom in")?;
    }
    view.focus()?;
    b.key("\u{e014}")?;
    until("keyboard pans the map", || {
        Ok((b
            .eval(
                "return document.querySelector('.map-viewport').scrollLeft",
                vec![],
            )?
            .as_f64()
            .unwrap_or(0.)
            > 0.)
            .then_some(()))
    })?;
    h.click("Fit")?;
    h.click("Milestone 3: Implement the interface.")?;
    ensure!(
        b.eval(
            "return document.querySelectorAll('.map-node[data-faded=true]').length === 1",
            vec![]
        )? == true
    );
    b.role("dialog", "Implement the interface.").visible()?;
    b.xpath("//*[@role='dialog']//button[@aria-label='Open milestone foundation']")
        .visible()?;
    h.button("Explain this").visible()?;
    h.click("Close dialog")?;
    if let Some(dir) = std::env::var_os("SWARM_TEST_SCREENSHOTS") {
        view.focus()?;
        b.screenshot(&std::path::PathBuf::from(dir).join(if h.mobile {
            "map-mobile.png"
        } else {
            "map-desktop.png"
        }))?;
    }
    b.label("Recent run activity").absent()?;
    h.click("Activity Log")?;
    b.label("Activity type").select("coordinator")?;
    b.label("Search activity").fill("nonexistent search")?;
    b.text("No matching activity.").visible()?;
    h.click("Coordinator chat")?;
    let status = h.button("View activity log: Ready for your review");
    if h.mobile {
        status.tap()?;
    } else {
        status.focus()?;
        b.key("\u{e007}")?;
    }
    b.label("Activity type").value("")?;
    b.label("Search activity").value("")?;
    b.label("Run activity").contains("Worker")?;
    b.label("View log for Create the run plan").focus()?;
    b.key("\u{e007}")?;
    b.role("dialog", "Create the run plan").visible()?;
    h.click("Explain this")?;
    b.role("log", "Coordinator conversation")
        .contains("This activity created the run plan.")?;
    h.no_overflow()
}
scenario!(
    desktop_assignment_map,
    mobile_assignment_map,
    assignment_map
);

fn run_metrics(h: &Harness) -> Result<()> {
    let b = &h.b;
    let run = h.svc.post("/runs",&json!({"path":h.repo("metrics")?, "prompt":"Add quote export","coordinator":"first","workers":["writer"]}))?;
    let path = common::run_path(&run)?;
    h.svc.wait_state(&path, "question")?;
    h.goto()?;
    h.run_button("Quote export").click()?;
    h.proxy.inject(Once::MetricsFailure);
    b.eval(
        r#"const held = new Promise(resolve => { window.releaseMetrics = resolve });
           const fetch = window.fetch;
           window.fetch = (...args) => {
             if (!args[0].includes("/metrics?")) return fetch(...args);
             window.fetch = fetch;
             return held.then(() => fetch(...args));
           };"#,
        vec![],
    )?;
    h.click("Metrics")?;
    h.loading_bar().contains("Loading usage…")?;
    b.eval("window.releaseMetrics()", vec![])?;
    h.button("Retry metrics").visible()?;
    h.loading_bar().absent()?;
    h.click("Retry metrics")?;
    let tokens = b.xpath("//dt[.='Tokens used']/following-sibling::dd");
    let active_agents = b.xpath("//dt[.='Agents working']/following-sibling::dd");
    active_agents.contains("0")?;
    tokens.contains("5")?;
    h.loading_bar().absent()?;
    b.role("combobox", "Chart view").select("pie")?;
    b.label("Usage share by agent and model").visible()?;
    b.xpath("//table[@aria-label='Usage by agent and model']//td[normalize-space()='writer']")
        .visible()?;
    b.xpath("//dt[.='Elapsed time']/following-sibling::dd")
        .visible()?;
    b.text("Zoom time range").click()?;
    b.label("From time").focus()?;
    b.key("\u{e014}")?;
    h.click("Reset zoom")?;
    b.role("combobox", "Chart view").select("total")?;
    h.svc
        .post(&path, &json!({"kind":"chat","text":"Stream this answer"}))?;
    tokens.contains("7")?;
    active_agents.contains("1")?;
    b.role("combobox", "Model").select("Fixture Chat")?;
    tokens.contains("2")?;
    b.role("combobox", "Agent").select("first")?;
    b.role("combobox", "Time range").select("15")?;
    b.role("combobox", "Role").select("plan")?;
    b.text("No usage in this selection.").visible()?;
    b.role("combobox", "Role").select("chat")?;
    b.role("combobox", "Metric").select("usd")?;
    b.role("combobox", "Chart view").select("interval")?;
    b.xpath("//*[@aria-label='Estimated cost over time, per interval']")
        .visible()?;
    h.click("Pause live updates")?;
    fs::write(h.dir.path().join("chat-gate"), "")?;
    h.click("Resume live updates")?;
    tokens.contains("5")?;
    b.role("combobox", "Model").select("")?;
    b.role("combobox", "Role").select("")?;
    tokens.contains("10")?;
    active_agents.contains("0")?;
    b.xpath("//dt[.='Estimated cost (USD)']/following-sibling::dd")
        .contains("$0.002")?;
    b.text("Cost unavailable for 1 turn").visible()?;
    b.role("combobox", "Metric").select("tokens")?;
    b.role("combobox", "Chart view").select("total")?;
    b.role("combobox", "Time range").select("all")?;
    b.text("Chart data").focus()?;
    b.key("\u{e007}")?;
    b.xpath("//th[.='Through']").visible()?;
    b.text("Chart data").click()?;
    if let Some(dir) = std::env::var_os("SWARM_TEST_SCREENSHOTS") {
        b.eval(
            "document.querySelector('.run-metrics').scrollIntoView();",
            vec![],
        )?;
        b.screenshot(&std::path::PathBuf::from(dir).join(if h.mobile {
            "metrics-mobile.png"
        } else {
            "metrics-desktop.png"
        }))?;
    }
    h.click("Milestones")?;
    b.eval(
        "const now = Date.now; Date.now = () => now() + 86400000;",
        vec![],
    )?;
    h.click("Metrics")?;
    b.role("combobox", "Time range").select("60")?;
    tokens.contains("10")?;
    // A copied zoom interval may belong to another run. Keep the chart inside this run.
    b.goto(&(b.url()? + "&from=9000000000000000&to=9000000000000001"))?;
    tokens.contains("10")?;
    b.label("Token usage over time, cumulative").visible()?;
    h.no_overflow()
}
scenario!(desktop_run_metrics, mobile_run_metrics, run_metrics);

fn concurrent_agent_count(h: &Harness) -> Result<()> {
    let goal = "Count concurrent agents";
    let run = h.svc.post(
        "/runs",
        &json!({"path":h.repo("concurrent-agents")?,"prompt":goal,"coordinator":"first",
            "workers":["writer","writer-two"],"delivery":"review"}),
    )?;
    let path = common::run_path(&run)?;
    h.goto()?;
    h.run_button(goal).click()?;
    h.click("Metrics")?;
    let count = "//dt[.='Agents working']/following-sibling::dd";
    let three = h.b.xpath(&format!("{count}[.='3']"));
    three.visible()?;
    h.b.role("combobox", "Agent").select("writer-two")?;
    three.visible()?;
    fs::write(h.dir.path().join("milestone-gate"), "")?;
    h.svc.wait_state(&path, "review")?;
    h.b.xpath(&format!("{count}[.='0']")).visible()?;
    h.no_overflow()
}
scenario!(
    desktop_concurrent_agent_count,
    mobile_concurrent_agent_count,
    concurrent_agent_count
);

fn stacked_metrics(h: &Harness) -> Result<()> {
    let run = h.svc.post(
        "/runs",
        &json!({
            "path":h.repo("stacked-metrics")?,"prompt":"Add quote export","coordinator":"first",
            "workers":["writer"],"delivery":"review"
        }),
    )?;
    let path = common::run_path(&run)?;
    h.svc.wait_state(&path, "question")?;
    h.svc
        .post(&path, &json!({"kind":"answer","text":"private"}))?;
    let run = h.svc.wait_state(&path, "review")?;
    let at = chrono::DateTime::parse_from_rfc3339(run["started"].as_str().context("run start")?)?
        .timestamp_millis();
    let folder = h
        .svc
        .state
        .join("runs")
        .join(run["id"].as_str().unwrap())
        .join("turns");
    let sample = |tokens, usd| json!({"tokens":tokens,"usd":usd});
    for (role, models, next) in [
        (
            "plan",
            json!({"Fixture shared":sample(20,0.01),"Fixture alternate":sample(30,0.03)}),
            json!({"Fixture shared":sample(15,0.008),"Fixture alternate":sample(30,0.03)}),
        ),
        (
            "worker",
            json!({"Fixture shared":sample(40,0.04)}),
            json!({"Fixture shared":sample(45,0.044)}),
        ),
    ] {
        let turn = common::item(&run["turns"], "role", &json!(role))?;
        fs::write(
            folder.join(turn["id"].as_str().unwrap()).join("usage.json"),
            serde_json::to_vec(
                &json!([{"at":at + 1000,"models":models},{"at":at + 30000,"models":next}]),
            )?,
        )?;
    }
    h.goto()?;
    h.run_button("Quote export").click()?;
    let b = &h.b;
    if b.eval(
        "return !!document.querySelector('[aria-label=\"Minimize chat\"]')",
        vec![],
    )? == true
    {
        h.click("Minimize chat")?;
    }
    h.click("Metrics")?;
    b.role("combobox", "Chart view").select("interval")?;
    let chart = b.label("Token usage over time, per interval");
    let first = chart.xpath(".//*[@data-usage-segment and starts-with(@aria-label,'first · Fixture shared, 20 tokens')]");
    let alternate = chart.xpath(".//*[@data-usage-segment and starts-with(@aria-label,'first · Fixture alternate, 30 tokens')]");
    let worker = chart.xpath(".//*[@data-usage-segment and starts-with(@aria-label,'writer · Fixture shared, 40 tokens')]");
    for bar in [&first, &alternate, &worker] {
        bar.visible()?;
        ensure!(
            bar.attr("x")? == first.attr("x")?,
            "same interval must share a bar"
        );
    }
    ensure!(first.attr("fill")? != worker.attr("fill")?);
    ensure!(first.attr("y")? != worker.attr("y")? && first.attr("y")? != alternate.attr("y")?);
    let correction = chart.xpath(".//*[@data-usage-segment and starts-with(@aria-label,'first · Fixture shared, -5 tokens')]");
    let increase = chart.xpath(".//*[@data-usage-segment and starts-with(@aria-label,'writer · Fixture shared, 5 tokens')]");
    correction.visible()?;
    ensure!(correction.attr("x")? == increase.attr("x")?);
    let correction_y: f64 = correction
        .attr("y")?
        .as_str()
        .context("correction position")?
        .parse()?;
    let increase_y: f64 = increase
        .attr("y")?
        .as_str()
        .context("increase position")?
        .parse()?;
    ensure!(
        correction_y > increase_y,
        "negative adjustments stack below zero"
    );
    let tip = b.role("tooltip", "");
    b.eval(
        "document.querySelector('.usage-plot').scrollIntoView({block:'center'})",
        vec![],
    )?;
    if h.mobile {
        worker.tap()?;
        tip.contains("writer · Fixture shared")?;
        tip.contains("40 tokens")?;
        alternate.tap()?;
        tip.contains("first · Fixture alternate")?;
    } else {
        worker.hover()?;
        tip.contains("writer · Fixture shared")?;
        tip.contains("40 tokens")?;
        alternate.focus()?;
        tip.contains("first · Fixture alternate")?;
        b.key("\u{e014}")?;
        tip.contains("first · Fixture shared")?;
        b.key("\u{e00c}")?;
        tip.absent()?;
    }
    if !h.mobile {
        let points = b.eval(
            "const matrix = document.querySelector('.usage-plot svg').getScreenCTM(); return [65, 400].map(x => { const p = new DOMPoint(x, 58).matrixTransform(matrix); return [p.x, p.y]; });",
            vec![],
        )?;
        for (kind, point, buttons) in [
            ("mousePressed", 0, 1),
            ("mouseMoved", 1, 1),
            ("mouseReleased", 1, 0),
        ] {
            b.cdp(
                "Input.dispatchMouseEvent",
                json!({
                    "type":kind, "x":points[point][0], "y":points[point][1], "button":"left",
                    "buttons":buttons, "clickCount":1
                }),
            )?;
        }
        b.role("combobox", "Time range").value("selection")?;
        h.click("Reset zoom")?;
    }
    b.text("Chart data").click()?;
    b.xpath("//table[.//th[.='Through']]//tr[td[.='writer · Fixture shared'] and td[.='40']]")
        .visible()?;
    b.text("Chart data").click()?;
    b.role("combobox", "Metric").select("usd")?;
    let cost = b.label("Estimated cost over time, per interval");
    cost.visible()?;
    b.role("combobox", "Model").select("Fixture shared")?;
    cost.xpath(".//*[@data-usage-segment and contains(@aria-label,'Fixture alternate')]")
        .absent()?;
    cost.xpath(
        ".//*[@data-usage-segment and starts-with(@aria-label,'writer · Fixture shared, $0.04')]",
    )
    .visible()?;
    h.no_overflow()
}
scenario!(
    desktop_stacked_metrics,
    mobile_stacked_metrics,
    stacked_metrics
);

fn full_chat_and_task_proposal(h: &Harness) -> Result<()> {
    let b = &h.b;
    let run = h.svc.post("/runs", &json!({"path":h.repo("proposal")?,"prompt":"Add quote export","coordinator":"first","workers":["writer"],"delivery":"review"}))?;
    let path = common::run_path(&run)?;
    h.svc.wait_state(&path, "question")?;
    h.svc
        .post(&path, &json!({"kind":"answer","text":"private"}))?;
    let review = h.svc.wait_state(&path, "review")?;
    h.svc.post(
        &path,
        &json!({"kind":"land","candidate":review["candidates"][0]["id"]}),
    )?;
    h.svc.wait_state(&path, "landed")?;
    h.goto()?;
    h.run_button("Quote export").click()?;
    b.xpath("//div[@class='coordinator-chat']//span[@class='composer-profile']")
        .contains("first · Harness default · Default effort")?;
    ensure!(
        b.xpath("//div[@class='coordinator-chat']//header[@class='chat-header']")
            .text_content()?
            .trim()
            == "Coordinator chat"
    );
    let launcher = h.button("Coordinator chat");
    let minimize = h.button("Minimize chat");
    ensure!(launcher.attr("aria-expanded")? == "true");
    b.label("Message").fill("Read the attachment")?;
    minimize.focus()?;
    b.key("\u{e007}")?;
    b.role("dialog", "Coordinator chat").absent()?;
    ensure!(launcher.attr("aria-expanded")? == "false");
    launcher.focused()?;
    launcher.click()?;
    b.label("Message").value("Read the attachment")?;
    h.click("Full screen coordinator chat")?;
    b.xpath("//div[@class='conversation-page']//span[@class='composer-profile']")
        .contains("first · Harness default · Default effort")?;
    b.role("region", "Coordinator chat").visible()?;
    b.xpath("//header[@class='chat-header']").absent()?;
    h.button("Return to dock").absent()?;
    b.label("Message").value("Read the attachment")?;
    h.click("Attachment options")?;
    h.button("Attach files").visible()?;
    b.key("\u{e00c}")?;
    ensure!(b.eval("const r=document.querySelector('.conversation-page .chat-composer').getBoundingClientRect(); return r.bottom <= innerHeight && r.bottom > innerHeight - 100 && !!document.querySelector('.composer-profile').textContent.trim()",vec![])? == true, "chat composer must be visible at the foot of the viewport");
    b.eval(r#"const dt = new DataTransfer(); dt.items.add(new File(["Measure receipt latency"], "work-notes.txt"));
      const picker = document.querySelector('input[aria-label="Chat attachments"]'); picker.files = dt.files;
      picker.dispatchEvent(new Event("change",{bubbles:true}));"#,vec![])?;
    h.button("Remove work-notes.txt").visible()?;
    h.click("Send")?;
    b.role("log", "Coordinator conversation")
        .contains("The notes ask to measure receipt latency.")?;
    h.no_overflow()?;
    if let Some(dir) = std::env::var_os("SWARM_TEST_SCREENSHOTS") {
        b.screenshot(&std::path::PathBuf::from(dir).join(if h.mobile {
            "chat-tab-mobile.png"
        } else {
            "chat-tab-desktop.png"
        }))?;
    }
    b.eval("history.back()", vec![])?;
    b.role("dialog", "Coordinator chat").visible()?;
    minimize.click()?;
    h.click("Milestones")?;
    h.click("Add task")?;
    b.label("What do you want to add?")
        .fill("Add a timing milestone after the gate")?;
    b.cdp(
        "Emulation.setEmulatedMedia",
        json!({"features":[{"name":"prefers-reduced-motion","value":if h.mobile {"reduce"} else {"no-preference"}}]}),
    )?;
    let create = h.button("Create milestone(s)");
    if h.mobile {
        create.tap()?;
    } else {
        create.focus()?;
        b.key("\u{e007}")?;
    }
    h.loading_bar()
        .contains("Coordinator is preparing milestones…")?;
    until(
        "coordinator turn running after the milestone request",
        || {
            let run = h.svc.get(&path)?;
            Ok(common::rows(&run["conversation"])
                .iter()
                .any(|message| message["intent"] == "add" && message["state"] == "running")
                .then_some(()))
        },
    )?;
    h.loading_bar()
        .xpath(".//*[@role='progressbar' and @data-state='indeterminate']")
        .visible()?;
    ensure!(
        b.eval(
            r#"const strip = document.querySelector(".window-status[data-loading=true]");
               const {top, width} = strip.getBoundingClientRect();
               const mark = strip.querySelector(".loading-mark");
               const bar = strip.querySelector("[data-slot=progress-indicator]");
               return top === 0 && width === document.documentElement.clientWidth
                 && getComputedStyle(mark).animationName === arguments[0]
                 && getComputedStyle(bar).animationName === arguments[1]"#,
            if h.mobile {
                vec![json!("none"), json!("none")]
            } else {
                vec![json!("loading-pivot"), json!("loading-bar")]
            },
        )? == true,
        "loading strip should span the top of the viewport and follow the motion preference"
    );
    h.no_overflow()?;
    if let Some(dir) = std::env::var_os("SWARM_TEST_SCREENSHOTS") {
        b.screenshot(&std::path::PathBuf::from(dir).join(if h.mobile {
            "milestone-loading-mobile.png"
        } else {
            "milestone-loading-desktop.png"
        }))?;
    }
    let chat_tab = h.button("Coordinator chat tab");
    until("pending milestone request in the URL", || {
        let url = b.url()?;
        Ok((url.contains("tab=milestones") && url.contains("proposalRequest=")).then_some(()))
    })?;
    b.reload()?;
    b.label("What do you want to add?")
        .value("Add a timing milestone after the gate")?;
    h.loading_bar()
        .contains("Coordinator is preparing milestones…")?;
    create.enabled(false)?;
    fs::write(h.dir.path().join("chat-gate"), "")?;
    let chat = b.role("region", "Coordinator chat");
    let reply = chat.xpath(".//article[@tabindex='-1']");
    reply.contains("We can record timings after the export is ready.")?;
    reply.contains("Record export timing.")?;
    reply.focused()?;
    ensure!(chat_tab.attr("aria-pressed")? == "true");
    b.role("dialog", "Add task").absent()?;
    h.loading_bar().absent()?;
    let destination = b.url()?;
    ensure!(destination.contains("tab=chat") && destination.contains("reply="));
    b.eval("history.back()", vec![])?;
    h.button("Add task").visible()?;
    b.role("dialog", "Add task").absent()?;
    b.eval("history.forward()", vec![])?;
    reply.focused()?;
    ensure!(b.url()? == destination);
    b.reload()?;
    reply.focused()?;
    let snapshot = h.svc.get(&path)?;
    ensure!(
        common::rows(&snapshot["conversation"])
            .iter()
            .filter(|m| m["intent"] == "add")
            .count()
            == 1
            && common::rows(&snapshot["tasks"])
                .iter()
                .all(|t| t["id"] != "timing"),
        "opening the reply should leave one request pending developer acceptance"
    );
    h.click("Add to run")?;
    reply.contains("Milestones added")?;
    h.click("Milestones")?;
    h.click("Kanban")?;
    b.xpath("//button[@data-slot='card' and @data-task='timing']")
        .click()?;
    ensure!(b.url()?.contains("milestone=timing") && b.url()?.contains("layout=board"));
    b.reload()?;
    b.role("dialog", "Record export timing.").visible()?;
    h.click("Open milestone export")?;
    b.xpath("//*[@role='dialog']//code[.='export']").visible()?;
    b.xpath("//button[@aria-expanded='false' and normalize-space(.)='Details']")
        .visible()?;
    h.click("Close dialog")?;
    h.click("Coordinator chat")?;
    let current = h.svc.wait_state(&path, "review")?;
    ensure!(current["round"] == 2 && current["tasks"][1]["round"] == 2);
    h.click("Table")?;
    h.click("Earlier milestones")?;
    h.click("Completed")?;
    b.xpath("//table[@aria-label='Milestones']/tbody/tr[1][td/button[@data-task='timing']]/td/time[@datetime]").visible()?;
    h.click("Completed")?;
    b.xpath("//table[@aria-label='Milestones']/tbody/tr[1][td/button[@data-task='export']]/td/time[@datetime]").visible()?;
    h.click("Kanban")?;
    if let Some(dir) = std::env::var_os("SWARM_TEST_SCREENSHOTS") {
        b.screenshot(&std::path::PathBuf::from(dir).join(if h.mobile {
            "kanban-mobile.png"
        } else {
            "kanban-desktop.png"
        }))?;
    }
    h.click("Activity Log")?;
    b.label("Search activity").fill("timing")?;
    b.label("Run activity").contains("Record export timing.")?;
    b.label("Activity agent").select("first")?;
    b.label("Run activity").contains("Add a timing milestone")?;
    let filtered = b.url()?;
    b.reload()?;
    h.button("Remove search filter timing").visible()?;
    h.click("Remove agent filter first")?;
    b.label("Activity agent").value("")?;
    b.label("Search activity").value("timing")?;
    b.eval("history.back()", vec![])?;
    h.button("Remove agent filter first").visible()?;
    ensure!(b.url()? == filtered);
    b.label("Search activity").fill("nonexistent search")?;
    b.text("No matching activity.").visible()?;
    h.click("Milestones")?;
    h.click("Add task")?;
    b.label("What do you want to add?")
        .fill("Try a proposal that fails")?;
    h.click("Create milestone(s)")?;
    let dialog = b.role("dialog", "Add task");
    dialog.contains("The fixture coordinator is unavailable.")?;
    h.loading_bar().absent()?;
    ensure!(b.url()?.contains("tab=milestones"));
    let text = b.label("What do you want to add?");
    text.value("Try a proposal that fails")?;
    text.fill("What would you need to plan another milestone?")?;
    h.click("Create milestone(s)")?;
    reply.contains("The export milestone owns the generated CSV.")?;
    reply.xpath(".//button[.='Add to run']").absent()?;
    h.no_overflow()
}
scenario!(
    desktop_milestone_proposals,
    mobile_milestone_proposals,
    full_chat_and_task_proposal
);
