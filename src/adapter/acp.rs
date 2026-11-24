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
            Ok(json!({"session":c.session,"text":c.text}))
        }
        Ok(result) => Ok(
            json!({"session":c.session,"error":format!("ACP turn stopped: {}",field(&result,"stopReason"))}),
        ),
        Err(e) => Ok(json!({"session":c.session,"error":e.to_string()})),
    }
}
