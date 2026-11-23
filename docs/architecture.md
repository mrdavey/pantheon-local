# Architecture

The service owns Git operations, SQLite state, candidate checks, and landing. The browser and CLI contact the same authenticated loopback API. A daemon lock permits one writer per state directory. The launcher either starts the service or authenticates to the existing loopback session before opening it in the browser. First-launch checks call native local login status commands for Codex and Claude Code and report executable discovery for the other adapters. Each transaction records the updated run and a numbered event. On opening the workbench, the browser reads current runs and their event cursor in one SQLite transaction. It then requests events after that cursor and fetches turn logs on demand.

## Process and Git lifetimes

Each model turn or check gets a durable job directory before the daemon launches its supervisor. The supervisor owns a process group and writes events and a final result to disk. After a daemon restart, the service reconciles receipts or records an uncertain outcome. The daemon consumes a completed receipt once. Stops cancel active process groups, and subsequent resumes reuse saved sessions where available.

The scheduler rebuilds its list of active runs from SQLite after restart. A stopped run with no outstanding job, or a landed run with no pending work, stays off the polling loop until a committed change wakes it. The scheduler starts all ready milestones without a per-run worker limit. Each milestone gets its own clone and Git refs from the assigned base. A worker can delegate within that milestone through its harness and is responsible for waiting, reviewing, and committing the result. The harness controls child sessions and their limits. Its milestone becomes reusable when the service verifies the reported SHA, confirms a changed tree, and imports the SHA into a daemon-owned ref. The service retains uncommitted files in the worker’s checkout and integrates the reported commit. Recovery without a report uses a clean checkout. A dependent worker receives imported commits in its checkout. Integration builds separate candidates for alternatives. A conflict repair must preserve both incoming histories and the files Git already merged cleanly. Workers borrowing from another implementation receive its full local Git ref. Within a candidate, the service combines relevant commits and pins the result before launching checks. After all checks pass, the developer can review that revision and request a local fast-forward. A changed target forces a newly tested candidate. A dirty checkout remains available for the developer’s own work.

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

The service captures one benchmark snapshot per run. The coordinator assigns a category and fallback worker to each milestone. The service can route it to an already-selected worker when it finds comparable evidence, then saves the reason and provenance with the milestone. Refreshes apply to subsequent runs.

Conversation uses a separate native session under the selected coordinator and shares the run's durable event history and token accounting. Replies can propose additional milestones. A human approval revalidates their IDs, dependencies, and workers against the current graph, adds any approved checks, and invalidates older passing candidates.

## Local execution

Use projects and adapters you trust to execute under your local account. The workbench provides independent checkouts, a scoped environment for evals, and separate browser origins for the workbench and previews. Configure each native worker’s sandbox and permissions for the project it handles. On macOS, the recipe and preview runner use a filesystem sandbox that confines writes to the disposable checkout and scratch directory and restricts reads of the developer checkout, service state, and common harness directories. Linux runs recipes with the local user’s filesystem privileges. The service retains local prompts, logs, and candidate evidence in the state directory. Hosted providers still receive the context sent through their own harness.

## Experiments and delivery

SQLite schema 2 adds project identities keyed by Git common directory and revisioned case history. Candidate measurements stay in supervised job receipts on disk. Current and earlier decisions remain associated with the tested Git commit and input fingerprint. The coordinator turns outcome observations into typed actions. Recording accepted actions with the consumed observation batch prevents replay after restart. Its inbox questions are independent of harness permission requests.

Candidate evaluation pins an independent driver and runs baseline and variants in separate checkouts. The same supervisor records structured observations and checks the checkout before and after the run. Measurements use one candidate at a time on the daemon. The evaluator trust boundary depends on local filesystem and native harness permissions. Keep untrusted repositories and eval executables off this service account.

Git landing uses the platform installation. The daemon serializes writes and verifies the expected target before landing. Automatic delivery uses the selected candidate and the explicitly configured release adapter. The adapter builds and verifies an artifact, then promotes its exact digest. The daemon journals each attempt and checks the destination after deployment or rollback. Existing CI stays available. See the [experiment guide](experiments.md) for recipe and adapter contracts.
