use super::*;

fn apply_settings(profile: &mut Value, settings: &Value) -> Result<()> {
    for key in ["model", "effort"] {
        let Some(value) = settings[key].as_str() else {
            continue;
        };
        let value = value.trim();
        if value == s(profile, key) {
            continue;
        }
        ensure!(
            value.len() <= 150 && !value.contains(['\n', '\r', '\0']),
            "invalid worker model setting"
        );
        ensure!(
            profile["kind"] != "custom",
            "this adapter manages its own model settings"
        );
        profile[key] = json!(value);
        profile[format!("benchmark_{key}")] = json!(value);
    }
    adapter::validate(profile)
}

/// Expand per-run slots into the same named profiles used by every dispatch path.
pub(super) fn configure(input: &mut Value, profiles: &mut Value) -> Result<()> {
    ensure!(
        input["coordinator_settings"].is_null() || input["worker_pool"].is_array(),
        "coordinator settings require a worker pool"
    );
    if let Some(slots) = input["worker_pool"].as_array() {
        ensure!(
            input["workers"].is_null(),
            "choose either workers or a worker pool"
        );
        let defaults = profiles.clone();
        if input["coordinator_settings"].is_object() {
            let coord = profiles
                .get_mut(s(input, "coordinator"))
                .context("unknown coordinator profile")?;
            apply_settings(coord, &input["coordinator_settings"])?;
        }
        let mut ids = Vec::new();
        for slot in slots {
            let base = s(slot, "profile");
            let mut profile = defaults
                .get(base)
                .context("unknown worker profile")?
                .clone();
            apply_settings(&mut profile, slot)?;
            let mut id = base.to_owned();
            if ids.contains(&id) || id == s(input, "coordinator") {
                let mut suffix = 2;
                loop {
                    id = format!("{base}-{suffix}");
                    if profiles.get(&id).is_none() {
                        break;
                    }
                    suffix += 1;
                }
            }
            profiles[&id] = profile;
            ids.push(id);
        }
        input["min_workers"] = json!(ids.len());
        input["workers"] = json!(ids);
    }
    if let Some(min) = input["min_workers"].as_u64() {
        let count = strings(&input["workers"])
            .into_iter()
            .collect::<std::collections::HashSet<_>>()
            .len();
        ensure!(
            min > 0 && min <= count as u64,
            "add enough worker agents to meet the minimum"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_workers_keep_distinct_settings_and_coordinator_identity() -> Result<()> {
        let mut profiles = json!({"codex":{"kind":"codex","model":"original"},"codex-2":{"kind":"custom","executable":"test"},"ag":{"kind":"antigravity"}});
        let mut input = json!({"coordinator":"codex","min_workers":1,"coordinator_settings":{"model":"coordinator-only"},"worker_pool":[
            {"profile":"codex","model":"alpha","effort":"high"},{"profile":"codex","model":"beta","effort":"low"},
            {"profile":"ag","model":"first","effort":"medium"},{"profile":"ag","model":"second","effort":"high"}]});
        configure(&mut input, &mut profiles)?;
        assert_eq!(
            input["workers"],
            json!(["codex-3", "codex-4", "ag", "ag-2"])
        );
        assert_eq!(input["min_workers"], 4);
        assert_eq!(profiles["codex"]["model"], "coordinator-only");
        assert_eq!(profiles["codex-3"]["effort"], "high");
        assert_eq!(profiles["codex-4"]["model"], "beta");
        assert_eq!(profiles["ag"]["model"], "first");
        assert_eq!(profiles["ag"]["effort"], "medium");
        assert_eq!(profiles["ag-2"]["model"], "second");
        assert_eq!(profiles["ag-2"]["effort"], "high");
        assert!(
            configure(
                &mut json!({"worker_pool":[{"profile":"missing"}]}),
                &mut profiles
            )
            .is_err()
        );
        assert!(
            configure(
                &mut json!({"workers":["codex"],"min_workers":2}),
                &mut profiles
            )
            .is_err()
        );
        Ok(())
    }
}
