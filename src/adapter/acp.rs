use super::*;

struct Client<'a, F> {
    reader: BufReader<ChildStdout>,
    writer: ChildStdin,
    emit: &'a F,
    session: String,
    text: String,
    message_id: String,
    id: u64,
}
impl<F: Fn(Value)> Client<'_, F> {
    fn send(&mut self, v: Value) -> Result<()> {
        serde_json::to_writer(&mut self.writer, &v)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.id += 1;
        self.send(json!({"jsonrpc":"2.0","id":self.id,"method":method,"params":params}))?;
        let mut buf = Vec::new();
        while line(&mut self.reader, &mut buf)? {
            let Ok(event) = serde_json::from_slice::<Value>(&buf) else {
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
                    if kind == "agent_message_chunk" {
                        if let Some(id) = update["messageId"].as_str()
                            && id != self.message_id
                        {
                            self.message_id = id.to_owned();
                            self.text.clear();
                        }
                        self.text.push_str(text);
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
        message_id: String::new(),
        id: 0,
    };
    let init=c.call("initialize",json!({"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"local-swarm","version":env!("CARGO_PKG_VERSION")}}))?;
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
    let modes = &session_info["modes"];
    if r["role"] == "worker"
        && r["profile"]["kind"] == "hermes"
        && modes["currentModeId"] != "accept_edits"
        && modes["availableModes"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|m| m["id"] == "accept_edits")
    {
        c.call(
            "session/set_mode",
            json!({"sessionId":c.session,"modeId":"accept_edits"}),
        )?;
    }
    c.text.clear();
    let result = c.call(
        "session/prompt",
        json!({"sessionId":c.session,"prompt":[{"type":"text","text":field(r,"prompt")}]}),
    );
    match result {
        Ok(result) if result["stopReason"] == "end_turn" && !c.text.is_empty() => {
            if r["role"] == "worker"
                && let Some(receipt) = worker_receipt(&c.text)
            {
                c.text = serde_json::to_string(&receipt)?;
            }
            Ok(json!({"session":c.session,"text":c.text}))
        }
        Ok(result) => Ok(
            json!({"session":c.session,"error":format!("ACP turn stopped: {}",field(&result,"stopReason"))}),
        ),
        Err(e) => Ok(json!({"session":c.session,"error":e.to_string()})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
