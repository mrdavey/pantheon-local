#!/usr/bin/env python3
"""Opt-in qualification with signed-in native coordinator and worker profiles."""
import argparse
import json
import tempfile
from pathlib import Path

from e2e import Service, git, repo, until
from experiments import experiment_recipe


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("profiles", type=Path, nargs="?")
    parser.add_argument("--coordinator", default="codex")
    parser.add_argument("--worker", action="append")
    parser.add_argument("--timeout", type=int, default=240)
    args = parser.parse_args()
    profiles = args.profiles.read_text() if args.profiles else json.dumps({
        "profiles": {"codex": {"kind": "codex"}},
    })
    with tempfile.TemporaryDirectory(prefix="swarm-native-experiment-") as directory:
        root = Path(directory)
        service = Service(root / "state")
        url = ""
        try:
            (service.state / "profiles.json").write_text(profiles)
            project = repo(root / "project")
            grader = root / "grader.py"
            grader.write_text("""import json
from pathlib import Path
p = Path('result.txt')
text = p.read_text().strip() if p.exists() else 'base'
relevance, latency = {'base': (1,2), 'correct': (7,3), 'wrong': (10,8)}[text]
print(json.dumps({'pass':True,'observed':text,'metrics':{'relevance':relevance,'latency':latency}}))
""")
            recipe = experiment_recipe()
            recipe["cases"][0].update(driver=str(grader), samples=1)
            recipe["rule"]["min_samples"] = 1
            run = service.api("/runs", {
                "path": str(project), "coordinator": args.coordinator, "workers": args.worker or ["codex"],
                "compare": True, "delivery": "auto", "recipe": recipe,
                "prompt": (
                    "Run a tiny two-variant coding experiment in this new CLI fixture. Use exactly two independent "
                    "milestones named relevance and throughput. Mark them as separate variants. Relevance must "
                    "create only result.txt containing correct followed by one newline. Throughput must create "
                    "only result.txt containing wrong followed by one newline. Commit both. These are fixture "
                    "labels. The accepted recipe will measure them. All product decisions for this experiment "
                    "are answered. Afterward I will send feedback for the case library."
                ),
            })
            url = "/runs/" + run["id"]
            print("Started native experiment", run["id"], flush=True)

            def landed():
                current = service.api(url)
                if current["state"] in ("landed", "blocked", "question") or current.get("observer_error"):
                    return current
                if current["state"] == "review" and current.get("selection", {}).get("state") == "undecided":
                    return current
                return None

            result = until(landed, seconds=args.timeout)
            if result["state"] != "landed":
                logs = [service.api(url + "/turns/" + t["id"])[-1400:] for t in result["turns"]
                        if t["role"] in ("plan", "observe") and t["state"] == "received"]
                raise AssertionError((result["state"], result.get("observer_error"), result.get("detail"), logs))
            assert (project / "result.txt").read_text() == "correct\n"
            print("Native candidate landed", git(project, "rev-parse", "HEAD"), flush=True)
            service.api(url, {"kind": "feedback", "text": (
                "Please save a draft eval case based on this observation: searching for an exact note title "
                "should rank that note first. Ask one focused product question in the inbox about ambiguous "
                "titles, with no blocking work."
            )})
            result = until(lambda: (r if (r := service.api(url)).get("inbox") or r.get("observer_error") else None),
                           seconds=args.timeout)
            cases = service.api(url + "/cases")
            assert cases and result.get("inbox"), (cases, result.get("observer_error"))
            print("Native coordinator saved", cases[0]["id"], "and opened an inbox question.")
        finally:
            if url:
                current = service.api(url)
                if current["state"] != "landed" and any(t["state"] == "running" for t in current["turns"]):
                    service.api(url, {"kind": "stop"})
            service.stop()


if __name__ == "__main__":
    main()
