use crate::{adapter, eval, service, supervisor};
use anyhow::{Context, Result, bail, ensure};
use clap::{Args, Parser, Subcommand};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

#[derive(Parser)]
#[command(name = "swarm", about = "Coordinate local coding agents", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Action>,
}
#[derive(Subcommand)]
enum Action {
    Open,
    Serve {
        #[arg(long, default_value = "127.0.0.1:0")]
        listen: String,
    },
    Doctor,
    Start(Start),
    Status {
        run: Option<String>,
    },
    Destinations,
    Cases(Page),
    History(Page),
    Preview(Item),
    Diff(Item),
    Log(Item),
    Evidence(Item),
    Answer(Change),
    Coordinator(Change),
    Chat(Change),
    AcceptProposal(Change),
    DismissProposal(Change),
    Approve(Change),
    Stop(Change),
    Resume(Change),
    Change(Change),
    Land(Change),
    Feedback(Change),
    AnswerInbox(Change),
    RetryCoordinator(Change),
    UpdateSuite(Change),
    Select(Change),
    KeepBaseline(Change),
    Combine(Change),
    NewRound(Change),
    CancelCombination(Change),
    Hold(Change),
    Unhold(Change),
    Delivery(Change),
    BlindReview(Change),
    Vote(Change),
    HoldRelease(Change),
    ResumeRelease(Change),
    CancelRelease(Change),
    ReconcileRelease(Change),
    #[command(name = "_supervise", hide = true)]
    Supervise {
        dir: PathBuf,
    },
}
#[derive(Args)]
struct Start {
    #[arg(long, default_value = ".")]
    path: PathBuf,
    #[arg(long, default_value = "")]
    prompt: String,
    #[arg(long)]
    coordinator: Option<String>,
    #[arg(long)]
    worker: Vec<String>,
    #[arg(long)]
    import: Vec<String>,
    #[arg(long)]
    compare: bool,
    #[arg(long, default_value = "review")]
    delivery: String,
    #[arg(long, default_value = "")]
    destination: String,
    #[arg(long, default_value = "")]
    target: String,
    #[arg(long)]
    no_init: bool,
    #[arg(long, default_value_t = 0)]
    tokens: i64,
    #[arg(long, default_value = "")]
    routing: String,
    #[arg(long)]
    recipe: Option<PathBuf>,
}
#[derive(Args)]
struct Page {
    #[arg(long, default_value_t = 0)]
    before: i64,
    run: String,
}
#[derive(Args)]
struct Item {
    run: String,
    id: String,
}
#[derive(Args)]
struct Change {
    run: String,
    #[arg(long)]
    donor: Vec<String>,
    #[arg(long)]
    id: Option<String>,
    #[arg(long)]
    revision: Option<i64>,
    #[arg(long)]
    task: Option<String>,
    #[arg(long)]
    text: Option<String>,
    #[arg(long)]
    candidate: Option<String>,
    #[arg(long)]
    tokens: Option<i64>,
    #[arg(long)]
    recipe: Option<PathBuf>,
}

#[derive(Deserialize)]
struct Session {
    url: String,
    token: String,
}
impl Session {
    fn load(dir: &Path) -> Result<Self> {
        let info: Self = serde_json::from_slice(
            &fs::read(dir.join("service.json"))
                .context("start the local service with swarm serve")?,
        )?;
        let addr = info
            .url
            .strip_prefix("http://")
            .context("invalid local service address")?
            .parse::<std::net::SocketAddrV4>()?;
        ensure!(
            addr.ip().is_loopback() && addr.port() != 0 && hex::decode(&info.token)?.len() == 32,
            "invalid local service session"
        );
        Ok(info)
    }
    fn client(timeout: Duration) -> Result<reqwest::blocking::Client> {
        Ok(reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout)
            .build()?)
    }
    fn running(dir: &Path) -> Option<String> {
        let session = Self::load(dir).ok()?;
        let response = Self::client(Duration::from_millis(500))
            .ok()?
            .get(format!("{}/api/session", session.url))
            .bearer_auth(&session.token)
            .send()
            .ok()?;
        (response.status() == 204).then(|| workbench_link(&session.url, &session.token))
    }
}
/// Keep the launch folder and token in the fragment, local to this browser tab.
pub fn workbench_link(origin: &str, token: &str) -> String {
    let mut link = format!("{origin}/#{token}");
    if let Ok(cwd) = std::env::current_dir()
        && cwd != Path::new("/")
        && let Some(path) = cwd.to_str()
    {
        let encoded = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("cwd", path)
            .finish();
        link.push('&');
        link.push_str(&encoded);
    }
    link
}
pub fn open_browser(link: &str) {
    use wait_timeout::ChildExt;
    let launch = || -> Result<()> {
        let program = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        let mut child = Command::new(program).arg(link).spawn()?;
        match child.wait_timeout(Duration::from_secs(5))? {
            Some(exit) if exit.success() => Ok(()),
            None => {
                child.kill()?;
                child.wait()?;
                bail!("browser launch timed out")
            }
            _ => bail!("browser launch failed"),
        }
    };
    if launch().is_err() {
        eprintln!("Open the Pantheon Local link above in your browser.");
    }
}
fn recipe(path: &Path) -> Result<Value> {
    let v = serde_json::from_slice(&fs::read(path)?)?;
    eval::validate(&v)?;
    Ok(v)
}

fn normalize_args<I: IntoIterator<Item = std::ffi::OsString>>(raw: I) -> Vec<std::ffi::OsString> {
    const VALUE_FLAGS: &[&str] = &[
        "listen",
        "path",
        "prompt",
        "coordinator",
        "worker",
        "import",
        "delivery",
        "destination",
        "target",
        "tokens",
        "routing",
        "recipe",
        "before",
        "donor",
        "id",
        "revision",
        "task",
        "text",
        "candidate",
    ];
    let mut args: Vec<_> = raw.into_iter().collect();
    let mut options = true;
    let mut expect_value = false;
    for a in args.iter_mut().skip(1) {
        if expect_value {
            expect_value = false;
            continue;
        }
        if !options {
            continue;
        }
        if *a == "--" {
            options = false;
            continue;
        }
        if let Some(v) = a.to_str().filter(|s| {
            s.starts_with('-')
                && !s.starts_with("--")
                && s.as_bytes().get(1).is_some_and(u8::is_ascii_alphabetic)
                && s.len() > 2
        }) {
            *a = format!("-{v}").into();
        }
        if let Some(flag) = a
            .to_str()
            .and_then(|s| s.strip_prefix("--"))
            .filter(|s| !s.contains('='))
            && VALUE_FLAGS.contains(&flag)
        {
            expect_value = true;
        }
    }
    args
}

pub fn execute() -> Result<()> {
    // Retain the original single-dash long options for existing shell scripts.
    let args = normalize_args(std::env::args_os());
    let kind = args
        .get(1)
        .and_then(|v| v.to_str())
        .unwrap_or("open")
        .to_owned();
    let action = Cli::parse_from(args).command.unwrap_or(Action::Open);
    if let Action::Supervise { dir } = action {
        return supervisor::supervise(&dir);
    }
    let dir = service::state_dir()?;
    let (path, body) = match action {
        Action::Open => {
            ensure!(
                crate::assets::asset("index.html").is_some(),
                "web interface missing: build web assets, then rebuild swarm"
            );
            if let Some(link) = Session::running(&dir) {
                println!("{link}");
                open_browser(&link);
                return Ok(());
            }
            println!("Keep this Terminal window open while you use Pantheon Local.");
            match service::serve(&dir, "127.0.0.1:0", true) {
                Err(e) if e.downcast_ref::<service::AlreadyRunning>().is_some() => {}
                result => return result,
            }
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(3) {
                if let Some(link) = Session::running(&dir) {
                    println!("{link}");
                    open_browser(&link);
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(100))
            }
            bail!("Pantheon Local is running. Use the link in its Terminal window.")
        }
        Action::Serve { listen } => return service::serve(&dir, &listen, false),
        Action::Doctor => {
            let setup = service::setup(&dir)?;
            println!("Profiles: {}", dir.join("profiles.json").display());
            for (name, p) in setup["profiles"].as_object().context("invalid profiles")? {
                println!(
                    "{name}: {}",
                    adapter::probe(p, Some(&dir)).unwrap_or_else(|e| e.to_string())
                );
            }
            return Ok(());
        }
        Action::Start(s) => {
            let setup = service::setup(&dir)?;
            let mut input = json!({"path":std::path::absolute(s.path)?,"prompt":s.prompt,"compare":s.compare,"delivery":s.delivery,
                "destination":s.destination,"target":s.target,"skip_init":s.no_init,"token_target":s.tokens,"routing":s.routing,"initial_files":s.import,
                "coordinator":s.coordinator.as_deref().or_else(||setup["coordinator"].as_str()).unwrap_or(""),
                "workers":if s.worker.is_empty(){setup["workers"].clone()}else{json!(s.worker)}});
            if let Some(path) = s.recipe {
                input["recipe"] = recipe(&path)?;
            }
            ("runs".into(), Some(input))
        }
        Action::Status { run } => (
            run.map_or_else(|| "runs".into(), |id| format!("runs/{id}")),
            None,
        ),
        Action::Destinations => ("destinations".into(), None),
        Action::Cases(p) | Action::History(p) => {
            (format!("runs/{}/{kind}?before={}", p.run, p.before), None)
        }
        Action::Diff(i) | Action::Evidence(i) | Action::Log(i) | Action::Preview(i) => {
            let endpoint = if kind == "log" { "turns" } else { &kind };
            (
                format!("runs/{}/{endpoint}/{}", i.run, i.id),
                (kind == "preview").then_some(Value::Null),
            )
        }
        Action::Supervise { .. } => unreachable!(),
        Action::Answer(c)
        | Action::Coordinator(c)
        | Action::Chat(c)
        | Action::AcceptProposal(c)
        | Action::DismissProposal(c)
        | Action::Approve(c)
        | Action::Stop(c)
        | Action::Resume(c)
        | Action::Change(c)
        | Action::Land(c)
        | Action::Feedback(c)
        | Action::AnswerInbox(c)
        | Action::RetryCoordinator(c)
        | Action::UpdateSuite(c)
        | Action::Select(c)
        | Action::KeepBaseline(c)
        | Action::Combine(c)
        | Action::NewRound(c)
        | Action::CancelCombination(c)
        | Action::Hold(c)
        | Action::Unhold(c)
        | Action::Delivery(c)
        | Action::BlindReview(c)
        | Action::Vote(c)
        | Action::HoldRelease(c)
        | Action::ResumeRelease(c)
        | Action::CancelRelease(c)
        | Action::ReconcileRelease(c) => {
            let mut value = json!({"kind":kind,"id":c.id,"expected_revision":c.revision,"task":c.task,"text":c.text,"candidate":c.candidate,"tokens":c.tokens,"donors":c.donor});
            value.as_object_mut().unwrap().retain(|_, v| !v.is_null());
            if let Some(path) = c.recipe {
                value["recipe"] = recipe(&path)?
            }
            (format!("runs/{}", c.run), Some(value))
        }
    };
    let session = Session::load(&dir)?;
    let url = format!("{}/api/{path}", session.url);
    let client = Session::client(Duration::from_secs(45))?;
    let response = match body {
        Some(v) => client.post(url).json(&v),
        None => client.get(url),
    }
    .bearer_auth(session.token)
    .send()?;
    let status = response.status();
    let output = response.text()?;
    if !status.is_success() {
        if let Ok(v) = serde_json::from_str::<Value>(&output) {
            bail!("{}", v["error"].as_str().unwrap_or(&output))
        }
        bail!("{status}: {output}")
    }
    if let Ok(v) = serde_json::from_str::<Value>(&output) {
        println!("{}", serde_json::to_string_pretty(&v)?)
    } else {
        print!("{output}")
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_dash_options_preserve_flag_values_starting_with_hyphens() {
        let input = [
            "swarm",
            "start",
            "-prompt",
            "-support negative numbers",
            "-compare",
            "--text",
            "-keep leading dash",
            "--",
            "-literal",
        ]
        .map(std::ffi::OsString::from);
        let got: Vec<_> = normalize_args(input)
            .into_iter()
            .map(|s| s.into_string().unwrap())
            .collect();
        assert_eq!(
            got,
            [
                "swarm",
                "start",
                "--prompt",
                "-support negative numbers",
                "--compare",
                "--text",
                "-keep leading dash",
                "--",
                "-literal",
            ]
        );
    }
}
