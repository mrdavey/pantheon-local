//! Strict request types for the local HTTP boundary. Durable JSON can contain
//! fields written by older versions, while client commands have a closed schema.
use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

macro_rules! request {
    ($name:ident { $($key:ident: $ty:ty),* $(,)? }) => {
        #[allow(dead_code)] // Deserialization checks every field before dispatch.
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct $name { $( $key: Option<$ty> ),* }
    };
}
request!(Start{
    subagents:bool, subagent_limit:u32, compare:bool, delivery:String,destination:String,routing:String,original_prompt:String,path:String,prompt:String,
    attachments:Vec<crate::attachments::Upload>,coordinator:String,workers:Vec<String>,initial_files:Vec<String>,skip_init:bool,target:String,token_target:i64,recipe:Recipe
});
request!(Command{attachments:Vec<crate::attachments::Upload>,id:String,expected_revision:i64,donors:Vec<String>,kind:String,task:String,text:String,candidate:String,tokens:i64,recipe:Recipe});
request!(Model {
    profile: String,
    model: String,
    effort: String,
    benchmark_model: String,
    benchmark_agent: String,
    benchmark_effort: String
});
request!(InitialFiles { path: String });
request!(Advice {
    attachments: Vec<crate::attachments::Upload>,
    profile: String,
    prompt: String
});
request!(Recipe{cases:Vec<Case>,rule:Rule,rubric:String,checks:Vec<Vec<String>>,preview:Vec<String>});
request!(Case {
    id: String,
    revision: i64,
    driver: String,
    program: String,
    interpreter: String,
    digest: String,
    input: Value,
    expected: Value,
    holdback: bool,
    required: bool,
    samples: i64,
    seed: i64
});
request!(Rule{metrics:Vec<Metric>,objective:String,minimum:f64,limits:BTreeMap<String,f64>,tie_break:String,min_samples:i64,min_lead:f64});
request!(Metric {
    name: String,
    unit: String,
    direction: String,
    aggregate: String
});

request!(Task{
    id:String,goal:String,consult:String,worker:String,eligible_workers:Vec<String>,class:String,depends:Vec<String>,paths:Vec<String>,variant:String,
    parent:String,donors:Vec<String>,round:i64,conflict:String,route:Value,delivered:i64,alternative:String,
    state:String,workspace:String,base:String,commit:String,session:String,question:String,reply:String,
    finding:String,repair_base:String,notes:Vec<String>
});
request!(Plan{title:String,tasks:Vec<Task>,recipe:Recipe,question:String});
request!(Worker {
    status: String,
    files: Vec<String>,
    commit: String,
    message: String,
    recipient: String
});
request!(Repair {
    task: String,
    instruction: String
});
request!(Chat{title:String,reply:String,tasks:Vec<Task>,checks:Vec<Vec<String>>});
request!(ObservationCase {
    id: String,
    revision: i64,
    scenario: String,
    expected: String,
    why: String,
    source: String,
    author: String,
    state: String,
    cursor: i64,
    active: bool
});
request!(Question{id:String,text:String,source:String,tasks:Vec<String>,choices:Vec<String>,
    blocking:bool,answer:String,state:String,revision:i64});
request!(Action{id:String,kind:String,r#ref:String,text:String,case:ObservationCase,
    question:Question,recipe:Recipe,tasks:Vec<Task>});
request!(Observer{actions:Vec<Action>});

pub fn validate_outcome(value: &Value, role: &str) -> Result<()> {
    match role {
        "plan" => {
            Plan::deserialize(value)?;
        }
        "worker" => {
            Worker::deserialize(value)?;
        }
        "repair" => {
            Repair::deserialize(value)?;
        }
        "chat" => {
            Chat::deserialize(value)?;
        }
        "observer" => {
            Observer::deserialize(value)?;
        }
        _ => bail!("unknown agent outcome type {role}"),
    }
    Ok(())
}

pub fn validate(value: &Value, schema: &str) -> Result<()> {
    match schema {
        "start" => {
            Start::deserialize(value)?;
        }
        "command" => {
            Command::deserialize(value)?;
        }
        "model" => {
            Model::deserialize(value)?;
        }
        "initial-files" => {
            InitialFiles::deserialize(value)?;
        }
        "advice" => {
            Advice::deserialize(value)?;
        }
        _ => bail!("unknown request type"),
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn rejects_unknown_nested_fields_and_wrong_types() {
        assert!(validate(&json!({"workers":"codex"}), "start").is_err());
        assert!(
            validate(
                &json!({"recipe":{"cases":[{"command":"example"}]}}),
                "start"
            )
            .is_err()
        );
        assert!(validate(&json!({"kind":"approve","revision":2}), "command").is_err());
    }
}
