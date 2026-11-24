use super::*;

fn git(args: &[&str]) -> Result<String> {
    let out = Command::new("git").args(args).output()?;
    ensure!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(String::from_utf8(out.stdout)?.trim_end().into())
}
fn ask(message: &str) -> Value {
    json!({"status":"question","message":message})
}
pub(super) fn respond(prompt: &str, ctx: &Value) -> Result<Value> {
    let id = prompt
        .lines()
        .find_map(|line| {
            line.strip_prefix("Milestone (")?
                .split_once("):")
                .map(|(id, _)| id)
        })
        .context("missing milestone")?;
    let answer = prompt
        .lines()
        .find_map(|line| line.strip_prefix("Answer to your question:"))
        .unwrap_or("")
        .trim_start();
    if id.starts_with("team-") {
        ensure!(
            ctx["checks_approved"] == true && !rows(&ctx["checks"]).is_empty(),
            "worker missing approved checks"
        );
    }
    let (name, content): (String, String) = match id {
        "team-format" if answer.is_empty() => {
            ensure!(
                rows(&ctx["peer_milestones"])
                    .iter()
                    .any(|t| t["id"] == "team-check"),
                "missing peer context"
            );
            return Ok(ask("Which date contract should the exporter use?"));
        }
        "team-format" => ("format.txt".into(), answer.into()),
        "team-guide" => (
            "guide.txt".into(),
            decision(ctx, "What should the export optimize?")
                .context("missing optimization goal")?
                .into(),
        ),
        "team-view" if answer.is_empty() => {
            return Ok(ask("Which empty-state label should customers see?"));
        }
        "team-view" => ("view.txt".into(), answer.into()),
        "team-check" => {
            ensure!(
                prompt.contains("Coordinator: Verify the ISO dates field."),
                "missing coordinator handoff"
            );
            ("verified.txt".into(), fs::read_to_string("format.txt")?)
        }
        "report" => {
            let mut children = refs(prompt, "refs/swarm/delegated/");
            if children.is_empty() {
                return Ok(ask("Can two workers draft the sections?"));
            }
            children.sort();
            children.dedup();
            ensure!(
                children
                    == [
                        "refs/swarm/delegated/section-one",
                        "refs/swarm/delegated/section-two"
                    ],
                "unexpected delegated work"
            );
            for child in children {
                git(&["merge", "--no-edit", &child])?;
            }
            ("docs/report.txt".into(), "reviewed both sections".into())
        }
        "right" => {
            fs::write("clean-merge.txt", "from right")?;
            git(&["add", "--", "clean-merge.txt"])?;
            ("overlap.txt".into(), "right".into())
        }
        _ if id.starts_with("mix-") => {
            let donors = refs(prompt, "refs/swarm/donors/");
            ensure!(donors.len() >= 2, "missing donor references");
            for donor in donors {
                git(&["rev-parse", "--verify", &donor])?;
            }
            ("mixed.txt".into(), "combined".into())
        }
        _ if id.starts_with("section-") => {
            fs::create_dir_all("docs")?;
            (format!("docs/{id}.txt"), id.into())
        }
        _ => {
            let pair = match id {
                "foundation" => ("FOUNDATION.md", "The API and UI share an interface."),
                "api" => ("api.txt", "wrong"),
                "ui" => ("ui.txt", "usable"),
                "export" => ("export.txt", "open quotes"),
                "correct" => ("result.txt", "correct"),
                "fast" => ("result.txt", "wrong"),
                "left" => ("overlap.txt", "left"),
                "timing" => ("timing.txt", "measured"),
                _ if id.starts_with("repair-") => ("api.txt", "correct"),
                _ if id.starts_with("resolve-") => ("overlap.txt", "both"),
                _ => bail!("unknown fixture milestone {id}"),
            };
            (pair.0.into(), pair.1.into())
        }
    };
    fs::write(&name, content)?;
    git(&["add", "--", &name])?;
    git(&["commit", "-m", &format!("Implement {id}")])?;
    Ok(
        json!({"status":"committed","commit":git(&["rev-parse","HEAD"])?,"message":format!("Committed {name}")}),
    )
}
