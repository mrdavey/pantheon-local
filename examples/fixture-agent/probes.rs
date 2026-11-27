//! Native CLI wire-protocol fixtures selected by the executable's symlink name.
use super::*;
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
fn next(reader: &mut impl BufRead) -> Result<Value> {
    let mut s = String::new();
    ensure!(
        reader.read_line(&mut s)? > 0,
        "unexpected end of ACP stream"
    );
    Ok(serde_json::from_str(&s)?)
}
pub(super) fn run(name: &str, args: &[String]) -> Result<()> {
    let mut input = BufReader::new(io::stdin().lock());
    match name {
        "mock-codex" => {
            ensure!(
                args == [
                    "-c",
                    "agents.enabled=false",
                    "-c",
                    "features.multi_agent_v2=false",
                    "exec",
                    "resume",
                    "--json",
                    "abc",
                    "-"
                ],
                "unexpected Codex arguments"
            );
            let mut prompt = String::new();
            input.read_line(&mut prompt)?;
            ensure!(prompt.trim_end() == "hello", "unexpected Codex prompt");
            for event in [
                json!({"type":"thread.started","thread_id":"abc"}),
                json!({"type":"item.completed","item":{"type":"agent_message","text":"done"}}),
                json!({"type":"turn.completed","usage":{"input_tokens":8,"output_tokens":2}}),
            ] {
                println!("{event}")
            }
        }
        "mock-codex-chat" => {
            ensure!(
                args == [
                    "-c",
                    "agents.enabled=false",
                    "-c",
                    "features.multi_agent_v2=false",
                    "app-server"
                ],
                "chat needs the streaming transport"
            );
            let init = next(&mut input)?;
            ensure!(init["method"] == "initialize", "missing initialize");
            println!("{}", json!({"id":init["id"],"result":{}}));
            ensure!(
                next(&mut input)?["method"] == "initialized",
                "missing initialized"
            );
            let mut session = next(&mut input)?;
            ensure!(
                matches!(
                    session["method"].as_str(),
                    Some("thread/start" | "thread/resume")
                ),
                "missing thread"
            );
            ensure!(
                session["params"]["sandbox"] == "read-only"
                    && session["params"]["approvalPolicy"] == "never",
                "wrong chat permissions"
            );
            ensure!(
                session["params"]["model"] == "configured-model",
                "wrong model"
            );
            if session["method"] == "thread/resume" {
                match session["params"]["threadId"].as_str() {
                    Some("held-chat") => {
                        println!(
                            "{}",
                            json!({"id":session["id"],"error":{"code":-32600,"message":"thread held-chat already has an active writer"}})
                        );
                        let fork = next(&mut input)?;
                        ensure!(
                            fork["method"] == "thread/fork" && fork["params"] == session["params"],
                            "fork must preserve the thread, model and permissions"
                        );
                        session = fork;
                    }
                    Some("unavailable-chat") => {
                        println!(
                            "{}",
                            json!({"id":session["id"],"error":{"code":-32600,"message":"thread unavailable"}})
                        );
                        ensure!(
                            input.read_line(&mut String::new())? == 0,
                            "unexpected retry"
                        );
                        return Ok(());
                    }
                    Some("native-chat") => (),
                    _ => anyhow::bail!("wrong resumed session"),
                }
            }
            println!(
                "{}",
                json!({"id":session["id"],"result":{"thread":{"id":"native-chat"}}})
            );
            let start = next(&mut input)?;
            ensure!(
                start["method"] == "turn/start"
                    && start["params"]["threadId"] == "native-chat"
                    && start["params"]["input"][0]["text"] == "hello"
                    && start["params"]["input"][1]["type"] == "localImage"
                    && start["params"]["input"][1]["path"]
                        .as_str()
                        .is_some_and(|s| s.ends_with("/screen.png")),
                "missing prompt"
            );
            println!("{}", json!({"id":start["id"],"result":{"turn":{"id":"t"}}}));
            let note = |method, params| println!("{}", json!({"method":method,"params":params}));
            note(
                "item/reasoning/textDelta",
                json!({"threadId":"native-chat","turnId":"t","delta":"private"}),
            );
            for delta in [r#"{"reply":"Good"#, r#" answer"}"#] {
                note(
                    "item/agentMessage/delta",
                    json!({"threadId":"native-chat","turnId":"t","itemId":"m","delta":delta}),
                );
            }
            note(
                "item/completed",
                json!({"threadId":"native-chat","turnId":"t","item":{"type":"agentMessage","id":"m","text":r#"{"reply":"Good answer"}"#}}),
            );
            for (last, total) in [(5, 105), (12, 117), (12, 117)] {
                note(
                    "thread/tokenUsage/updated",
                    json!({"threadId":"native-chat","turnId":"t","tokenUsage":{"last":{"totalTokens":last},"total":{"totalTokens":total}}}),
                );
            }
            note(
                "turn/completed",
                json!({"threadId":"native-chat","turn":{"id":"t","status":"completed"}}),
            );
            ensure!(
                input.read_line(&mut String::new())? == 0,
                "duplicate chat turn"
            );
            fs::write("chat-closed", "released")?;
        }
        "mock-claude" => {
            println!("{}", json!({"type":"system","session_id":"abc"}));
            println!(
                "{}",
                json!({"type":"result","is_error":true,"result":"sign in"})
            );
        }
        "mock-antigravity-acp" => {
            ensure!(args == ["acp"], "expected Antigravity ACP mode");
            let init = next(&mut input)?;
            ensure!(init["method"] == "initialize");
            println!(
                "{}",
                json!({"id":init["id"],"result":{"agentCapabilities":{"loadSession":false}}})
            );
            let session = next(&mut input)?;
            ensure!(
                session["method"] == "session/resume"
                    && session["params"]["sessionId"] == "agy-session"
            );
            println!(
                "{}",
                json!({"id":session["id"],"result":{"models":{"currentModelId":"gemini-fixture"}}})
            );
            let prompt = next(&mut input)?;
            ensure!(prompt["method"] == "session/prompt");
            for update in [
                json!({"sessionUpdate":"usage_update","used":200000,"size":1000000}),
                json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"done"}}),
            ] {
                println!(
                    "{}",
                    json!({"method":"session/update","params":{"sessionId":"agy-session","update":update}})
                );
            }
            println!(
                "{}",
                json!({"id":prompt["id"],"result":{"stopReason":"end_turn","usage":{
                    "totalTokens":170,"inputTokens":100,"outputTokens":20,"cachedReadTokens":50,"thoughtTokens":5
                }}})
            );
        }
        "mock-acp" | "mock-desktop-acp" => {
            let fresh = name == "mock-desktop-acp";
            if fresh {
                ensure!(
                    args == ["--profile", "desktop-work", "acp"],
                    "wrong Desktop profile"
                );
            }

            ensure!(
                next(&mut input)?["method"] == "initialize",
                "expected ACP initialize"
            );
            println!(
                "{}",
                json!({"id":1,"result":{"agentCapabilities":{"loadSession":true}}})
            );
            let request = next(&mut input)?;
            if fresh {
                ensure!(
                    request["method"] == "session/new"
                        && request["params"]["cwd"] == json!(std::env::current_dir()?),
                    "wrong new session workspace"
                );
                let cwd = std::env::current_dir()?;
                let mut session = json!({
                    "sessionId": "desktop-session",
                    "modes": {"currentModeId":"default","availableModes":[{"id":"accept_edits"}]},
                    "models": {
                        "currentModelId":"service:gpt-6-astra",
                        "availableModels":[{"modelId":"other:gpt-6-luna"},{"modelId":"service:gpt-6-luna"}]
                    },
                    "configOptions":[{"id":"thinking","category":"thought_level","type":"select","currentValue":"high","options":[{"value":"high"},{"value":"medium"}]}]
                });
                if cwd.ends_with("advisor-options") || cwd.ends_with("worker-options") {
                    session["configOptions"] = json!([{
                        "id":"model","category":"model","type":"select","currentValue":"service:gpt-6-astra",
                        "options":[{"group":"Service","options":[{"value":"service:gpt-6-luna"}]}]
                    }]);
                }
                println!("{}", json!({"id":2,"result":session}));
            } else {
                ensure!(
                    request["method"] == "session/load" && request["params"]["sessionId"] == "abc",
                    "expected ACP session load"
                );
                println!("{}", json!({"id":2,"result":{}}));
            }
            if fresh && std::env::current_dir()?.ends_with("worker") {
                let mode = next(&mut input)?;
                ensure!(
                    mode["method"] == "session/set_mode"
                        && mode["params"]["modeId"] == "accept_edits",
                    "missing worker mode"
                );
                println!("{}", json!({"id":mode["id"],"result":{}}));
            }
            let cwd = std::env::current_dir()?;
            if fresh
                && (cwd.ends_with("advisor")
                    || cwd.ends_with("advisor-options")
                    || cwd.ends_with("worker-options"))
            {
                let model = next(&mut input)?;
                if cwd.ends_with("advisor-options") || cwd.ends_with("worker-options") {
                    ensure!(
                        model["method"] == "session/set_config_option"
                            && model["params"]["configId"] == "model"
                            && model["params"]["value"] == "service:gpt-6-luna",
                        "expected advertised model option"
                    );
                    println!(
                        "{}",
                        json!({"id":model["id"], "result":{"configOptions":[{"id":"thinking","category":"thought_level","type":"select","currentValue":"low","options":[{"value":"medium"}]}]}})
                    );
                } else {
                    ensure!(
                        model["method"] == "session/set_model"
                            && model["params"]["modelId"] == "service:gpt-6-luna",
                        "expected advertised Hermes model"
                    );
                    println!("{}", json!({"id":model["id"],"result":{}}));
                }
                let effort = next(&mut input)?;
                ensure!(
                    effort["method"] == "session/set_config_option"
                        && effort["params"]["value"] == "medium"
                        && effort["params"]["configId"] == "thinking",
                    "expected advertised medium reasoning"
                );
                println!(
                    "{}",
                    json!({"id":effort["id"],"result":{"configOptions":[{"id":"edit-mode","category":"mode","type":"select","currentValue":"default","options":[{"value":"accept_edits"}]}]}})
                );
            }
            if fresh && cwd.ends_with("worker-options") {
                let mode = next(&mut input)?;
                ensure!(
                    mode["method"] == "session/set_config_option"
                        && mode["params"]["configId"] == "edit-mode"
                        && mode["params"]["value"] == "accept_edits",
                    "expected advertised ACP mode"
                );
                println!("{}", json!({"id":mode["id"],"result":{}}));
            }
            let prompt = next(&mut input)?;
            ensure!(prompt["method"] == "session/prompt", "expected ACP prompt");
            if name == "mock-acp" {
                let link = &prompt["params"]["prompt"][1];
                ensure!(
                    link["type"] == "resource_link"
                        && link["name"] == "scope.md"
                        && link["uri"]
                            .as_str()
                            .is_some_and(|s| s.starts_with("file:") && s.ends_with("/scope.md"))
                );
            }
            let options = if fresh {
                json!([{"kind":"allow_once","optionId":"yes"},{"kind":"reject_once","optionId":"no"}])
            } else {
                json!([])
            };
            println!(
                "{}",
                json!({"id":9,"method":"session/request_permission","params":{"options":options}})
            );
            let reply = next(&mut input)?;
            let expected = if fresh {
                json!({"outcome":"selected","optionId":"no"})
            } else {
                json!({"outcome":"cancelled"})
            };
            ensure!(
                reply["id"] == 9 && reply["result"]["outcome"] == expected,
                "incorrect permission response"
            );
            let chunks = if fresh && (cwd.ends_with("worker") || cwd.ends_with("worker-options")) {
                [
                    ("progress", "Checking the project."),
                    ("final", "Finished the checks.\n\n"),
                    (
                        "final",
                        r#"{"status":"committed","commit":"abc","message":"Checks passed"}"#,
                    ),
                ]
            } else {
                [
                    ("progress", "Checking the project."),
                    ("final", "do"),
                    ("final", "ne"),
                ]
            };
            for (id, text) in chunks {
                println!(
                    "{}",
                    json!({"method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","messageId":id,"content":{"text":text}}}})
                );
            }
            println!(
                "{}",
                json!({"id":prompt["id"],"result":{"stopReason":"end_turn"}})
            );
        }
        "mock-logged-in" => println!("Logged in with ChatGPT"),
        "mock-logged-out" => {
            eprintln!("Not logged in");
            std::process::exit(1)
        }
        "mock-claude-connected" => println!(
            "{}",
            json!({"loggedIn":true,"email":"private@example.invalid"})
        ),
        "mock-unknown" => println!("{{}}"),
        "mock-hanging-login" => std::thread::sleep(Duration::from_secs(30)),
        "mock-installed" => bail!("unused command"),
        "mock-resistant" => {
            signal_hook::flag::register(
                signal_hook::consts::SIGTERM,
                Arc::new(AtomicBool::new(false)),
            )?;
            fs::write("worker.pid", std::process::id().to_string())?;
            std::thread::sleep(Duration::from_secs(30));
        }
        "mock-sandbox" => {
            fs::write("inside", "ok")?;
            fs::write(
                Path::new(&env::var_os("SWARM_DATA_DIR").context("missing data directory")?)
                    .join("inside"),
                "ok",
            )?;
            ensure!(
                fs::read("../token.json").is_err() && fs::write("../outside", "bad").is_err(),
                "sandbox allowed an external file"
            );
        }
        _ => bail!("unknown native fixture {name}"),
    }
    Ok(())
}
