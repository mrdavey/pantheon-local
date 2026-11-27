use super::*;

struct Client<'a, F> {
    reader: BufReader<ChildStdout>,
    writer: ChildStdin,
    emit: &'a F,
    session: String,
    text: String,
    previous_text: String,
    message_id: String,
    id: u64,
    cost_before_prompt: Option<f64>,
    current_model: Option<String>,
}
impl<F: Fn(Value)> Client<'_, F> {
    fn send(&mut self, v: Value) -> Result<()> {
        serde_json::to_writer(&mut self.writer, &v)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }
    fn set_choice(&mut self, setting: &Value, choice: &str) -> Result<Value> {
        self.call(
            "session/set_config_option",
            json!({"sessionId":self.session, "configId":setting["id"], "type":"select", "value":choice}),
        )
    }
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let prompting = method == "session/prompt";
        self.id += 1;
        self.send(json!({"jsonrpc":"2.0","id":self.id,"method":method,"params":params}))?;
        let mut buf = Vec::new();
        while line(&mut self.reader, &mut buf)? {
            let Ok(event) = serde_json::from_slice::<Value>(&buf) else {
                (self.emit)(
                    json!({"type":"stdout","text":String::from_utf8_lossy(&buf).trim_end_matches('\n')}),
                );
                continue;
            };
            if let Some(method) = event["method"].as_str() {
                if !event["id"].is_null() {
                    let mut response = json!({"jsonrpc":"2.0","id":event["id"]});
                    if method == "session/request_permission" {
                        // Hermes keeps a cancelled approval open for another viewer.
                        let denial = event["params"]["options"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .find(|o| o["kind"] == "reject_once")
                            .and_then(|o| o["optionId"].as_str());
                        response["result"] = match denial {
                            Some(id) => json!({"outcome":{"outcome":"selected","optionId":id}}),
                            None => json!({"outcome":{"outcome":"cancelled"}}),
                        }
                    } else {
                        response["error"] =
                            json!({"code":-32601,"message":"Host capability unavailable"})
                    }
                    (self.emit)(
                        json!({"type":method,"session":self.session,"raw":event["params"]}),
                    );
                    self.send(response)?;
                } else {
                    let update = &event["params"]["update"];
                    let kind = update["sessionUpdate"].as_str().unwrap_or(method);
                    let text = field(&update["content"], "text");
                    if kind == "usage_update"
                        && event["params"]["sessionId"] == self.session
                        && update["cost"]["currency"] == "USD"
                        && let Some(amount) = update["cost"]["amount"].as_f64()
                    {
                        if !prompting {
                            self.cost_before_prompt = Some(amount);
                        } else if let Some(start) = self.cost_before_prompt
                            && amount >= start
                        {
                            (self.emit)(
                                json!({"type":"usage","model":self.current_model,"cost_usd":amount-start}),
                            );
                        }
                    }
                    let parent_update = event["params"]["sessionId"]
                        .as_str()
                        .is_none_or(|id| id == self.session);
                    if parent_update && kind == "agent_message_chunk" {
                        if let Some(id) = update["messageId"].as_str()
                            && id != self.message_id
                        {
                            if !self.text.is_empty() {
                                self.previous_text = std::mem::take(&mut self.text);
                            }
                            self.message_id = id.to_owned();
                        }
                        self.text.push_str(text);
                    } else if parent_update
                        && kind == "tool_call"
                        && self.message_id.is_empty()
                        && !self.text.is_empty()
                    {
                        self.previous_text = std::mem::take(&mut self.text);
                    }
                    (self.emit)(
                        json!({"type":kind,"session":self.session,"text":text,"raw":event["params"]}),
                    );
                }
            } else if event["id"].as_u64() == Some(self.id) {
                if event["error"].is_object() {
                    bail!("{}", field(&event["error"], "message"));
                }
                return Ok(event["result"].clone());
            }
        }
        bail!("ACP connection closed during {method}")
    }
}
/// Only choose a named compact model within the current provider and model family.
fn fast_model<'a>(current: &str, mut available: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let split = current.rfind([':', '/'])? + 1;
    let (provider, model) = current.split_at(split);
    let family = ["gpt-", "claude-", "gemini-"]
        .into_iter()
        .find(|family| model.starts_with(family))?;
    let compact = |name: &str| match family {
        "gpt-" => matches!(name, "gpt-6-luna" | "gpt-5.4-mini"),
        "claude-" => name.starts_with("claude-haiku-") || name.starts_with("claude-3-5-haiku"),
        _ => matches!(
            name,
            "gemini-2.5-flash" | "gemini-3-flash" | "gemini-3-flash-preview"
        ),
    };
    if compact(model) {
        return None;
    }
    available.find(|id| id.strip_prefix(provider).is_some_and(compact))
}

fn options(setting: &Value) -> impl Iterator<Item = &str> {
    setting["options"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|item| {
            item["value"].as_str().into_iter().chain(
                item["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|opt| opt["value"].as_str()),
            )
        })
}

fn category<'a>(settings: &'a Value, key: &str) -> Option<&'a Value> {
    settings
        .as_array()?
        .iter()
        .find(|s| s["category"] == key && s["type"] == "select")
}

fn apply_profile_settings<F: Fn(Value)>(
    c: &mut Client<F>,
    info: &Value,
    profile: &Value,
) -> Result<Value> {
    let mut settings = info["configOptions"].clone();
    for (key, group) in [("model", "model"), ("effort", "thought_level")] {
        let choice = field(profile, key);
        if choice.is_empty() {
            continue;
        }
        if let Some(setting) = category(&settings, group) {
            if setting["currentValue"] == choice {
                continue;
            }
            ensure!(
                options(setting).any(|v| v == choice),
                "This session does not offer {key} {choice:?}. Check the worker’s model settings."
            );
            let changed = c.set_choice(setting, choice)?;
            if changed["configOptions"].is_array() {
                settings = changed["configOptions"].clone();
            }
        } else if key == "model" && info["models"]["currentModelId"] == choice {
            continue;
        } else if key == "model"
            && info["models"]["availableModels"]
                .as_array()
                .is_some_and(|rows| rows.iter().any(|m| m["modelId"] == choice))
        {
            c.call(
                "session/set_model",
                json!({"sessionId":c.session,"modelId":choice}),
            )?;
        } else {
            bail!(
                "This session does not advertise {key} {choice:?}. Use the harness default or a supported value."
            );
        }
    }
    Ok(settings)
}

fn apply_advisor_settings<F: Fn(Value)>(c: &mut Client<F>, info: &Value) {
    let mut settings = info["configOptions"].clone();
    if let Some(setting) = category(&settings, "model") {
        if let Some(fast) = fast_model(field(setting, "currentValue"), options(setting))
            && let Ok(changed) = c.set_choice(setting, fast)
            && changed["configOptions"].is_array()
        {
            settings = changed["configOptions"].clone();
        }
    } else {
        let models = &info["models"];
        let ids = models["availableModels"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| m["modelId"].as_str());
        if let Some(fast) = fast_model(field(models, "currentModelId"), ids) {
            let _ = c.call(
                "session/set_model",
                json!({"sessionId":c.session,"modelId":fast}),
            );
        }
    }
    if let Some(setting) = category(&settings, "thought_level")
        && setting["currentValue"] != "medium"
        && options(setting).any(|effort| effort == "medium")
    {
        let _ = c.set_choice(setting, "medium");
    }
}

/// Hermes can summarize its work before the receipt, replay it or append a verifier report.
/// Only fold an identical receipt. Leave ambiguous responses to strict decoding.
fn worker_receipt(text: &str) -> Option<Value> {
    let text = text.trim();
    let (summary, body) = text.split_at(text.find('{')?);
    if !summary.is_empty() && !summary.ends_with('\n') {
        return None;
    }
    let mut stream = serde_json::Deserializer::from_str(body).into_iter::<Value>();
    let mut receipt = stream.next()?.ok()?;
    let (original, suffix) = body.split_at(stream.byte_offset());
    let suffix = suffix.trim();
    if (summary.is_empty() && suffix.is_empty()) || !receipt.is_object() {
        return None;
    }
    let tail = suffix.strip_prefix(original).unwrap_or(suffix).trim();
    if tail.is_empty() {
        return Some(receipt);
    }

    // A committed milestone still goes through HEAD, ancestry and scope checks.
    // Preserve the native warning for the coordinator in the handoff itself.
    let mut lines = tail.lines();
    let heading = lines.next()?;
    if receipt["status"] != "committed"
        || !heading.starts_with("⚠️ File-mutation verifier:")
        || !lines
            .clone()
            .any(|line| line.trim_start().starts_with("• "))
        || !lines.all(|line| line.trim_start().starts_with("• "))
    {
        return None;
    }
    let summary = receipt.get_mut("message")?;
    *summary = json!(format!("{}\n\n{tail}", summary.as_str()?));
    Some(receipt)
}

pub(super) fn exchange(
    r: &Value,
    stdin: ChildStdin,
    stdout: ChildStdout,
    emit: &impl Fn(Value),
) -> Result<Value> {
    let mut c = Client {
        reader: BufReader::new(stdout),
        writer: stdin,
        emit,
        session: field(r, "session").into(),
        text: String::new(),
        previous_text: String::new(),
        message_id: String::new(),
        id: 0,
        cost_before_prompt: field(r, "session").is_empty().then_some(0.0),
        current_model: None,
    };
    let init=c.call("initialize",json!({"protocolVersion":1,"clientCapabilities":{"subagents":{}},"clientInfo":{"name":"local-swarm","version":env!("CARGO_PKG_VERSION")}}))?;
    let mut params = json!({"cwd":r["dir"],"mcpServers":[]});
    let session_info = if c.session.is_empty() {
        let session = c.call("session/new", params)?;
        c.session = field(&session, "sessionId").into();
        session
    } else {
        params["sessionId"] = json!(c.session);
        c.call(
            if init["agentCapabilities"]["loadSession"] == true {
                "session/load"
            } else {
                "session/resume"
            },
            params,
        )?
    };
    ensure!(!c.session.is_empty(), "ACP returned no session ID");
    emit(json!({"type":"session","session":c.session}));
    if r["role"] == "advisor" {
        apply_advisor_settings(&mut c, &session_info);
    } else {
        let settings = apply_profile_settings(&mut c, &session_info, &r["profile"])?;
        c.current_model = category(&settings, "model")
            .and_then(|s| s["currentValue"].as_str())
            .or_else(|| r["profile"]["model"].as_str().filter(|s| !s.is_empty()))
            .or_else(|| session_info["models"]["currentModelId"].as_str())
            .map(str::to_owned);
        if r["role"] == "worker" && r["profile"]["kind"] == "hermes" {
            let modes = &session_info["modes"];
            if let Some(setting) = category(&settings, "mode") {
                if setting["currentValue"] != "accept_edits"
                    && options(setting).any(|v| v == "accept_edits")
                {
                    c.set_choice(setting, "accept_edits")?;
                }
            } else if modes["currentModeId"] != "accept_edits"
                && modes["availableModes"]
                    .as_array()
                    .is_some_and(|rows| rows.iter().any(|m| m["id"] == "accept_edits"))
            {
                c.call(
                    "session/set_mode",
                    json!({"sessionId":c.session,"modeId":"accept_edits"}),
                )?;
            }
        }
    }
    c.text.clear();
    c.previous_text.clear();
    let mut content = vec![json!({"type":"text","text":field(r,"prompt")})];
    for file in r["attachments"].as_array().into_iter().flatten() {
        if let Ok(uri) = url::Url::from_file_path(field(file, "path")) {
            content.push(json!({"type":"resource_link","uri":uri.as_str(),"name":file["name"]}));
        }
    }
    let result = c.call(
        "session/prompt",
        json!({"sessionId":c.session,"prompt":content}),
    );
    let antigravity = r["profile"]["kind"] == "antigravity";
    let usage = result.as_ref().ok().map(|reply| &reply["usage"]);
    if let Some(usage) = usage {
        let normalized = if antigravity {
            antigravity_usage(usage)
        } else {
            usage_receipt(
                usage,
                r.get("acp_baseline"),
                !field(r, "session").is_empty(),
            )
        };
        if let Some(mut receipt) = normalized {
            receipt["model"] = json!(c.current_model);
            emit(receipt);
        }
    }
    if c.text.is_empty() && !c.previous_text.is_empty() {
        c.text = std::mem::take(&mut c.previous_text);
    }
    let mut receipt = match &result {
        Ok(result) if result["stopReason"] == "end_turn" && !c.text.is_empty() => {
            if r["role"] == "worker" && super::decode_typed_outcome("worker", &c.text).is_err() {
                if let Some(receipt) = worker_receipt(&c.text) {
                    c.text = serde_json::to_string(&receipt)?;
                } else if !c.previous_text.is_empty() {
                    let combined =
                        format!("{}\n\n{}", c.previous_text.trim_end(), c.text.trim_start());
                    if let Some(receipt) = worker_receipt(&combined) {
                        c.text = serde_json::to_string(&receipt)?;
                    }
                }
            }
            json!({"session":c.session,"text":c.text})
        }
        Ok(result) => {
            json!({"session":c.session,"error":format!("ACP turn stopped: {}",field(result,"stopReason"))})
        }
        Err(e) => json!({"session":c.session,"error":e.to_string()}),
    };
    if let Some(usage) = usage.filter(|usage| usage.is_object()) {
        receipt["acp_usage"] = usage.clone();
        if antigravity && let Some(name) = c.current_model {
            receipt["acp_model"] = json!(name);
        }
    }
    Ok(receipt)
}

// Antigravity returns prompt totals, including when session/resume is used.
pub(crate) fn antigravity_usage(usage: &Value) -> Option<Value> {
    usage_receipt(usage, None, false)
}

// Hermes returns session totals. A resumed session needs a baseline.
// Keep the total even if the optional billing categories do not reconcile.
fn usage_receipt(usage: &Value, prior: Option<&Value>, resumed: bool) -> Option<Value> {
    let delta = |key: &str| {
        let old = if resumed { prior?[key].as_u64()? } else { 0 };
        usage[key].as_u64()?.checked_sub(old)
    };
    let total = delta("totalTokens")?;
    let mut event = json!({"type":"usage","tokens":total});
    if let Some(reasoning) = ["reasoningTokens", "thinkingTokens", "reasoningOutputTokens"]
        .into_iter()
        .find_map(delta)
    {
        event["reasoning_tokens"] = reasoning.into();
    }
    if let (Some(input), Some(output)) = (delta("inputTokens"), delta("outputTokens"))
        && let Some(all_input) = total.checked_sub(output)
    {
        let cached = delta("cachedReadTokens").unwrap_or(0);
        let write = delta("cachedWriteTokens").unwrap_or(0);
        if let Some(cached_input) = cached.checked_add(write)
            && cached_input <= all_input
            && (input == all_input || input.checked_add(cached_input) == Some(all_input))
        {
            event["input_tokens"] = json!(all_input);
            event["output_tokens"] = json!(output);
            event["cached_input_tokens"] = json!(cached);
            event["cache_write_input_tokens"] = json!(write);
        }
    }
    Some(event)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resumed_usage_counts_only_the_current_turn() {
        let first =
            json!({"totalTokens":90,"inputTokens":60,"outputTokens":30,"cachedReadTokens":20});
        let next =
            json!({"totalTokens":120,"inputTokens":80,"outputTokens":40,"cachedReadTokens":28});
        let receipt = usage_receipt(&next, Some(&first), true).unwrap();
        assert_eq!(
            receipt,
            json!({"type":"usage","tokens":30,"input_tokens":20,"output_tokens":10,"cached_input_tokens":8,"cache_write_input_tokens":0})
        );
        assert!(usage_receipt(&next, None, true).is_none());
    }

    #[test]
    fn advisor_preserves_provider_identity_and_native_fallbacks() {
        let ids = [
            "custom:other/gpt-6-luna",
            "custom:mine/claude-haiku-4-5",
            "custom:mine/gpt-6-luna",
        ];
        assert_eq!(
            fast_model("custom:mine/gpt-6-astra", ids.into_iter()),
            Some(ids[2])
        );
        assert_eq!(fast_model("custom:mine/gpt-6-luna", ids.into_iter()), None);
        assert_eq!(fast_model("custom:mine/unknown", ids.into_iter()), None);
        assert_eq!(fast_model("gpt-6-astra", ids.into_iter()), None);
    }
    #[test]
    fn merges_replayed_worker_receipts_and_keeps_native_warnings() -> Result<()> {
        let original = r#"{"status":"committed","commit":"abc","message":"Storage tested"}"#;
        let warning =
            "⚠️ File-mutation verifier: 1 file edit(s) FAILED this turn.\n  • scratch: denied";
        let replay = format!("{original}\n\n{original}");
        assert_eq!(
            worker_receipt(&replay),
            Some(serde_json::from_str(original)?)
        );
        for prefix in [original, replay.as_str()] {
            let receipt = worker_receipt(&format!("{prefix}\n\n{warning}")).unwrap();
            assert_eq!(receipt["commit"], "abc");
            assert_eq!(receipt["message"], format!("Storage tested\n\n{warning}"));
            crate::wire::validate_outcome(&receipt, "worker")?;
        }
        assert_eq!(
            worker_receipt(&format!("Checked the finished CLI.\n\n{original}")),
            Some(serde_json::from_str(original)?)
        );
        for invalid in [
            format!("Example: {original}"),
            format!("There was an earlier {{object}}.\n{original}"),
            format!("{original}\n{}", original.replace("abc", "def")),
            format!("{original}\n{warning}\n{original}"),
            format!("{}\n{warning}", original.replace("committed", "ready")),
            format!("{original}\nunexpected prose"),
        ] {
            assert!(worker_receipt(&invalid).is_none());
        }
        Ok(())
    }
    #[test]
    fn acp_worker_handles_unkeyed_tool_boundaries_and_split_verifier_messages() -> Result<()> {
        for script in [
            // Unkeyed progress before a tool call followed by unkeyed JSON receipt without newline.
            concat!(
                "read _init\n",
                "echo '{\"id\":1,\"result\":{\"agentCapabilities\":{\"loadSession\":true}}}'\n",
                "read _load\n",
                "echo '{\"id\":2,\"result\":{}}'\n",
                "read _prompt\n",
                "echo '{\"method\":\"session/update\",\"params\":{\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"text\":\"Inspecting files.\"}}}}'\n",
                "echo '{\"method\":\"session/update\",\"params\":{\"update\":{\"sessionUpdate\":\"tool_call\",\"title\":\"git\"}}}'\n",
                "echo '{\"method\":\"session/update\",\"params\":{\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"text\":\"{\\\"status\\\":\\\"committed\\\",\\\"commit\\\":\\\"abc\\\",\\\"message\\\":\\\"Done\\\"}\"}}}}'\n",
                "echo '{\"id\":3,\"result\":{\"stopReason\":\"end_turn\"}}'\n",
            ),
            // Keyed JSON receipt followed by a separate messageId verifier warning.
            concat!(
                "read _init\n",
                "echo '{\"id\":1,\"result\":{\"agentCapabilities\":{\"loadSession\":true}}}'\n",
                "read _load\n",
                "echo '{\"id\":2,\"result\":{}}'\n",
                "read _prompt\n",
                "echo '{\"method\":\"session/update\",\"params\":{\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"messageId\":\"m1\",\"content\":{\"text\":\"{\\\"status\\\":\\\"committed\\\",\\\"commit\\\":\\\"abc\\\",\\\"message\\\":\\\"Done\\\"}\"}}}}'\n",
                "echo '{\"method\":\"session/update\",\"params\":{\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"messageId\":\"m2\",\"content\":{\"text\":\"⚠️ File-mutation verifier: 1 edit failed\\n  • scratch: denied\"}}}}'\n",
                "echo '{\"id\":3,\"result\":{\"stopReason\":\"end_turn\"}}'\n",
            ),
        ] {
            let mut child = Command::new("/bin/sh")
                .args(["-c", script])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()?;
            let stdin = child.stdin.take().unwrap();
            let stdout = child.stdout.take().unwrap();
            let req = json!({"role":"worker","session":"s1","dir":".","prompt":"test","profile":{"kind":"hermes"}});
            let out = exchange(&req, stdin, stdout, &|_| {})?;
            let _ = child.wait();
            let decoded = super::super::decode_typed_outcome("worker", field(&out, "text"))?;
            assert_eq!(decoded["commit"], "abc");
        }
        Ok(())
    }
}
