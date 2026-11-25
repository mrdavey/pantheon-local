# Start your first run

## Open the workbench

Follow the [source build instructions](../README.md#build-from-source), or run `./swarm open` from an extracted archive. Keep Terminal open. If the browser doesn’t open, use the link printed in Terminal.

## Connect an agent

**Agent setup** checks for Git and installed agent profiles. Select **Use** beside a configured agent, or open **Set up new agent** for installation and sign-in instructions. Select **Check again** to refresh the list.

| Agent | Setup |
| --- | --- |
| Codex | Run `codex login`. [Install Codex](https://developers.openai.com/codex/cli/) |
| Claude Code | Run `claude auth login`. [Install Claude Code](https://code.claude.com/docs/en/quickstart) |
| Hermes | Configure your local agent in Hermes Desktop. Select **Hermes · default** or a named Desktop profile as coordinator or worker. Each turn starts or resumes a separate ACP session using that profile. [Install Hermes](https://hermes-agent.nousresearch.com/docs/getting-started/quickstart) |
| Antigravity | Run `agy` and finish sign-in. [Install Antigravity](https://antigravity.google/docs/cli-getting-started) |

For Hermes, Antigravity and custom adapters, **Installed** confirms the executable was found. Check provider access in the CLI.

## Start and review your first run

1. In **New run**, enter a checkable goal. For example, “Create `hello.txt` containing `hello` followed by a single newline.” Use **Attach files** or drop reference files into the brief (up to 16 files, 16 MiB combined). The team receives private file copies and short text excerpts. **Improve brief** suggests an editable version. Codex uses GPT-6 Luna and Claude Code uses Haiku, both with medium reasoning. Hermes and Antigravity use a known compact model from the same provider and medium reasoning when their sessions offer those settings. Custom adapters control their own models.
2. Confirm the project folder and open **Team** to choose your coordinator and workers. The folder defaults to where you ran `swarm open`. Use **Browse…** to change it. Commit the edits you want agents to build on, then select **Start run**.
3. Answer any planning question. Review the proposed checks, edit them in **View or edit JSON** if needed, then select **Approve checks**. For this example, the check should verify the file’s exact contents.
4. In **Work**, open **View checks** and **View diff**. If the recipe includes a preview, use **Start preview**. New workbench runs auto-land a passing candidate on a clean target. To review first, choose **Advanced settings → Delivery → Review before landing** before starting the run.

Open **Full brief** beneath the run title to read the original request. In **Needs you**, answer a question or retry a blocked milestone. **Dismiss** moves it to **Show history**, where you can restore it. A blocking question still needs an answer. Open **Coordinator chat** to ask about the run or request more work. [Team coordination](model-routing.md)

Use the gear beside a run to pause, resume, require review, or delete it. Deletion removes its history and private checkouts once work has stopped. Your project and landed commits stay on disk. Closing the tab leaves the service running. Reopen the launcher to reconnect, or press Control-C in Terminal to stop the service.

## If setup gets stuck

- **Project folder has files but no commits.** Choose an empty folder for a new project. To use existing files, use **Advanced settings → Initial import files → Choose files…**. Select multiple files, choose **Add more files…** to add another batch, or remove individual entries before starting. These files become the first Git commit. Pantheon checks the selection before initializing a new repository.
- **Folder access denied on Mac.** In System Settings → Privacy & Security → Files & Folders, allow your launching terminal to access the project folder, then reopen the workbench. You can also choose a project outside the protected folder.
- **Browse is unavailable on Linux.** Enter the project path, or install Zenity or KDialog on the computer running Pantheon Local.
- **Git is missing.** Install Git and check `git --version`. Configure your Git name and email if the first commit asks for them.
- **An agent is missing.** Check its version in Terminal, then launch the workbench from the same shell.
- **A turn fails.** Open **Work → View activity** and choose the turn’s **View log**. Check provider access in the agent CLI.
- **You need a named profile.** Run `swarm doctor` (or `./swarm doctor` in the archive folder) to locate `profiles.json`. Adapt `examples/profiles.json`, then reopen **Agent setup**.
