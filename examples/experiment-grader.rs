//! Independent deterministic evaluator for the daemon and browser tests.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{fs, io};
fn main() -> Result<()> {
    let case: Value = serde_json::from_reader(io::stdin().lock())?;
    let implementation = match fs::read_to_string("result.txt") {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => "starting".into(),
        Err(e) => return Err(e.into()),
    };
    let (mut relevance, latency) = match implementation.trim_end() {
        "starting" => (3, 2),
        "wrong" => (10, 8),
        "correct" => (7, 3),
        _ => bail!("unexpected fixture data"),
    };
    if std::path::Path::new("mixed.txt").try_exists()? {
        relevance += 1
    }
    let sample = case["sample"].as_i64().context("missing sample")?;
    println!(
        "{}",
        json!({"pass":true,"observed":implementation,"metrics":{"relevance":relevance,"latency":latency as f64+(sample as f64-1.)/10.}})
    );
    Ok(())
}
