//! Disposable filesystem implementation of the release adapter protocol.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};
fn field<'a>(request: &'a Value, name: &str) -> Result<&'a str> {
    request[name]
        .as_str()
        .with_context(|| format!("missing {name}"))
}
fn main() -> Result<()> {
    let phase = env::args().nth(1).context("missing release phase")?;
    let req: Value = serde_json::from_reader(io::stdin().lock())?;
    let artifact = Path::new(field(&req, "artifact")?);
    match phase.as_str() {
        "build" => {
            fs::copy(
                Path::new(field(&req, "source")?).join("result.txt"),
                artifact,
            )?;
        }
        "verify" => ensure!(
            format!("sha256:{:x}", Sha256::digest(fs::read(artifact)?)) == field(&req, "digest")?,
            "artifact digest mismatch"
        ),
        "status" | "deploy" | "rollback" => {
            let live = env::var("SWARM_TEST_LIVE")?;
            let marker = |suffix: &str| PathBuf::from(format!("{live}.{suffix}"));
            match phase.as_str() {
                "status" => {
                    ensure!(!marker("unavailable").exists(), "status unavailable");
                    print!("{}", fs::read_to_string(&live)?)
                }
                "deploy" => {
                    if marker("forbid-promotion").exists() {
                        fs::write(marker("unexpected-promotion"), "")?;
                        bail!("unexpected deployment")
                    }
                    fs::write(&live,json!({"digest":field(&req,"digest")?,"healthy":!marker("unhealthy").exists()}).to_string())?;
                    if marker("interrupt").exists() {
                        thread::sleep(Duration::from_secs(4));
                        bail!("deployment interrupted")
                    }
                }
                _ => fs::write(
                    &live,
                    json!({"digest":field(&req,"previous")?,"healthy":true}).to_string(),
                )?,
            }
        }
        _ => bail!("unknown release phase {phase}"),
    }
    Ok(())
}
