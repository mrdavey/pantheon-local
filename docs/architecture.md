# Architecture

One Rust executable contains the CLI, loopback service, turn supervisor and compiled React workbench. The browser and CLI use the same authenticated HTTP API for Git operations, checks and landing. One daemon writes to each SQLite state directory. If run setup returns an error, it discards the new run’s private files. On startup the browser reads the runs and event cursor in one transaction, then follows numbered events and loads turn logs on demand. Git operations and agent commands run outside the async HTTP executor. A single writer serializes run changes while the UI reads committed state. Deleting a stopped or landed run replaces its event history with a deletion notice for connected clients. Startup retries cleanup of its private files.

## Process and Git lifetimes

Each model turn or check has a durable job directory. Its supervisor owns the process group and writes a final result to disk. After a restart, the daemon consumes each completed result once or records an uncertain outcome. Pausing cancels active process groups. Resuming reuses saved sessions where available.

The scheduler restores active runs from SQLite. It starts every ready milestone in its own Git clone. A finite subagent limit counts running child milestones across the run, including after a daemon restart. Uncapped workers can use native subagents within their scope. The parent reviews and integrates the result. The service resolves a full or unique abbreviated commit ID, verifies the changed tree, then imports the commit for dependent workers. If a sandbox prevents a Git commit, a worker can return `ready` with an exact list of reviewed files. The service checks the milestone scope and commits those files in its private checkout. Other uncommitted files stay there. Merge repairs need a commit with both parents. Recovery without a reported commit requires a clean checkout. Repair prompts include the last 12,000 bytes of the check log. The full log stays in the job directory.

Each alternative gets a separate candidate. Conflict repairs must preserve both histories and cleanly merged files. A worker combining approaches gets Git refs for its donors. The service pins each candidate before testing it. Landing requires a local fast-forward and a clean target checkout. A moved target requires fresh checks. If the target is unavailable during landing, the scheduler records the error once and checks again. After an interrupted landing it verifies the branch before recording success or returning the decision to you.

## Native adapters

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

On chat turns, a custom adapter can stream the JSON response in `agent_message_chunk` events with a `text` delta. Keep the final typed result. The workbench displays only the answer in the top-level `reply` field. Codex chat uses its local app-server, Claude uses partial messages, and Hermes streams ACP message updates.

Omit usage when a harness has no reliable receipt. Return a failed turn in the result’s `error` field. The supervisor cancels a turn by stopping its process group.

The service saves brief attachments under the private run directory, alongside its other inputs. Agents receive the paths and text excerpts with their run context. Initial imports are separate and become part of the project’s first commit.

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
