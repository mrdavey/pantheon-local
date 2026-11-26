# Add an agent adapter

A custom executable is the shortest way to connect another coding agent. Register it in the state directory’s `profiles.json` under a unique profile name. The executable must be runnable by the local user.

```json
{"profiles":{"my-agent":{"kind":"custom","executable":"/absolute/path/to/my-adapter"}}}
```

1. Implement `--version` and `turn-v1`. Read one JSON request from standard input and write newline-delimited JSON to standard output. See the [request, events, and result examples](architecture.md#native-adapters) and [fixture agent](../examples/fixture-agent/main.rs).
2. Use the supplied working directory and role. Pass the prompt to the native harness with its normal account, skill discovery, and permissions. Return a session identifier when the harness supports resuming. Workers must return the commit they actually created.
3. Stream public progress events and emit one final result. Report provider errors and handle cancellation of the process group. Include token and cost fields only when the provider supplies reliable measurements. Document which model settings and delegation controls the harness supports.
4. Open **Agent setup**, choose **Check again**, and exercise the roles you support in a disposable project. Cover normal completion, missing credentials, failure, cancellation, and resume with deterministic fixtures. Keep optional native account tests separate.

For a built-in adapter, wire the harness into [profile validation and process dispatch](../src/adapter.rs), [model discovery](../src/adapter/models.rs), the compatibility checks, and [Agent setup](../web/src/setup.jsx). Add its installation and account instructions, supported platforms, and an [example profile](../examples/profiles.json). Update the CI target dependencies when the integration introduces new build or test inputs. Run the [contributor checks](../CONTRIBUTING.md).

## Open an integration pull request

For a suggestion, give the agent name and what you would use it for. Installation details, implementation plans, and test results are optional. GitHub needs a change to compare when opening a PR, so a branch with a short Markdown proposal is enough.

For an implemented adapter, push the changes to a topic branch or fork. In **Agent setup → Set up new agent**, choose **Request new agent integration** and select that branch on GitHub. Fill in the parts of the [adapter PR template](../.github/PULL_REQUEST_TEMPLATE/agent_adapter.md) that apply. For a direct PR form, substitute your fork owner and branch in `https://github.com/mrdavey/pantheon-local/compare/main...OWNER:BRANCH?quick_pull=1&template=agent_adapter.md`.
