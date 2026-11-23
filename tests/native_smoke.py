#!/usr/bin/env python3
"""Opt-in native qualification using the user's selected profiles and accounts."""

import argparse
import json
import tempfile
from pathlib import Path

from e2e import Service, git, repo, until


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("profiles", type=Path)
    parser.add_argument("--coordinator", required=True)
    parser.add_argument("--worker", required=True, action="append")
    parser.add_argument("--timeout", type=int, default=240, help="Seconds to wait for a reviewable candidate")
    parser.add_argument("--browser", help="Path to an installed Chromium executable for the greenfield web run")
    args = parser.parse_args()
    profiles = json.loads(args.profiles.read_text())
    with tempfile.TemporaryDirectory(prefix="swarm-native-") as tmp:
        root = Path(tmp)
        service = Service(root / "state")
        started = None
        try:
            (root / "state" / "profiles.json").write_text(json.dumps(profiles))
            project = repo(root / "quotes")
            if args.browser:
                check = ["node", str(Path(__file__).resolve().parents[1] / "examples" / "quotes-check.mjs"), args.browser]
                prompt = (
                    "Build a tiny private quote tracker. Use a foundation milestone with a README, .gitignore, and shared contract, then parallel independent API and UI milestones. "
                    "Use Python 3 stdlib HTTPServer on 127.0.0.1 and SWARM_PORT. Persist in SWARM_DATA_DIR. Backend owns app.py. UI owns index.html. "
                    "GET / serves index.html; GET /quotes returns [{id,client,amount,status}]; POST /quotes with {client,amount} stores an open quote; "
                    "POST /accept with {id} accepts it. Use Client and Amount field labels, an Add quote button, table rows with Accept buttons, Accepted status, "
                    'and exact text such as "Outstanding: $75.00" for open amounts. Commit each milestone.'
                )
            else:
                (project / "README.md").write_text("Existing quote tracker\n")
                (project / "quotes.py").write_text('QUOTES=[{"client":"North, Inc.","status":"open","amount":125},{"client":"Closed","status":"accepted","amount":75}]\n')
                git(project, "add", "--", "README.md", "quotes.py")
                git(project, "commit", "-m", "Add existing quote tracker")
                (project / "README.md").write_text("Unrelated staged note\n")
                git(project, "add", "README.md")
                (project / "scratch.txt").write_text("Untracked")
                staged = git(project, "ls-files", "--stage")
                check = ["python3", "-c",
                         'import csv,io,subprocess,sys; rows=list(csv.DictReader(io.StringIO(subprocess.check_output([sys.executable,"export.py"],text=True)))); '
                         'assert rows==[{"client":"North, Inc.","amount":"125"}], rows']
                prompt = ("Add export.py to this existing project in one milestone. Write CSV to stdout with header client,amount and only open quotes from "
                          "QUOTES in quotes.py. Escape comma-containing names. Keep the change to export.py and commit it.")
            started = service.api("/runs", {"path": str(project), "prompt": prompt, "coordinator": args.coordinator,
                                            "workers": args.worker, "recipe": {"checks": [check]}})

            def finished():
                run = service.api("/runs/" + started["id"])
                blocked = any(task["state"] == "blocked" for task in (run["tasks"] or []))
                return run if blocked or run["state"] in ("review", "blocked", "question") else None

            try:
                run = until(finished, seconds=args.timeout)
            except AssertionError as issue:
                last = service.api("/runs/" + started["id"])
                raise RuntimeError(f"Timed out in {last['state']}: {[(task['id'], task['state']) for task in (last['tasks'] or [])]}") from issue
            if run["state"] != "review":
                raise RuntimeError(f"Native run {run['id']} reached {run['state']}: {run.get('detail','')}; "
                                   f"{[(task['id'], task['state'], task.get('finding','')) for task in (run['tasks'] or [])]}")
            candidate = next(entry for entry in run["candidates"] if entry["state"] == "passed")
            if args.browser:
                landed = service.api("/runs/" + run["id"], {"kind": "land", "candidate": candidate["id"]})
                assert landed["state"] == "landed" and git(project, "rev-parse", "HEAD") == candidate["commit"]
            else:
                diff = service.api("/runs/" + run["id"] + "/diff/" + candidate["id"])
                paths = [line for line in diff.splitlines() if line.startswith("diff --git ")]
                assert paths == ["diff --git a/export.py b/export.py"], paths
                assert git(project, "ls-files", "--stage") == staged
                assert (project / "scratch.txt").read_text() == "Untracked"
            print("Verified native candidate:", candidate["commit"])
            print("Milestone commits:", [(task["id"], task["worker"], task.get("commit")) for task in run["tasks"]])
        finally:
            try:
                if started:
                    final = service.api("/runs/" + started["id"])
                    if any(turn["state"] == "running" for turn in final["turns"]):
                        service.api("/runs/" + started["id"], {"kind": "stop"})
            finally:
                service.stop()


if __name__ == "__main__":
    main()
