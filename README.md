# Pantheon Local

Pantheon Local coordinates command-line coding agents on a local Git project. In the browser, you give the team a goal, answer the coordinator’s questions, and review a tested commit. For a single patch and a review, a direct CLI handoff is usually simpler.

## When to use it

- **Split one change across a team.** A coordinator can assign the API, interface, docs and integration check to four workers in separate checkouts. It clarifies what matters, passes decisions to the workers that need them, and brings unresolved questions to **Needs you**. Independent work continues. [Team coordination](docs/model-routing.md) · [Example briefs](examples/README.md)
- **Choose between implementations.** Test the starting commit and each alternative against the same checks. For search, that might mean finding more relevant results within a latency limit. Review the diff and measurements, keep the baseline, or combine parts of two candidates and test again. [Experiments and delivery](docs/experiments.md)

[Website and screenshots](https://pantheon.talk/local)

## Quick start

### Build from source

On macOS or Linux, install rustup, Node.js 24, Git 2.39 or newer, and a coding-agent CLI. From the repository root, build the React workbench and launch the Rust executable.

```sh
npm --prefix web ci
npm --prefix web run build
cargo build --locked --release
./target/release/swarm open
```

If you have a Pantheon Local archive for your platform, extract it and run `./swarm open` from its folder. On macOS you can also double-click **Open Pantheon Local.command**.

In an interactive terminal, `swarm` and `swarm open` open the browser and show a rotating Pantheon mark while the workbench is online. Keyboard shortcuts appear in bold cyan when terminal colors are enabled. Click the local address or press **Enter** to open the signed-in workbench. Update notices appear below the address and link to the release. Keep the service window open. **Ctrl+C** stops the service in its original window or closes a reconnected view. Use `REDUCE_MOTION=1` for a still mark or `SWARM_PLAIN=1` for plain text. `swarm serve` and redirected output always print the link.

### Start a run

1. Configure your agent in its CLI or Hermes Desktop. Open **New run**, describe the goal, and attach any reference files. Choose a coordinator and workers under **Team**. Open a worker’s settings to change its model or thinking effort, and choose whether to allow or limit subagents.
2. Confirm the project folder (it defaults to the folder where you ran `swarm open`), then select **Start run**.
3. Answer any planning questions, then approve the checks. Follow live assignments and dependencies in **Agent map** and monitor tokens and estimated cost in **Metrics**. Passing work auto-lands on your clean target branch unless you choose **Review before landing** in **Advanced settings**. [First run and troubleshooting](docs/getting-started.md)

## What runs where

The service stores Git checkouts, logs and results on your computer. Each agent uses its own tools, permissions and provider account. Hosted providers receive the context their CLIs send. Use trusted projects and scripts. [Architecture and adapter contract](docs/architecture.md)

## Contribute

[Development checks and bug reports](CONTRIBUTING.md) · [Apache-2.0 license](LICENSE) · [Notice](NOTICE)
