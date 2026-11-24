#!/usr/bin/env python3
"""A deterministic participant using the public adapter protocol."""

import json
import re
import subprocess
import sys
from pathlib import Path

if sys.argv[1] == "--version":
    print("fixture-agent 1")
    raise SystemExit
request = json.load(sys.stdin)
prompt = request["prompt"]
context = json.loads(prompt.split("\n", 1)[0].removeprefix("Run context: ")) if prompt.startswith("Run context: ") else {}
session = request.get("session") or "fixture-" + request["role"]


def end(value):
    print(json.dumps({"event": {"type": "session", "session": session}}))
    print(json.dumps({"result": {"session": session, "text": json.dumps(value), "tokens": 5}}))


def task(name, goal, worker="writer", **extra):
    return {"id": name, "goal": goal, "worker": worker, **extra}


if request["role"] == "advisor":
    print(json.dumps({"result": {"text": "Keep quotes private and check that changes survive a restart."}}))
elif request["role"] == "observer":
    state = json.loads(prompt.split("\nState: ", 1)[1])
    actions = []
    for event in state["Observations"]:
        if event["kind"] == "worker-question" and event["ref"] == "team-format":
            answer = next(d["answer"] for d in context["planning_decisions"] if d["question"] == "Which date format?")
            actions.extend([
                {"id": "answer-format", "kind": "reply_to_worker", "ref": "team-format", "text": answer},
                {"id": "tell-verifier", "kind": "message_worker", "ref": "team-check", "text": "Verify the " + answer + " field."},
            ])
        if event["kind"] == "developer-feedback":
            revisions = [case["revision"] for case in state["Cases"] if case["id"] == "saved-case"]
            actions.extend([
                {"id": "case-" + event["id"], "kind": "save_case_revision", "ref": event["id"],
                 "case": {"id": "saved-case", "revision": max(revisions, default=0) + 1,
                          "scenario": event["text"], "expected": "Preserve user-facing behavior.",
                          "why": "Record an observed edge case.", "state": "draft"}},
                {"id": "ask-" + event["id"], "kind": "ask", "ref": event["id"],
                 "question": {"text": "Should this behavior be required?", "choices": ["Yes", "No"]}},
            ])
        if event["kind"] == "worker-question" and event["ref"] == "report":
            actions.append({"id": "split-report", "kind": "delegate", "ref": "report",
                            "tasks": [task("section-one", "Write the first section."),
                                      task("section-two", "Write the second section.")]})
        if event["kind"] == "integration-conflict":
            actions.append({"id": "repair", "kind": "repair_conflict", "ref": event["ref"],
                            "tasks": [task("resolve-" + event["id"], "Include both requirements.")]})
        if event["kind"] == "combination":
            actions.append({"id": "mix", "kind": "combine", "ref": event["ref"],
                            "tasks": [task("mix-" + event["id"], "Keep the result and bring over the requested feature.")]})
    end({"actions": actions})
elif request["role"] == "chat":
    if "Add a timing milestone" in prompt:
        end({"reply": "We can record timings after the export is ready.",
             "tasks": [task("timing", "Record export timing.", depends=["export"], **{"class": "testing"})],
             "checks": [["python3", "-c", "from pathlib import Path; assert Path('timing.txt').read_text() == 'measured'"]]})
    else:
        end({"reply": "The export milestone owns the generated CSV."})
elif request["role"] == "coordinator":
    if "integrated candidate" in prompt:
        end({"task": "api", "instruction": "Repair the invalid api.txt result using the eval evidence."})
    elif "Build the team export" in prompt:
        decisions = {item["question"]: item["answer"] for item in context.get("planning_decisions", [])}
        if "Which date format?" not in decisions:
            end({"question": "Which date format?", "tasks": [task("draft", "Draft the export.")]})
        elif "What should the export optimize?" not in decisions:
            end({"question": "What should the export optimize?"})
        else:
            assert decisions == {"Which date format?": "ISO dates", "What should the export optimize?": "readability"}
            tasks = [task("team-format", "Write the date format.", paths=["format.txt"]),
                     task("team-view", "Write the empty-state label.", "writer-two", paths=["view.txt"]),
                     task("team-guide", "Document the optimization goal.", "second", paths=["guide.txt"]),
                     task("team-check", "Verify the format contract.", "verifier", depends=["team-format"], paths=["verified.txt"])]
            check = ["python3", "-c", "from pathlib import Path; assert [Path(f).read_text() for f in "
                     "('format.txt','view.txt','guide.txt','verified.txt')] == ['ISO dates','No quotes','readability','ISO dates']"]
            end({"tasks": tasks, "recipe": {"checks": [check]}})
    elif "Delegate the report" in prompt:
        end({"tasks": [task("report", "Assemble the final report.", paths=["docs"])],
             "recipe": {"checks": [["true"]]}})
    elif "Resolve overlap" in prompt:
        end({"tasks": [task("left", "Add left behavior."), task("right", "Add right behavior.")],
             "recipe": {"checks": [["python3", "-c", "from pathlib import Path; assert Path('overlap.txt').read_text() == 'both'"]]}})
    elif "Measure approaches" in prompt:
        end({"tasks": [task("correct", "Favor relevance.", variant="balanced"),
                       task("fast", "Favor throughput.", "writer-two", variant="fast")],
             "recipe": {"checks": [["true"]]}})
    elif "Compare approaches" in prompt:
        end({"tasks": [
            task("correct", "Retain correctness.", alternative="approach"),
            task("fast", "Explore a faster variant.", "writer-two", alternative="approach"),
        ], "recipe": {"checks": [["python3", "-c",
            "from pathlib import Path; assert Path('result.txt').read_text() == 'correct'"]]}})
    elif "Build the fixture" in prompt:
        end({"tasks": [
            task("foundation", "Create a project explanation."),
            task("api", "Implement persistence.", depends=["foundation"]),
            task("ui", "Implement the interface.", "writer-two", depends=["foundation"]),
        ], "recipe": {"checks": [["python3", "-c",
            "from pathlib import Path; assert Path('api.txt').read_text() == 'correct'; assert Path('ui.txt').read_text() == 'usable'"]]}})
    elif any(item["answer"] == "private" for item in context.get("planning_decisions", [])):
        end({"tasks": [task("export", "Add export.py to print open quotes only.", **{"class": "implementation"})],
             "recipe": {"checks": [["python3", "export.py"]]}})
    else:
        end({"tasks": [], "recipe": {"checks": []}, "question": "Who can view quotes?"})
else:
    match = re.search(r"^Milestone \(([^)]+)\):", prompt, re.M)
    milestone = match[1]
    files = {
        "foundation": ("FOUNDATION.md", "The API and UI share an interface."),
        "api": ("api.txt", "wrong"),
        "ui": ("ui.txt", "usable"),
        "export": ("export.py", "print('open quotes')\n"),
        "correct": ("result.txt", "correct"),
        "fast": ("result.txt", "wrong"),
        "left": ("overlap.txt", "left"),
        "right": ("overlap.txt", "right"),
        "timing": ("timing.txt", "measured"),
    }
    if milestone.startswith("team-"):
        assert context["checks_approved"] and context["checks"]
        decisions = {item["question"]: item["answer"] for item in context["planning_decisions"]}
        reply = re.search(r"^Answer to your question: (.*)$", prompt, re.M)[1]
        if milestone == "team-format":
            if not reply:
                assert any(peer["id"] == "team-check" for peer in context["peer_milestones"])
                end({"status": "question", "message": "Which date contract should the exporter use?"})
                raise SystemExit
            files[milestone] = ("format.txt", reply)
        if milestone == "team-guide":
            files[milestone] = ("guide.txt", decisions["What should the export optimize?"])
        if milestone == "team-view":
            if not reply:
                end({"status": "question", "message": "Which empty-state label should customers see?"})
                raise SystemExit
            files[milestone] = ("view.txt", reply)
        if milestone == "team-check":
            assert "Coordinator: Verify the ISO dates field." in prompt
            files[milestone] = ("verified.txt", Path("format.txt").read_text())
    if milestone == "report":
        children = re.findall(r"refs/swarm/delegated/[a-z0-9-]+", prompt)
        if not children:
            end({"status": "question", "message": "Can two workers draft the sections?"})
            raise SystemExit
        assert set(children) == {"refs/swarm/delegated/section-one", "refs/swarm/delegated/section-two"}
        for ref in children:
            subprocess.run(["git", "merge", "--no-edit", ref], check=True, stdout=sys.stderr)
        files[milestone] = ("docs/report.txt", "reviewed both sections")
    if milestone.startswith("section-"):
        Path("docs").mkdir(exist_ok=True)
        files[milestone] = ("docs/" + milestone + ".txt", milestone)
    name, content = ("api.txt", "correct") if milestone.startswith("repair-") else (
        ("mixed.txt", "combined") if milestone.startswith("mix-") else
        ("overlap.txt", "both") if milestone.startswith("resolve-") else files[milestone])
    if milestone.startswith("mix-"):
        refs = re.findall(r"refs/swarm/donors/[0-9a-f]+", prompt)
        assert len(refs) >= 2
        for ref in refs:
            subprocess.run(["git", "rev-parse", "--verify", ref], check=True, stdout=sys.stderr)
    if milestone == "right":
        Path("clean-merge.txt").write_text("from right")
        subprocess.run(["git", "add", "--", "clean-merge.txt"], check=True)
    Path(name).write_text(content)
    subprocess.run(["git", "add", "--", name], check=True)
    subprocess.run(["git", "commit", "-m", f"Implement {milestone}"], check=True, stdout=sys.stderr)
    sha = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    end({"status": "committed", "commit": sha, "message": f"Committed {name}"})
