# Architecture

One Rust executable contains the CLI, loopback service, turn supervisor and compiled React workbench. The browser and CLI use the same authenticated HTTP API for Git operations, checks and landing. One daemon writes to each SQLite state directory. If run setup returns an error, it discards the new run’s private files. On startup the browser reads the runs and event cursor in one transaction, then follows numbered events and loads turn logs on demand. Git operations and agent commands run outside the async HTTP executor. A single writer serializes run changes while the UI reads committed state. Deleting a stopped or landed run replaces its event history with a deletion notice for connected clients. Startup retries cleanup of its private files.

The workbench uses query parameters for navigation on the same loopback origin. The browser history tracks pages, run tabs, milestone layouts and detail views. The terminal launch fragment is consumed separately into session storage. Refreshing a route reloads the run snapshot and restores the view without replaying a command.

The foreground launcher draws the Pantheon mark on a terminal's alternate screen. The HTTP server accepts connections independently, and the renderer writes nonblocking frames to its own terminal handle. An OSC 8 hyperlink displays the loopback origin and carries the signed workbench URL in its destination. Enter opens the same URL. Update and diagnostic messages render on the launch screen. A second interactive launcher observes the existing service and closes independently. Headless launches and `swarm serve` use plain output.

The Activity Log indexes turn metadata at `GET /api/runs/{id}/activity`. It accepts `q`, `agent`, `role`, `state`, `sort` (newest or oldest), `page`, and `size`. It returns the matching page and count. Opening a milestone requests its own worker receipts at `GET /api/runs/{id}/milestones/{task}/usage`. Unknown costs and partial usage stay explicit.

The dock and Coordinator chat tab share one conversation and composer. The chat command accepts optional brief-format file uploads, stores bytes in a private per-message directory and sends the coordinator the selected files and text excerpts. The Add task dialog submits a `propose-milestones` command through the same serialized chat queue. A reply references its originating message. Both interfaces use `accept-proposal` to validate dependencies, route owners and incorporate acceptance checks. Runs awaiting check approval retain that decision. Accepting a proposal on a landed run opens a new round.

The activity viewer reads up to 25 messages in reverse append order from `GET /api/runs/{id}/turns/{turn}/log`. The response includes `entries`, the snapshot's `end` offset and an optional `older` cursor. Passing an offset as `before` revisits a stable page while output grows. Live polling sends the previous `end` as `known` to skip rereading unchanged output. The supervisor timestamps agent events as they are recorded. For external command output it records the observation time of each appended byte range in `output.times` at 250 ms intervals and on exit. Older data uses a native event timestamp when available.

A chat command can reference one milestone (`task`), implementation (`candidate`), or activity (`turn`) from the same run. For an activity, the daemon resolves the turn ID and sends its role, agent, status, timing, and up to 12 recent log excerpts to the coordinator. The Activity Log’s coordinator filter includes planning, review, repair, pricing, and conversation turns even when a worker shares the coordinator’s profile.

## Process and Git lifetimes

Each model turn or check has a durable job directory. Its supervisor owns the process group and writes a final result to disk. After a restart, the daemon consumes each completed result once or records an uncertain outcome. Pausing cancels active process groups. Resuming reuses saved sessions where available.

The scheduler restores active runs from SQLite. It starts every ready milestone in its own Git clone. A finite subagent limit counts running child milestones across the run, including after a daemon restart. Uncapped workers can use native subagents within their scope. The parent reviews and integrates the result. The service resolves a full or unique abbreviated commit ID, verifies the changed tree, then imports the commit for dependent workers. If a sandbox prevents a Git commit, a worker can return `ready` with an exact list of reviewed files. The service checks the milestone scope and commits those files in its milestone checkout. Other uncommitted files stay there. Merge repairs need a commit with both parents. Recovery without a reported commit requires a clean checkout. Repair prompts include the last 12,000 bytes of the check log. The full log stays in the job directory.

Each alternative gets a separate candidate. Conflict repairs must preserve both histories and cleanly merged files. A worker combining approaches gets Git refs for its donors. The service pins each candidate before testing it. Landing requires a local fast-forward and a clean target checkout. A moved target requires fresh checks. If the target is unavailable during landing, the scheduler records the error once and checks again. After an interrupted landing it verifies the branch before recording success or returning the decision to you.

## Native adapters

For setup, contribution steps and fixture coverage, see [Add an agent adapter](agent-adapters.md).

The adapter accepts a role, working directory, prompt, optionally an existing session ID, and a user-configured profile. It returns session and activity events plus a final response. The native harness retains its own account, plugins, tools and skill discovery. Pantheon also lists portable SKILL.md files from the assigned directory, checkout root and ~/.agents/skills in the turn context. A project skill takes precedence over a global skill of the same name. The harness reads the files on demand with its existing permissions. Hermes Desktop profiles use the local Hermes runtime over ACP. Pantheon Local passes the profile and assigned checkout to separate sessions for coordinator and worker turns. Hermes workers use the native Accept Edits mode when advertised. Pantheon declines additional permission requests. The adapter reads the final worker receipt after a text summary and folds identical replays. It retains native edit warnings on commit handoffs. The service verifies the commit in Git. Coordinator and worker outcomes are parsed as typed JSON. A worker session that ends with an unreadable receipt gets up to two follow-ups in the same checkout. An explicit blocker is shown immediately. When you switch coordinators, a generation number ensures only the active coordinator can update the run.

To add an adapter, create an executable that accepts `--version` and `turn-v1`. On a turn, read one JSON object from stdin and stream JSON lines on stdout.

```json
{"profile":{"kind":"custom","executable":"/local/my-adapter"},"role":"worker","dir":"/local/checkout","prompt":"Implement the milestone","session":"previous-id"}
```

```json
{"event":{"type":"session","session":"session-id"}}
{"event":{"type":"activity","text":"Checking the diff"}}
{"result":{"session":"session-id","text":"{\"status\":\"committed\",\"commit\":\"actual-git-sha\"}","tokens":1024}}
```

On chat turns, a custom adapter can stream the JSON response in `agent_message_chunk` events with a `text` delta. Keep the final typed result. The workbench displays only the answer in the top-level `reply` field. Codex chat uses its local app-server, Claude uses partial messages, and Hermes streams ACP message updates. Refreshing the workbench reconnects to the saved conversation while the daemon finishes the active turn. After a Codex chat reply, the adapter closes the app-server connection so it can release its writer. If another writer holds the saved thread when a later message resumes it, the adapter forks that history before sending the message and keeps the new thread ID for subsequent replies.

Omit usage when a harness has no reliable receipt. Return a failed turn in the result’s `error` field. The supervisor cancels a turn by stopping its process group.

The service saves brief attachments under the private run directory, alongside its other inputs. Agents receive the paths and text excerpts with their run context. The brief dialog reads attachments by index through the authenticated run API. The browser renders raster images and supported UTF-8 text and provides downloads for other formats. Initial imports are separate and become part of the project’s first commit.

## Worker routing and conversation

Each agent turn receives saved planning answers, the round goal and approved criteria. Workers see the IDs, goals and file scopes of active peers in the same approach. A planning question takes precedence over tasks in the same response. An observer can revise the owner, goal, scope and prerequisites of an unstarted top-level milestone. The service validates the resulting graph and preserves progress if dispatch wins a race with a revision. Delegated milestones inherit the parent’s file scope, prerequisites and variant. Their commits return to the parent for integration.

While workers are busy, routine completions share a coordinator turn. Questions, findings, developer input or a full batch trigger review. After review the scheduler creates one scoped inbox item for a worker still awaiting an answer, unless it already has one or is waiting on delegated work. A worker must process guidance received during a turn before its commit can be accepted.

Benchmark-enabled runs retain one snapshot and the reason for each assignment. Conversation uses a separate native session. Accepting a proposal revalidates the milestones against the current graph, adds approved checks and invalidates older passing candidates. [Routing settings and conversation commands](model-routing.md)

## Local execution

Use projects and adapters you trust to execute under your local account. The workbench provides independent checkouts, a scoped environment for evals, and separate browser origins for the workbench and previews. Configure each native worker’s sandbox and permissions for the project it handles. On macOS, the recipe and preview runner use a filesystem sandbox that confines writes to the disposable checkout and scratch directory and restricts reads of the developer checkout, service state, and common harness directories. Linux runs recipes with the local user’s filesystem privileges. The service retains local prompts, logs, and candidate evidence in the state directory. Each harness sends context to its configured provider.

## Experiments and delivery

SQLite identifies each project by its Git common directory and stores each revision of its evaluation cases. Each measurement report records the candidate’s results, tested commit and input fingerprint on disk. The coordinator turns outcome observations into typed actions. Recording accepted actions with the consumed observation batch prevents replay after restart. Its inbox questions are independent of harness permission requests.

Candidate evaluation pins an independent driver and runs baseline and variants in separate checkouts. The same supervisor records structured observations and checks the checkout before and after the run. Measurements use one candidate at a time on the daemon. The evaluator trust boundary depends on local filesystem and native harness permissions. Keep untrusted repositories and eval executables off this service account.

Landing advances the local Git branch to the tested candidate. The daemon serializes writes and verifies the expected target. Auto-land uses the same Git checks. **Auto-land and deploy** additionally requires a configured release adapter. The adapter builds and verifies an artifact, then promotes its exact digest. The daemon journals each attempt and checks the destination after deployment or rollback. [Experiment and release adapter contracts](experiments.md)

## Run usage

The supervisor records timestamped, cumulative per-turn measurements in a private, atomically replaced `usage.json`. It coalesces receipts within a second and compacts long series. Codex exec can report model-call usage in its local session log while a worker runs. The adapter watches only the matching thread, then reconciles the final CLI counters against the previous receipt for that thread. Completed receipts reconcile live counts. The authenticated `GET /api/runs/{id}/metrics?from=N` returns agent turns, model-level samples and the first turn index to revisit. Command checks and releases are excluded. Older turns fall back to their saved result and available native usage evidence.

Custom adapters can stream `{"event":{"type":"usage","model":"model-id","tokens":120,"cost_usd":0.001}}`. Counters are cumulative for the current turn. Include final totals in `result.tokens` and `result.cost_usd`. Omit unavailable fields. Claude results can identify several models in one turn. ACP context-window occupancy is excluded from consumption, and cumulative session costs require a known starting balance.

Each new run records a working folder under the selected project directory. A self-ignoring local .gitignore keeps milestone clones out of the project index. Evaluation clones, integration state and receipts remain private. The open-folder endpoint accepts `{"working":true}` to open the run's working directory, including the original location for legacy runs.

Itemized usage events can supply `input_tokens` (including cache reads and writes), `cached_input_tokens`, `cache_write_input_tokens` and `output_tokens`. When an itemized receipt lacks a price, a separate coordinator turn looks for published USD rates per million tokens. An authenticated POST to `/api/runs/{id}/metrics/prices` retries missing prices, including for historical runs. The scheduler deduplicates concurrent lookups and respects the run's token target. Coordinator-reported sources and rates are saved per run. Estimates apply only to recorded categories with known rates. A total-only receipt needs a single rate across all categories. Provider-reported costs take precedence.

Milestone details read `GET /api/runs/{id}/milestones/{task}/timing`. Dispatch, receipt and commit acceptance timestamps define elapsed wall time. For older runs, the service reads the first worker job and final result file timestamps. Missing evidence yields an unavailable duration.

The milestone table reads completion times saved on committed tasks. For older runs it uses the last worker turn or batches receipt lookups through the read-only `/api/runs/{id}/milestones/completions` endpoint. Table, Kanban, map, and coordinator proposals resolve dependency links against the complete run history.
