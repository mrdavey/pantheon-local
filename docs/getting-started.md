# Open Pantheon Local on your computer

## Open the workbench

1. Download and extract the [macOS archive](https://github.com/mrdavey/pantheon-local/releases/latest). For other platforms or an Intel Mac, follow [Build from source](../README.md#build-from-source).
2. On a Mac, double-click **Open Pantheon Local.command** in the extracted folder. Your default browser opens the workbench. Keep the Terminal window open while you use it.
3. From a terminal, you can open the same workbench with `./swarm open` from the extracted folder. If the browser does not open, use the link printed in Terminal.

Open the launcher again to reconnect to a running instance. Closing the browser tab leaves your runs visible when you reopen it. Use **Pause run** to pause a run. **Resume** returns to the open question, review, or work in progress. You can answer a planning question in **Needs you** while paused. Work continues with that answer when you resume. A check interrupted by a pause runs again. To stop the local service, press Control-C in its Terminal window.

## Connect an agent

The first screen checks for Git and lists local agent profiles, with usable profiles first. Use **Check again** after installing Git or signing in. One signed-in CLI can coordinate and do the work in separate sessions. You can add more agents later.

| Agent | How to connect |
| --- | --- |
| Codex | [Install the CLI](https://developers.openai.com/codex/cli/). In Terminal, use `codex login`, then select **Check again** in Pantheon Local. |
| Claude Code | [Install the CLI](https://code.claude.com/docs/en/quickstart). In Terminal, use `claude auth login`, then select **Check again**. |
| Hermes | [Install Hermes](https://hermes-agent.nousresearch.com/docs/getting-started/quickstart). In Terminal, run `hermes model`. Select your provider and model, then finish setup in Hermes. Choose the Hermes profile in Pantheon Local. |
| Antigravity | [Install the CLI](https://antigravity.google/docs/cli-getting-started) and run `agy` in Terminal. Finish its setup and sign in there. |

Complete sign-in in the agent’s own CLI. With Hermes, Antigravity, or a custom adapter, the setup screen can confirm that the executable is installed. Use the agent’s own setup to confirm provider access. The listed adapters differ in the amount of native testing they have received. [Agent qualification](../README.md#what-runs-where)

## Start and review your first run

1. Select **Use** for your chosen agent. Pantheon Local fills in the coordinator and worker for the draft run. You can change them before starting.
2. Enter the full path to a folder on this computer. Use an empty folder for a new project or pick an existing Git checkout. On macOS, drag the folder from Finder into Terminal to see its path.
3. Describe what you want. For a quick test, use an empty folder and ask for “a hello.txt file containing hello and a final newline.” Use **Improve brief** if you want help shaping the request.
4. Select **Start run**. Answer any questions and review the acceptance commands. Open **View or edit JSON** to inspect the full recipe or make changes before approving it.
5. Review the checks and diff. If the change has a web preview, select **Start preview**, then open it. The landing button names the target branch. The run shows the exact commit once the update succeeds.

Use **Conversation** to ask the coordinator about a run or propose more work. With multiple workers, **Worker routing** lets you opt into benchmark comparisons. [Model routing and conversation](model-routing.md)

Ready milestones can run in parallel, including multiple sessions with the same worker profile. Workers can use native subagents when their harness supports delegation and their settings allow it. Provider usage limits and the machine's resources still apply. For Codex, see its [subagent settings](https://developers.openai.com/codex/subagents).

For existing projects, commit any edits you want the agents to build on before starting. Use **Agent setup** in the sidebar whenever you install another harness or need to recheck sign-in.

Open **Needs you** for coordinator questions across runs. For comparisons, open **Experiment** to see the selected candidate, browse eval evidence, and request a combination of approaches. **Run options** contains automatic delivery settings. [Experiments and delivery](experiments.md)

## If setup gets stuck

- **Git is missing.** Install Git and run `git --version` in Terminal. For an initial Git commit, configure your Git name and email if Git prompts for them.
- **An installed agent is missing.** Run its version command in Terminal, then run `./swarm open` from that same terminal. This uses the PATH from your shell.
- **An agent turn fails after setup.** Open **Activity** and **View log** in the run. Check sign-in and provider access in the native CLI.
- **You want named model profiles.** Run `./swarm doctor` to find the local `profiles.json` path. Use the sample at `examples/profiles.json` as a starting point. Removing a custom override restores agent discovery with the built-in profiles. Open **Agent setup** again after editing the file.
