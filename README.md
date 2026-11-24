# Pantheon Local

Pantheon Local is a browser workbench for running command-line coding agents on your own Git projects. A coordinator breaks a goal into milestones, and workers commit their changes in separate checkouts.

If Codex writes a patch and Claude reviews the diff, a direct CLI handoff is usually simpler. Pantheon Local is useful when you would otherwise be passing commits between dependent tasks, running the same checks on competing versions, or tracking which revision passed. It handles those steps and shows you the diff and results before you decide what to land.

## When to use it

- **Coordinate a team on one change.** For a quote tracker, a coordinator and four workers can split the API, interface, docs and integration check. Agree on the data format first. The API and interface can then proceed in parallel, and the integration check can verify that accepting a quote updates the balance after a restart. [Example briefs and checks](examples/README.md)
- **Choose between implementations.** To improve search, run the same queries against the current version and each alternative. Compare relevant hits and response time in **Experiment**. Keep the baseline, select a candidate or ask a worker to combine the best parts and test again. [Experiments and delivery](docs/experiments.md)
- **Steer work in progress.** The coordinator can answer worker questions from your brief, earlier answers and approved checks. Product decisions reach **Needs you** while unrelated milestones continue. Use **Conversation** to ask about a decision or add work. [Team coordination and worker routing](docs/model-routing.md)

[Website and screenshots](https://pantheon.talk/local)

## Quick start

1. On an Apple Silicon Mac, extract the release archive and double-click **Open Pantheon Local.command**. Keep Terminal open. On Linux or an Intel Mac, build from source below. [Download the macOS archive](https://github.com/mrdavey/pantheon-local/releases/latest)
2. Sign in to your coding agent in its own CLI. Choose it in **Agent setup**. One CLI can coordinate and work in separate sessions.
3. Enter a project folder and a goal, then select **Start run**. Approve the proposed checks. In **Work**, review the check output and Git diff before landing the commit. Automatic landing is opt-in. Deployment also requires a configured release adapter.

[Agent setup and troubleshooting](docs/getting-started.md)

### Build from source

Build with Go 1.26+, Node.js 24 and Git 2.39+. Running a coding task also requires a configured agent CLI. Python 3 runs the example checks and test fixtures.

```sh
git clone https://github.com/mrdavey/pantheon-local.git
cd pantheon-local
npm --prefix web ci
npm --prefix web run build
go build -o bin/swarm ./cmd/swarm
./bin/swarm open
```

## What runs where

The workbench, Git checkouts, logs and check results live on your computer. No Pantheon account is required. Your agents use their own accounts and send prompts and project context to their configured providers. Use projects and scripts you trust.

Codex CLI has completed signed-in runs on macOS. Automated tests cover the Claude Code and Hermes adapters with simulated agents. Signed-in multi-agent runs for those adapters and native testing for Antigravity are still pending. [Architecture and adapter contract](docs/architecture.md)

## Contribute

[Run the checks or open an issue](CONTRIBUTING.md). Apache-2.0 licensed. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
