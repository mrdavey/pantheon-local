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
                args == ["exec", "resume", "--json", "abc", "-"],
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
        "mock-claude" => {
            println!("{}", json!({"type":"system","session_id":"abc"}));
            println!(
                "{}",
                json!({"type":"result","is_error":true,"result":"sign in"})
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
                println!(
                    "{}",
                    json!({"id":2,"result":{"sessionId":"desktop-session","modes":{"currentModeId":"default","availableModes":[{"id":"accept_edits"}]}}})
                );
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
            let prompt = next(&mut input)?;
            ensure!(prompt["method"] == "session/prompt", "expected ACP prompt");
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
            let chunks = if fresh && std::env::current_dir()?.ends_with("worker") {
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
