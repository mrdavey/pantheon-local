//! Antigravity's ACP usage notifications describe context occupancy. Its local
//! transcript includes consumed tokens per model call while a prompt runs.
use super::*;
use chrono::DateTime;
use std::{
    env,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::PathBuf,
};

const MAX_TRANSCRIPT_BYTES: u64 = 4 * 1024 * 1024;
const SESSION_LOG_BYTES: u64 = 64 * 1024;

fn logged_session(dir: &Path) -> Option<String> {
    let file = fs::File::open(dir.join("events.log")).ok()?;
    BufReader::new(file.take(SESSION_LOG_BYTES))
        .lines()
        .map_while(|line| line.ok())
        .filter(|line| line.contains(r#""session""#))
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
        .find(|event| event["type"] == "session")
        .and_then(|event| event["session"].as_str().map(str::to_owned))
}

fn transcript_path(home: &Path, session: &str) -> Option<PathBuf> {
    // ACP session IDs are opaque in general. Antigravity's local transcript
    // directories use UUIDs, so never interpret a returned ID as a path.
    let bytes = session.as_bytes();
    (bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, byte)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                *byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        }))
    .then(|| {
        home.join(".gemini/antigravity-cli/brain")
            .join(session)
            .join(".system_generated/logs/transcript.jsonl")
    })
}

#[derive(Deserialize)]
struct Step {
    #[serde(rename = "type")]
    kind: String,
    step_index: u64,
    created_at: DateTime<Utc>,
    model: Option<String>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_tokens: Option<u64>,
    cache_write_tokens: Option<u64>,
}

fn transcript(
    path: &Path,
    started: i64,
    ended: Option<i64>,
    profile_model: &str,
) -> Option<Vec<Sample>> {
    if !path.symlink_metadata().ok()?.is_file() {
        return None;
    }
    let mut file = fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let start = length.saturating_sub(MAX_TRANSCRIPT_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut reader = BufReader::new(file.take(length - start));
    let mut line = Vec::new();
    if start > 0 {
        reader.read_until(b'\n', &mut line).ok()?;
    }
    let mut samples = Vec::new();
    let mut prompt = None;
    let mut seen = HashSet::new();
    let mut totals: BTreeMap<String, Reading> = BTreeMap::new();
    let mut previous_at = started;
    let started_second = started.div_euclid(1000) * 1000;
    let mut truncated_prefix = start > 0;
    loop {
        line.clear();
        reader.read_until(b'\n', &mut line).ok()?;
        if line.last() != Some(&b'\n') {
            break;
        }
        let Ok(step) = serde_json::from_slice::<Step>(&line) else {
            continue;
        };
        let at = step.created_at.timestamp_millis();
        if truncated_prefix {
            // The first prompt could have fallen outside the read window.
            if at >= started_second {
                return None;
            }
            truncated_prefix = false;
        }
        // The CLI records whole seconds. Use the prompt boundary as well so
        // we can distinguish work from a previous turn in the same session.
        if at < started_second || ended.is_some_and(|end| at > end) {
            continue;
        }
        if step.kind == "USER_INPUT"
            && let Some(previous) = prompt.replace(step.step_index)
            && previous != step.step_index
        {
            // More than one possible prompt means there is no reliable match.
            return None;
        }
        if step.kind != "PLANNER_RESPONSE"
            || !prompt.is_some_and(|index| step.step_index > index)
            || !seen.insert(step.step_index)
        {
            continue;
        }
        let (Some(input), Some(output)) = (step.input_tokens, step.output_tokens) else {
            continue;
        };
        let name = step
            .model
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| profile_model.into());
        let reading = totals.entry(name).or_default();
        let breakdown = reading.breakdown.get_or_insert_with(Breakdown::default);
        breakdown.input = breakdown.input.checked_add(input)?;
        breakdown.output = breakdown.output.checked_add(output)?;
        breakdown.cached = breakdown
            .cached
            .checked_add(step.cache_read_tokens.unwrap_or(0))?;
        breakdown.cache_write = breakdown
            .cache_write
            .checked_add(step.cache_write_tokens.unwrap_or(0))?;
        reading.tokens = Some(
            breakdown
                .input
                .checked_add(breakdown.output)?
                .checked_add(breakdown.cached)?
                .checked_add(breakdown.cache_write)?,
        );
        previous_at = previous_at.max(at);
        if samples
            .last()
            .is_some_and(|sample: &Sample| sample.at == previous_at)
        {
            samples.pop();
        }
        samples.push(Sample {
            at: previous_at,
            models: totals.clone(),
        });
        if samples.len() >= 512 {
            samples = samples.drain(..).skip(1).step_by(2).collect();
        }
    }
    Some(samples)
}

pub(super) fn history(
    dir: &Path,
    turn: &Value,
    profile_model: &str,
    recorded: Vec<Sample>,
) -> Vec<Sample> {
    history_in(
        dir,
        turn,
        profile_model,
        recorded,
        env::var_os("HOME").as_deref().map(Path::new),
    )
}

fn history_in(
    dir: &Path,
    turn: &Value,
    profile_model: &str,
    recorded: Vec<Sample>,
    home: Option<&Path>,
) -> Vec<Sample> {
    let result: Value = fs::File::open(dir.join("result.json"))
        .ok()
        .and_then(|file| serde_json::from_reader(file).ok())
        .unwrap_or_default();
    let ended = modified_millis(dir, "result.json").or_else(|| turn["finished"].as_i64());
    let session = turn["acp_session"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| result["session"].as_str().filter(|s| !s.is_empty()))
        .map(str::to_owned)
        .or_else(|| logged_session(dir))
        .or_else(|| {
            let job: Value =
                serde_json::from_reader(fs::File::open(dir.join("job.json")).ok()?).ok()?;
            job["request"]["session"].as_str().map(str::to_owned)
        });
    let mut live = home
        .zip(session.as_deref())
        .and_then(|(home, session)| transcript_path(home, session))
        .and_then(|path| transcript(&path, turn["started"].as_i64()?, ended, profile_model))
        .unwrap_or_default();
    let saved = recorded.last().and_then(|s| s.models.first_key_value());
    let Some(receipt) = crate::adapter::antigravity_usage(&result["acp_usage"])
        .or_else(|| crate::adapter::antigravity_usage(&turn["acp_usage"]))
    else {
        // A receipt from an older CLI can still be useful if it reported tokens.
        return if saved.is_some_and(|(_, r)| r.tokens.is_some()) || live.is_empty() {
            recorded
        } else {
            live
        };
    };
    let final_tokens = receipt["tokens"].as_u64().unwrap_or_default();
    let live_models = live.last().map(|sample| &sample.models);
    let live_total = live_models.and_then(|models| {
        models
            .values()
            .try_fold(0u64, |total, row| total.checked_add(row.tokens?))
    });
    let multiple_models = live_models.is_some_and(|models| models.len() > 1);
    // The final receipt has no model split. Retain the transcript's split
    // only when its sum matches the provider receipt and there is no price
    // to attribute to the aggregate.
    if multiple_models
        && live_total == Some(final_tokens)
        && saved.is_none_or(|(_, row)| row.usd.is_none())
    {
        return live;
    }
    if multiple_models || live_total.is_some_and(|total| total > final_tokens) {
        live.clear();
    }
    let name = result["acp_model"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            saved
                .map(|(name, _)| name.as_str())
                .filter(|name| *name != "Unknown model")
        })
        .or_else(|| {
            live.last()
                .and_then(|sample| sample.models.first_key_value())
                .map(|(name, _)| name.as_str())
        })
        .unwrap_or(profile_model)
        .to_owned();
    for sample in &mut live {
        if sample.models.len() == 1
            && let Some((_, row)) = sample.models.pop_first()
        {
            sample.models.insert(name.clone(), row);
        }
    }
    let at = ended
        .or_else(|| recorded.last().map(|s| s.at))
        .or_else(|| turn["started"].as_i64())
        .unwrap_or_default();
    let final_sample = Sample {
        at,
        models: BTreeMap::from([(
            name,
            Reading {
                tokens: Some(final_tokens),
                usd: saved.and_then(|(_, row)| row.usd),
                breakdown: Breakdown::native(&receipt),
            },
        )]),
    };
    // The final ACP receipt supersedes the old recorded sample, which may
    // have been calculated as a difference between separate turn totals.
    if live
        .last()
        .is_some_and(|prior| prior.at >= at || prior.models == final_sample.models)
    {
        live.pop();
    }
    live.push(final_sample);
    live
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn live_transcript_counts_only_the_matching_prompt() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let home = tmp.path();
        let session = "ef811a19-a416-4ef8-951c-bb4b896b584d";
        let path = transcript_path(home, session).unwrap();
        assert!(transcript_path(home, "../../another/session").is_none());
        fs::create_dir_all(path.parent().unwrap())?;
        let mut file = fs::File::create(path)?;
        let started = DateTime::parse_from_rfc3339("2026-11-27T13:17:00.755Z")?.timestamp_millis();
        let user = |index, time| json!({"type":"USER_INPUT","step_index":index,"created_at":time});
        let call = |index, time, input, cached, output| {
            json!({"type":"PLANNER_RESPONSE","step_index":index,"created_at":time,
                "model":"gemini-fixture","input_tokens":input,"cache_read_tokens":cached,"output_tokens":output})
        };
        let response = call(8, "2026-11-27T13:17:04Z", 30, 10, 5);
        for step in [
            user(1, "2026-11-26T13:17:01Z"),
            call(2, "2026-11-26T13:17:02Z", 1000, 500, 200),
            user(7, "2026-11-27T13:17:01Z"),
            response.clone(),
            response,
            call(9, "2026-11-27T13:17:06Z", 50, 20, 10),
        ] {
            writeln!(file, "{step}")?;
        }
        write!(file, "{}", call(10, "2026-11-27T13:17:07Z", 99, 0, 0))?;
        fs::write(
            home.join("events.log"),
            format!("{}\n", json!({"type":"session","session":session})),
        )?;
        let turn = json!({"state":"running","started":started});
        let actual = history_in(home, &turn, "Unknown model", vec![], Some(home));
        assert_eq!(actual.len(), 2);
        let row = &actual[1].models["gemini-fixture"];
        assert_eq!(row.tokens, Some(125));
        assert_eq!(
            row.breakdown,
            Some(Breakdown {
                input: 80,
                cached: 30,
                output: 15,
                cache_write: 0
            })
        );

        writeln!(file)?;
        writeln!(file, "{}", user(11, "2026-11-27T13:17:10Z"))?;
        assert!(history_in(home, &turn, "Unknown model", vec![], Some(home)).is_empty());
        Ok(())
    }

    #[test]
    fn final_turn_receipt_corrects_saved_antigravity_totals() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let turn = json!({"started":1000,"finished":4000,"acp_usage":{
            "totalTokens":170,"inputTokens":100,"outputTokens":20,"cachedReadTokens":50
        }});
        let stale = Sample {
            at: 3000,
            models: BTreeMap::from([(
                "gemini-fixture".into(),
                Reading {
                    tokens: Some(22),
                    ..Reading::default()
                },
            )]),
        };
        for recorded in [vec![], vec![stale]] {
            let history = history_in(temp.path(), &turn, "gemini-fixture", recorded, None);
            assert_eq!(history.len(), 1);
            assert_eq!(history[0].at, 4000);
            let row = &history[0].models["gemini-fixture"];
            assert_eq!(row.tokens, Some(170));
            assert_eq!(row.breakdown.as_ref().unwrap().cached, 50);
        }
        Ok(())
    }
}
