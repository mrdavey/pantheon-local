//! Deterministic local agent for integration and browser tests.
mod observer;
mod plan;
mod probes;
mod worker;

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    env, fs,
    io::{self, BufRead, BufReader, Write},
    net::TcpListener,
    path::Path,
    process::Command,
};
fn rows(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}
fn text<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v[k].as_str().with_context(|| format!("missing {k}"))
}
fn decision<'a>(ctx: &'a Value, question: &str) -> Option<&'a str> {
    rows(&ctx["planning_decisions"])
        .iter()
        .find(|d| d["question"] == question)?
        .get("answer")?
        .as_str()
}
fn refs(prompt: &str, prefix: &str) -> Vec<String> {
    prompt
        .match_indices(prefix)
        .filter_map(|(start, _)| {
            let tail = &prompt[start + prefix.len()..];
            let suffix = tail
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .next()?;
            (!suffix.is_empty()).then(|| format!("{prefix}{suffix}"))
        })
        .collect()
}
fn task(id: &str, goal: &str, worker: &str, extra: Value) -> Value {
    let mut t = json!({"id":id,"goal":goal,"worker":worker});
    if let Some(extra) = extra.as_object() {
        for (k, v) in extra {
            t[k] = v.clone()
        }
    }
    t
}
fn check(pairs: &[&str]) -> Result<Value> {
    let mut command = vec![json!(env::current_exe()?), json!("check")];
    command.extend(pairs.iter().map(|s| json!(s)));
    Ok(json!(command))
}
fn preview_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
}
fn preview_response(path: &str) -> Result<(&'static str, &'static str, Vec<u8>)> {
    if path == "/" {
        let mut entries = Vec::new();
        for entry in fs::read_dir(".")? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if preview_name(&name) && entry.file_type()?.is_file() {
                entries.push(format!(r#"<a href="/{name}">{name}</a>"#));
            }
        }
        entries.sort();
        return Ok(("200 OK", "text/html", entries.join("<br>").into_bytes()));
    }
    if let Some(name) = path.strip_prefix('/')
        && preview_name(name)
        && fs::symlink_metadata(name).is_ok_and(|m| m.is_file())
    {
        return Ok(("200 OK", "text/plain", fs::read(name)?));
    }
    Ok(("404 Not Found", "text/plain", Vec::new()))
}
fn preview() -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", env::var("PORT")?.parse::<u16>()?))?;
    for socket in listener.incoming() {
        let mut socket = socket?;
        let mut request = String::new();
        if BufReader::new(&socket).read_line(&mut request)? == 0 {
            continue;
        }
        let route = request.split_whitespace().nth(1).unwrap_or("");
        let (status, mime, body) = preview_response(route)?;
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = socket
            .write_all(response.as_bytes())
            .and_then(|_| socket.write_all(&body));
    }
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let name = Path::new(&args[0])
        .file_name()
        .and_then(|p| p.to_str())
        .unwrap_or("");
    if matches!(name, "open" | "xdg-open") {
        let mut output = fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(env::var_os("SWARM_TEST_OPENED").context("missing opener log")?)?;
        writeln!(output, "{}", args.get(1).context("missing browser URL")?)?;
        return Ok(());
    }
    if name == "fake-codex" {
        if env::var_os("SWARM_TEST_SIGNED_IN").is_some_and(|p| Path::new(&p).is_file()) {
            println!("Logged in using ChatGPT");
            return Ok(());
        }
        eprintln!("Not logged in");
        std::process::exit(1)
    }
    if name.starts_with("mock-") {
        return probes::run(name, &args[1..]);
    }
    match args.get(1).map(String::as_str) {
        Some("--version") => {
            println!("fixture-agent 1");
            return Ok(());
        }
        Some("check") => {
            let (pairs, remainder) = args[2..].as_chunks::<2>();
            ensure!(remainder.is_empty(), "check expects file/value pairs");
            for [file, expected] in pairs {
                ensure!(
                    fs::read_to_string(file)? == *expected,
                    "incorrect content in {}",
                    file
                );
            }
            return Ok(());
        }
        Some("check-output") if args.len() == 4 => {
            let out = Command::new(&args[2]).output()?;
            ensure!(
                out.status.success(),
                "program failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            ensure!(
                String::from_utf8(out.stdout)?.replace("\r\n", "\n")
                    == args[3].replace("\r\n", "\n"),
                "unexpected stdout"
            );
            return Ok(());
        }
        Some("serve-preview") => return preview(),
        Some("turn-v1") => {}
        _ => bail!("expected turn-v1 or a fixture check"),
    }
    let r: Value = serde_json::from_reader(io::stdin().lock())?;
    let prompt = text(&r, "prompt")?;
    let role = text(&r, "role")?;
    let ctx: Value = match prompt
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("Run context: "))
    {
        Some(data) => serde_json::from_str(data)?,
        None => json!({}),
    };
    if role == "advisor" {
        println!(
            "{}",
            json!({"result":{"text":"Keep quotes private and check that changes survive a restart."}})
        );
        return Ok(());
    }
    if role == "coordinator" && prompt.contains("Measure tariff lookup") {
        println!(
            "{}",
            json!({"event":{"type":"usage","model":"Fixture Tariff","input_tokens":4,"cached_input_tokens":1,"output_tokens":1,"tokens":5}})
        );
    }
    let response = match role {
        "pricing" => {
            json!({"rates":{"Fixture Tariff":{"input":2,"cached":1,"output":5,"source":"https://pricing.example/fixture"}}})
        }
        "observer" => observer::respond(prompt, &ctx)?,
        "chat" if prompt.ends_with("Stream this answer") => {
            let gate = env::var("SWARM_TEST_CHAT_GATE")?;
            println!(
                "{}",
                json!({"event":{"type":"usage","model":"Fixture Chat","tokens":2,"cost_usd":0.002}})
            );
            let fragment = r#"{"reply":"The live answer"#;
            println!(
                "{}",
                json!({"event":{"type":"agent_message_chunk","text":fragment}})
            );
            io::stdout().flush()?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !Path::new(&gate).is_file() {
                ensure!(
                    std::time::Instant::now() < deadline,
                    "browser did not receive the reply fragment"
                );
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            println!(
                "{}",
                json!({"event":{"type":"agent_message_chunk","text":" is complete.\"}"}})
            );
            json!({"reply":"The live answer is complete."})
        }
        "chat" if prompt.ends_with("Explain this milestone.") => {
            ensure!(
                prompt.contains("Selected milestone evidence")
                    && prompt.contains("Write the `open quotes` export."),
                "missing selected milestone"
            );
            json!({"reply":"This milestone writes the open quotes export."})
        }
        "chat" if prompt.ends_with("Explain this implementation.") => {
            ensure!(
                prompt.contains("Selected implementation evidence")
                    && prompt.contains("diff --git")
                    && prompt.contains("export.txt"),
                "missing implementation diff"
            );
            json!({"reply":"This implementation adds the checked export file."})
        }
        "chat" if prompt.ends_with("Read the attachment") => {
            let attachment = &r["attachments"][0];
            ensure!(
                attachment["name"] == "work-notes.txt"
                    && prompt.contains("Measure receipt latency")
            );
            ensure!(
                std::fs::read_to_string(attachment["path"].as_str().context("file path")?)?
                    == "Measure receipt latency"
            );
            json!({"reply":"The notes ask to measure receipt latency."})
        }
        "chat" if prompt.ends_with("Add a timing milestone") => {
            json!({"reply":"We can record timings after the export is ready.",
            "tasks":[task("timing","Record export timing.","writer",json!({"depends":["export"],"class":"testing"}))],
            "checks":[check(&["timing.txt","measured"])?]})
        }
        "chat" => json!({"reply":"The export milestone owns the generated CSV."}),
        "coordinator" => plan::respond(prompt, &ctx)?,
        "worker" => worker::respond(prompt, &ctx)?,
        _ => bail!("unknown fixture role {role}"),
    };
    let session = r["session"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("fixture-{role}"));
    println!("{}", json!({"event":{"type":"session","session":session}}));
    println!(
        "{}",
        json!({"result":{"session":session,"text":response.to_string(),"tokens":5}})
    );
    Ok(())
}
