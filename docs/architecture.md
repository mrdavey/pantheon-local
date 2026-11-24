# Architecture

One Rust executable contains the CLI, loopback service, turn supervisor and compiled React workbench. The browser and CLI use the same authenticated HTTP API for Git operations, checks and landing. One daemon writes to each SQLite state directory. On startup the browser reads the runs and event cursor in one transaction, then follows numbered events and loads turn logs on demand. Git operations and agent commands run outside the async HTTP executor. A single writer serializes run changes while the UI reads committed state.

## Process and Git lifetimes

Each model turn or check has a durable job directory. Its supervisor owns the process group and writes a final result to disk. After a restart, the daemon consumes each completed result once or records an uncertain outcome. Pausing cancels active process groups. Resuming reuses saved sessions where available.

The scheduler restores active runs from SQLite. It starts every ready milestone in its own Git clone. A worker can use native subagents within its scope and must review and commit their work. The service verifies the reported commit and changed tree, then imports the commit for dependent workers. Uncommitted files stay in the worker’s checkout. Recovery without a reported commit requires a clean checkout.

Each alternative gets a separate candidate. Conflict repairs must preserve both histories and cleanly merged files. A worker combining approaches gets Git refs for its donors. The service pins each candidate before testing it. Landing requires a local fast-forward and a clean target checkout. A moved target requires fresh checks.

## Native adapters

The adapter accepts a role, working directory, prompt, optionally an existing session ID, and a user-configured profile. It returns session and activity events plus a final response. The native harness retains its own account, tools, and context. Coordinator and worker outcomes are parsed as typed JSON. A generation prevents an old coordinator from mutating a run after a switch.

To add an adapter, create an executable that accepts `--version` and `turn-v1`. On a turn, read one JSON object from stdin and stream JSON lines on stdout.

```json
{"profile":{"kind":"custom","executable":"/local/my-adapter"},"role":"worker","dir":"/local/checkout","prompt":"Implement the milestone","session":"previous-id"}
```

```json
{"event":{"type":"session","session":"session-id"}}
{"event":{"type":"activity","text":"Checking the diff"}}
{"result":{"session":"session-id","text":"{\"status\":\"committed\",\"commit\":\"actual-git-sha\"}","tokens":1024}}
```

Omit usage when a harness has no reliable receipt. Use the result's `error` field for a failed turn. Process termination is the cancellation surface. Model profile configuration is local to the developer.

## Worker routing and conversation

Each agent turn receives saved planning answers, the round goal and approved criteria. Workers see the IDs, goals and file scopes of active peers in the same approach. A planning question takes precedence over tasks in the same response. Delegated milestones inherit the parent’s file scope, prerequisites and variant. Their commits return to the parent for integration.

While workers are busy, routine completions share a coordinator turn. Questions, findings, developer input or a full batch trigger review. After review the scheduler creates one scoped inbox item for a worker still awaiting an answer, unless it already has one or is waiting on delegated work. A worker must process guidance received during a turn before its commit can be accepted.

Benchmark-enabled runs retain one snapshot and the reason for each assignment. Conversation uses a separate native session. Accepting a proposal revalidates the milestones against the current graph, adds approved checks and invalidates older passing candidates. [Routing settings and conversation commands](model-routing.md)

## Local execution

Use projects and adapters you trust to execute under your local account. The workbench provides independent checkouts, a scoped environment for evals, and separate browser origins for the workbench and previews. Configure each native worker’s sandbox and permissions for the project it handles. On macOS, the recipe and preview runner use a filesystem sandbox that confines writes to the disposable checkout and scratch directory and restricts reads of the developer checkout, service state, and common harness directories. Linux runs recipes with the local user’s filesystem privileges. The service retains local prompts, logs, and candidate evidence in the state directory. Hosted providers still receive the context sent through their own harness.

## Experiments and delivery

The Rust runtime opens existing SQLite state and supervisor receipts in place. SQLite schema 2 records project identities keyed by Git common directory and revisioned case history. Candidate measurements stay in supervised job receipts on disk. Current and earlier decisions remain associated with the tested Git commit and input fingerprint. The coordinator turns outcome observations into typed actions. Recording accepted actions with the consumed observation batch prevents replay after restart. Its inbox questions are independent of harness permission requests.

Candidate evaluation pins an independent driver and runs baseline and variants in separate checkouts. The same supervisor records structured observations and checks the checkout before and after the run. Measurements use one candidate at a time on the daemon. The evaluator trust boundary depends on local filesystem and native harness permissions. Keep untrusted repositories and eval executables off this service account.

Landing advances the local Git branch to the tested candidate. The daemon serializes writes and verifies the expected target. Auto-land uses the same Git checks. **Auto-land and deploy** additionally requires a configured release adapter. The adapter builds and verifies an artifact, then promotes its exact digest. The daemon journals each attempt and checks the destination after deployment or rollback. [Experiment and release adapter contracts](experiments.md)
