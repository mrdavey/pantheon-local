//! Browser acceptance check for a candidate quote tracker written in Rust.
#[path = "../tests/browser/mod.rs"]
mod browser;

use anyhow::{Context, Result, ensure};
use browser::Browser;
use clap::Parser;
use reqwest::blocking::Client;
use std::{
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command},
    thread::sleep,
    time::{Duration, Instant},
};

#[derive(Parser)]
struct Args {
    /// Chrome or Chromium executable.
    chrome: PathBuf,
    /// ChromeDriver matching the installed browser.
    driver: PathBuf,
    /// Compiler returned by rustup which rustc.
    rustc: PathBuf,
}

struct App(Child);
impl Drop for App {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
impl App {
    fn start(binary: &Path, data: &Path, port: u16, url: &str) -> Result<Self> {
        let mut app = Self(
            Command::new(binary)
                .env("SWARM_PORT", port.to_string())
                .env("SWARM_DATA_DIR", data)
                .spawn()
                .context("launch candidate quote tracker")?,
        );
        let http = Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(300))
            .build()?;
        let start = Instant::now();
        loop {
            ensure!(
                app.0.try_wait()?.is_none(),
                "quote tracker exited during startup"
            );
            if http.get(url).send().is_ok_and(|r| r.status().is_success()) {
                return Ok(app);
            }
            ensure!(
                start.elapsed() < Duration::from_secs(10),
                "quote tracker did not serve {url}"
            );
            sleep(Duration::from_millis(100));
        }
    }
}
fn main() -> Result<()> {
    let args = Args::parse();
    let artifacts = std::env::var_os("SWARM_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    let work = tempfile::tempdir_in(&artifacts)?;
    let binary = work.path().join("quotes");
    let data = work.path().join("data");
    fs::create_dir(&data)?;
    let status = Command::new(args.rustc)
        .args(["--edition=2024", "app.rs", "-o"])
        .arg(&binary)
        .status()
        .context("compile the candidate's app.rs")?;
    ensure!(status.success(), "app.rs failed to compile");
    let port = TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port();
    let url = format!("http://127.0.0.1:{port}");
    let app = App::start(&binary, &data, port, &url)?;
    let browser = Browser::launch_with_driver(Some(&args.chrome), &args.driver, false)?;
    let check = (|| -> Result<()> {
        browser.goto(&url)?;
        browser.text("Outstanding: $0.00").visible()?;
        for (client, amount, total) in [("North", "125", "$125.00"), ("South", "75", "$200.00")] {
            browser.label("Client").fill(client)?;
            browser.label("Amount").fill(amount)?;
            browser.role("button", "Add quote").click()?;
            browser.text(&format!("Outstanding: {total}")).visible()?;
        }
        browser
            .xpath("//tr[contains(.,'North')]//button[normalize-space(.)='Accept']")
            .click()?;
        browser.text("Outstanding: $75.00").visible()?;
        drop(app);
        let _restarted = App::start(&binary, &data, port, &url)?;
        browser.reload()?;
        browser
            .xpath("//tr[contains(.,'North') and contains(.,'Accepted')]")
            .visible()?;
        browser.text("Outstanding: $75.00").visible()?;
        browser.screenshot(&artifacts.join("quotes.png"))
    })();
    let trace = browser.trace(&artifacts.join("quotes-trace.json"));
    check?;
    trace
}
