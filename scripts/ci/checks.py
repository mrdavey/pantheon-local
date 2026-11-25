"""Select Actions checks from the tested Git tree and recent successful steps."""
import hashlib
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

ALL = {"fmt", "lint", "tests", "browser", "package"}
RUST = {"fmt", "lint", "tests"}
WEB = {"browser"}
CODE = RUST | WEB
DOCS = {"README.md", "CONTRIBUTING.md", "SECURITY.md", "AGENTS.md",
        "LICENSE", "NOTICE", ".gitignore", "examples/README.md"}
RECEIPT = re.compile(r"Pass (?:fmt|lint|tests|browser|package) [0-9a-f]{64}")


def affected(path):
    if path in DOCS or path.startswith("docs/"):
        return set()
    if path.startswith("web/"):
        return WEB
    if path in {".github/workflows/release.yml", "scripts/package.sh",
                "scripts/open-pantheon-local.command", "scripts/ci/verify-package.sh"}:
        return {"package"}
    if path.startswith(("tests/browser/", "tests/workbench/", "tests/common/")):
        return CODE
    if path.startswith("tests/") and path != "tests/workbench.rs":
        return RUST
    if path.startswith(("src/", "examples/")) or path == "tests/workbench.rs":
        return CODE if path.endswith(".rs") else CODE - {"fmt"}
    # New build configuration and unclassified paths get full coverage.
    return ALL


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


def plan(base):
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
    test_target = ""
    checks = set(ALL)
    if run.get("conclusion") == "success":
        diff = subprocess.run(["git", "diff", "--no-renames", "--name-only", "-z",
                               base, "HEAD", "--"], capture_output=True)
        if diff.returncode == 0:
            paths = [os.fsdecode(p) for p in diff.stdout.split(b"\0") if p]
            checks = set().union(*(affected(path) for path in paths))
            test_paths = [Path(path) for path in paths if "tests" in affected(path)]
            if len(test_paths) == 1:
                path = test_paths[0]
                if (path.parent == Path("tests") and re.fullmatch(r"[\w-]+\.rs", path.name)
                        and path.is_file()):
                    test_target = path.stem
    if release:
        checks.discard("package")  # The release verifies its own archives.
    return sorted(checks), test_target


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
             ".github/workflows/ci.yml", "scripts/ci/checks.py"],
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


def inputs(path):
    # Paths that supply bytes to a check may be broader than the changes that
    # warrant running it. The CLI embeds web/dist and release archives ship docs.
    checks = set(affected(path))
    if path.startswith("web/") or checks == ALL:
        checks.add("assets")
    if path.startswith("web/"):
        checks.update({"tests", "lint"})
    if (path.startswith(("src/", "examples/", "web/", "docs/",
                        "tests/common/", "tests/browser/"))
            or path in {"tests/e2e.rs", "tests/experiments.rs", "README.md",
                        "CONTRIBUTING.md", "SECURITY.md", "LICENSE", "NOTICE"}):
        checks.add("package")
    if path.startswith(("src/", "examples/", "tests/")) and not path.endswith(".rs"):
        checks.discard("fmt")
    return checks


def fingerprints(checks):
    image = os.environ.get("ImageVersion")
    if not image:
        # An unidentified image can only provide evidence for this attempt.
        image = f'run-{os.environ["GITHUB_RUN_ID"]}-{os.environ["GITHUB_RUN_ATTEMPT"]}'
    common = [os.environ["RUNNER_OS"], os.environ.get("ImageOS"), image, os.environ["RUNNER_ARCH"]]
    common.extend(subprocess.check_output(cmd, text=True).strip() for cmd in
                  (["rustc", "-vV"], ["cargo", "-V"], ["git", "--version"]))
    node = [subprocess.check_output([cmd, "--version"], text=True).strip() for cmd in ("node", "npm")]
    hashes = {}
    for check in checks:
        target = os.environ.get("TEST_TARGET", "") if check == "tests" else ""
        runtime = [] if check == "fmt" else node
        hashes[check] = hashlib.sha256(json.dumps([check, target, common, runtime]).encode())
    for entry in git("ls-tree", "-rz", "HEAD").split(b"\0"):
        if entry:
            path = os.fsdecode(entry.split(b"\t", 1)[1])
            for check in inputs(path) & hashes.keys():
                hashes[check].update(entry + b"\0")
    return {check: digest.hexdigest() for check, digest in hashes.items()}


def output(key, value):
    with Path(os.environ["GITHUB_OUTPUT"]).open("a") as stream:
        print(f"{key}={json.dumps(value) if not isinstance(value, str) else value}", file=stream)


if __name__ == "__main__":
    if sys.argv[1] == "plan":
        checks, test_target = plan(sys.argv[2])
        output("test_target", test_target)
        output("checks", checks)
        platforms = ["ubuntu-24.04"] if checks else []
        if set(checks) - {"fmt", "browser"}:
            platforms.append("macos-latest")
        output("platforms", platforms)
        output("history", successful_steps() if checks else [])
        print("Selected checks:", ", ".join(checks) or "none")
    else:
        checks = set(json.loads(os.environ["CHECKS"]))
        if os.environ["RUNNER_OS"] != "Linux":
            checks -= {"fmt", "browser"}
        history = set(json.loads(os.environ["HISTORY"]))
        for name, digest in fingerprints(checks | {"assets"}).items():
            if name == "assets":
                output("assets", f"web-dist-{digest}")
                continue
            key = f"Pass {name} {digest}"
            output(name, key)
            output(f"run_{name}", key not in history)
            print(f"{name}: {'reuse ' + key if key in history else 'run'}")
