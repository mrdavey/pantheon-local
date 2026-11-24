//! Read local model preferences without starting agents or contacting providers.
use serde_json::{Map, Value, json};
use std::{
    collections::HashMap,
    env, fs,
    io::Read,
    path::{Component, Path, PathBuf},
    rc::Rc,
};

fn setting(value: &Value) -> Option<&str> {
    clean(value.as_str()?)
}

fn clean(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 150
        && !value.chars().any(char::is_control)
        && !value.contains("${"))
    .then_some(value)
}

fn read(path: &Path) -> Value {
    const LIMIT: u64 = 4 * 1024 * 1024;
    let Ok(metadata) = path.metadata() else {
        return Value::Null;
    };
    if !metadata.is_file() || metadata.len() > LIMIT {
        return Value::Null;
    }
    let mut raw = String::new();
    if fs::File::open(path)
        .and_then(|f| f.take(LIMIT + 1).read_to_string(&mut raw))
        .is_err()
        || raw.len() as u64 > LIMIT
    {
        return Value::Null;
    }
    match path.extension().and_then(|s| s.to_str()) {
        Some("toml") => toml::from_str(&raw).ok(),
        Some("yaml") => serde_saphyr::from_str(&raw).ok(),
        _ => serde_json::from_str(&raw).ok(),
    }
    .unwrap_or_default()
}

fn directory(var: &str, fallback: PathBuf) -> PathBuf {
    env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or(fallback)
}

fn model_info(
    model: Option<&str>,
    label: Option<&str>,
    source: &str,
    effort: Option<&str>,
    effort_source: &str,
) -> Value {
    json!({"model":model,"label":label,"source":source,"effort":effort,"effort_source":effort_source})
}

fn codex(profile: &Value, config: &Value, catalog: &Value) -> Value {
    let overlay = setting(&config["profile"]).and_then(|name| config["profiles"].get(name));
    let get = |key: &str| {
        overlay
            .and_then(|p| setting(&p[key]))
            .or_else(|| setting(&config[key]))
    };
    let native = get("model");
    let effective = setting(&profile["model"]).or(native);
    let entry = effective.and_then(|id| {
        catalog["models"]
            .as_array()?
            .iter()
            .find(|m| m["slug"] == id)
    });
    let label = entry
        .and_then(|m| setting(&m["display_name"]))
        .or(effective);
    let configured = get("model_reasoning_effort");
    let effort = configured.or_else(|| entry.and_then(|m| setting(&m["default_reasoning_level"])));
    model_info(
        native,
        label,
        "Codex config.toml",
        effort,
        if configured.is_some() {
            "Codex config.toml"
        } else {
            "Codex model catalog"
        },
    )
}

fn claude(profile: &Value, config: &Value) -> Value {
    let env_setting = |key: &str| {
        env::var(key)
            .ok()
            .and_then(|v| clean(&v).map(str::to_owned))
            .or_else(|| setting(&config["env"][key]).map(str::to_owned))
    };
    let from_env = env_setting("ANTHROPIC_MODEL");
    let model = from_env.as_deref().or_else(|| setting(&config["model"]));
    let resolve = |id: &str| match id {
        "haiku" | "sonnet" | "opus" | "fable" => env_setting(&format!(
            "ANTHROPIC_DEFAULT_{}_MODEL",
            id.to_ascii_uppercase()
        )),
        _ => None,
    };
    let pinned = model.and_then(resolve);
    let native = pinned.as_deref().or(model);
    let effective = setting(&profile["model"]).or(native);
    let effective_pin = effective.and_then(resolve);
    let label = effective_pin.as_deref().or(effective);
    let effort_env = env_setting("CLAUDE_CODE_EFFORT_LEVEL");
    let per_model = label.and_then(|id| setting(&config["modelSettings"][id]["effortLevel"]));
    // The legacy user-level effortLevel applies only to older model generations.
    // Other models require a per-model value or an explicit environment setting.
    let legacy_effort = label
        .filter(|m| {
            m.starts_with("claude-opus-4-")
                || m.starts_with("claude-sonnet-4-")
                || *m == "claude-opus-5"
                || m.starts_with("claude-fable-5-1")
        })
        .and_then(|_| setting(&config["effortLevel"]));
    let effort = effort_env.as_deref().or(per_model).or(legacy_effort);
    let mut detected = model_info(
        native,
        label,
        if pinned.is_some() {
            "Claude Code model alias"
        } else if from_env.is_some() {
            "ANTHROPIC_MODEL"
        } else {
            "Claude Code settings.json"
        },
        effort,
        if effort_env.is_some() {
            "CLAUDE_CODE_EFFORT_LEVEL"
        } else {
            "Claude Code settings.json"
        },
    );
    if label.is_some_and(|name| {
        matches!(
            name,
            "haiku" | "sonnet" | "opus" | "fable" | "best" | "opusplan" | "default"
        )
    }) {
        detected["alias"] = json!(true);
    }
    detected
}

fn hermes(config: &Value) -> Value {
    let model = setting(&config["model"]["default"])
        .or_else(|| setting(&config["model"]["model"]))
        .or_else(|| setting(&config["model"]));
    let agent = &config["agent"];
    let overrides = agent["reasoning_overrides"].as_object();
    let direct = model.and_then(|m| overrides?.get(m)).and_then(setting);
    // A Hermes build can resolve additional aliases. Avoid attributing a global
    // reasoning level when a model-specific override could apply.
    let effort = direct.or_else(|| {
        if overrides.is_none_or(|o| o.is_empty()) {
            setting(&agent["reasoning_effort"])
        } else {
            None
        }
    });
    model_info(
        model,
        model,
        "Hermes profile config.yaml",
        effort,
        "Hermes profile config.yaml",
    )
}

fn profile_dir(root: &Path, profile: &Value) -> Option<PathBuf> {
    let mut sticky = String::new();
    let name = if let Some(explicit) = setting(&profile["hermes_profile"]) {
        explicit
    } else {
        if let Some(dir) = env::var_os("HERMES_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            && dir.parent().is_some_and(|p| p.ends_with("profiles"))
        {
            return Some(dir);
        }
        let path = root.join("active_profile");
        if let Ok(info) = path.metadata()
            && info.is_file()
            && info.len() <= 150
            && let Ok(file) = fs::File::open(path)
        {
            let _ = file.take(151).read_to_string(&mut sticky);
        }
        clean(sticky.trim_start_matches('\u{feff}')).unwrap_or("default")
    };
    if name == "default" {
        return Some(root.into());
    }
    let mut parts = Path::new(name).components();
    (matches!(parts.next(), Some(Component::Normal(_))) && parts.next().is_none())
        .then(|| root.join("profiles").join(name))
}

pub fn discover(profiles: &mut Map<String, Value>, state: &Path) {
    let home = env::var_os("HOME").map(PathBuf::from);
    let mut files = HashMap::new();
    let mut load = |path: PathBuf| {
        files
            .entry(path.clone())
            .or_insert_with(|| Rc::new(read(&path)))
            .clone()
    };
    let catalog = crate::routing::local_rows(state);
    for p in profiles.values_mut() {
        if !p.is_object() {
            continue;
        }
        let mut detected = match (p["kind"].as_str(), home.as_deref()) {
            (Some("codex"), Some(home)) => {
                let root = directory("CODEX_HOME", home.join(".codex"));
                codex(
                    p,
                    &load(root.join("config.toml")),
                    &load(root.join("models_cache.json")),
                )
            }
            (Some("claude"), Some(home)) => {
                let root = directory("CLAUDE_CONFIG_DIR", home.join(".claude"));
                claude(p, &load(root.join("settings.json")))
            }
            (Some("hermes"), Some(home)) => profile_dir(&super::hermes::root(home), p)
                .map(|root| hermes(&load(root.join("config.yaml"))))
                .unwrap_or_default(),
            (Some("antigravity"), Some(home)) => {
                let cfg = load(home.join(".gemini/antigravity-cli/settings.json"));
                // Antigravity stores a display label, not a provider model ID.
                model_info(
                    None,
                    setting(&cfg["model"]),
                    "Antigravity settings.json",
                    None,
                    "",
                )
            }
            _ => json!({}),
        };
        crate::routing::suggest(p, &mut detected, &catalog);
        p["detected"] = detected;
    }
}
