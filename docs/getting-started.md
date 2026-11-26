# Start your first run

## Open the workbench

Follow the [source build instructions](../README.md#build-from-source), or run `./swarm open` from an extracted archive. Keep Terminal open while you use the workbench. Click the displayed HTTP address using your terminal’s link gesture, or press **Enter** to open the browser. The link includes the session key in its destination. Typing the short address in a fresh tab requires opening the workbench from Terminal to sign in. When an update appears below the address, click it for release details.

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

1. Open **Runs**, choose **+ New run** if you already have runs, then enter a checkable goal. For example, “Create `hello.txt` containing `hello` followed by a single newline.” Use **Attach files** or drop reference files into the brief (up to 16 files, 16 MiB combined). The team receives private file copies and short text excerpts. **Improve brief** suggests an editable version. Codex uses GPT-6 Luna and Claude Code uses Haiku, both with medium reasoning. Hermes and Antigravity use a known compact model from the same provider and medium reasoning when their sessions offer those settings. Custom adapters control their own models.
2. Confirm the project folder and open **Team** to choose your coordinator and workers, adjust their model settings and set a subagent limit. The folder defaults to where you ran `swarm open`. Use **Browse…** to change it. Commit the edits you want agents to build on, then select **Start run**.
3. Answer any planning question. Expand a named check to see the exact command. Review the proposed checks, edit them in **View or edit JSON** if needed, then select **Approve checks**. For this example, the check should verify the file’s exact contents.
4. In **Milestones**, open **View checks** and **View diff**. If the recipe includes a preview, use **Start preview**. New workbench runs auto-land a passing candidate on a clean target. To review first, choose **Advanced settings → Delivery → Review before landing** before starting the run.

Open **Run settings → View brief** to read the formatted request in a larger dialog. It previews uploaded images and supported text files and provides downloads for other attachments. In **Needs you**, answer a question or retry a blocked milestone. **Dismiss** moves it to **Show history**, where you can restore it. A blocking question still needs an answer. The open **Coordinator chat** shows what the coordinator is doing. Use **Milestones → Map** to inspect a worker’s assignments and their dependencies. [Team coordination](model-routing.md)

Open **Run settings** from the gear on the run page to pause, resume, require review, or delete the run. Deletion removes its history and private checkouts once work has stopped. Your project and landed commits stay on disk. Closing the tab leaves the service running. Reopen the launcher to reconnect, or press Control-C in Terminal to stop the service.

## Inspect a run

In **Run settings → Workspace**, the working folder link opens the milestone checkouts. New runs put them under **Pantheon work <run ID>** in the folder you selected. Each milestone has its own copy so several agents can edit in parallel. Passing work appears on the target branch when it lands. Older runs open their original working folder in the service state directory. Deleting a new run keeps its project-side milestone folders.

The workbench opens in dark mode. Use **Appearance** in the top bar to choose Light or follow your system.

In **Milestones**, switch between **Table**, **Map**, and **Kanban**. Select any table row or board card to open its details. Choose a table heading to sort (the Milestone column uses the milestone number), then choose it again to reverse the order. **Completed** shows local date and time and keeps milestones without a completion time at the bottom. Select a dependency in the details panel to open that milestone, including work from earlier rounds.

On the map, choose a colored worker filter to dim other workers’ assignments. Choose **All** to clear it. Select an assignment to trace its prerequisites and downstream work. Click the canvas to clear the chain. Selecting the card opens the same assignment sidebar as the table and board, including status, tokens, estimated cost, duration, and **Explain this**. Elapsed time starts at the first worker turn and includes waits between attempts. It updates each second during work and shows the final duration after completion. **Full screen** expands the map and Escape returns to the run.

Choose **Add task**, describe the work, then select **Create milestone(s)**. Review the proposed owners, dependencies, and extra checks before choosing **Add to run**. You can also request milestones in coordinator chat. Adding work after landing starts a new round.

Use **Coordinator chat** at the bottom to show or hide the dock. The dock’s full-screen button opens the **Coordinator chat** tab. Your conversation and current draft follow you between the two views. The composer shows the coordinator’s agent, model, and thinking effort. In the tab, choose **+ → Attach files** to send up to 16 files totaling 16 MiB with a message.

In **Metrics**, choose an agent, model, role or time range. Time ranges end at the last activity for an idle run. Drag across a line or bar chart to zoom, or use the sliders under **Zoom time range**. Choose the pie view to compare shares. **Fetch token prices** asks the coordinator to look up published rates for missing estimates. See **How cost is calculated** for sources.

## If setup gets stuck

- **Project folder has files but no commits.** Choose an empty folder for a new project. To use existing files, use **Advanced settings → Initial import files → Choose files…**. Select multiple files, choose **Add more files…** to add another batch, or remove individual entries before starting. These files become the first Git commit. Pantheon checks the selection before initializing a new repository.
- **Folder access denied on Mac.** In System Settings → Privacy & Security → Files & Folders, allow your launching terminal to access the project folder, then reopen the workbench. You can also choose a project outside the protected folder.
- **Browse is unavailable on Linux.** Enter the project path, or install Zenity or KDialog on the computer running Pantheon Local.
- **Git is missing.** Install Git and check `git --version`. Configure your Git name and email if the first commit asks for them.
- **An agent is missing.** Check its version in Terminal, then launch the workbench from the same shell.
- **A turn fails.** Open **Activity Log** and select a turn to read its log. Search or filter turns by agent, status or type, change the order, and page through the results. Expand individual events to see their details. The newest messages appear at the top with local date, time and age. During a live turn, the latest page refreshes every two seconds. Scroll down or choose **Pause updates** to hold your place. Use **Older** and **Newer** to browse, then **Latest** to return to the newest output. Earlier runs can show **Time unavailable** for messages saved without a timestamp. For provider errors, check access in the agent CLI.
- **You need a named profile.** Run `swarm doctor` (or `./swarm doctor` in the archive folder) to locate `profiles.json`. Adapt `examples/profiles.json`, then reopen **Agent setup**.

Choose the status in coordinator chat to see coordinator turns in **Activity Log**. Search or filter by type, agent, and status. Each active filter has an **×** to remove it. Select an activity row to read the log. **Explain this** in the detail header sends the selected activity, milestone, or implementation to coordinator chat. On Ctrl+C, the interactive launcher returns to the terminal with a confirmation that you can close the window.
