mod acp;
mod codex_chat;
mod codex_usage;
pub mod hermes;
pub mod models;
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    env, fs,
    io::{self, BufRead, BufReader, Read, Seek, Write},
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    thread,
    time::Duration,
};
use wait_timeout::ChildExt;

fn field<'a>(obj: &'a Value, key: &str) -> &'a str {
    obj[key].as_str().unwrap_or_default()
}
pub fn binary(p: &Value) -> std::borrow::Cow<'_, str> {
    if p["kind"] == "hermes"
        && field(p, "executable").is_empty()
        && !installed("hermes")
        && let Some(home) = env::var_os("HOME").map(PathBuf::from)
        && let Some(path) = hermes::desktop_executable(&home, &hermes::root(&home))
    {
        return path.to_string_lossy().into_owned().into();
    }
    match field(p, "executable") {
        "" => match field(p, "kind") {
            "antigravity" => "agy",
            k => k,
        },
        s => s,
    }
    .into()
}
pub fn validate(p: &Value) -> Result<()> {
    let effort = field(p, "effort");
    let valid = match field(p, "kind") {
        "codex" | "hermes" | "antigravity" => {
            effort.len() <= 32
                && effort
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        }
        "claude" => ["low", "medium", "high", "xhigh", "max"].contains(&effort),
        _ => false,
    };
    ensure!(
        effort.is_empty() || valid,
        "unsupported reasoning effort for this harness"
    );
    match field(p, "kind") {
        "codex" | "claude" | "hermes" | "antigravity" => Ok(()),
        "custom" => {
            ensure!(
                !field(p, "executable").is_empty(),
                "custom adapter needs an executable"
            );
            Ok(())
        }
        k => bail!("unknown adapter {k:?}"),
    }
}
pub fn installed(name: &str) -> bool {
    let executable = |p: PathBuf| {
        fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if name.contains('/') {
        return executable(PathBuf::from(name));
    }
    env::var_os("PATH")
        .is_some_and(|paths| env::split_paths(&paths).any(|p| executable(p.join(name))))
}
fn inspect(
    p: &Value,
    args: &[&str],
    timeout: u64,
    combined: bool,
    state: Option<&Path>,
) -> Result<(bool, String)> {
    inspect_limited(p, args, timeout, combined, state, 64 * 1024)
}

fn inspect_limited(
    p: &Value,
    args: &[&str],
    timeout: u64,
    combined: bool,
    state: Option<&Path>,
    limit: u64,
) -> Result<(bool, String)> {
    let mut capture = tempfile::tempfile()?;
    let mut cmd = Command::new(binary(p).as_ref());
    cmd.args(args)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(capture.try_clone()?);
    if combined {
        cmd.stderr(capture.try_clone()?);
    } else {
        cmd.stderr(Stdio::null());
    }
    if let Some(state) = state {
        crate::run::release::scrub_worker(&mut cmd, state)?;
    }
    let mut child = cmd.spawn()?;
    let pgid = nix::unistd::Pid::from_raw(child.id() as i32);
    let status = match child.wait_timeout(Duration::from_secs(timeout))? {
        Some(s) => s,
        None => {
            let _ = nix::sys::signal::killpg(pgid, nix::sys::signal::Signal::SIGKILL);
            let _ = child.kill();
            let _ = child.wait();
            bail!("agent status probe timed out")
        }
    };
    let _ = nix::sys::signal::killpg(pgid, nix::sys::signal::Signal::SIGKILL);
    capture.rewind()?;
    let mut out = Vec::new();
    capture.take(limit).read_to_end(&mut out)?;
    Ok((
        status.success(),
        String::from_utf8_lossy(&out).trim().into(),
    ))
}
pub fn probe(p: &Value, state: Option<&Path>) -> Result<String> {
    validate(p)?;
    let (ok, version) = inspect(p, &["--version"], 5, true, state)?;
    ensure!(ok, "{} version probe failed", binary(p));
    Ok(version)
}
pub fn availability(p: &Value, state: Option<&Path>) -> Value {
    let state = if validate(p).is_err() {
        "invalid"
    } else if !installed(&binary(p)) {
        "missing"
    } else {
        match field(p, "kind") {
            "codex" => match inspect(p, &["login", "status"], 2, true, state) {
                Ok((_, s)) if s.to_lowercase().starts_with("not logged in") => "sign-in",
                Ok((true, s)) if s.to_lowercase().starts_with("logged in") => "connected",
                _ => "sign-in-unknown",
            },
            "claude" => match inspect(p, &["auth", "status", "--json"], 2, false, state)
                .ok()
                .and_then(|(_, s)| serde_json::from_str::<Value>(&s).ok())
                .and_then(|v| v["loggedIn"].as_bool())
            {
                Some(true) => "connected",
                Some(false) => "sign-in",
                None => "sign-in-unknown",
            },
            _ => "installed",
        }
    };
    json!({"state":state})
}
fn args(request: &Value) -> Vec<String> {
    let profile = &request["profile"];
    let kind = field(profile, "kind");
    let role = field(request, "role");
    let session = field(request, "session");
    let model = field(profile, "model");
    let effort = field(profile, "effort");
    let effort_override = format!("model_reasoning_effort={effort:?}");
    let mut args = Vec::new();
    // Finite runs use scheduler-owned children so independent workers share one limit.
    let native_children = role == "worker"
        && request["subagents"] != false
        && request["subagent_limit"].as_u64().is_none();
    if kind == "codex" {
        if role == "pricing" {
            args.extend(["-c", r#"web_search="live""#]);
        }
        if !native_children {
            args.extend([
                "-c",
                "agents.enabled=false",
                "-c",
                "features.multi_agent_v2=false",
            ]);
        }
        if role != "advisor" && !effort.is_empty() {
            args.extend(["-c", &effort_override]);
        }
    }
    match kind {
        "hermes" | "antigravity" => {
            let name = field(profile, "hermes_profile");
            if kind == "hermes" && !name.is_empty() {
                args.extend(["--profile", name])
            }
            args.push("acp")
        }
        "custom" => args.push("turn-v1"),
        "codex" if role == "chat" => args.push("app-server"),
        "codex" => {
            args.push("exec");
            if !session.is_empty() {
                args.push("resume")
            }
            args.push("--json");
            if matches!(role, "advisor" | "chat" | "observer" | "pricing") {
                args.push("--skip-git-repo-check")
            }
            if session.is_empty() {
                args.extend([
                    "--sandbox",
                    if role == "worker" {
                        "workspace-write"
                    } else {
                        "read-only"
                    },
                ])
            }
            if role == "advisor" {
                args.extend([
                    "--model",
                    "gpt-6-luna",
                    "-c",
                    r#"model_reasoning_effort="medium""#,
                ]);
            } else if !model.is_empty() {
                args.extend(["--model", model])
            }
            if !session.is_empty() {
                args.push(session)
            }
            args.push("-")
        }
        _ => {
            args.extend(["-p", "--verbose", "--output-format", "stream-json"]);
            if role == "chat" {
                args.push("--include-partial-messages");
            }
            if role != "advisor" && !effort.is_empty() {
                args.extend(["--effort", effort]);
            }
            if role == "worker" && !native_children {
                args.extend(["--disallowedTools", "Agent", "Task"]);
            }
            if role == "pricing" {
                args.extend([
                    "--tools",
                    "WebSearch,WebFetch",
                    "--allowedTools",
                    "WebSearch,WebFetch",
                ]);
            } else if role == "chat"
                && let Some(files) = request["attachments"].as_array().filter(|v| !v.is_empty())
            {
                args.extend(["--tools", "Read", "--allowedTools", "Read"]);
                for file in files {
                    if let Some(dir) = file["path"]
                        .as_str()
                        .and_then(|p| std::path::Path::new(p).parent())
                        .and_then(|p| p.to_str())
                    {
                        args.extend(["--add-dir", dir]);
                    }
                }
            } else if role != "worker" {
                args.extend(["--tools", ""])
            }
            if role == "advisor" {
                args.extend(["--model", "haiku", "--effort", "medium"]);
            } else if !model.is_empty() {
                args.extend(["--model", model])
            }
            if !session.is_empty() {
                args.extend(["--resume", session])
            }
        }
    }
    args.into_iter().map(str::to_owned).collect()
}
const MAX_LINE: usize = 8 * 1024 * 1024;
fn line(input: &mut impl BufRead, buf: &mut Vec<u8>) -> Result<bool> {
    buf.clear();
    let n = input.take(MAX_LINE as u64 + 1).read_until(b'\n', buf)?;
    ensure!(n <= MAX_LINE, "agent event exceeds 8 MiB");
    Ok(n != 0)
}
/// Reap children even when JSON decoding, pipe IO, or session negotiation fails.
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let pgid = nix::unistd::Pid::from_raw(self.0.id() as i32);
        let _ = nix::sys::signal::killpg(pgid, nix::sys::signal::Signal::SIGKILL);
        let _ = self.0.kill();
        let _ = self.0.wait();
        crate::supervisor::set_active_child_group(0);
    }
}
pub fn run(r: &Value, state: Option<&Path>, emit: impl Fn(Value) + Sync) -> Value {
    match execute(r, state, &emit) {
        Ok(v) => v,
        Err(e) => json!({"error":e.to_string(),"session":field(r,"session")}),
    }
}
fn execute(r: &Value, state: Option<&Path>, emit: &(impl Fn(Value) + Sync)) -> Result<Value> {
    let p = &r["profile"];
    validate(p)?;
    let mut cmd = Command::new(binary(p).as_ref());
    if field(p, "kind") == "claude"
        && field(r, "role") != "advisor"
        && !field(p, "effort").is_empty()
    {
        cmd.env("CLAUDE_CODE_EFFORT_LEVEL", field(p, "effort"));
    }
    cmd.args(args(r))
        .current_dir(field(r, "dir"))
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(state) = state {
        crate::run::release::scrub_worker(&mut cmd, state)?
    }
    let mut proc = Process(
        cmd.spawn()
            .with_context(|| format!("start {}", binary(p)))?,
    );
    crate::supervisor::set_active_child_group(proc.0.id() as i32);
    let child = &mut proc.0;
    let pgid = nix::unistd::Pid::from_raw(child.id() as i32);
    let stderr = child.stderr.take().context("missing stderr")?;
    let stdin = child.stdin.take().context("missing stdin")?;
    let stdout = child.stdout.take().context("missing stdout")?;
    thread::scope(|scope| {
        scope.spawn(move || {
            let mut reader = BufReader::new(stderr);
            loop {
                match reader.fill_buf() {
                    Ok([]) | Err(_) => break,
                    Ok(bytes) => {
                        let n = bytes
                            .iter()
                            .position(|b| *b == b'\n')
                            .map_or(bytes.len(), |i| i + 1);
                        emit(json!({"type":"stderr","text":String::from_utf8_lossy(&bytes[..n])}));
                        reader.consume(n);
                    }
                }
            }
        });
        let acp = matches!(field(p, "kind"), "hermes" | "antigravity");
        let codex_chat = field(p, "kind") == "codex" && field(r, "role") == "chat";
        let result = if codex_chat {
            codex_chat::exchange(r, stdin, stdout, emit)
        } else if field(p, "kind") == "codex" {
            codex_usage::stream_with_usage(r, stdin, stdout, emit)
        } else if acp {
            acp::exchange(r, stdin, stdout, emit)
        } else {
            stream(r, stdin, stdout, emit)
        };
        // Closing stdin lets app-server release its thread writer normally.
        // Retain bounded cleanup for a server that never finishes shutdown.
        let chat_still_running =
            codex_chat && child.wait_timeout(Duration::from_secs(2))?.is_none();
        if acp || chat_still_running || (!codex_chat && result.is_err()) {
            let _ = nix::sys::signal::killpg(pgid, nix::sys::signal::Signal::SIGKILL);
            let _ = child.kill();
        }
        let status = child.wait()?;
        let _ = nix::sys::signal::killpg(pgid, nix::sys::signal::Signal::SIGKILL);
        let result = result?;
        ensure!(
            acp || codex_chat || status.success() || !field(&result, "error").is_empty(),
            "agent exited with {status}"
        );
        ensure!(
            !field(&result, "text").is_empty() || !field(&result, "error").is_empty(),
            "adapter returned no final response"
        );
        Ok(result)
    })
}
fn stream(
    r: &Value,
    mut stdin: ChildStdin,
    stdout: ChildStdout,
    emit: &impl Fn(Value),
) -> Result<Value> {
    let kind = field(&r["profile"], "kind");
    let payload = if kind == "custom" {
        let mut data = serde_json::to_vec(r)?;
        data.push(b'\n');
        data
    } else {
        field(r, "prompt").as_bytes().to_vec()
    };
    // Writing concurrently prevents a full stdout pipe from blocking prompt delivery.
    let writer = thread::spawn(move || stdin.write_all(&payload));
    let mut result = json!({});
    if !field(r, "session").is_empty() {
        result["session"] = r["session"].clone();
    }
    let mut reader = BufReader::new(stdout);
    let mut buf = Vec::new();
    while line(&mut reader, &mut buf)? {
        let event = serde_json::from_slice::<Value>(&buf).unwrap_or_default();
        if kind == "custom" {
            if event["event"].is_object() {
                if !field(&event["event"], "session").is_empty() {
                    result["session"] = event["event"]["session"].clone();
                }
                emit(event["event"].clone());
                continue;
            }
            if event["result"].is_object() {
                result = event["result"].clone();
                continue;
            }
        } else if !field(&event, "type").is_empty() {
            for key in ["thread_id", "session_id"] {
                if !field(&event, key).is_empty() {
                    result["session"] = event[key].clone();
                    break;
                }
            }
            let ty = field(&event, "type");
            let message = if ty == "result" {
                field(&event, "result")
            } else {
                field(&event["item"], "text")
            };
            if (ty == "result" || event["item"]["type"] == "agent_message") && !message.is_empty() {
                result["text"] = json!(message)
            }
            if event["is_error"] == true || ty == "turn.failed" {
                result["error"] = json!(if field(&event, "result").is_empty() {
                    "native turn failed"
                } else {
                    field(&event, "result")
                });
            }
            if kind == "codex" && ty == "turn.completed" && event["usage"].is_object() {
                let delta = crate::metrics::turn_usage(&event["usage"], r.get("codex_baseline"));
                if let Some(total) = delta["input_tokens"]
                    .as_u64()
                    .zip(delta["output_tokens"].as_u64())
                    .and_then(|(i, o)| i.checked_add(o))
                {
                    result["tokens"] = total.into();
                }
                result["codex_usage"] = event["usage"].clone();
                emit(json!({"type":ty,"session":result["session"],"turn_usage":delta,"raw":event}));
                continue;
            }
            if ty == "result" && event["usage"].is_object() {
                let mut tokens = 0i64;
                for key in [
                    "input_tokens",
                    "output_tokens",
                    "cache_creation_input_tokens",
                    "cache_read_input_tokens",
                ] {
                    if kind == "claude" || !key.starts_with("cache_") {
                        tokens = tokens
                            .checked_add(event["usage"][key].as_i64().unwrap_or(0))
                            .context("token count overflow")?
                    }
                }
                result["tokens"] = json!(tokens);
            }
            emit(json!({"type":ty,"session":field(&result,"session"),"text":message,"raw":event}));
            continue;
        }
        emit(json!({"type":"stdout","text":String::from_utf8_lossy(&buf).trim_end_matches('\n')}));
    }
    match writer
        .join()
        .map_err(|_| anyhow::anyhow!("agent input writer failed"))?
    {
        Err(e) if e.kind() != io::ErrorKind::BrokenPipe => return Err(e.into()),
        _ => (),
    }
    Ok(result)
}
pub fn decode_outcome(text: &str) -> Result<Value> {
    let text = text.trim();
    let unfenced = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```"))
        .filter(|rest| rest.starts_with('\n') || rest.starts_with("\r\n"))
        .and_then(|rest| rest.trim_start_matches(['\r', '\n']).strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(text);
    Ok(serde_json::from_str(unfenced)?)
}

pub fn decode_typed_outcome(role: &str, text: &str) -> Result<Value> {
    let value = decode_outcome(text)?;
    crate::wire::validate_outcome(&value, role)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_examples::native_fixture;
    #[test]
    fn native_worker_policy_survives_resume_and_honors_effort() {
        for session in ["", "resumed"] {
            for kind in ["codex", "claude"] {
                let mut req = json!({"profile":{"kind":kind,"effort":"high"},"role":"worker","session":session,"subagents":true});
                for limit in [Value::Null, json!(2)] {
                    req["subagent_limit"] = limit.clone();
                    let cli = args(&req);
                    assert!(
                        cli.contains(
                            &if kind == "codex" {
                                r#"model_reasoning_effort="high""#
                            } else {
                                "high"
                            }
                            .to_owned()
                        )
                    );
                    let disabled = if kind == "codex" {
                        "agents.enabled=false"
                    } else {
                        "--disallowedTools"
                    };
                    assert_eq!(cli.contains(&disabled.to_owned()), limit.is_number());
                    let mut off = req.clone();
                    off["subagents"] = json!(false);
                    assert!(args(&off).contains(&disabled.to_owned()));
                }
            }
        }
    }

    #[test]
    fn brief_uses_a_fast_model_with_medium_reasoning_only_for_advice() {
        for (kind, fast, effort) in [
            ("codex", "gpt-6-luna", r#"model_reasoning_effort="medium""#),
            ("claude", "haiku", "medium"),
        ] {
            let profile = json!({"kind":kind,"model":"coordinator-model"});
            let advice = args(&json!({"profile":profile,"role":"advisor"}));
            assert!(advice.windows(2).any(|p| p == ["--model", fast]));
            assert!(advice.contains(&effort.to_owned()));
            let plan = args(&json!({"profile":profile,"role":"plan"}));
            assert!(
                plan.windows(2)
                    .any(|p| p == ["--model", "coordinator-model"])
            );
            assert!(!plan.contains(&effort.to_owned()));
        }
    }
    #[test]
    fn native_adapters_decode_events_and_usage() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let request = json!({"profile":{"kind":"codex","executable":native_fixture(temp.path(),"mock-codex")?},"session":"abc","prompt":"hello\n","dir":temp.path()});
        assert_eq!(
            run(&request, None, |_| {}),
            json!({"text":"done","session":"abc","tokens":10,"codex_usage":{"input_tokens":8,"output_tokens":2}})
        );
        let request = json!({"profile":{"kind":"claude","executable":native_fixture(temp.path(),"mock-claude")?},"dir":temp.path()});
        let result = run(&request, None, |_| {});
        assert_eq!(result["error"], "sign in");
        assert_eq!(result["session"], "abc");
        Ok(())
    }
    #[test]
    fn codex_chat_resumes_or_forks_a_busy_thread_before_streaming_one_reply() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let executable = native_fixture(dir.path(), "mock-codex-chat")?;
        for session in ["", "held-chat", "native-chat"] {
            let events = std::sync::Mutex::new(Vec::new());
            let request = json!({"profile":{"kind":"codex","executable":executable,"model":"configured-model"},"session":session,"role":"chat","prompt":"hello","dir":dir.path(),"attachments":[{"name":"screen.png","path":dir.path().join("screen.png")}]});
            let result = run(&request, None, |event| events.lock().unwrap().push(event));
            assert_eq!(
                result,
                json!({"session":"native-chat","text":r#"{"reply":"Good answer"}"#,"tokens":17})
            );
            assert_eq!(
                fs::read_to_string(dir.path().join("chat-closed"))?,
                "released"
            );
            fs::remove_file(dir.path().join("chat-closed"))?;
            let events = events.into_inner().unwrap();
            assert_eq!(events.len(), 6);
            assert_eq!(events[3]["tokens"], 5);
            assert_eq!(events[4]["tokens"], 17);
            assert_eq!(events[5]["tokens"], 17);
            assert_eq!(events[1]["type"], "agent_message_chunk");
            assert_eq!(events[2]["text"], r#" answer"}"#);
        }
        let request = json!({"profile":{"kind":"codex","executable":executable,"model":"configured-model"},"session":"unavailable-chat","role":"chat","prompt":"hello","dir":dir.path()});
        assert_eq!(
            run(&request, None, |_| {})["error"],
            "Codex chat: thread unavailable"
        );
        Ok(())
    }
    #[test]
    fn acp_loads_existing_session_and_refuses_permissions() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let request = json!({"profile":{"kind":"hermes","executable":native_fixture(temp.path(),"mock-acp")?},"session":"abc","dir":temp.path(),"role":"chat","attachments":[{"name":"scope.md","path":temp.path().join("scope.md")}]});
        assert_eq!(
            run(&request, None, |_| {}),
            json!({"text":"done","session":"abc"})
        );
        Ok(())
    }
    #[test]
    fn desktop_profile_starts_each_role_with_settings_for_its_session() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let profile = json!({"kind":"hermes","hermes_profile":"desktop-work","executable":native_fixture(temp.path(),"mock-desktop-acp")?});
        for role_dir in [
            "coordinator",
            "worker",
            "advisor",
            "advisor-options",
            "worker-options",
        ] {
            let role = role_dir.strip_suffix("-options").unwrap_or(role_dir);
            let dir = temp.path().join(role_dir);
            fs::create_dir(&dir)?;
            let mut profile = profile.clone();
            if role_dir == "worker-options" {
                profile["model"] = json!("service:gpt-6-luna");
                profile["effort"] = json!("medium");
            }
            let request =
                json!({"profile":profile,"role":role,"dir":dir.canonicalize()?,"prompt":"fixture"});
            let result = run(&request, None, |_| {});
            assert_eq!(result["session"], "desktop-session");
            if role == "worker" {
                let receipt = decode_typed_outcome(role, field(&result, "text"))?;
                assert_eq!(receipt["commit"], "abc");
                assert_eq!(receipt["message"], "Checks passed");
            } else {
                assert_eq!(result["text"], "done");
            }
        }
        Ok(())
    }
    #[test]
    fn availability_keeps_account_information_private() -> Result<()> {
        let temp = tempfile::tempdir()?;
        for (kind, name, state) in [
            ("codex", "mock-logged-in", "connected"),
            ("codex", "mock-logged-out", "sign-in"),
            ("claude", "mock-claude-connected", "connected"),
            ("claude", "mock-unknown", "sign-in-unknown"),
            ("hermes", "mock-installed", "installed"),
        ] {
            let profile = json!({"kind":kind,"executable":native_fixture(temp.path(),name)?});
            assert_eq!(availability(&profile, None), json!({"state":state}));
        }
        Ok(())
    }
    #[test]
    fn hung_login_probe_releases_setup() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let profile =
            json!({"kind":"codex","executable":native_fixture(temp.path(), "mock-hanging-login")?});
        let started = std::time::Instant::now();
        assert_eq!(availability(&profile, None)["state"], "sign-in-unknown");
        assert!(started.elapsed() < Duration::from_secs(10));
        Ok(())
    }
    #[test]
    fn decode_single_fenced_json_document() {
        assert_eq!(
            decode_outcome("```json\n{\"known\":true}\n```").unwrap()["known"],
            true
        );
        assert!(decode_outcome("{} {}").is_err());
        assert!(decode_typed_outcome("plan", r#"{"tasks":[{"slug":"typo"}]}"#).is_err());
        assert!(decode_typed_outcome("worker", r#"{"status":"committed","commit":42}"#).is_err());
        assert!(
            decode_typed_outcome(
                "observer",
                r#"{"actions":[{"kind":"ask","ref":"abc","question":{"text":"Which?"}}]}"#
            )
            .is_ok()
        );
    }
}
