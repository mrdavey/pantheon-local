# Team coordination and worker routing

## Run a team

Choose a coordinator and the workers you want to use. Describe the outcome, how you’ll test it, and what you want to improve. For example, “Make quote search faster on our existing queries. Exact title matches must still rank first.”

The coordinator asks before assigning work if a missing decision would change the implementation or its checks. Planning answers survive a restart or coordinator change. Once you approve the checks, workers can start independent milestones in parallel.

A worker can ask the coordinator or send a finding to a peer. If your brief requires consultation before implementation, the worker gets a separate turn to ask. The coordinator’s answer stays in the milestone notes. Only questions needing your decision appear in **Needs you**. Unrelated work continues.

The coordinator reviews completed milestones in batches and can assign a correction before the combined checks begin.

## Talk to the coordinator

Open **Coordinator chat** to ask about a decision or request more work. **Explain this** in milestone or implementation details asks about that item and its available changes. Replies appear as the coordinator writes. Enter sends your message. Shift+Enter starts a new line. While a run is working or ready for review, **Add milestones** accepts a proposal and its extra checks. Auto-land waits for the conversation and proposal to be resolved. The updated candidate must pass all approved checks. After landing, use **Start another round** in **Experiment**.

From the CLI, use `swarm chat --text 'Why was this worker chosen?' <run-id>`, then `swarm status <run-id>`. Accept a proposal with `swarm accept-proposal --task <proposal-id> <run-id>`.

## Choose how to assign workers

The coordinator assigns workers by default. In **Agent setup → Settings**, Pantheon reads your local Codex, Claude Code, Hermes or Antigravity preferences and prefills the model, agent and reasoning fields it can identify. Antigravity reports a model label. Codex can also supply a display name and default effort from its local model catalog. Account-selected defaults may be unknown.

A match names the local or cached benchmark suite. You can override any benchmark field or set the runtime **Model ID** for Codex and Claude Code. **Use detected settings** and save to follow the native defaults again. Hermes and Antigravity runtime choices stay in the native agent.

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
