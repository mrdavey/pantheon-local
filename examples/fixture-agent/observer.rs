use super::*;

pub(super) fn respond(prompt: &str, ctx: &Value) -> Result<Value> {
    let state: Value = serde_json::from_str(
        prompt
            .split_once("\nState: ")
            .context("missing observer state")?
            .1,
    )?;
    let mut actions = Vec::new();
    for event in rows(&state["Observations"]) {
        let id = text(event, "id")?;
        let reference = &event["ref"];
        match text(event,"kind")?{
            "worker-question" if reference=="team-format"=>{
                let answer=decision(ctx,"Which date format?").context("missing date decision")?;
                actions.extend([json!({"id":"answer-format","kind":"reply_to_worker","ref":"team-format","text":answer}),
                    json!({"id":"tell-verifier","kind":"message_worker","ref":"team-check","text":format!("Verify the {answer} field.")})]);
            }
            "developer-feedback"=>{
                let revision=rows(&state["Cases"]).iter().filter(|c|c["id"]=="saved-case").filter_map(|c|c["revision"].as_u64()).max().unwrap_or(0)+1;
                actions.extend([json!({"id":format!("case-{id}"),"kind":"save_case_revision","ref":id,"case":{"id":"saved-case","revision":revision,"scenario":event["text"],"expected":"Preserve user-facing behavior.","why":"Record an observed edge case.","state":"draft"}}),
                    json!({"id":format!("ask-{id}"),"kind":"ask","ref":id,"question":{"text":"Should this behavior be required?","choices":["Yes","No"]}})]);
            }
            "worker-question" if reference=="report"=>actions.push(json!({"id":"split-report","kind":"delegate","ref":"report","tasks":[task("section-one","Write the first section.","writer",Value::Null),task("section-two","Write the second section.","writer",Value::Null)]})),
            "integration-conflict"=>actions.push(json!({"id":"repair","kind":"repair_conflict","ref":reference,"tasks":[task(&format!("resolve-{id}"),"Include both requirements.","writer",Value::Null)]})),
            "combination"=>actions.push(json!({"id":"mix","kind":"combine","ref":reference,"tasks":[task(&format!("mix-{id}"),"Keep the result and bring over the requested feature.","writer",Value::Null)]})),
            _=>{}
        }
    }
    Ok(json!({"actions":actions}))
}
