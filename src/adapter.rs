mod acp;
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    env, fs,
    io::{self, BufRead, BufReader, Read, Seek, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    thread,
    time::Duration,
};
use wait_timeout::ChildExt;

fn field<'a>(obj: &'a Value, key: &str) -> &'a str {
    obj[key].as_str().unwrap_or_default()
}
pub fn binary(p: &Value) -> &str {
    match field(p, "executable") {
        "" => match field(p, "kind") {
            "antigravity" => "agy",
            k => k,
        },
        s => s,
    }
}
pub fn validate(p: &Value) -> Result<()> {
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
fn inspect(p: &Value, args: &[&str], timeout: u64, combined: bool) -> Result<(bool, String)> {
    let mut capture = tempfile::tempfile()?;
    let mut cmd = Command::new(binary(p));
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(capture.try_clone()?);
    if combined {
        cmd.stderr(capture.try_clone()?);
    } else {
        cmd.stderr(Stdio::null());
    }
    let mut child = cmd.spawn()?;
    let status = match child.wait_timeout(Duration::from_secs(timeout))? {
        Some(s) => s,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            bail!("agent status probe timed out")
        }
    };
    capture.rewind()?;
    let mut out = Vec::new();
    capture.take(64 * 1024).read_to_end(&mut out)?;
    Ok((
        status.success(),
        String::from_utf8_lossy(&out).trim().into(),
    ))
}
pub fn probe(p: &Value) -> Result<String> {
    validate(p)?;
    let (ok, version) = inspect(p, &["--version"], 5, true)?;
    ensure!(ok, "{} version probe failed", binary(p));
    Ok(version)
}
pub fn availability(p: &Value) -> Value {
    let state = if validate(p).is_err() {
        "invalid"
    } else if !installed(binary(p)) {
        "missing"
    } else {
        match field(p, "kind") {
            "codex" => match inspect(p, &["login", "status"], 2, true) {
                Ok((_, s)) if s.to_lowercase().starts_with("not logged in") => "sign-in",
                Ok((true, s)) if s.to_lowercase().starts_with("logged in") => "connected",
                _ => "sign-in-unknown",
            },
            "claude" => match inspect(p, &["auth", "status", "--json"], 2, false)
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
    let mut args = Vec::new();
    match kind {
        "hermes" | "antigravity" => {
            let name = field(profile, "hermes_profile");
            if kind == "hermes" && !name.is_empty() {
                args.extend(["--profile", name])
            }
            args.push("acp")
        }
        "custom" => args.push("turn-v1"),
        "codex" => {
            args.push("exec");
            if !session.is_empty() {
                args.push("resume")
            }
            args.push("--json");
            if matches!(role, "advisor" | "chat" | "observer") {
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
            if !model.is_empty() {
                args.extend(["--model", model])
            }
            if !session.is_empty() {
                args.push(session)
            }
            args.push("-")
        }
        _ => {
            args.extend(["-p", "--verbose", "--output-format", "stream-json"]);
            if role != "worker" {
                args.extend(["--tools", ""])
            }
            if !model.is_empty() {
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
        let _ = self.0.kill();
        let _ = self.0.wait();
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
    let mut cmd = Command::new(binary(p));
    cmd.args(args(r))
        .current_dir(field(r, "dir"))
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
    let child = &mut proc.0;
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
        let result = if acp {
            acp::exchange(r, stdin, stdout, emit)
        } else {
            stream(r, stdin, stdout, emit)
        };
        if acp || result.is_err() {
            let _ = child.kill();
        }
        let status = child.wait()?;
        let result = result?;
        ensure!(
            acp || status.success() || !field(&result, "error").is_empty(),
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
            if matches!(ty, "turn.completed" | "result") && event["usage"].is_object() {
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
    let text = text
        .strip_prefix("```json\n")
        .and_then(|v| v.strip_suffix("\n```"))
        .unwrap_or(text);
    Ok(serde_json::from_str(text)?)
}

pub fn decode_typed_outcome(role: &str, text: &str) -> Result<Value> {
    let value = decode_outcome(text)?;
    crate::wire::validate_outcome(&value, role)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn script(dir: &Path, body: &str) -> Result<String> {
        let file = dir.join("agent");
        fs::write(&file, format!("#!/bin/sh\n{body}\n"))?;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o700))?;
        Ok(file.to_string_lossy().into())
    }
    #[test]
    fn native_adapters_decode_events_and_usage() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let body = r#"test "$1 $2 $3 $4 $5" = "exec resume --json abc -" || exit 8
read -r prompt
test "$prompt" = hello || exit 9
echo '{"type":"thread.started","thread_id":"abc"}'
echo '{"type":"item.completed","item":{"type":"agent_message","text":"done"}}'
echo '{"type":"turn.completed","usage":{"input_tokens":8,"output_tokens":2}}'"#;
        let request = json!({"profile":{"kind":"codex","executable":script(temp.path(),body)?},"session":"abc","prompt":"hello\n","dir":temp.path()});
        let result = run(&request, None, |_| {});
        assert_eq!(result, json!({"text":"done","session":"abc","tokens":10}));
        let body = r#"echo '{"type":"system","session_id":"abc"}'
echo '{"type":"result","is_error":true,"result":"sign in"}'"#;
        let result = run(
            &json!({"profile":{"kind":"claude","executable":script(temp.path(),body)?},"dir":temp.path()}),
            None,
            |_| {},
        );
        assert_eq!(result["error"], "sign in");
        assert_eq!(result["session"], "abc");
        Ok(())
    }
    #[test]
    fn acp_loads_existing_session_and_refuses_permissions() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let body = r#"read -r init
echo '{"id":1,"result":{"agentCapabilities":{"loadSession":true}}}'
read -r load
case "$load" in *session/load*abc*) ;; *) exit 10;; esac
echo '{"id":2,"result":{}}'
read -r prompt
echo '{"id":9,"method":"session/request_permission","params":{}}'
read -r reply
case "$reply" in *cancelled*) ;; *) exit 11;; esac
echo '{"method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"text":"done"}}}}'
echo '{"id":3,"result":{"stopReason":"end_turn"}}'"#;
        let result = run(
            &json!({"profile":{"kind":"hermes","executable":script(temp.path(),body)?},"session":"abc","dir":temp.path()}),
            None,
            |_| {},
        );
        assert_eq!(result, json!({"text":"done","session":"abc"}));
        Ok(())
    }
    #[test]
    fn availability_keeps_account_information_private() -> Result<()> {
        let temp = tempfile::tempdir()?;
        for (kind, cmd, state) in [
            ("codex", "echo 'Logged in with ChatGPT'", "connected"),
            ("codex", "echo 'Not logged in' >&2; exit 1", "sign-in"),
            (
                "claude",
                "echo '{\"loggedIn\":true,\"email\":\"private@example.com\"}'",
                "connected",
            ),
            ("claude", "echo '{}'", "sign-in-unknown"),
            ("hermes", "exit 4", "installed"),
        ] {
            let profile = json!({"kind":kind,"executable":script(temp.path(),cmd)?});
            assert_eq!(availability(&profile), json!({"state":state}));
        }
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
