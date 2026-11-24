# Open Pantheon Local on your computer

## Open the workbench

On an Apple Silicon Mac, extract the archive and double-click **Open Pantheon Local.command**, or run `./swarm open` from the extracted folder. Keep Terminal open while you work. If the browser doesn't open, use the link printed in Terminal. [Download the macOS archive](https://github.com/mrdavey/pantheon-local/releases/latest)

On Linux or an Intel Mac, follow the source build instructions. [Build from source](../README.md#build-from-source)

## Connect an agent

**Agent setup** checks for Git and lists local agent profiles. One configured CLI can coordinate and run several worker sessions at once, within your provider’s limits. Workers can also delegate within a milestone if their CLI supports subagents. Use **Check again** after installing Git or signing in.

| Agent | How to connect |
| --- | --- |
| Codex | [Install the CLI](https://developers.openai.com/codex/cli/). In Terminal, use `codex login`, then select **Check again** in Pantheon Local. |
| Claude Code | [Install the CLI](https://code.claude.com/docs/en/quickstart). In Terminal, use `claude auth login`, then select **Check again**. |
| Hermes | [Install Hermes](https://hermes-agent.nousresearch.com/docs/getting-started/quickstart). In Terminal, run `hermes model`. Select your provider and model, then finish setup in Hermes. Choose the Hermes profile in Pantheon Local. |
| Antigravity | [Install the CLI](https://antigravity.google/docs/cli-getting-started) and run `agy` in Terminal. Finish its setup and sign in there. |

For Hermes, Antigravity and custom adapters, **Installed** means Pantheon Local found the executable. Confirm provider access in the CLI. [Agent testing status](../README.md#what-runs-where)

## Start and review your first run

1. In **Agent setup**, select **Use** beside your agent profile to fill both roles. You can change the coordinator or workers before starting.
2. Enter the full path to an empty folder for a new project, or to an existing Git checkout. Workers start from a commit. Commit any edits you want them to use. Other local edits stay in your checkout.
3. Give the run a goal you can check. For a first run in an empty folder, try “Create `hello.txt` containing the word `hello` followed by a single newline.” Use **Improve brief** if you want help refining the goal, then select **Start run**.
4. Review the coordinator’s proposed checks. For the example, the check should read the file and verify its exact contents. Use **View or edit JSON** to change the recipe, then select **Approve checks**.
5. In **Work**, open **View checks** and **View diff**. Use **Start preview** if the recipe includes a web preview. Choose **Request change** for another revision or land the passing commit on the named branch. A checked-out target must be clean to land. The workbench shows the landed commit.

## While a run is open

- **Needs you** collects open questions across runs. Use **Conversation** to ask about a decision or request more work. [Coordinator conversation and worker routing](model-routing.md)
- **Pause run** interrupts active work. You can answer a planning question while paused. Select **Resume** to continue. An interrupted check runs again.
- Closing the tab leaves the service running. Open the launcher again to reconnect. To stop the service, press Control-C in its Terminal window.
- For a comparison, enable **Compare approaches against a baseline** in **Run options** before starting. Open **Experiment** to review the results. [Experiments and delivery](experiments.md)

## If setup gets stuck

- **Git is missing.** Install Git and run `git --version` in Terminal. For an initial Git commit, configure your Git name and email if Git prompts for them.
- **An installed agent is missing.** Run its version command in Terminal, then run `./swarm open` from that same terminal. This uses the PATH from your shell.
- **An agent turn fails after setup.** Open **Activity** and **View log** in the run. Check sign-in and provider access in the native CLI.
- **You want named model profiles.** Run `./swarm doctor` to find the local `profiles.json` path. Use the sample at `examples/profiles.json` as a starting point. Removing a custom override restores agent discovery with the built-in profiles. Open **Agent setup** again after editing the file.
