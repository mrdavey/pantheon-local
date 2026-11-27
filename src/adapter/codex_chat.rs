//! Codex exec reports completed messages. The native app-server also emits text deltas.
use super::*;

fn send(input: &mut ChildStdin, request: Value) -> Result<()> {
    serde_json::to_writer(&mut *input, &request)?;
    input.write_all(b"\n")?;
    input.flush()?;
    Ok(())
}

fn thread_settings(request: &Value) -> Value {
    let mut settings = json!({"cwd":request["dir"],"sandbox":"read-only","approvalPolicy":"never"});
    let model = field(&request["profile"], "model");
    if !model.is_empty() {
        settings["model"] = model.into();
    }
    let previous = field(request, "session");
    if !previous.is_empty() {
        settings["threadId"] = previous.into();
    }
    settings
}

pub(super) fn exchange(
    request: &Value,
    mut input: ChildStdin,
    output: ChildStdout,
    emit: &impl Fn(Value),
) -> Result<Value> {
    send(
        &mut input,
        json!({"id":1,"method":"initialize","params":{
            "clientInfo":{"name":"pantheon_local","version":env!("CARGO_PKG_VERSION")},
            "capabilities":{}
        }}),
    )?;
    let mut output = BufReader::new(output);
    let mut bytes = Vec::new();
    let mut session = String::new();
    let mut turn = String::new();
    let mut item = String::new();
    let mut answer = String::new();
    let mut tokens = None;
    let mut baseline = None;
    let mut model = Value::Null;
    while line(&mut output, &mut bytes)? {
        let event: Value = serde_json::from_slice(&bytes)?;
        let params = &event["params"];
        if let Some(method) = event["method"].as_str() {
            if !event["id"].is_null() {
                send(
                    &mut input,
                    json!({"id":event["id"],"error":{"code":-32601,"message":"Host capability unavailable"}}),
                )?;
                continue;
            }
            if session.is_empty()
                || params["threadId"] != session
                || (!turn.is_empty() && params["turnId"].is_string() && params["turnId"] != turn)
            {
                continue;
            }
            match method {
                "item/agentMessage/delta" => {
                    let id = field(params, "itemId");
                    if id != item {
                        item = id.into();
                        answer.clear();
                    }
                    let delta = field(params, "delta");
                    ensure!(
                        answer.len().saturating_add(delta.len()) <= MAX_LINE,
                        "Codex chat message exceeds 8 MiB"
                    );
                    answer.push_str(delta);
                    emit(
                        json!({"type":"agent_message_chunk","messageId":id,"session":session,"text":delta}),
                    );
                }
                "item/completed" if params["item"]["type"] == "agentMessage" => {
                    answer = field(&params["item"], "text").into();
                }
                "item/completed" if params["item"]["type"] == "reasoning" => {
                    let summary = params["item"]["summary"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !summary.is_empty() {
                        emit(json!({"type":"reasoning_summary","text":summary,"session":session}));
                    }
                }
                "item/started" | "item/completed"
                    if matches!(
                        params["item"]["type"].as_str(),
                        Some("commandExecution" | "fileChange" | "mcpToolCall" | "webSearch")
                    ) =>
                {
                    emit(json!({"type":method,"session":session,"raw":params}));
                }
                "thread/tokenUsage/updated" => {
                    let usage = &params["tokenUsage"];
                    let last = &usage["last"];
                    let total = &usage["total"];
                    // A resumed thread includes earlier turns. Subtract the counters
                    // preceding this turn's first reported model call.
                    let keys = [
                        "totalTokens",
                        "inputTokens",
                        "cachedInputTokens",
                        "outputTokens",
                        "reasoningOutputTokens",
                    ];
                    let before = baseline.get_or_insert_with(|| {
                        keys.map(|key| total[key].as_u64()?.checked_sub(last[key].as_u64()?))
                    });
                    let delta = |i: usize| {
                        let key = keys[i];
                        before[i]
                            .and_then(|b| total[key].as_u64()?.checked_sub(b))
                            .or_else(|| last[key].as_u64())
                    };
                    tokens = delta(0);
                    emit(
                        json!({"type":"usage","model":model,"tokens":tokens,"input_tokens":delta(1),
                        "cached_input_tokens":delta(2),"output_tokens":delta(3),"reasoning_output_tokens":delta(4)}),
                    );
                }
                "turn/completed" if turn.is_empty() || params["turn"]["id"] == turn => {
                    let completed = &params["turn"];
                    if completed["status"] != "completed" {
                        let detail = completed["error"]["message"]
                            .as_str()
                            .unwrap_or("Codex chat was interrupted");
                        return Ok(json!({"error":detail,"session":session}));
                    }
                    let mut result = json!({"text":answer,"session":session});
                    if let Some(tokens) = tokens {
                        result["tokens"] = tokens.into();
                    }
                    return Ok(result);
                }
                _ => (),
            }
            continue;
        }
        let id = event["id"].as_u64();
        if let Some(error) = event["error"].as_object() {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("request failed");
            let previous = field(request, "session");
            // Only a rejected resume is safe to recover: no turn was submitted.
            // A fork retains history and leaves the existing writer alone.
            if id == Some(2)
                && !previous.is_empty()
                && message == format!("thread {previous} already has an active writer")
            {
                send(
                    &mut input,
                    json!({"id":4,"method":"thread/fork","params":thread_settings(request)}),
                )?;
                continue;
            }
            bail!("Codex chat: {message}");
        }
        match id {
            Some(1) => {
                send(&mut input, json!({"method":"initialized"}))?;
                let method = if field(request, "session").is_empty() {
                    "thread/start"
                } else {
                    "thread/resume"
                };
                send(
                    &mut input,
                    json!({"id":2,"method":method,"params":thread_settings(request)}),
                )?;
            }
            Some(2 | 4) => {
                model = event["result"]["model"].clone();
                session = event["result"]["thread"]["id"]
                    .as_str()
                    .context("Codex returned no chat session")?
                    .into();
                emit(json!({"type":"session","session":session}));
                let mut content = vec![json!({"type":"text","text":request["prompt"]})];
                for file in request["attachments"].as_array().into_iter().flatten() {
                    let name = field(file, "name").to_lowercase();
                    if [".png", ".jpg", ".jpeg", ".webp", ".gif"]
                        .iter()
                        .any(|ext| name.ends_with(ext))
                    {
                        content.push(json!({"type":"localImage","path":file["path"]}));
                    }
                }
                send(
                    &mut input,
                    json!({"id":3,"method":"turn/start","params":{
                        "threadId":session,"input":content
                    }}),
                )?;
            }
            Some(3) => {
                turn = event["result"]["turn"]["id"]
                    .as_str()
                    .context("Codex returned no turn ID")?
                    .into();
            }
            _ => (),
        }
    }
    bail!("Codex closed the chat before completing its reply")
}
