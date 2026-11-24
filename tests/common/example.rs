//! Resolve a test helper from a Cargo build or extracted release archive.
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub fn example(name: &str) -> Result<PathBuf> {
    let runner = std::env::current_exe()?;
    let parent = runner.parent().context("test executable has no parent")?;
    let profile = parent.parent().context("test profile directory")?;
    let path = if parent.ends_with("tests") && profile.join("swarm").is_file() {
        parent.join("fixtures").join(name)
    } else {
        profile.join("examples").join(name)
    };
    path.canonicalize().with_context(|| {
        format!(
            "fixture {name} missing at {}; run cargo build --locked --examples",
            path.display()
        )
    })
}

pub fn native_fixture(dir: &Path, name: &str) -> Result<PathBuf> {
    let executable = dir.join(name);
    std::os::unix::fs::symlink(example("fixture-agent")?, &executable)?;
    Ok(executable)
}
