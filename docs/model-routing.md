# Team coordination and worker routing

## Run a team

Choose a coordinator and the workers you want to use. Describe the outcome, how you’ll test it, and what you want to improve. For example, “Make quote search faster on our existing queries. Exact title matches must still rank first.”

The coordinator asks before assigning work if a missing decision would change the implementation or its checks. Planning answers survive a restart or coordinator change. Once you approve the checks, workers can start independent milestones in parallel.

A worker can ask the coordinator or send a finding to a peer. If you already chose ISO dates for an export, the coordinator can answer the exporter and tell the verifier which format to check. Each worker receives new guidance on its next turn. Routine completions are grouped for review. A question that still needs your decision appears in **Needs you** while unrelated work continues.

## Talk to the coordinator

Open **Coordinator chat** to ask about a decision or request more work. While a run is working or ready for review, **Add milestones** accepts a proposal and its extra checks. Auto-land waits for the conversation and proposal to be resolved. The updated candidate must pass all approved checks. After landing, use **Start another round** in **Experiment**.

From the CLI, use `swarm chat --text 'Why was this worker chosen?' <run-id>`, then `swarm status <run-id>`. Accept a proposal with `swarm accept-proposal --task <proposal-id> <run-id>`.

## Choose how to assign workers

The coordinator assigns workers by default. For benchmark-based assignments, open **Settings** beside each agent in **Agent setup** and match its leaderboard model, agent and effort. **Model ID** sets the Codex or Claude Code runtime model. Choose the Hermes runtime model in its Desktop profile or CLI. Choose the Antigravity model in its CLI.

Select at least two worker profiles, then choose **Advanced settings → Worker routing** before starting the run. On a milestone, **Why this model?** shows its benchmark evidence. **Why this worker?** explains a fallback.

| Routing setting | Selection |
| --- | --- |
| Balance benchmark score and cost | Lowest cost per trial within 5 percentage points of the best score |
| Highest benchmark score | Highest score, with lower cost breaking a tie when available |
| Favor lower cost | Lowest cost per trial within 15 percentage points of the best score |

For implementation, debugging, testing and automation, Pantheon Local uses Terminal-Bench. Each run keeps its snapshot. The cache refreshes after 24 hours and can be reused for up to seven days if the feed is unavailable. Costs are benchmark averages per trial. If fewer than two profiles have comparable evidence, the coordinator’s assignment stands. [Terminal-Bench leaderboard](https://hub.harborframework.com/datasets/terminal-bench/terminal-bench/5?tab=leaderboard)

### Use your own eval results

Run `swarm doctor` to locate the state directory. Add a `benchmarks.json` file with a `rows` array. Comparable rows share a `suite`, HTTPS `source` and task `class`. Each row needs a `model`, `agent`, `date` (YYYY-MM-DD) and `score` (0–100). Include `cost_per_trial` in USD for cost-aware routing and `effort` when relevant. Rows expire after 180 days.

Supported classes are `implementation`, `debugging`, `testing`, `automation`, `documentation` and `design`. Use named profiles in `profiles.json` for multiple models from the same CLI. Optional matching fields are `benchmark_model`, `benchmark_agent` and `benchmark_effort`. For balanced routing from the CLI, add `--routing balanced` to `swarm start`.
