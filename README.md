# Pantheon Local

Run a swarm of coding agents on your computer. Give them a goal, let them work in parallel, and step in when they need a decision. When they try different approaches, use your own checks to decide what ships.

```text
                  YOU
             goal + checks
                   |
              coordinator
             /           \
        worker A       worker B
             \           /
            compare results
                   |
             land what passes
```

[Website and screenshots](https://pantheon.talk/local)

## Quick start

On Apple Silicon, download the [macOS archive](https://github.com/mrdavey/pantheon-local/releases/latest). Extract it and open **Open Pantheon Local.command**. Keep Terminal open while you work. If your browser doesn’t open, use the link printed in Terminal.

Sign in to an agent CLI you already use. In **Agent setup**, choose an agent. One CLI can coordinate and work in separate sessions. Pick an empty folder or an existing Git repo, describe the work, and select **Start run**. Review the proposed checks before approving them.

[Agent setup and troubleshooting](docs/getting-started.md)

### Build from source

Requires Go 1.26+, Node.js 24, Git 2.39+, and a supported agent CLI. Python 3 runs the example checks and test fixtures.

```sh
git clone https://github.com/mrdavey/pantheon-local.git
cd pantheon-local
npm --prefix web ci
npm --prefix web run build
go build -o bin/swarm ./cmd/swarm
./bin/swarm open
```

## Swarm riding

The coordinator turns a brief into milestones and starts the work that can run in parallel. Workers commit in separate Git checkouts. Follow them in the browser and answer questions in **Needs you**. Use **Conversation** to ask why a choice was made or add work.

Say you want better search. Define a few queries that should work and a latency limit. Let workers try different approaches. **Experiment** shows the baseline, the results for each candidate, and what landed. You can keep the baseline or ask the swarm to combine parts of two candidates.

Review before landing is the default. Opt into automatic landing when your checks are ready. Deployment uses an adapter you configure.

- [Experiments and delivery](docs/experiments.md)
- [Worker routing and coordinator conversation](docs/model-routing.md)
- [Example briefs and checks](examples/README.md)

## What runs where

The workbench, Git operations and run history stay on your computer. Your agents use their own accounts and send prompts and project context to their providers as configured. Use projects and scripts you trust.

Codex CLI has passed signed-in runs on macOS. Claude Code and Hermes have fixture coverage, with signed-in multi-agent qualification pending. Antigravity uses the ACP adapter and still needs native qualification. [Architecture and adapter contract](docs/architecture.md)

## Contribute

[Run the checks or open an issue](CONTRIBUTING.md). Apache-2.0 licensed. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
