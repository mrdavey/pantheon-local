//! Exercise the same profile discovery and model-save boundaries used by the workbench.
mod common;
use anyhow::Result;
use serde_json::json;
use std::{collections::BTreeMap, fs};

#[test]
fn reads_local_model_defaults_without_pinning_them_or_returning_private_config() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let home = dir.path().join("home");
    let codex = home.join("codex");
    let claude = home.join("claude");
    let hermes = home.join("hermes");
    let agy = home.join(".gemini/antigravity-cli");
    for folder in [&codex, &claude, &hermes.join("profiles/writer"), &agy] {
        fs::create_dir_all(folder)?;
    }
    fs::write(
        codex.join("config.toml"),
        "model = 'base'\nprofile = 'local'\n[profiles.local]\nmodel = 'demo-1.5'\n",
    )?;
    fs::write(codex.join("models_cache.json"), json!({"models":[{"slug":"demo-1.5","display_name":"Demo 1.5","default_reasoning_level":"medium"}],"private":"private-test-sentinel"}).to_string())?;
    fs::write(claude.join("settings.json"),json!({"model":"ignored","effortLevel":"low","env":{"ANTHROPIC_DEFAULT_SONNET_MODEL":"claude-sonnet-test","API_KEY":"private-test-sentinel"}}).to_string())?;
    fs::write(hermes.join("active_profile"), "writer\n")?;
    fs::write(hermes.join("config.yaml"), "model: 'root-default'\n")?;
    fs::write(
        hermes.join("profiles/writer/config.yaml"),
        "model:\n  default: publisher/writer\nagent:\n  reasoning_effort: low\n  reasoning_overrides:\n    publisher/writer: high\n",
    )?;
    fs::write(
        agy.join("settings.json"),
        r#"{"model":"Gemini Flash (Preview)"}"#,
    )?;
    let env = BTreeMap::from([
        ("HOME".into(), home),
        ("CODEX_HOME".into(), codex.clone()),
        ("CLAUDE_CONFIG_DIR".into(), claude),
        ("HERMES_HOME".into(), hermes),
        ("ANTHROPIC_MODEL".into(), "sonnet".into()),
        (
            "ANTHROPIC_DEFAULT_SONNET_MODEL".into(),
            "claude-sonnet-test".into(),
        ),
        ("CLAUDE_CODE_EFFORT_LEVEL".into(), "high".into()),
    ]);
    let svc = common::Service::with_profiles(
        dir.path(),
        env,
        &json!({"profiles":{
            "codex":{"kind":"codex"}, "claude":{"kind":"claude"}, "hermes":{"kind":"hermes","hermes_profile":"default"},
            "sticky":{"kind":"hermes"},"agy":{"kind":"antigravity"}, "custom":{"kind":"custom","executable":"true"}
        }}),
    )?;
    fs::write(svc.state.join("benchmarks.json"),json!({"rows":[{"model":"Demo 1.5","effort":"medium","agent":"Codex CLI",
        "suite":"Test bench","source":"https://example.invalid/demo","date":chrono::Utc::now().date_naive(),"class":"terminal","score":95}]}).to_string())?;
    let setup = svc.get("/setup")?;
    assert!(!setup.to_string().contains("private-test-sentinel"));
    let models = &setup["profiles"];
    let c = &models["codex"]["detected"];
    assert_eq!(c["model"], "demo-1.5");
    assert_eq!(c["label"], "Demo 1.5");
    assert_eq!(c["effort"], "medium");
    assert_eq!(c["routing"]["agent"], "Codex CLI");
    assert_eq!(c["routing"]["source"], "Test bench");
    assert_eq!(models["claude"]["detected"]["model"], "claude-sonnet-test");
    assert_eq!(models["claude"]["detected"]["effort"], "high");
    assert_eq!(
        models["hermes-writer"]["detected"]["model"],
        "publisher/writer"
    );
    assert_eq!(models["hermes-writer"]["detected"]["effort"], "high");
    assert_eq!(models["sticky"]["detected"]["model"], "publisher/writer");
    assert_eq!(models["hermes"]["detected"]["model"], "root-default");
    let saved_hermes = svc.post(
        "/setup/model",
        &json!({"profile":"hermes-writer","model":"service:worker-model","effort":"high"}),
    )?;
    assert_eq!(
        saved_hermes["profiles"]["hermes-writer"]["model"],
        "service:worker-model"
    );
    assert_eq!(saved_hermes["profiles"]["hermes-writer"]["effort"], "high");
    assert_eq!(models["agy"]["detected"]["label"], "Gemini Flash (Preview)");
    assert!(models["agy"]["detected"]["model"].is_null());
    assert!(models["custom"]["detected"]["model"].is_null());

    let saved = svc.post(
        "/setup/model",
        &json!({"profile":"codex","model":"override","effort":"ultra"}),
    )?;
    assert_eq!(saved["profiles"]["codex"]["detected"]["label"], "override");
    assert!(saved["profiles"]["codex"]["detected"]["routing"]["source"].is_null());
    assert_eq!(saved["profiles"]["codex"]["effort"], "ultra");
    let edit = svc.post(
        "/setup/model",
        &json!({"profile":"codex","benchmark_effort":"low"}),
    )?;
    assert_eq!(edit["profiles"]["codex"]["model"], "override");
    let disk: serde_json::Value =
        serde_json::from_slice(&fs::read(svc.state.join("profiles.json"))?)?;
    assert!(disk["profiles"]["codex"].get("detected").is_none());

    fs::write(codex.join("config.toml"), "model = 'replacement'\n")?;
    let reset = svc.post("/setup/model", &json!({"profile":"codex","model":""}))?;
    assert_eq!(
        reset["profiles"]["codex"]["detected"]["model"],
        "replacement"
    );
    assert!(reset["profiles"]["codex"]["detected"]["effort"].is_null());
    Ok(())
}
