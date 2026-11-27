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
            "worker-blocked" if reference=="retry-work" && event["text"]=="cache missing" =>
                actions.push(json!({"id":"fix-cache","kind":"retry_milestone","ref":reference,"text":"Rebuild the cache"})),
            "worker-blocked" if reference=="bad-base" => {
                actions.push(json!({"id":"create-replacement","kind":"delegate","tasks":[task("replacement","Verify the correct base","writer",Value::Null)]}));
                actions.push(json!({"id":"replace","kind":"supersede_milestone","ref":reference,"replacement":"replacement","text":"Use the clean verification checkout"}));
            }
            "worker-question" if reference=="team-format"=>{
                let answer=decision(ctx,"Which date format?").context("missing date decision")?;
                actions.extend([json!({"id":"answer-format","kind":"reply_to_worker","ref":"team-format","text":answer}),
                    json!({"id":"tell-verifier","kind":"message_worker","ref":"team-check","text":format!("Verify the {answer} field.")})]);
            }
            "comparison-ready" if prompt.contains("Strengthen the acceptance checks") => {
                let mut checks = rows(&state["ActiveSuite"]["checks"]).to_vec();
                let extra = check(&["export.txt","open quotes"])?;
                if !checks.contains(&extra) {
                    checks.push(extra);
                    actions.extend([
                        json!({"id":"record-export-case","kind":"save_case_revision","ref":id,
                            "case":{"id":"export-contract","revision":1,"scenario":"Export the quotes.",
                            "expected":"Export contains open quotes.","why":"Verify the file contract.","state":"draft"}}),
                        json!({"id":"check-export","kind":"propose_suite","recipe":{"checks":checks,"cases":[{
                            "id":"export-contract","revision":1,"required":true,"interpreter":"/bin/sh",
                            "program":"test \"$(cat export.txt)\" = 'open quotes' && printf '%s\\n' '{\"pass\":true,\"observed\":\"open quotes\"}'"
                        }]}})
                    ]);
                }
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
