#!/usr/bin/env python3
"""Exercise measured selection, the coordinator inbox, and release recovery."""
import json
import os
import tempfile
from pathlib import Path

from e2e import REPO, Service, git, repo, until


def experiment_recipe():
    return {
        "checks": [["true"]],
        "cases": [{"id": "search", "driver": str(REPO / "tests" / "experiment-grader.py"),
                   "interpreter": "python3", "samples": 3, "required": True, "holdback": True}],
        "rule": {"metrics": [{"name": "relevance", "unit": "hits", "direction": "higher"},
                             {"name": "latency", "unit": "ms", "direction": "lower"}],
                 "objective": "relevance", "limits": {"latency": 5}, "minimum": 2, "min_samples": 3},
    }


def main():
    with tempfile.TemporaryDirectory(prefix="swarm-experiments-") as temp:
        root = Path(temp)
        live = root / "live.json"
        live.write_text(json.dumps({"digest": "sha256:previous", "healthy": True}))
        os.environ["SWARM_TEST_LIVE"] = str(live)
        svc = Service(root / "state")
        try:
            (svc.state / "destinations.json").write_text(json.dumps({
                "demo": {"executable": str(REPO / "tests" / "fixture-release.py"),
                         "environment": ["SWARM_TEST_LIVE"], "rollback": True},
            }))
            assert svc.api("/destinations") == ["demo"]

            def start(path, **options):
                run = svc.api("/runs", {
                    "path": str(path), "prompt": "Measure approaches", "coordinator": "first",
                    "workers": ["writer", "writer-two"], "compare": True,
                    "recipe": experiment_recipe(), **options,
                })
                run["url"] = "/runs/" + run["id"]
                return run

            project = repo(root / "notes")
            run = start(project, delivery="auto")
            landed = svc.wait_state(run, "landed")
            chosen = landed["selection"]["candidate"]
            best = next(c for c in landed["candidates"] if c["id"] == chosen)
            loser = next(c for c in landed["candidates"] if c["name"] == "fast")
            assert best["name"] == "balanced", landed
            assert best["eligible"] and loser["state"] == "passed" and not loser["eligible"]
            assert git(project, "rev-parse", "HEAD") == best["commit"]
            evidence = svc.api(run["url"] + "/evidence/" + chosen)
            assert len(evidence["samples"]) == 3 and evidence["commit"] == best["commit"]

            commend = {"kind": "feedback", "id": "record-case",
                       "candidate": loser["id"], "text": "Search by title should keep its exact match."}
            svc.api(run["url"], commend)
            svc.api(run["url"], commend)
            question = until(lambda: next(
                (q for q in (svc.api(run["url"]).get("inbox") or []) if q["state"] == "open"), None))
            svc.stop(hard=True)
            svc.start()
            assert len(svc.api(run["url"] + "/cases")) == 1
            svc.api(run["url"], {"kind": "answer-inbox", "task": question["id"], "text": "Yes"})
            svc.api(run["url"], {"kind": "combine", "candidate": chosen,
                                 "donors": [loser["id"]], "text": "Add the faster formatter only."})
            combined = until(lambda: (
                current if (current := svc.api(run["url"]))["state"] == "review" and current["round"] == 2 else None))
            mixed = next(c for c in combined["candidates"] if c["round"] == 2 and not c.get("baseline"))
            assert mixed["parents"] == [chosen, loser["id"]] and mixed["score"]["relevance"]["value"] == 8
            assert combined["past_selections"][-1]["candidate"] == chosen and combined["held"]
            assert git(project, "rev-parse", "HEAD") == best["commit"]

            # A frozen revision is useful across threshold edits, but a change to
            # an input produces a different observation and fingerprint.
            old_reports = {c["name"]: c.get("evidence") for c in combined["candidates"] if c["round"] == 2}
            changed = experiment_recipe()
            changed["rule"]["minimum"] = 0.5
            svc.api(run["url"], {"kind": "update-suite", "recipe": changed})
            reranked = svc.wait_state(run, "review")
            recalc = {c["name"]: c.get("evidence") for c in reranked["candidates"]
                      if c.get("policy") == reranked["policy"] and c["round"] == 2}
            assert recalc and recalc == old_reports, reranked
            changed["cases"][0]["input"] = {"query": "different"}
            svc.api(run["url"], {"kind": "update-suite", "recipe": changed})
            remeasured = svc.wait_state(run, "review")
            fresh = {c["name"]: c.get("evidence") for c in remeasured["candidates"]
                     if c.get("policy") == remeasured["policy"] and c["round"] == 2}
            assert fresh and all(report != old_reports[name] for name, report in fresh.items()), remeasured

            no_gain = experiment_recipe()
            no_gain["rule"]["minimum"] = 101
            run = start(repo(root / "baseline"), delivery="auto", recipe=no_gain)
            current = svc.wait_state(run, "review")
            assert current["selection"]["state"] == "baseline"
            assert not any(c.get("landed_at") for c in current.get("past_selections", []))

            conflict_project = repo(root / "conflict")
            (conflict_project / "overlap.txt").write_text("before")
            git(conflict_project, "add", "overlap.txt")
            git(conflict_project, "commit", "-m", "Shared starting point")
            run = start(conflict_project, prompt="Resolve overlap", delivery="auto",
                        compare=False, recipe={"checks": [["python3", "-c", "from pathlib import Path; assert Path('overlap.txt').read_text() == 'both'"]]})
            svc.wait_state(run, "landed")
            assert (conflict_project / "overlap.txt").read_text() == "both"
            assert (conflict_project / "clean-merge.txt").read_text() == "from right"

            release_repo = repo(root / "release")
            Path(str(live) + ".interrupt").touch()
            run = start(release_repo, delivery="deploy", destination="demo")
            until(lambda: json.loads(live.read_text())["digest"] != "sha256:previous", seconds=35)
            svc.stop(hard=True)
            svc.start()

            def release_state(expected):
                return until(lambda: (
                    entry if (entry := svc.api(run["url"]).get("releases", [{}])[-1]).get("state") == expected
                    else None), seconds=40)

            release = release_state("deployed")
            assert release["digest"] == release["observed"] and release["commit"] == git(release_repo, "rev-parse", "HEAD")
            Path(str(live) + ".interrupt").unlink()

            # A different commit can build the exact artifact already serving users.
            Path(str(live) + ".forbid-promotion").touch()
            same_project = repo(root / "identical-artifact")
            run = start(same_project, delivery="deploy", destination="demo")
            svc.wait_state(run, "landed")
            repeated = until(lambda: (
                entry if (entry := svc.api(run["url"])["releases"][-1])["state"] in {"deployed", "failed"}
                else None), seconds=20)
            assert repeated["state"] == "deployed" and repeated["digest"] == repeated["observed"], repeated
            assert repeated["commit"] == git(same_project, "rev-parse", "HEAD")
            assert not Path(str(live) + ".unexpected-promotion").exists()
            Path(str(live) + ".forbid-promotion").unlink()

            # Restore a known different version so the next run exercises rollback.
            live.write_text(json.dumps({"digest": "sha256:previous", "healthy": True}))
            Path(str(live) + ".unhealthy").touch()
            run = start(repo(root / "rollback"), delivery="deploy", destination="demo")
            rolled = release_state("rolled-back")
            assert rolled["observed"] == "sha256:previous", rolled
        finally:
            svc.stop()
    print("Experiments, durable inbox, post-landing combination, conflict repair and release recovery passed.")


if __name__ == "__main__":
    main()
