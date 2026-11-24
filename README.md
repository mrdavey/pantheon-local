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

### Start a run

1. Configure your agent in its CLI or Hermes Desktop. In **Agent setup**, select a profile. On the run form, choose a coordinator and workers. One CLI can fill both roles.
2. Browse to a project folder and enter a goal, then select **Start run**.
3. Answer any planning questions, approve the checks, and review the passing candidate before landing it on your branch. [First run and troubleshooting](docs/getting-started.md)

## What runs where

The service stores Git checkouts, logs and results on your computer. Each agent uses its own tools, permissions and provider account. Hosted providers receive the context their CLIs send. Use trusted projects and scripts. [Architecture and adapter contract](docs/architecture.md)

## Contribute

[Development checks and bug reports](CONTRIBUTING.md) · [Apache-2.0 license](LICENSE) · [Notice](NOTICE)
