# Team coordination and worker routing

## Run a team

New runs start with one coordinator and three workers. In **Team**, choose **Add agent**, pick a harness, set the model ID and thinking effort, and select **Add to pool**. You can add several workers using the same harness. To change one, open its **Settings** and choose **Apply to this agent**. The coordinator has its own model settings. Each worker in the pool owns work in the plan. Dependencies still determine which milestones can run concurrently. Describe the outcome, how you’ll test it, and what you want to improve. For example, “Make quote search faster on our existing queries. Exact title matches must still rank first.”

The coordinator asks before assigning work if a missing decision would change the implementation or its checks. Planning answers survive a restart or coordinator change. Once you approve the checks, workers can start independent milestones in parallel.

A worker can ask the coordinator or send a finding to a peer. If your brief requires consultation before implementation, the worker gets a separate turn to ask. The coordinator’s answer stays in the milestone notes. Only questions needing your decision appear in **Needs you**. Unrelated work continues.

The coordinator sees each selected worker’s model and thinking effort. It can restrict a milestone’s benchmark comparison to suitable workers, split independent work across the team, and revise unstarted assignments as findings arrive. It can send guidance to a running worker or add follow-up work within the brief.

Under **New run → Team**, choose **Settings** beside a worker to edit its model and thinking effort for that agent in the new run. Each run keeps the settings it started with. Codex and Claude Code receive model and effort overrides at launch. Hermes and Antigravity use the options their ACP session advertises. An unavailable override produces an error naming the setting.

Turn off **Allow subagents** to keep assignments with the primary team. With **Unlimited subagents**, Pantheon adds no cap and workers may use native delegation within their harness limits. For a shared cap, clear **Unlimited subagents** and enter the maximum concurrent child milestones. Workers can request child assignments from the coordinator. The coordinator also looks for independent child milestones that can shorten delivery or improve verification. The parent integrates and checks the combined result. Ready primary milestones can still run while child slots are full. Codex and Claude native delegation tools are disabled for capped or disabled runs. ACP and custom adapters receive the policy in the prompt and must honor it. The CLI equivalents are `--no-subagents` and `--subagent-limit N`.

Open **Milestones → Map** to see assignments by worker. Lines connect prerequisites to consumers and child results to parents. Select a worker filter to highlight its assignments or select a milestone to trace the dependency chain. The details icon opens the assignment, notes, usage and result. Use **Earlier milestones** to include previous rounds. Drag, scroll or use the arrow keys to pan. The zoom controls and **Fit** change the scale. Native harness work that has no Pantheon milestone stays in the parent’s activity log.

## Talk to the coordinator

**Coordinator chat** opens with the current run status. Use it to ask about a decision or request more work. **Explain this** in milestone or implementation details asks about that item and its available changes. Replies appear as the coordinator writes. Enter sends your message. Shift+Enter starts a new line. **Add to run** accepts a proposal and its extra checks. **Milestones → Add task** uses the same coordinator and approval. Auto-land waits for the conversation and proposal to be resolved. The updated candidate must pass all approved checks. Accepting a proposal after landing starts a new round from the current target.

From the CLI, use `swarm chat --text 'Why was this worker chosen?' <run-id>`, then `swarm status <run-id>`. Accept a proposal with `swarm accept-proposal --task <proposal-id> <run-id>`.

## Choose how to assign workers

The coordinator assigns workers by default. In **Agent setup → Settings**, Pantheon reads your local Codex, Claude Code, Hermes or Antigravity preferences and prefills the model, agent and reasoning fields it can identify. Antigravity reports a model label. Codex can also supply a display name and default effort from its local model catalog. Account-selected defaults may be unknown.

A match names the local or cached benchmark suite. You can override any benchmark field or set the runtime **Model ID** for Codex and Claude Code. **Use detected settings** and save to follow the native defaults again. Hermes and Antigravity accept exact model IDs and reasoning options offered by their ACP session. Clear overrides to follow the native profile.

In **Team → Add agent**, type in **Model ID** to filter suggestions, use the arrow keys and Enter to select one, or enter a custom ID. Codex supplies its catalog through `codex debug models` with a local cache fallback. Antigravity supplies IDs through `agy models`. Hermes uses the selected profile’s provider model cache. Claude Code suggestions use configured model IDs and aliases. Custom adapters control their own model selection.

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

## Monitor run usage

Open **Metrics** in a run to see token counts, estimated USD cost, and turns by agent and model. **Agents working** counts distinct agents with an active turn across the run, including coordinator turns. Usage can be filtered by time range, agent, model, or role. Switch between cumulative usage and usage per interval, or open **Chart data** for exact values. Pause live updates to inspect a point in time. The view refreshes every 2.5 seconds while a turn is active and the tab is open. An agent may report usage during a turn or only when it finishes.

Costs use harness receipts where available. For supported Codex models, Pantheon estimates standard short-context API cost from input, cached input, and output tokens at published rates dated November 25, 2026. Subscription charges, tool fees, cache-write premiums, and service-tier adjustments are outside this estimate. Unreported costs and tokens are shown as unavailable. Older runs use saved turn receipts and their timestamps. [OpenAI API pricing](https://developers.openai.com/api/docs/pricing)

The run API accepts a `worker_pool` array of objects with a configured `profile` and optional `model` and `effort`, and optional `coordinator_settings` with its own model and effort. The service derives the required worker count from the number of entries in the pool. Existing callers can still send `workers` as a list of configured profile names. Each pool entry gets a separate run-local worker ID, visible in assignments and usage. Overrides apply only to that run. Custom adapters manage their own model settings.
