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
fn historical_failures_are_indexed_once_without_waking_a_landed_run() -> Result<()> {
    let (dir, engine) = fixture()?;
    let mut r = json!({"id":"legacy","state":"landed","tasks":[{"id":"verify","state":"superseded","previous_failure":"incorrect base","superseded_by":"recheck"}],
        "turns":[{"id":"worker-log","role":"worker","task":"verify","finished":100}]});
    engine.store.save(&mut r, "landed", &[])?;
    drop(engine);
    for _ in 0..2 {
        let db = Store::open(dir.path())?;
        let saved = db.get("legacy")?;
        let items = a(&saved["failure_log"]);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["started"], 100);
        assert_eq!(items[0]["decision"], "Superseded by recheck.");
        assert_eq!(saved["revision"], 2);
        assert!(db.scheduled_runs()?.is_empty());
    }
    Ok(())
}
#[test]
fn coordinator_can_identify_and_log_an_issue_in_an_ordinary_handoff() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut r = json!({"state":"working","project":"project","input":{"workers":["writer"]},"tasks":[{"id":"export","state":"committed"}]});
    observe(
        &mut r,
        "worker-completed",
        "export",
        "Export is complete but large input is slow.",
    );
    let event = r["observations"][0]["id"].clone();
    let actions = json!({"actions":[{"id":"diagnose","kind":"assess_failure","ref":event,"text":"The large-input check found repeated parsing."},
        {"id":"schedule","kind":"delegate","tasks":[{"id":"optimize","worker":"writer","goal":"Avoid repeated parsing and verify unchanged results","depends":["export"]}]}]});
    experiment::finish_observation(
        &mut r,
        &json!({"task":event,"id":"review"}),
        &json!({"text":actions.to_string()}),
    )?;
    assert_eq!(r["failure_log"][0]["state"], "reviewed");
    assert_eq!(r["tasks"][1]["id"], "optimize");
    assert!(s(&r["failure_log"][0], "decision").contains("optimize"));
    assert!(a(&r["observations"]).is_empty());
    let page = crate::activity::page(&r, dir.path(), "role=recovery&milestone=export&q=parsing")?;
    assert_eq!(page["total"], 1);
    Ok(())
}

#[test]
fn failed_checks_supply_diagnostics_but_holdback_output_stays_private() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let job = dir.path().join("turns/check");
    std::fs::create_dir_all(&job)?;
    std::fs::write(
        job.join("events.log"),
        "Chromium could not create its profile socket",
    )?;
    let mut r = json!({"id":"run","project":"p","state":"evaluating","turns":[{"id":"check","task":"candidate","role":"eval","state":"received"}],"candidates":[{"id":"candidate","tasks":["worker"],"state":"failed","commit":"deadbeef"}]});
    observe(
        &mut r,
        "eval-failed",
        "candidate",
        "Command exited with code 1",
    );
    assert_eq!(r["failure_log"][0]["source"], "check");
    assert_eq!(r["failure_log"][0]["milestones"], json!(["worker"]));
    assert!(experiment::observer_prompt(&r, &[], dir.path()).contains("Chromium could not create"));
    r["input"]["recipe"]["cases"] = json!([{"holdback":true}]);
    assert!(
        !experiment::observer_prompt(&r, &[], dir.path()).contains("Chromium could not create")
    );
    Ok(())
}
#[test]
fn restarting_an_old_run_can_supersede_a_blocker_with_work_already_committed() -> Result<()> {
    let (dir, engine) = fixture()?;
    // A pre-recovery daemon persisted the failed validation, then a later
    // worker completed an equivalent verification milestone on a fresh base.
    let mut legacy = json!({"id":"legacy","project":"project","state":"working","recipe_approved":true,
        "tasks":[{"id":"fix","state":"committed","commit":"abc"},{"id":"verify","depends":["fix"],"state":"blocked","finding":"milestone must commit a descendant of its assigned base"},
            {"id":"recheck","depends":["fix"],"goal":"Verify the combined fixes","finding":"Both checks passed","commit":"def","state":"committed"}]});
    engine.store.save(&mut legacy, "seed", &[])?;
    drop(engine);
    let engine = Engine {
        dir: dir.path().to_owned(),
        store: Store::open(dir.path())?,
    };
    engine.tick()?;
    let mut run = engine.store.get("legacy")?;
    let failure = &run["observations"][0];
    assert_eq!(failure["ref"], "verify");
    assert_eq!(failure["kind"], "worker-blocked");
    let turn = json!({"id":"review","role":"observe","task":failure["id"]});
    let prompt = experiment::observer_prompt(&run, &[], dir.path());
    assert!(
        prompt.contains("Both checks passed") && prompt.contains("descendant of its assigned base")
    );
    let decision = json!({"actions":[{"id":"reuse","kind":"supersede_milestone","ref":"verify","replacement":"recheck","text":"The committed recheck covers both fixes"}]});
    experiment::finish_observation(&mut run, &turn, &json!({"text":decision.to_string()}))?;
    assert_eq!(run["tasks"][1]["superseded_by"], "recheck");
    assert_eq!(run["tasks"][2]["commit"], "def");
    assert_eq!(a(&run["tasks"]).len(), 3);
    assert!(committed(&run));
    assert!(a(&run["observations"]).is_empty());
    assert!(a(&run["inbox"]).is_empty());
    Ok(())
}
#[test]
fn operator_can_resolve_a_blocked_review_with_a_committed_replacement() -> Result<()> {
    let (_dir, e) = fixture()?;
    let mut r = json!({"id":"run","state":"working","project":"test","tasks":[{"id":"failed","state":"blocked","finding":"bad base"},{"id":"replacement","state":"pending"},{"id":"downstream","state":"pending","depends":["failed"]}]});
    e.save(&mut r, "seed")?;
    let action = json!({"kind":"supersede-milestone","task":"failed","replacement":"replacement","text":"Verified equivalent work","id":"accept"});
    assert!(e.command("run", action.clone()).is_err());
    r["tasks"][1]["state"] = json!("committed");
    r["tasks"][1]["commit"] = json!("accepted-commit");
    r["observer_error"] = json!("previous decision failed");
    e.save(&mut r, "committed")?;
    let saved = e.command("run", action.clone())?;
    assert_eq!(saved["tasks"][0]["superseded_by"], "replacement");
    assert_eq!(saved["tasks"][2]["depends"], json!(["replacement"]));
    assert!(a(&saved["observations"]).is_empty());
    assert_eq!(saved["observer_error"], "");
    assert_eq!(saved, e.command("run", action)?);
    Ok(())
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

#[test]
fn inbox_dismissal_preserves_decisions_and_retries_surface_new_blockers() -> Result<()> {
    let (_dir, e) = fixture()?;
    let mut run = json!({"id":"issues","state":"working","inbox":[{"id":"q","state":"open","blocking":true}],"tasks":[{"id":"fix","state":"blocked"}]});
    e.save(&mut run, "seed")?;
    for (item, id) in [("question", "q"), ("milestone", "fix")] {
        let dismiss = json!({"kind":"dismiss-issue","text":item,"task":id});
        e.command("issues", dismiss.clone())?;
        e.command(
            "issues",
            json!({"kind":"restore-issue","text":item,"task":id}),
        )?;
        e.command("issues", dismiss)?;
    }
    let run = e.store.get("issues")?;
    assert_eq!(run["inbox"][0]["state"], "open");
    assert_eq!(run["inbox"][0]["blocking"], true);
    assert_eq!(run["inbox"][0]["inbox_dismissed"], true);
    assert_eq!(run["tasks"][0]["state"], "blocked");
    let resumed = e.command("issues", json!({"kind":"resume","task":"fix"}))?;
    assert_eq!(resumed["tasks"][0]["inbox_dismissed"], false);
    assert_eq!(resumed["tasks"][0]["state"], "pending");
    Ok(())
}

fn committed_project(dir: &Path) -> Result<&str> {
    let root = dir.to_str().unwrap();
    project::git(root, &["init", "-b", "main"])?;
    for (key, value) in [
        ("user.name", "Fixture"),
        ("user.email", "test@example.invalid"),
        ("commit.gpgsign", "false"),
    ] {
        project::git(root, &["config", key, value])?;
    }
    project::git(root, &["commit", "--allow-empty", "-m", "Base"])?;
    Ok(root)
}

#[test]
fn missing_landing_target_records_one_error_and_recovers_when_restored() -> Result<()> {
    let (_dir, engine) = fixture()?;
    let project = tempfile::tempdir()?;
    let root = committed_project(project.path())?;
    let base = project::head(root)?;
    project::git(root, &["commit", "--allow-empty", "-m", "Candidate"])?;
    let commit = project::head(root)?;
    for (id, landing) in [("before", ""), ("after", "first"), ("held", "first")] {
        let target = format!("refs/heads/{id}");
        let mut run = json!({"id":id,"state":"review","landing":landing,"round":1,"policy":1,
            "repo":{"Root":root,"Target":target},"input":{"delivery":"auto"},
            "selection":{"state":"selected","candidate":"first","round":1,"policy":1},
            "candidates":[{"id":"first","state":"passed","base":base,"commit":commit,"round":1,"policy":1}]});
        if id == "before" {
            project::clone_repo(
                root,
                &engine.integration(&run),
                &commit,
                "swarm-integration",
            )?;
        }
        engine.save(&mut run, "selected")?;
    }
    engine.tick()?;
    let after_first_tick = engine.store.snapshot()?;
    for id in ["before", "after"] {
        assert!(!s(&engine.store.get(id)?, "detail").is_empty());
    }
    engine.tick()?;
    assert_eq!(
        after_first_tick["cursor"],
        engine.store.snapshot()?["cursor"]
    );
    assert_eq!(engine.store.get("after")?["landing"], "first");
    for (branch, head) in [("before", &base), ("after", &commit), ("held", &base)] {
        project::git(root, &["branch", branch, head])?;
    }
    engine.tick()?;
    for id in ["before", "after"] {
        assert_eq!(engine.store.get(id)?["state"], "landed");
        assert_eq!(project::git(root, &["rev-parse", id])?, commit);
    }
    let held = engine.store.get("held")?;
    assert_eq!(held["held"], true);
    assert_eq!(held["state"], "review");
    assert_eq!(project::git(root, &["rev-parse", "held"])?, base);
    Ok(())
}

#[test]
fn rejected_start_removes_its_pinned_inputs_without_touching_other_runs() -> Result<()> {
    let (state, engine) = fixture()?;
    let runs = state.path().join("runs");
    fs::create_dir_all(runs.join("existing"))?;
    let project = tempfile::tempdir()?;
    let root = committed_project(project.path())?;
    let profile = json!({"worker":{"kind":"custom","executable":std::env::current_exe()?}});
    let input = json!({"path":root,"prompt":"Test","coordinator":"worker","workers":["worker"],
        "recipe":{"cases":[{"id":"one","program":"example evaluator","interpreter":"no-such-pantheon-interpreter"}]}});
    let error = engine.start(input, profile).unwrap_err();
    assert!(
        error.to_string().contains("interpreter unavailable"),
        "{error}"
    );
    assert!(engine.store.list()?.is_empty());
    assert_eq!(fs::read_dir(&runs)?.count(), 1);
    assert!(runs.join("existing").is_dir());
    Ok(())
}

#[test]
fn required_consultation_rejects_premature_handoffs() -> Result<()> {
    let (_dir, engine) = fixture()?;
    for status in ["ready", "committed"] {
        let mut r =
            json!({"tasks":[{"id":"storage","consult":"Confirm the format","state":"running"}]});
        let result = json!({"text":json!({"status":status,"message":"Already implemented","files":["store.rs"]}).to_string()});
        engine.accept_worker(&mut r, &json!({"task":"storage"}), &result)?;
        assert_eq!(r["tasks"][0]["state"], "blocked");
        assert!(s(&r["tasks"][0], "finding").contains("must ask its coordinator"));
        assert!(r["tasks"][0]["commit"].is_null());
    }
    Ok(())
}

#[test]
fn final_handoff_review_can_assign_a_fix_before_evaluation() -> Result<()> {
    let (_dir, engine) = fixture()?;
    let turn = json!({"id":"observer","role":"observe","state":"running","task":"event","generation":1,"round":1,"policy":1});
    let mut r = json!({"id":"review","state":"working","generation":1,"round":1,"policy":1,
        "input":{"workers":["writer"]},"tasks":[{"id":"storage","state":"committed"}],
        "turns":[turn],"observations":[{"id":"event","kind":"worker-completed","ref":"storage"}]});
    engine.save(&mut r, "seed")?;
    let dir = engine.jobdir(&r, "observer");
    fs::create_dir_all(&dir)?;
    supervisor::write_json(&dir.join("job.json"), &json!({}))?;
    engine.tick()?;
    assert_eq!(engine.store.get("review")?["state"], "working");
    let actions = json!({"actions":[{"id":"fix","kind":"delegate","tasks":[{"id":"fix-storage","worker":"writer","goal":"Reject blank records","paths":["store.rs"],"depends":["storage"]}]}]});
    supervisor::write_json(
        &dir.join("result.json"),
        &json!({"text":actions.to_string()}),
    )?;
    engine.tick()?;
    let run = engine.store.get("review")?;
    assert_eq!(run["tasks"][1]["state"], "pending");
    assert!(a(&run["candidates"]).is_empty());
    Ok(())
}

#[test]
fn selected_candidate_waits_quietly_on_a_scoped_question() -> Result<()> {
    let (_dir, engine) = fixture()?;
    let mut r = json!({"id":"run","state":"review","round":1,"policy":1,"input":{"delivery":"auto"},
        "repo":{"Target":"refs/heads/main"},"selection":{"state":"selected","round":1,"policy":1,"candidate":"candidate"},
        "inbox":[{"state":"open","blocking":true,"tasks":["storage"]}],
        "candidates":[{"id":"candidate","state":"passed","tasks":["storage"],"round":1,"policy":1}]});
    engine.save(&mut r, "selected")?;
    engine.tick()?;
    assert_eq!(engine.store.get("run")?, r);
    Ok(())
}

#[test]
fn consultation_turn_preserves_queued_notes_for_implementation() -> Result<()> {
    let (_dir, engine) = fixture()?;
    let mut r = json!({
        "id":"consult","state":"working","generation":1,"round":1,"policy":1,
        "tasks":[{"id":"api","state":"running","consult":"Confirm auth","notes":["peer: Use bearer header"],"delivered":0}]
    });
    engine.save(&mut r, "seed")?;
    engine.agent_turn(&mut r, "worker", "api", json!({"prompt":"Ask"}))?;
    assert_eq!(r["turns"][0]["messages"], 0);
    let tid = s(&r["turns"][0], "id").to_owned();
    supervisor::write_json(
        &engine.jobdir(&r, &tid).join("result.json"),
        &json!({"text":r#"{"status":"question","message":"Bearer or cookie?"}"#}),
    )?;
    engine.tick()?;
    let saved = engine.store.get("consult")?;
    assert_eq!(saved["tasks"][0]["state"], "question");
    assert_eq!(saved["tasks"][0]["delivered"], 0);
    Ok(())
}

#[test]
fn tick_preserves_landed_state_rolls_back_failed_dispatch_and_pauses_conflicted_budget()
-> Result<()> {
    let (_dir, engine) = fixture()?;
    let mut landed = json!({
        "id":"landed","state":"landed","detail":"abc123","deploy_config":{"executable":"/bin/true"},
        "repo":{"Root":"/nonexistent-pantheon-repo","Target":"refs/heads/main"},
        "releases":[{"id":"rel","state":"queued","commit":"abc123","destination":"prod"}]
    });
    engine.save(&mut landed, "seed")?;
    engine.tick()?;
    let after = engine.store.get("landed")?;
    assert_eq!(after["state"], "landed");
    assert_eq!(after["detail"], "abc123");
    assert_eq!(after["releases"][0]["state"], "failed");
    let rev = after["revision"].clone();
    engine.tick()?;
    assert_eq!(engine.store.get("landed")?["revision"], rev);

    let mut broken = json!({
        "id":"dispatch-fail","state":"working","recipe_approved":true,
        "input":{"prompt":"Test","attachments":[{"name":"missing.txt","size":4,"path":"attachments/0/missing.txt"}]},
        "conversation":[{"id":"m1","role":"user","text":"Hello","state":"queued"}]
    });
    engine.save(&mut broken, "seed")?;
    engine.tick()?;
    let broken_after = engine.store.get("dispatch-fail")?;
    assert_eq!(broken_after["state"], "blocked");
    assert_eq!(broken_after["conversation"][0]["state"], "queued");
    assert!(a(&broken_after["turns"]).is_empty());

    let mut round2 = json!({
        "id":"r2","state":"blocked","round":2,"task_start":1,
        "input":{"coordinator":"old"},"profiles":{"old":{},"new":{"kind":"codex"}},
        "tasks":[{"id":"t1","state":"committed"}]
    });
    engine.save(&mut round2, "seed")?;
    let switched = engine.command("r2", json!({"kind":"coordinator","text":"new"}))?;
    assert_eq!(switched["state"], "planning");

    let mut conflicted = json!({
        "id":"conflict-budget","state":"evaluating","round":1,"policy":1,"tokens":5,
        "input":{"token_target":5},"observations":[{"id":"o1","kind":"integration-conflict","ref":"c1"}],
        "candidates":[{"id":"c1","state":"conflict","round":1,"policy":1}]
    });
    engine.save(&mut conflicted, "seed")?;
    engine.tick()?;
    assert_eq!(engine.store.get("conflict-budget")?["state"], "budget");
    let resumed = engine.command("conflict-budget", json!({"kind":"resume","tokens":10}))?;
    assert_eq!(resumed["state"], "evaluating");
    Ok(())
}

#[test]
fn conflict_repairs_survive_suite_updates_allow_auto_repair_and_retry_cleanly() -> Result<()> {
    let (_dir, engine) = fixture()?;
    let mut r = json!({
        "id":"conflict-flow","state":"review","round":1,"policy":1,"recipe_approved":true,
        "repo":{"Head":"base-sha"},
        "input":{"recipe":{"checks":[["original"]]}},
        "tasks":[
            {"id":"m1","state":"committed"},
            {"id":"m2","state":"committed"},
            {"id":"m3","state":"committed"},
            {"id":"cr","state":"committed","conflict":"c1","repair_base":"m1-sha"}
        ],
        "candidates":[
            {"id":"c1","state":"superseded","round":1,"policy":1,"base":"base-sha","tasks":["m1","m2","m3"],"pending":["m2","m3"]},
            {"id":"c2","state":"passed","round":1,"policy":1,"base":"base-sha","tasks":["cr","m3"],"commit":"c2-sha"}
        ]
    });
    engine.save(&mut r, "seed")?;
    let updated = engine.command(
        "conflict-flow",
        json!({"kind":"update-suite","recipe":{"checks":[["updated"]]}}),
    )?;
    assert_eq!(updated["state"], "evaluating");
    assert_eq!(updated["candidates"][2]["tasks"], json!(["cr", "m3"]));

    // Failed conflict-repaired candidate remains eligible for automatic repair.
    let mut failed = updated;
    failed["candidates"][2]["state"] = json!("failed");
    failed["candidates"][2]["commit"] = json!("c3-sha");
    engine.save(&mut failed, "eval-failed")?;
    engine.tick()?;
    assert_eq!(engine.store.get("conflict-flow")?["state"], "repairing");

    // Retrying a blocked conflict-repair task restores the conflicted candidate.
    let mut blocked_cr = json!({
        "id":"retry-cr","state":"review","round":1,"policy":1,"recipe_approved":true,
        "tasks":[
            {"id":"m1","state":"committed"},
            {"id":"m2","state":"committed"},
            {"id":"cr","state":"blocked","conflict":"c1","repair_base":"m1-sha"}
        ],
        "candidates":[
            {"id":"c1","state":"failed","round":1,"policy":1,"base":"base-sha","tasks":["m1","m2"],"pending":["m2"],"conflict_paths":["overlap.txt"]}
        ]
    });
    engine.save(&mut blocked_cr, "seed")?;
    let retried = engine.command("retry-cr", json!({"kind":"resume","task":"cr"}))?;
    assert_eq!(retried["state"], "evaluating");
    assert_eq!(retried["tasks"][2]["state"], "pending");
    assert_eq!(retried["candidates"][0]["state"], "conflict");
    Ok(())
}

#[test]
fn detached_head_landing_checks_ancestry_and_allows_retry_after_dirty_checkout() -> Result<()> {
    let (_dir, engine) = fixture()?;
    let project = tempfile::tempdir()?;
    let root = committed_project(project.path())?;
    let base = project::head(root)?;
    project::git(root, &["checkout", "--orphan", "unrelated"])?;
    project::git(root, &["commit", "--allow-empty", "-m", "Unrelated"])?;
    project::git(root, &["switch", "--detach", &base])?;
    project::git(root, &["commit", "--allow-empty", "-m", "Candidate"])?;
    let candidate = project::head(root)?;
    project::git(root, &["switch", "main"])?;
    fs::write(project.path().join("dirty.txt"), "local")?;

    let mut run = json!({
        "id":"land-retry","state":"review","round":1,"policy":1,
        "repo":{"Root":root,"Head":base,"Target":""},
        "input":{"delivery":"review"},
        "candidates":[{"id":"c1","state":"passed","base":base,"commit":candidate,"round":1,"policy":1}]
    });
    project::clone_repo(
        root,
        &engine.integration(&run),
        &candidate,
        "swarm-integration",
    )?;
    engine.save(&mut run, "seed")?;

    assert!(
        engine
            .command(
                "land-retry",
                json!({"kind":"land","id":"try-unrelated","candidate":"c1","text":"unrelated"})
            )
            .is_err()
    );
    assert_eq!(s(&engine.store.get("land-retry")?["repo"], "Target"), "");

    let land_main = json!({"kind":"land","id":"land-main","candidate":"c1","text":"main"});
    assert!(engine.command("land-retry", land_main.clone()).is_err());
    fs::remove_file(project.path().join("dirty.txt"))?;
    let landed = engine.command("land-retry", land_main)?;
    assert_eq!(landed["state"], "landed");
    assert_eq!(project::target_head(root, "refs/heads/main")?, candidate);
    Ok(())
}

#[test]
fn child_capacity_is_shared_without_delaying_primary_workers() {
    let mut r = json!({"input":{"subagent_limit":1},"tasks":[
        {"id":"first","parent":"a","state":"running"},
        {"id":"second","parent":"b","state":"pending"},
        {"id":"third","parent":"a","state":"pending"},
        {"id":"independent","state":"pending"}]});
    assert_eq!(scheduler::ready_workers(&r), vec![3]);
    r["tasks"][0]["state"] = json!("committed");
    assert_eq!(scheduler::ready_workers(&r), vec![1, 3]);
    r["input"]["subagent_limit"] = Value::Null;
    assert_eq!(scheduler::ready_workers(&r), vec![1, 2, 3]);
}

#[test]
fn all_turns_get_effective_models_without_treating_benchmarks_as_runtime_settings() {
    let r = json!({"input":{"workers":["primary","secondary","unknown"]},"profiles":{
        "primary":{"model":"configured-model","effort":"high","detected":{"model":"old","effort":"medium"}},
        "secondary":{"detected":{"label":"Native model","effort":"low"}},
        "unknown":{"benchmark_model":"Benchmark alias","benchmark_effort":"max"}}});
    let team = &context(&r, None)["team"];
    assert_eq!(team[0]["model"], "configured-model");
    assert_eq!(team[0]["thinking_effort"], "high");
    assert_eq!(team[1]["model"], "Native model");
    assert_eq!(team[1]["thinking_effort"], "low");
    assert!(team[2]["model"].is_null() && team[2]["thinking_effort"].is_null());
}

#[test]
fn shared_skills_prefer_the_private_checkout_and_include_global_skills() -> Result<()> {
    let root = tempfile::tempdir()?;
    let checkout = root.path().join("worker");
    let home = root.path().join("home");
    let nested = checkout.join("package");
    fs::create_dir_all(&nested)?;
    fs::create_dir(checkout.join(".git"))?;
    let skill = |base: &Path, name: &str| -> Result<PathBuf> {
        let dir = base.join(".agents/skills").join(name);
        fs::create_dir_all(&dir)?;
        let file = dir.join("SKILL.md");
        fs::write(&file, "Fixture instructions")?;
        Ok(file)
    };
    skill(&home, "shared")?;
    let local = skill(&checkout, "shared")?;
    let global = skill(&home, "global-only")?;
    assert_eq!(
        worker::shared_skills(&nested, root.path(), Some(&home)),
        vec![global, local]
    );
    Ok(())
}

#[test]
fn benchmark_routing_respects_the_planners_capability_pool() -> Result<()> {
    let (_dir, engine) = fixture()?;
    let run = json!({"id":"pool","input":{"routing":"quality","workers":["strong","fast"]},
        "profiles":{"strong":{"benchmark_model":"Pro","benchmark_agent":"CLI"},"fast":{"benchmark_model":"Small","benchmark_agent":"CLI"}}});
    fs::create_dir_all(engine.root(&run))?;
    let row = |model: &str, score: i64| json!({"model":model,"agent":"CLI","score":score,"class":"debugging","suite":"test","date":"2026-01-01"});
    fs::write(
        engine.root(&run).join("benchmarks.json"),
        json!({"rows":[row("Pro",70),row("Small",99)]}).to_string(),
    )?;
    let mut tasks = vec![
        json!({"id":"debug","class":"debugging","goal":"Resolve ambiguity","worker":"strong","eligible_workers":["strong"]}),
    ];
    validate_tasks(&tasks, &[], &run["input"]["workers"])?;
    engine.assign(&run, &mut tasks);
    assert_eq!(tasks[0]["worker"], "strong");
    Ok(())
}

#[test]
fn chat_files_and_proposal_acceptance_replay_once() -> Result<()> {
    let (_dir, engine) = fixture()?;
    let mut run = json!({"id":"files","state":"checks","input":{"workers":["writer"]},"round":1,"proposed_recipe":{"checks":[["original"]]}});
    fs::create_dir_all(engine.root(&run))?;
    engine.save(&mut run, "seed")?;
    let upload = json!({"id":"upload","kind":"chat","text":"Review scope","attachments":[{"name":"scope.md","data":"b25l"}]});
    let uploaded = engine.command("files", upload.clone())?;
    let file = &uploaded["conversation"][0]["attachments"][0];
    assert_eq!(
        fs::read_to_string(engine.root(&uploaded).join(s(file, "path")))?,
        "one"
    );
    assert!(file.get("data").is_none());
    assert_eq!(
        engine.command("files", upload.clone())?["conversation"],
        uploaded["conversation"]
    );
    let mut changed = upload;
    changed["attachments"][0]["data"] = json!("dHdv");
    assert!(engine.command("files", changed).is_err());
    let mut run = uploaded;
    run["conversation"][0]["state"] = json!("answered");
    engine.save(&mut run, "chat completed")?;
    let mut run = engine.command(
        "files",
        json!({"kind":"propose-milestones","id":"request","text":"Add tests"}),
    )?;
    assert_eq!(run["conversation"][1]["intent"], "add");
    let id = s(&run["conversation"][1], "id").to_owned();
    let proposal = json!({"reply":"Test the parser.","tasks":[{"id":"test","worker":"writer","goal":"Test parsing"}],"checks":[["extra"]]});
    engine.finish_chat(&mut run, &id, &json!({"text":proposal.to_string()}));
    assert_eq!(run["conversation"][2]["reply_to"], id);
    engine.save(&mut run, "proposed")?;
    let accept =
        json!({"kind":"accept-proposal","id":"accept","task":run["conversation"][2]["id"]});
    let accepted = engine.command("files", accept.clone())?;
    assert_eq!(accepted["state"], "checks");
    assert_eq!(
        accepted["proposed_recipe"]["checks"],
        json!([["original"], ["extra"]])
    );
    assert_eq!(accepted["tasks"][0]["round"], 1);
    assert_eq!(engine.command("files", accept)?["tasks"], accepted["tasks"]);
    Ok(())
}
