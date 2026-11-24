#!/usr/bin/env python3
"""Exercise the compiled daemon, real Git, subprocess adapters and landing."""

import json
import os
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
BIN = REPO / "bin" / "swarm"
if not BIN.exists():
    BIN = REPO / "swarm"


def git(path, *args):
    return subprocess.check_output(["git", "-C", path, *args], text=True, stderr=subprocess.STDOUT).strip()


def repo(path):
    path.mkdir()
    git(path, "init", "-b", "main")
    git(path, "config", "user.name", "Pantheon Local Test")
    git(path, "config", "user.email", "test@example.invalid")
    return path


def until(fn, seconds=25):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        got = fn()
        if got:
            return got
        time.sleep(0.15)
    raise AssertionError("condition did not become true")


class Service:
    def __init__(self, state):
        self.state = state
        state.mkdir()
        profile = {"kind": "custom", "executable": str(REPO / "tests" / "fixture-agent.py")}
        (state / "profiles.json").write_text(json.dumps({
            "profiles": {key: profile for key in ("first", "second", "writer", "writer-two", "verifier")},
        }))
        self.start()

    def start(self):
        env = os.environ | {"SWARM_STATE_DIR": str(self.state)}
        self.proc = subprocess.Popen([BIN, "serve"], env=env, stdout=subprocess.DEVNULL)
        meta = self.state / "service.json"
        self.info = until(lambda: json.loads(meta.read_text()) if meta.exists() else None)

    def stop(self, hard=False):
        if hard:
            self.proc.kill()
        else:
            self.proc.terminate()
        self.proc.wait(timeout=15)
        (self.state / "service.json").unlink(missing_ok=True)

    def api(self, path, value=None):
        req = urllib.request.Request(self.info["url"] + "/api" + path,
                                     data=None if value is None else json.dumps(value).encode(),
                                     headers={"Authorization": "Bearer " + self.info["token"], "Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=10) as response:
            return json.load(response)

    def wait_state(self, run, *states):
        def check():
            latest = self.api("/runs/" + run["id"])
            if latest["state"] == "blocked" and "blocked" not in states:
                raise AssertionError(latest)
            return latest if latest["state"] in states else None

        try:
            return until(check)
        except AssertionError as err:
            raise AssertionError(self.api("/runs/" + run["id"])) from err


def exercise_team(svc, project):
    run = svc.api("/runs", {"path": str(project), "prompt": "Build the team export", "coordinator": "first",
                            "workers": ["writer", "writer-two", "second", "verifier"]})
    url = "/runs/" + run["id"]
    first = svc.wait_state(run, "question")
    assert first["question"] == "Which date format?" and not first["tasks"]
    svc.api(url, {"kind": "answer", "text": "ISO dates"})
    second = svc.wait_state(run, "question")
    assert second["question"] == "What should the export optimize?"
    svc.stop()
    svc.start()
    svc.api(url, {"kind": "answer-inbox", "task": second["inbox"][-1]["id"], "text": "readability"})
    plan = svc.wait_state(run, "checks")
    assert not any(turn["role"] == "worker" for turn in plan["turns"])
    assert len({t["worker"] for t in plan["tasks"]}) == 4
    svc.api(url, {"kind": "approve"})

    def remaining_question():
        state = svc.api(url)
        assert not state.get("observer_error"), state
        waiting = [q for q in state["inbox"] if q["state"] == "open"]
        if all(t["state"] == "committed" for t in state["tasks"] if t["id"] != "team-view") and waiting:
            assert len(waiting) == 1 and waiting[0]["tasks"] == ["team-view"], state
            return waiting[0]

    question = until(remaining_question)
    svc.stop()
    svc.start()
    assert [q for q in svc.api(url)["inbox"] if q["state"] == "open"] == [question]
    svc.api(url, {"kind": "answer-inbox", "task": question["id"], "text": "No quotes"})
    ready = svc.wait_state(run, "review")
    passed = next(c for c in ready["candidates"] if c["state"] == "passed")
    svc.api(url, {"kind": "land", "candidate": passed["id"]})
    assert git(project, "rev-parse", "HEAD") == passed["commit"]


def main():
    with tempfile.TemporaryDirectory(prefix="local-swarm-e2e-") as directory:
        root = Path(directory)
        svc = Service(root / "state")
        try:
            exercise_team(svc, repo(root / "team"))
            app = repo(root / "new")
            run = svc.api("/runs", {"path": str(app), "prompt": "Build the fixture", "coordinator": "first", "workers": ["writer", "writer-two"]})
            svc.wait_state(run, "checks")
            svc.api("/runs/" + run["id"], {"kind": "approve"})
            until(lambda: any(t["state"] == "running" and t["role"] == "worker"
                              for t in svc.api("/runs/" + run["id"])["turns"]))
            svc.stop(hard=True)
            svc.start()
            ready = svc.wait_state(run, "review")
            assert len([t for t in ready["turns"] if t["role"] == "worker"]) == 4, ready
            assert any(c["state"] == "failed" for c in ready["candidates"]), ready
            passed = next(c for c in ready["candidates"] if c["state"] == "passed")
            landed = svc.api("/runs/" + run["id"], {"kind": "land", "candidate": passed["id"]})
            assert landed["state"] == "landed" and git(app, "rev-parse", "HEAD") == passed["commit"]
            assert (app / "api.txt").read_text() == "correct"
            assert len(git(app, "log", "--format=%H").splitlines()) >= 5

            existing = repo(root / "existing")
            (existing / "README.md").write_text("before\n")
            git(existing, "add", "README.md")
            git(existing, "commit", "-m", "Base")
            (existing / "README.md").write_text("staged\n")
            git(existing, "add", "README.md")
            (existing / "scratch.txt").write_text("private\n")
            index = git(existing, "ls-files", "--stage")
            original = git(existing, "status", "--porcelain")
            run = svc.api("/runs", {"path": str(existing), "prompt": "Add quote export", "coordinator": "first", "workers": ["writer"]})
            svc.wait_state(run, "question")
            svc.api("/runs/" + run["id"], {"kind": "coordinator", "text": "second"})
            svc.api("/runs/" + run["id"], {"kind": "answer", "text": "private"})
            svc.wait_state(run, "checks")
            svc.api("/runs/" + run["id"], {"kind": "approve"})
            ready = svc.wait_state(run, "review")
            assert ready["input"]["coordinator"] == "second"
            candidate = next(c for c in ready["candidates"] if c["state"] == "passed")
            try:
                svc.api("/runs/" + run["id"], {"kind": "land", "candidate": candidate["id"]})
            except urllib.error.HTTPError as err:
                assert "local changes" in err.read().decode()
            else:
                raise AssertionError("dirty checkout admitted")
            assert git(existing, "ls-files", "--stage") == index
            assert git(existing, "status", "--porcelain") == original
            assert (existing / "scratch.txt").read_text() == "private\n"
            git(existing, "commit", "-m", "Land developer work separately")
            (existing / "scratch.txt").unlink()
            stale = svc.api("/runs/" + run["id"], {"kind": "land", "candidate": candidate["id"]})
            assert stale["state"] == "evaluating" and stale["candidates"][0]["state"] == "stale"
            ready = svc.wait_state(run, "review")
            passing = next(c for c in ready["candidates"] if c["state"] == "passed")
            assert passing["id"] != candidate["id"]
            svc.api("/runs/" + run["id"], {"kind": "land", "candidate": passing["id"]})
            assert git(existing, "rev-parse", "HEAD") == passing["commit"]
            assert (existing / "README.md").read_text() == "staged\n"
            delegated = repo(root / "delegated")
            run = svc.api("/runs", {"path": str(delegated), "prompt": "Delegate the report",
                                    "coordinator": "first", "workers": ["writer"], "delivery": "auto",
                                    "recipe": {"checks": [["python3", "-c",
                                        "from pathlib import Path; assert Path('docs/report.txt').read_text() == 'reviewed both sections'"]]}})
            finished = svc.wait_state(run, "landed")
            children = [task for task in finished["tasks"] if task.get("parent") == "report"]
            assert len(children) == 2 and all(task["paths"] == ["docs"] for task in children)
            assert sum(turn["role"] == "worker" and turn["task"] == "report" for turn in finished["turns"]) == 2
            for child in children:
                assert (delegated / "docs" / (child["id"] + ".txt")).read_text() == child["id"]

            comparison = repo(root / "comparison")
            run = svc.api("/runs", {"path": str(comparison), "prompt": "Compare approaches", "coordinator": "second", "workers": ["writer", "writer-two"], "token_target": 5})
            svc.wait_state(run, "checks")
            svc.api("/runs/" + run["id"], {"kind": "approve"})
            paused = svc.wait_state(run, "budget")
            assert all(t["role"] != "worker" for t in paused["turns"])
            svc.api("/runs/" + run["id"], {"kind": "resume", "tokens": 0})
            ready = svc.wait_state(run, "review")
            candidates = ready["candidates"]
            assert len(candidates) == 2 and {c["state"] for c in candidates} == {"passed", "failed"}
            assert all(len(c["tasks"]) == 1 and c.get("commit") for c in candidates)
            bad = next(c for c in candidates if c["state"] == "failed")
            try:
                svc.api("/runs/" + run["id"], {"kind": "land", "candidate": bad["id"]})
            except urllib.error.HTTPError:
                pass
            else:
                raise AssertionError("failing alternative admitted")
            good = next(c for c in candidates if c["state"] == "passed")
            svc.api("/runs/" + run["id"], {"kind": "land", "candidate": good["id"]})
            assert (comparison / "result.txt").read_text() == "correct"
        finally:
            svc.stop()
    print("Daemon checks passed for team coordination, bootstrap, recovery, repair, dirty repositories, stale landing and competing candidates.")


if __name__ == "__main__":
    main()
