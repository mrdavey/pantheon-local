use super::*;

pub(super) fn respond(prompt: &str, ctx: &Value) -> Result<Value> {
    let milestone = |id, goal, extra| task(id, goal, "writer", extra);
    let build = |tasks: Vec<Value>, files: &[&str]| -> Result<Value> {
        Ok(json!({"tasks":tasks,"recipe":{"checks":[check(files)?]}}))
    };
    if prompt.contains("Recover two failed milestones automatically") {
        return build(
            vec![
                milestone("retry-work", "Retry the checkout", Value::Null),
                milestone("bad-base", "Verify the checkout", Value::Null),
                milestone(
                    "final-check",
                    "Check the result",
                    json!({"depends":["bad-base"]}),
                ),
            ],
            &["retry.txt", "fixed", "final.txt", "verified"],
        );
    }
    if prompt.contains("integrated candidate") {
        return Ok(
            json!({"task":"api","instruction":"Repair the invalid api.txt result using the eval evidence."}),
        );
    }
    if prompt.contains("Build the team export") {
        if decision(ctx, "Which date format?").is_none() {
            return Ok(
                json!({"question":"Which date format?","tasks":[milestone("draft","Draft the export.",Value::Null)]}),
            );
        }
        if decision(ctx, "What should the export optimize?").is_none() {
            return Ok(json!({"question":"What should the export optimize?"}));
        }
        ensure!(
            decision(ctx, "Which date format?") == Some("ISO dates")
                && decision(ctx, "What should the export optimize?") == Some("readability"),
            "lost team planning answers"
        );
        return build(
            vec![
                milestone(
                    "team-format",
                    "Write the date format.",
                    json!({"paths":["format.txt"],"consult":"Confirm the date format."}),
                ),
                task(
                    "team-view",
                    "Write the empty-state label.",
                    "writer-two",
                    json!({"paths":["view.txt"]}),
                ),
                task(
                    "team-guide",
                    "Document the optimization goal.",
                    "second",
                    json!({"paths":["guide.txt"]}),
                ),
                task(
                    "team-check",
                    "Verify the format contract.",
                    "verifier",
                    json!({"depends":["team-format"],"paths":["verified.txt"]}),
                ),
            ],
            &[
                "format.txt",
                "ISO dates",
                "view.txt",
                "No quotes",
                "guide.txt",
                "readability",
                "verified.txt",
                "ISO dates",
            ],
        );
    }
    if prompt.contains("Recover the fixture milestone") {
        return build(
            vec![milestone(
                "recover-export",
                "Recover the export.",
                json!({"paths":["export.txt"]}),
            )],
            &["export.txt", "ready"],
        );
    }
    if prompt.contains("Delegate the report") {
        return build(
            vec![milestone(
                "report",
                "Assemble the final report.",
                json!({"paths":["docs"]}),
            )],
            &[],
        );
    }
    if prompt.contains("Resolve overlap") {
        return build(
            vec![
                milestone("left", "Add left behavior.", Value::Null),
                milestone("right", "Add right behavior.", Value::Null),
            ],
            &["overlap.txt", "both"],
        );
    }
    if prompt.contains("Measure approaches") {
        return build(
            vec![
                milestone("correct", "Favor relevance.", json!({"variant":"balanced"})),
                task(
                    "fast",
                    "Favor throughput.",
                    "writer-two",
                    json!({"variant":"fast"}),
                ),
            ],
            &[],
        );
    }
    if prompt.contains("Compare approaches") {
        return build(
            vec![
                milestone(
                    "correct",
                    "Retain correctness.",
                    json!({"alternative":"approach"}),
                ),
                task(
                    "fast",
                    "Explore a faster variant.",
                    "writer-two",
                    json!({"alternative":"approach"}),
                ),
            ],
            &["result.txt", "correct"],
        );
    }
    if prompt.contains("Build the fixture") || prompt.contains("Inspect the assignment map") {
        let mut plan = build(
            vec![
                milestone("foundation", "Create a project explanation.", Value::Null),
                milestone(
                    "api",
                    if prompt.contains("Inspect the assignment map") {
                        "Add persistence."
                    } else {
                        "Implement persistence."
                    },
                    json!({"depends":["foundation"]}),
                ),
                task(
                    "ui",
                    "Implement the interface.",
                    "writer-two",
                    json!({"depends":["foundation"]}),
                ),
            ],
            &["api.txt", "correct", "ui.txt", "usable"],
        )?;
        if prompt.contains("Inspect the assignment map") {
            plan["recipe"]["checks"] = json!([["/usr/bin/env", "PATH=/fixture/bin", "npm", "ci"]]);
        }
        return Ok(plan);
    }
    if rows(&ctx["planning_decisions"])
        .iter()
        .any(|d| d["answer"] == "private")
    {
        return build(
            vec![milestone(
                "export",
                "Write the `open quotes` export.",
                json!({"class":"implementation"}),
            )],
            &["export.txt", "open quotes"],
        );
    }
    Ok(
        json!({"title":"Quote export","tasks":[],"recipe":{"checks":[]},"question":"Who can view quotes?"}),
    )
}
