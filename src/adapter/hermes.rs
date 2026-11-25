//! Local Hermes Desktop and ACP share a runtime and profile directories.
use super::*;
use serde_json::Map;

pub(super) fn desktop_executable(home: &Path, root: &Path) -> Option<PathBuf> {
    [
        home.join(".local/bin/hermes"),
        root.join("hermes-agent/.hermes/bin/hermes"),
        root.join("hermes-agent/venv/bin/hermes"),
    ]
    .into_iter()
    .find(|p| p.to_str().is_some_and(installed))
}

pub fn root(home: &Path) -> PathBuf {
    if let Some(root) = env::var_os("HERMES_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        if let Some(parent) = root.parent().filter(|p| p.ends_with("profiles"))
            && let Some(base) = parent.parent()
        {
            return base.to_owned();
        }
        return root;
    }
    home.join(format!(
        ".hermes{}",
        env::var("HERMES_DATA_DIR_SUFFIX").unwrap_or_default()
    ))
}

pub fn add_profiles(profiles: &mut Map<String, Value>, root: &Path, settings: &Value) {
    let Some(base) = profiles.get("hermes").filter(|v| v["kind"] == "hermes") else {
        return;
    };
    let executable = base.get("executable").cloned();
    let dir = root.join("profiles");
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if name == "default"
            || name.len() > 64
            || !name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
            || !name
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'-'))
            || dir.join(".deleted").join(&name).exists()
            || ![
                "config.yaml",
                "profile.yaml",
                ".env",
                "SOUL.md",
                "auth.json",
                "state.db",
            ]
            .iter()
            .any(|f| entry.path().join(f).is_file())
            || profiles
                .values()
                .any(|p| p["kind"] == "hermes" && p["hermes_profile"] == name)
        {
            continue;
        }
        let key = format!("hermes-{name}");
        let mut profile = json!({"kind":"hermes","hermes_profile":name});
        if let Some(exe) = &executable {
            profile["executable"] = exe.clone();
        }
        for field in [
            "model",
            "effort",
            "benchmark_model",
            "benchmark_agent",
            "benchmark_effort",
        ] {
            if let Some(value) = settings[&key][field].as_str() {
                profile[field] = json!(value);
            }
        }
        profiles.entry(key).or_insert(profile);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_discovery_respects_profile_identity_and_local_overrides() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("profiles");
        for name in ["writer", "retired", "override", "empty"] {
            fs::create_dir_all(dir.join(name)).unwrap();
            if name != "empty" {
                fs::write(dir.join(name).join("SOUL.md"), "").unwrap();
            }
        }
        fs::create_dir_all(dir.join(".deleted/retired")).unwrap();
        let mut cfg = json!({"hermes":{"kind":"hermes","executable":"/fixture"},"hermes-override":{"kind":"custom"}});
        add_profiles(
            cfg.as_object_mut().unwrap(),
            tmp.path(),
            &json!({"hermes-writer":{"benchmark_model":"x"}}),
        );
        assert_eq!(cfg.as_object().unwrap().len(), 3);
        assert_eq!(cfg["hermes-override"]["kind"], "custom");
        let p = &cfg["hermes-writer"];
        assert_eq!(p["executable"], "/fixture");
        assert_eq!(p["benchmark_model"], "x");
        assert_eq!(args(&json!({"profile":p})), ["--profile", "writer", "acp"]);
    }
}
