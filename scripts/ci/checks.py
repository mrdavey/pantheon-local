"""Select Actions checks from the tested Git tree and recent successful steps."""
import hashlib
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

from targets import LINUX, MAC, PLATFORMS, Targets, cargo_args, needs_assets, needs_node, on_platform

RECEIPT = re.compile(r"Pass (?:fmt|lint|tests|browser|web|package|build) [0-9a-f]{64}")


def git(*args):
    return subprocess.check_output(["git", *args])


def api(endpoint):
    repo = os.environ["GITHUB_REPOSITORY"]
    result = subprocess.run(["gh", "api", f"repos/{repo}/actions/{endpoint}"],
                            capture_output=True)
    if result.returncode:
        print("Actions history unavailable. Running checks without that evidence.",
              file=sys.stderr)
        return {}
    return json.loads(result.stdout)


def main_run(base):
    if not base or base == "0" * 40:
        return {}
    runs = api(f"workflows/ci.yml/runs?event=push&branch=main&head_sha={base}"
               "&per_page=1").get("workflow_runs", [])
    return runs[0] if runs else {}


def changed_paths(base):
    diff = git("diff", "--no-renames", "--name-only", "-z", base, "HEAD", "--")
    return [os.fsdecode(p) for p in diff.split(b"\0") if p]


def plan(base, model):
    release = os.environ.get("RELEASE_TAG") == "true"
    if release:
        base = git("rev-parse", "HEAD").decode().strip()
    run = main_run(base)
    if release and run.get("status") in {"queued", "in_progress", "waiting", "pending", "requested"}:
        # Start fallback checks if the main run disappears, fails or stalls.
        print(f'Waiting for main CI run {run["id"]}', flush=True)
        for _ in range(40):
            time.sleep(15)
            run = main_run(base)
            if not run or run.get("status") == "completed":
                break
    verified = run.get("conclusion") == "success"
    if not verified and not release and base and base != "0" * 40:
        # Rapid pushes can reach CI before the previous main run finishes.
        # Include pending changes since the last green ancestor.
        recent = api("workflows/ci.yml/runs?event=push&branch=main&status=success&per_page=100")
        for candidate in recent.get("workflow_runs", []):
            sha = candidate["head_sha"]
            if subprocess.run(["git", "merge-base", "--is-ancestor", sha, base],
                              stderr=subprocess.DEVNULL).returncode == 0:
                base, verified = sha, True
                break
    paths = None
    if verified:
        try:
            paths = changed_paths(base)
        except subprocess.CalledProcessError:
            print("Comparison unavailable. Selecting full coverage.", file=sys.stderr)
    scope = model.select(paths)
    if release:
        scope.pop("package", None)  # Release verifies its own archives on all platforms.
        scope.pop("web", None)
        scope["browser"] = True
    return scope


def successful_steps():
    repo = os.environ["GITHUB_REPOSITORY"]
    recent = api("workflows/ci.yml/runs?per_page=20").get("workflow_runs", [])
    endpoints = []
    # A full rerun can reuse steps from the preceding attempt of this run.
    attempt = int(os.environ.get("GITHUB_RUN_ATTEMPT", "1"))
    if attempt > 1:
        endpoints.append(f'runs/{os.environ["GITHUB_RUN_ID"]}/attempts/{attempt - 1}/jobs')
    for run in recent:
        if (run["status"] != "completed" or not run["head_repository"]
                or run["head_repository"]["full_name"] != repo
                or not (run["event"] == "pull_request"
                        or (run["event"] == "push" and run["head_branch"] == "main"))):
            continue
        # PR workflows can change their own step names. Only accept records
        # from repository branches with the same workflow and selector.
        recipe = subprocess.run(
            ["git", "diff", "--quiet", run["head_sha"], "HEAD", "--",
             ".github/workflows/ci.yml", "scripts/ci/checks.py", "scripts/ci/targets.py"],
            stderr=subprocess.DEVNULL)
        if recipe.returncode == 0:
            endpoints.append(f'runs/{run["id"]}/jobs')
    latest = {}
    for endpoint in endpoints:
        page = api(f"{endpoint}?per_page=100")
        if "jobs" not in page:
            return []
        for job in page["jobs"]:
            for step in job.get("steps", []):
                name = step["name"]
                if RECEIPT.fullmatch(name) and step["conclusion"] != "skipped":
                    result = (step["completed_at"] or "", step["conclusion"] != "success")
                    latest[name] = max(latest.get(name, ("", False)), result)
    return sorted(name for name, (_, failed) in latest.items() if not failed)


def tree():
    entries = {}
    for entry in git("ls-tree", "-rz", "HEAD").split(b"\0"):
        if entry:
            entries[os.fsdecode(entry.split(b"\t", 1)[1])] = entry
    return entries


def fingerprints(scope, model, entries):
    image = os.environ.get("ImageVersion")
    if not image:
        # An unidentified image can only provide evidence for this attempt.
        image = f'run-{os.environ["GITHUB_RUN_ID"]}-{os.environ["GITHUB_RUN_ATTEMPT"]}'
    common = [os.environ["RUNNER_OS"], os.environ.get("ImageOS"), image, os.environ["RUNNER_ARCH"]]
    common.append(subprocess.check_output(["git", "--version"], text=True).strip())
    rust = []
    if scope.keys() - {"assets", "web"}:
        rust = [subprocess.check_output(cmd, text=True).strip()
                for cmd in (["rustc", "-vV"], ["cargo", "-V"])]
    owners = {path: model.owner(path) for path in entries}
    hashes = {}
    for check in scope:
        inputs = model.inputs(check, scope)
        runtime = list(rust) if check not in {"assets", "web"} else []
        if "web" in inputs or (check == "tests" and ("test:ci" in inputs or "*" in scope[check])):
            runtime.extend(subprocess.check_output([cmd, "--version"], text=True).strip()
                           for cmd in ("node", "npm"))
        if check == "web":
            runtime.append(subprocess.check_output([os.environ.get("CHROME_BIN", "google-chrome"),
                                                    "--version"], text=True).strip())
        digest = hashlib.sha256(json.dumps([check, scope[check], common, runtime]).encode())
        for path, entry in entries.items():
            if owners[path] in inputs or (check == "fmt" and path.endswith(".rs")):
                digest.update(entry + b"\0")
        hashes[check] = digest.hexdigest()
    return hashes


def output(key, value):
    with Path(os.environ["GITHUB_OUTPUT"]).open("a") as stream:
        print(f"{key}={json.dumps(value) if not isinstance(value, str) else value}", file=stream)


if __name__ == "__main__":
    mode = sys.argv[1]
    if mode in {"lint", "build", "tests"}:
        targets = json.loads(os.environ["BUILD_TARGETS"] if mode == "build" else os.environ["SCOPE"])
        if mode != "build":
            targets = targets[mode]
        command = {"lint": "clippy", "build": "build", "tests": "test"}[mode]
        args = ["cargo", command, "--locked", *cargo_args(targets, mode)]
        if mode == "lint":
            args.extend(["--", "-D", "warnings"])
        print("Running", *args, flush=True)
        subprocess.run(args, check=True)
        if mode == "lint" and targets != ["*"]:
            # Cargo's test profile checks cfg(test) code in the named binary/examples.
            harnesses = [t for t in targets if not t.startswith("test:")]
            if harnesses:
                subprocess.run(["cargo", "clippy", "--locked", "--profile", "test",
                                *cargo_args(harnesses, mode), "--", "-D", "warnings"], check=True)
        sys.exit(0)

    entries = tree()
    model = Targets(entries)
    if mode == "explain":
        scope = model.select(changed_paths(sys.argv[2]))
        print(json.dumps({"checks": scope, "fixtures_and_builds": model.prerequisites(scope),
                          "platforms": [p for p in PLATFORMS if on_platform(scope, p)]},
                         indent=2, sort_keys=True))
    elif mode == "plan":
        scope = plan(sys.argv[2], model)
        samples = scope.pop("samples", [])
        if samples == ["*"]:
            samples = [p for p in entries if model.owner(p) == "sample"]
        for path in samples:
            if path in entries:
                json.loads(Path(path).read_bytes())
                print("Validated", path)
        output("scope", scope)
        output("checks", sorted(scope))
        output("platforms", [label for label in PLATFORMS if on_platform(scope, label)])
        output("node_platforms", [label for label in PLATFORMS if needs_node(on_platform(scope, label))])
        output("history", successful_steps() if scope else [])
        print("Selected checks:", json.dumps(scope, sort_keys=True))
    else:
        scope = json.loads(os.environ["SCOPE"])
        if mode == "web":
            scope = {"web": scope["web"]}
        else:
            os_label = LINUX if os.environ["RUNNER_OS"] == "Linux" else MAC
            scope = on_platform(scope, os_label)
            if needs_assets(scope):
                scope["assets"] = True
        history = set(json.loads(os.environ["HISTORY"]))
        pending = {}
        for name, digest in fingerprints(scope, model, entries).items():
            if name == "assets":
                output("assets", f"web-dist-{digest}")
                continue
            key = f"Pass {name} {digest}"
            output(name, key)
            output(f"run_{name}", key not in history)
            if key not in history:
                pending[name] = scope[name]
            print(f"{name}: {'reuse ' + key if key in history else 'run'}")
        if mode != "web":
            output("build_targets", model.prerequisites(pending))
            # Keep the cached input identity consistent even when other steps reuse results.
            output("need_assets", needs_assets(scope) and bool(pending.keys() & {"lint", "tests", "build", "browser"}))
