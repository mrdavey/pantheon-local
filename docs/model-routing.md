# Model routing and coordinator conversation

## Run a team

Select one coordinator and the worker profiles you want to use. Give the coordinator an outcome people can observe. For example, “Make quote search faster. Keep exact title matches first and tell me how latency changes on our existing queries.”

The coordinator looks for gaps that would change the work or its tests. If a choice matters, it asks before assigning milestones. Your planning answers travel with the run, including after a restart or coordinator change. Review the proposed checks before workers start.

During the run, a worker can ask the coordinator or send a finding to a peer milestone. The coordinator can answer from agreed decisions and pass an interface change to the workers that need it. Guidance reaches a worker on its next turn. If a question still needs you, answer it in **Needs you**. Only the affected work waits.

## Choose how to assign workers

The coordinator chooses workers by default. Benchmark routing is off until you opt in for a run.

1. In **Agent setup**, open **Model and benchmark** for each worker profile you want to compare. Use the model and harness you actually run. For Codex and Claude Code, **Model ID** is passed to the native CLI. For Hermes and Antigravity, choose the runtime model in the native agent.
2. If the CLI model ID differs from the name in the leaderboard, enter **Model name on the benchmark**. Add an effort label for a profile tied to a particular reasoning effort. Enter the benchmark’s agent name if it uses a different label for your harness.
3. Start a run with at least two workers. Select a benchmark mode under **Worker routing**. On a milestone, open **Why this model?** to see the decision, score, benchmark cost, and source.

The coordinator creates the milestones and supplies a task category and fallback worker. Pantheon Local compares selected profiles using the latest matching rows available in the run's benchmark snapshot.

| Routing setting | How it chooses |
| --- | --- |
| Balance benchmark score and cost | Choose the lowest cost per trial within five percentage points of the best score. |
| Highest benchmark score | Choose the highest score and use cost to break ties where available. |
| Favor lower cost | Choose the lowest cost per trial within fifteen percentage points of the best score. |
| Coordinator chooses (default) | Use the coordinator's assignment with benchmark routing off. |

For implementation, debugging, testing, and automation, the built-in source is the [official Terminal-Bench 4.1 leaderboard](https://hub.harborframework.com/datasets/terminal-bench/terminal-bench/5?tab=leaderboard). Pantheon Local can reuse a snapshot for 24 hours and refreshes it when a benchmark-enabled run needs newer data. If the feed is down, a snapshot from the last seven days can be used. The chosen snapshot stays with the run. Published results estimate how a model and harness performed under benchmark conditions. The displayed dollars are the reported mean cost per trial. Your subscription, usage limits, and provider charges are determined by your account.

Profiles need evidence from the same suite and category to be compared. The selected mode also needs usable cost data when it optimizes for cost. With too little evidence, Pantheon Local uses the coordinator's fallback and shows why. A model set to the native CLI default has an unknown identity until you explicitly match it to a benchmark row.

### Use your own eval results

Create `benchmarks.json` in the Pantheon Local state directory. Use `swarm doctor` to find the directory. The file contains a `rows` array, with entries that share a `suite`, `source`, and `class` when you want to compare them. Each entry includes `model`, `agent`, `date` in YYYY-MM-DD format, and `score` on a 0–100 scale. Include `cost_per_trial` in USD for cost-aware ranking and `effort` if relevant. Use an HTTPS link for the source. Pantheon Local prefers evidence specific to the task category when enough profiles match. Each new run filters invalid or expired rows, including cached results. Rows expire after 180 days.

Categories are `implementation`, `debugging`, `testing`, `automation`, `documentation`, and `design`. Use named profiles in `profiles.json` to compare multiple models from the same harness. The optional fields are `benchmark_model`, `benchmark_agent`, and `benchmark_effort`. The CLI supports `swarm start --routing balanced` alongside the usual worker flags.

## Talk to the coordinator

Open a run and choose **Conversation**. Ask why work was split, what a result means, or what the coordinator would add next. The replies use the run's milestone, routing, and candidate state. The conversation remains available when you reopen the run.

If you ask to add work, the coordinator can propose milestones and extra checks. Review the proposal and its commands before choosing **Add milestones**. Added milestones use the existing dependency graph and routing settings. Automatic landing waits while the coordinator answers you and while a proposal awaits your decision. The new candidate must pass the original checks and any approved additions. You can accept changes while a run is working or ready for review. After landing, use **Start another round** or **Improve this version** in the Experiment view.

From the terminal, use `swarm chat --text 'Why was this worker chosen?' <run-id>`. Use `swarm status <run-id>` to see the reply and proposal ID, then `swarm accept-proposal --task <proposal-id> <run-id>` to accept it.
