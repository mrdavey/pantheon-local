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
                    json!({"id":2,"result":{"sessionId":"desktop-session"}})
                );
            } else {
                ensure!(
                    request["method"] == "session/load" && request["params"]["sessionId"] == "abc",
                    "expected ACP session load"
                );
                println!("{}", json!({"id":2,"result":{}}));
            }
            ensure!(
                next(&mut input)?["method"] == "session/prompt",
                "expected ACP prompt"
            );
            println!(
                "{}",
                json!({"id":9,"method":"session/request_permission","params":{}})
            );
            let reply = next(&mut input)?;
            ensure!(
                reply["id"] == 9 && reply["result"]["outcome"]["outcome"] == "cancelled",
                "permission granted"
            );
            println!(
                "{}",
                json!({"method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"text":"done"}}}})
            );
            println!("{}", json!({"id":3,"result":{"stopReason":"end_turn"}}));
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
