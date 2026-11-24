use super::*;
fn fixture() -> Result<(tempfile::TempDir, Engine)> {
    let dir = tempfile::tempdir()?;
    let e = Engine {
        dir: dir.path().to_owned(),
        store: Store::open(dir.path())?,
    };
    Ok((dir, e))
}
#[test]
fn worker_context_only_shares_approved_checks_and_current_peers() {
    let mut r = json!({"answer":"private quotes","recipe_approved":true,"input":{"recipe":{"checks":[["check-contract"]],"rubric":"readable","rule":{"objective":"latency"},"cases":[{"holdback":true,"driver":"/secret","expected":"hidden"}]}},"tasks":[{"id":"self","variant":"first","state":"running"},{"id":"peer","variant":"first","state":"pending"},{"id":"other","variant":"second","state":"running"},{"id":"finished","variant":"first","state":"committed"}]});
    let c = context(&r, Some(&r["tasks"][0]));
    assert_eq!(c["checks"][0][0], "check-contract");
    assert_eq!(c["planning_decisions"][0]["answer"], "private quotes");
    assert_eq!(c["review_criteria"], "readable");
    assert_eq!(c["selection_rule"]["objective"], "latency");
    assert_eq!(a(&c["peer_milestones"]).len(), 1);
    assert_eq!(c["peer_milestones"][0]["id"], "peer");
    assert!(!c.to_string().contains("secret"));
    assert!(!c.to_string().contains("hidden"));
    r["recipe_approved"] = json!(false);
    assert!(context(&r, None)["checks"].is_null());
}
#[test]
fn rejects_dependency_cycles_cross_variant_and_service_owned_fields() {
    let workers = json!(["x"]);
    let mut tasks = vec![
        json!({"id":"a","worker":"x","goal":"A","depends":["b"]}),
        json!({"id":"b","worker":"x","goal":"B"}),
    ];
    assert!(validate_tasks(&tasks, &[], &workers).is_ok());
    tasks[1]["depends"] = json!(["a"]);
    assert!(validate_tasks(&tasks, &[], &workers).is_err());
    tasks[1]["depends"] = json!([]);
    tasks[1]["variant"] = json!("fast");
    assert!(validate_tasks(&tasks, &[], &workers).is_err());
    tasks[0]["variant"] = json!("fast");
    tasks[0]["workspace"] = json!("/tmp");
    assert!(validate_tasks(&tasks, &[], &workers).is_err());
    tasks[0]["workspace"] = Value::Null;
    tasks[1]["paths"] = json!(["../outside"]);
    assert!(validate_tasks(&tasks, &[], &workers).is_err())
}
#[test]
fn stop_resume_preserves_in_flight_phase_and_requeues_cancelled_evaluations() -> Result<()> {
    let (_dir, e) = fixture()?;
    for phase in [
        "question",
        "checks",
        "review",
        "repairing",
        "evaluating",
        "working",
    ] {
        let mut r = json!({"id":phase,"state":phase,"candidate":[],"recipe_approved":true,"tasks":[{"id":"done","state":"committed"}]});
        if phase == "evaluating" {
            r["candidates"] = json!([{"id":"passed","state":"passed"},{"id":"cancelled","state":"cancelled","base":"original","tasks":["done"]}]);
        }
        e.save(&mut r, "seed")?;
        for _ in 0..2 {
            e.command(phase, json!({"kind":"stop"}))?;
        }
        let got = e.command(phase, json!({"kind":"resume"}))?;
        assert_eq!(got["state"], phase);
        if phase == "evaluating" {
            assert_eq!(got["candidates"][0]["state"], "passed");
            assert_eq!(got["candidates"][1]["state"], "superseded");
            assert_eq!(got["candidates"][2]["state"], "queued");
            assert_eq!(got["candidates"][2]["base"], "original");
            assert_ne!(got["candidates"][2]["id"], "cancelled");
        }
    }
    Ok(())
}
#[test]
fn paused_plan_accepts_answer_before_resuming() -> Result<()> {
    let (_dir, e) = fixture()?;
    let mut r = json!({"id":"team","state":"question","question":"Which format?","inbox":[{"id":"q","source":"plan","state":"open"}]});
    e.save(&mut r, "seed")?;
    e.command("team", json!({"kind":"stop"}))?;
    let paused = e.command(
        "team",
        json!({"kind":"answer-inbox","task":"q","text":"CSV"}),
    )?;
    assert_eq!(paused["state"], "stopped");
    assert_eq!(paused["inbox"][0]["state"], "answered");
    let resumed = e.command("team", json!({"kind":"resume"}))?;
    assert_eq!(resumed["state"], "planning");
    assert_eq!(resumed["answer"], "CSV");
    Ok(())
}
#[test]
fn accepted_proposal_keeps_original_checks_and_repair_dependency() -> Result<()> {
    let (_dir, e) = fixture()?;
    let mut r = json!({"id":"chat","state":"review","input":{"workers":["writer"],"recipe":{"checks":[["original"]]}},"tasks":[{"id":"base","state":"committed","worker":"writer"},{"id":"repair","state":"committed","worker":"writer","repair_base":"abc"}],"candidates":[{"id":"old","state":"passed"}],"conversation":[{"id":"u","state":"running"}]});
    let reply = json!({"reply":"Add a follow-up","title":"Quote export","tasks":[{"id":"next","goal":"Write a check","worker":"writer","depends":["base"]}],"checks":[["added"]]});
    e.finish_chat(&mut r, "u", &json!({"text":reply.to_string()}));
    let id = s(&r["conversation"][1], "id").to_owned();
    assert_eq!(r["conversation"][1]["proposal_state"], "pending");
    assert_eq!(r["title"], "Quote export");
    e.save(&mut r, "seed")?;
    let action = json!({"kind":"accept-proposal","task":id});
    let got = e.command("chat", action.clone())?;
    assert_eq!(got["state"], "working");
    assert_eq!(got["tasks"][2]["depends"], json!(["base", "repair"]));
    assert_eq!(got["candidates"][0]["state"], "superseded");
    assert_eq!(
        got["input"]["recipe"]["checks"],
        json!([["original"], ["added"]])
    );
    assert!(e.command("chat", action).is_err());
    Ok(())
}
#[test]
fn chat_failure_releases_pending_message_without_stopping_work() -> Result<()> {
    let (_dir, e) = fixture()?;
    let mut r = json!({"id":"chat","state":"working"});
    e.save(&mut r, "seed")?;
    let action = json!({"kind":"chat","text":"Why?"});
    let mut r = e.command("chat", action.clone())?;
    assert!(e.command("chat", action.clone()).is_err());
    let id = s(&r["conversation"][0], "id").to_owned();
    e.finish_chat(&mut r, &id, &json!({"error":"connection failed"}));
    e.save(&mut r, "received")?;
    assert_eq!(r["state"], "working");
    assert_eq!(r["conversation"][0]["state"], "failed");
    assert!(e.command("chat", action).is_ok());
    Ok(())
}
#[test]
fn developer_answer_resumes_waiting_worker_and_updates_active_workers() {
    let mut r = json!({"inbox":[{"id":"q","state":"open","text":"Which format?","tasks":["api","docs"],"blocking":true}],"tasks":[{"id":"api","state":"question"},{"id":"docs","state":"running"},{"id":"other","state":"running"}]});
    assert!(held(&r, "api"));
    record_answer(&mut r, 0, "CSV");
    assert_eq!(r["tasks"][0]["state"], "pending");
    assert_eq!(r["tasks"][0]["reply"], "CSV");
    assert_eq!(
        r["tasks"][1]["notes"][0],
        "Developer answered Which format? — CSV"
    );
    assert!(r["tasks"][2]["notes"].is_null());
    assert!(!held(&r, "api"))
}
#[test]
fn scheduler_fences_unknown_phase_across_repeated_ticks() -> Result<()> {
    let (_dir, engine) = fixture()?;
    let mut run = json!({"id":"future","state":"future-phase","landing":"pending","conversation":[{"id":"q","state":"queued"}],"turns":[]});
    engine.save(&mut run, "seed")?;
    engine.tick()?;
    let fenced = engine.store.get("future")?;
    assert_eq!(fenced["state"], "future-phase");
    assert_eq!(fenced["landing"], "pending");
    assert!(s(&fenced, "detail").contains("unsupported run phase"));
    engine.tick()?;
    assert_eq!(engine.store.get("future")?, fenced);
    Ok(())
}
#[test]
fn scheduler_releases_independent_workers_and_respects_dependencies() {
    let mut run = json!({"task_start":1,"tasks":[
        {"id":"old","state":"committed"},{"id":"a","state":"pending","depends":["old"]},
        {"id":"b","state":"pending"},{"id":"c","state":"pending"},{"id":"d","state":"pending"},
        {"id":"parent","state":"pending"},{"id":"child","state":"pending","parent":"parent","depends":["a"]},
        {"id":"held","state":"pending"}
    ],"inbox":[{"state":"open","blocking":true,"tasks":["held"]}]});
    assert_eq!(scheduler::ready_workers(&run), [1, 2, 3, 4]);
    run["tasks"][1]["state"] = json!("committed");
    assert_eq!(scheduler::ready_workers(&run), [2, 3, 4, 6]);
    run["tasks"][6]["state"] = json!("committed");
    assert_eq!(scheduler::ready_workers(&run), [2, 3, 4, 5]);
}
#[test]
fn invalid_release_authority_never_initializes_project() -> Result<()> {
    let (_dir, engine) = fixture()?;
    let project = tempfile::tempdir()?;
    let profile = json!({"worker":{"kind":"custom","executable":std::env::current_exe()?}});
    let input = json!({
        "path":project.path(), "prompt":"Implement a change", "coordinator":"worker",
        "workers":["worker"], "delivery":"deploy", "destination":"missing"
    });
    assert!(engine.start(input, profile).is_err());
    assert!(!project.path().join(".git").exists());
    assert!(engine.store.list()?.is_empty());
    Ok(())
}

#[test]
fn follow_up_changes_only_affect_the_current_round_and_suite() -> Result<()> {
    let (_dir, engine) = fixture()?;
    let mut run = json!({
        "id":"follow-up","state":"review","round":2,"policy":2,"task_start":1,
        "input":{"workers":["writer"]},
        "tasks":[{"id":"prior-repair","worker":"writer","state":"committed","repair_base":"old"}],
        "candidates":[
            {"id":"previous-round","state":"passed","round":1,"policy":2},
            {"id":"previous-suite","state":"passed","round":2,"policy":1},
            {"id":"current","state":"passed","round":2,"policy":2},
            {"id":"no-evidence","state":"inconclusive","round":2,"policy":2}
        ],
        "conversation":[{"id":"proposal","proposal_state":"pending","tasks":[
            {"id":"docs","worker":"writer","goal":"Document this round"}
        ]}]
    });
    engine.save(&mut run, "seed")?;
    for candidate in ["previous-round", "previous-suite"] {
        assert!(
            engine
                .command(
                    "follow-up",
                    json!({"kind":"change","candidate":candidate,"text":"Update it"})
                )
                .is_err()
        );
    }
    let revised = engine.command(
        "follow-up",
        json!({"kind":"change","candidate":"current","text":"Update it"}),
    )?;
    assert_eq!(revised["repair_id"], "current");
    assert_eq!(a(&revised["candidates"])[..2], a(&run["candidates"])[..2]);
    run["state"] = json!("review");
    engine.save(&mut run, "seed proposal")?;
    let extended = engine.command(
        "follow-up",
        json!({"kind":"accept-proposal","task":"proposal"}),
    )?;
    assert!(a(&extended["tasks"][1]["depends"]).is_empty());
    assert_eq!(a(&extended["candidates"])[..2], a(&run["candidates"])[..2]);
    assert_eq!(extended["candidates"][2]["state"], "superseded");
    assert_eq!(extended["candidates"][3]["state"], "superseded");
    Ok(())
}

#[test]
fn later_round_can_repair_and_resume_at_its_token_target() -> Result<()> {
    let (dir, engine) = fixture()?;
    let mut run = json!({"id":"repair","state":"evaluating","round":2,"policy":1,"task_start":1,
        "recipe_approved":true,"input":{"token_target":5},"tokens":5,
        "tasks":[{"id":"old","repair_base":"earlier-failure","state":"committed"},{"id":"current","state":"committed"}],
        "candidates":[{"id":"failed","state":"failed","commit":"this-failure","round":2,"policy":1}]
    });
    engine.save(&mut run, "seed")?;
    engine.tick()?;
    assert_eq!(engine.store.get("repair")?["state"], "repairing");
    engine.tick()?;
    assert_eq!(engine.store.get("repair")?["state"], "budget");
    engine.command("repair", json!({"kind":"stop"}))?;
    drop(engine);
    let engine = Engine {
        dir: dir.path().into(),
        store: Store::open(dir.path())?,
    };
    let resumed = engine.command("repair", json!({"kind":"resume","tokens":10}))?;
    assert_eq!(resumed["state"], "repairing");
    assert_eq!(resumed["repair_id"], "failed");
    Ok(())
}

#[test]
fn suite_revision_approved_while_paused_is_evaluated_after_resume() -> Result<()> {
    let (_dir, engine) = fixture()?;
    let mut run = json!({"id":"paused","state":"review","policy":1,"round":1,
        "recipe_approved":true,"input":{"recipe":{"checks":[["original"]]}},
        "repo":{"Head":"base"},"tasks":[{"id":"api","state":"committed"}],
        "candidates":[{"id":"old","state":"passed","round":1,"policy":1}]
    });
    engine.save(&mut run, "seed")?;
    engine.command("paused", json!({"kind":"stop"}))?;
    let updated = engine.command(
        "paused",
        json!({"kind":"update-suite","recipe":{"checks":[["updated"]]}}),
    )?;
    assert_eq!(updated["state"], "stopped");
    let resumed = engine.command("paused", json!({"kind":"resume"}))?;
    assert_eq!(resumed["state"], "evaluating");
    assert_eq!(resumed["candidates"][1]["policy"], 2);
    assert_eq!(resumed["candidates"][1]["state"], "queued");
    Ok(())
}

#[test]
fn incomplete_handoffs_resume_the_session_and_stop_after_two_retries() -> Result<()> {
    let (_dir, engine) = fixture()?;
    let mut run = json!({"tasks":[{"id":"store","state":"running"}]});
    let turn = json!({"task":"store"});
    let progress = json!({"session":"storage-session","text":"Waiting for a review."});
    for expected in 1..=2 {
        engine.accept_worker(&mut run, &turn, &progress)?;
        let task = &run["tasks"][0];
        assert_eq!(task["session"], "storage-session");
        assert_eq!(task["state"], "pending");
        assert_eq!(task["handoff_retries"], expected);
    }
    let mut completed = run.clone();
    engine.accept_worker(&mut completed, &turn, &json!({
        "session":"storage-session","text":r#"{"status":"blocked","message":"Read-only filesystem"}"#
    }))?;
    assert_eq!(completed["tasks"][0]["state"], "blocked");
    assert_eq!(completed["tasks"][0]["finding"], "Read-only filesystem");
    assert!(completed["tasks"][0].get("handoff_retries").is_none());

    engine.accept_worker(&mut run, &turn, &progress)?;
    assert_eq!(run["tasks"][0]["state"], "blocked");
    assert!(s(&run["tasks"][0], "finding").contains("readable milestone receipt"));
    Ok(())
}
