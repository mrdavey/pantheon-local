"""Cargo target ownership, shared sources, and runtime dependencies used by CI."""
from fnmatch import fnmatchcase
from pathlib import PurePosixPath

LINUX = "ubuntu-24.04"
MAC = "macos-latest"
PLATFORMS = (LINUX, MAC)
BIN = "bin:swarm"
AGENT = "example:fixture-agent"
RELEASE = "example:fixture-release"
GRADER = "example:experiment-grader"

# Files shared across Cargo target directories through mod or #[path].
SHARED = {
    BIN: {"example-helper", "unit-source"},
    "test:supervision": {"example-helper"},
    "test:workbench": {"common", "example-helper", "webdriver"},
    "example:quotes-check": {"webdriver"},
}
# Executables launched by each offline suite. New suites conservatively get all
# examples until their runtime contract is recorded here.
FIXTURES = {
    BIN: {AGENT},
    "test:ci": set(),
    "test:e2e": {AGENT},
    "test:experiments": {AGENT, RELEASE, GRADER},
    "test:model_settings": set(),
    "test:service": {AGENT},
    "test:supervision": {AGENT},
    "test:workbench": {AGENT, RELEASE, GRADER},
}

# Match shared ownership before applying Cargo's test/example directory conventions.
OWNERS = {
    "policy": (".github/workflows/ci.yml", "scripts/ci/checks.py", "scripts/ci/targets.py"),
    "cargo": ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "build.rs", ".cargo/*"),
    "format": (".rustfmt.toml", "rustfmt.toml"),
    "clippy": (".clippy.toml", "clippy.toml"),
    "web-check": ("scripts/ci/check-web.sh",),
    "web": ("web/*", ".npmrc"),
    "mac-package": ("scripts/open-pantheon-local.command",),
    "package": ("scripts/package.sh", "scripts/ci/verify-package.sh", ".github/workflows/release.yml"),
    "unit-source": ("src/run/tests.rs",),
    "unknown": ("src/lib.rs", "src/bin/*"),
    BIN: ("src/*",),
    "example-helper": ("tests/common/example.rs",),
    "common": ("tests/common/*",),
    "webdriver": ("tests/browser/*",),
    "archive-docs": ("LICENSE", "NOTICE", "README.md", "CONTRIBUTING.md", "SECURITY.md", "docs/*"),
}


def cargo_target(path):
    parts = PurePosixPath(path).parts
    if parts[0] not in {"tests", "examples"}:
        return None
    kind = "test" if parts[0] == "tests" else "example"
    if len(parts) == 2 and parts[1].endswith(".rs"):
        return kind + ":" + parts[1][:-3]
    if len(parts) == 3 and parts[2] == "main.rs":
        return kind + ":" + parts[1]
    return None


class Targets:
    def __init__(self, paths):
        self.paths = set(paths)
        self.targets = {t for p in self.paths if (t := cargo_target(p))}
        self.extra_products = "src/lib.rs" in self.paths or any(p.startswith("src/bin/") for p in self.paths)
        self.examples = {t for t in self.targets if t.startswith("example:")}
        self.sources = {}
        self.tests = {}
        self.fixtures = {}
        for target in self.targets | {BIN}:
            default = {"common", "example-helper"} if target.startswith("test:") else set()
            sources = {target, "cargo"} | SHARED.get(target, default)
            self.sources[target] = sources
        for target, sources in self.sources.items():
            if target == "test:native":
                continue  # Signed-in qualification is explicitly opt-in.
            fixtures = FIXTURES.get(target, self.examples if target.startswith("test:") else set())
            self.fixtures[target] = fixtures
            runtime = {BIN} if target.startswith("test:") and target != "test:ci" else set()
            dependencies = set().union(*(self.sources.get(f, {f}) for f in fixtures))
            self.tests[target] = sources | dependencies | runtime

    def owner(self, path):
        p = PurePosixPath(path)
        for owner, patterns in OWNERS.items():
            if any(fnmatchcase(path, pattern) for pattern in patterns):
                return owner
        if target := cargo_target(path):
            return target if target in self.targets else "unknown"
        if len(p.parts) > 2 and p.parts[0] in {"tests", "examples"}:
            prefix = "test:" if p.parts[0] == "tests" else "example:"
            target = prefix + p.parts[1]
            if target in self.targets:
                return target
        if p.parent == PurePosixPath("examples") and p.suffix == ".json":
            return "sample"
        if p.parts[0] == "examples" and p.suffix == ".md":
            return "archive-docs"
        if path == ".gitignore" or p.suffix == ".md" or path.startswith(".github/ISSUE_TEMPLATE/"):
            return "docs"
        return "unknown"

    def select(self, paths):
        owners = {self.owner(p) for p in paths} if paths is not None else {"unknown"}
        if self.extra_products and owners & (self.targets | {BIN, "unit-source", "common", "webdriver", "example-helper"}):
            # Shared src modules can also be compiled into those products.
            owners.add("cargo")
        if owners & {"unknown", "policy"}:
            # Includes manifest-defined targets outside Cargo's default layout.
            return dict(fmt=True, lint=["*"], build=["*"], tests=["*"], browser=True,
                        web=True, package=list(PLATFORMS), samples=["*"])
        plan = {}
        # These files are copied or transformed by name in scripts/package.sh.
        required = {"LICENSE", "NOTICE", "README.md", "CONTRIBUTING.md", "SECURITY.md",
                    "docs/getting-started.md"}
        if (set(paths) - self.paths) & required:
            owners.add("package")
        if ("format" in owners or "rust-toolchain.toml" in paths or "Cargo.toml" in paths
                or any(p.endswith(".rs") and self.owner(p) not in {"web", "archive-docs", "docs"}
                       for p in paths)):
            plan["fmt"] = True
        lint = sorted(t for t, sources in self.sources.items() if sources & owners)
        if lint or "clippy" in owners:
            plan["lint"] = ["*"] if "clippy" in owners or "cargo" in owners else lint
        tests = sorted(t for t, inputs in self.tests.items() if inputs & owners)
        if tests:
            plan["tests"] = ["*"] if "cargo" in owners else tests
        build = sorted({t for t in self.examples if self.sources[t] & owners} | (owners & {BIN}))
        if build or "cargo" in owners:
            plan["build"] = ["*"] if "cargo" in owners else build
        if "test:workbench" in tests or "cargo" in owners:
            plan["browser"] = True
        if owners & {"web", "web-check"}:
            plan["web"] = True
        if owners & {"package", "cargo"}:
            plan["package"] = list(PLATFORMS)
        elif "mac-package" in owners:
            plan["package"] = [MAC]
        if "sample" in owners:
            plan["samples"] = sorted(p for p in paths if self.owner(p) == "sample")
        return plan

    def prerequisites(self, plan):
        tests = plan.get("tests", [])
        if "*" in tests or "*" in plan.get("build", []):
            return ["*"]
        consumers = set(tests)
        if plan.get("browser"):
            consumers.add("test:workbench")
        return sorted(set(plan.get("build", []))
                      | set().union(*(self.fixtures.get(t, set()) for t in consumers)))

    def inputs(self, check, scope):
        # Include embedded assets and archive contents when recording evidence,
        # even for files whose own validation runs in another job.
        common = {"policy", "unknown"}
        if check in {"assets", "web"}:
            return common | ({"web", "web-check"} if check == "web" else {"web"})
        if check == "fmt":
            return common | {"format", "cargo"}
        if check == "package":
            launcher = {"mac-package"} if MAC in scope["package"] else set()
            return common | launcher | {"package", "cargo", BIN, "archive-docs", "web",
                             "sample", "common", "example-helper", "webdriver",
                             "test:e2e", "test:experiments"} | self.examples
        groups = self.tests if check in {"tests", "browser"} else self.sources
        targets = ["test:workbench"] if check == "browser" else scope[check]
        if "*" in targets:
            selected = self.examples | {BIN} if check == "build" else self.targets | {BIN}
        else:
            selected = targets
        inputs = set().union(*(groups.get(t, self.sources.get(t, {t, "cargo"})) for t in selected))
        # Build.rs embeds the bundle when the selected commands need the UI.
        if needs_assets(scope) or "package" in scope:
            inputs.add("web")
        if check == "lint":
            inputs.add("clippy")
        # Integration-test executables are compiled against the current CLI.
        if any(t.startswith("test:") for t in selected):
            inputs.add(BIN)
        return common | inputs


def needs_assets(plan):
    return bool(plan.get("browser") or set(plan.get("tests", [])) & {"*", "test:workbench"}
                or ("package" in plan and plan.keys() & {"lint", "build", "tests"}))


def needs_node(plan):
    return bool("package" in plan or needs_assets(plan) or set(plan.get("tests", [])) & {"*", "test:ci"})


def on_platform(plan, os):
    result = dict(plan)
    for name in ("web", "samples"):
        result.pop(name, None)
    if os != LINUX:
        for name in ("fmt", "browser"):
            result.pop(name, None)
    if "package" in result:
        if os in result["package"]:
            result["package"] = [os]
        else:
            del result["package"]
    return result


def cargo_args(targets, check):
    if targets == ["*"]:
        return {"tests": [], "lint": ["--all-targets"], "build": ["--bins", "--examples"]}[check]
    args = []
    for target in targets:
        kind, name = target.split(":", 1)
        args.extend(["--" + kind, name])
    return args
