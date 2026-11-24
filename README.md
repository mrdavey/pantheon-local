# Pantheon Local

Pantheon Local coordinates command-line coding agents on a Git project from a browser on your computer. For a patch from Codex and a review from Claude, a direct CLI handoff is usually simpler.

## When to use it

- **Split one change across a team.** A coordinator can assign the API, interface, docs and integration check to four workers in separate checkouts. It clarifies what matters, passes decisions to the workers that need them, and brings unresolved questions to **Needs you**. Independent work continues. [Team coordination](docs/model-routing.md) · [Example briefs](examples/README.md)
- **Choose between implementations.** Test the starting commit and each alternative against the same checks. For search, that might mean finding more relevant results within a latency limit. Review the diff and measurements, keep the baseline, or combine parts of two candidates and test again. [Experiments and delivery](docs/experiments.md)

[Website and screenshots](https://pantheon.talk/local)

## Quick start

1. On an Apple Silicon Mac, extract the archive and open **Open Pantheon Local.command**. Keep Terminal open. For shared planning answers and direct coordinator messages to workers, build this checkout from source. The v0.2.0 archive predates those changes. [macOS download](https://github.com/mrdavey/pantheon-local/releases/latest)
2. Sign in to your agent CLI and choose it in **Agent setup**. One CLI can fill both coordinator and worker roles.
3. Enter a project folder and goal, then select **Start run**. Approve the checks and review the tested commit before landing it on your local branch. [First run and troubleshooting](docs/getting-started.md)

### Build from source

On macOS or Linux, use Go 1.26+, Node.js 24 and Git 2.39+. Python 3 runs the example checks and test fixtures. From the repository root, run

```sh
npm --prefix web ci
npm --prefix web run build
go build -o bin/swarm ./cmd/swarm
./bin/swarm open
```

## What runs where

Git checkouts, logs and results stay on your computer. Your agents send prompts and project context to their configured providers. No Pantheon account is required. Use projects and scripts you trust.

Codex CLI has completed signed-in runs on macOS. Claude Code and Hermes have simulated adapter tests. Signed-in multi-agent testing for those adapters and native testing for Antigravity are pending. [Architecture and adapter contract](docs/architecture.md)

## Contribute

[Development checks and bug reports](CONTRIBUTING.md) · [Apache-2.0 license](LICENSE) · [Notice](NOTICE)
