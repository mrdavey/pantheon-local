# Start your first run

## Open the workbench

Follow the [source build instructions](../README.md#build-from-source), or run `./swarm open` from an extracted archive. Keep Terminal open. If the browser doesn’t open, use the link printed in Terminal.

## Connect an agent

**Agent setup** checks for Git and installed agent profiles. Choose an available profile and select **Use**. Open **Set up another agent** for installation or sign-in instructions, then select **Check again**.

| Agent | Setup |
| --- | --- |
| Codex | Run `codex login`. [Install Codex](https://developers.openai.com/codex/cli/) |
| Claude Code | Run `claude auth login`. [Install Claude Code](https://code.claude.com/docs/en/quickstart) |
| Hermes | Configure your local agent in Hermes Desktop. Select **Hermes · default** or a named Desktop profile as coordinator or worker. Each turn starts or resumes a separate ACP session using that profile. [Install Hermes](https://hermes-agent.nousresearch.com/docs/getting-started/quickstart) |
| Antigravity | Run `agy` and finish sign-in. [Install Antigravity](https://antigravity.google/docs/cli-getting-started) |

For Hermes, Antigravity and custom adapters, **Installed** confirms the executable was found. Check provider access in the CLI.

## Start and review your first run

1. Select **Use** beside an agent to fill both roles, or choose a coordinator and several workers. Select **Browse…** or enter the full path to an empty folder or an existing Git checkout. Commit any edits you want workers to build on.
2. Give the run a checkable goal. In an empty folder, try “Create `hello.txt` containing `hello` followed by a single newline.” **Improve brief** can help refine it. Select **Start run**.
3. Answer any planning question. Review the proposed checks, edit them in **View or edit JSON** if needed, then select **Approve checks**. For this example, the check should verify the file’s exact contents.
4. In **Work**, open **View checks** and **View diff**. If the recipe includes a preview, use **Start preview**. Choose **Request change** or land the tested commit on the named branch. A checked-out target must be clean.

Use **Needs you** for decisions and **Conversation** to ask about a run or request more work. [Team coordination](model-routing.md)

**Pause run** interrupts active work. You can answer a planning question while paused. **Resume** continues the run and repeats an interrupted check. Closing the tab leaves the service running. Reopen the launcher to reconnect, or press Control-C in Terminal to stop the service.

## If setup gets stuck

- **Folder access denied on Mac.** In System Settings → Privacy & Security → Files & Folders, allow your launching terminal to access the project folder, then reopen the workbench. You can also choose a project outside the protected folder.
- **Browse is unavailable on Linux.** Enter the project path, or install Zenity or KDialog on the computer running Pantheon Local.
- **Git is missing.** Install Git and check `git --version`. Configure your Git name and email if the first commit asks for them.
- **An agent is missing.** Check its version in Terminal, then launch the workbench from the same shell.
- **A turn fails.** Open **Activity** and **View log**. Check provider access in the agent CLI.
- **You need a named profile.** Run `swarm doctor` (or `./swarm doctor` in the archive folder) to locate `profiles.json`. Adapt `examples/profiles.json`, then reopen **Agent setup**.
