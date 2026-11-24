# Examples

- **Try a first run.** Ask for `hello.txt` containing `hello` and a final newline. Paste the example recipe into **View or edit JSON** before approving the checks, or use `--recipe examples/hello-recipe.json` with `swarm start`. The check reads the resulting file and compares its entire contents. [Hello recipe](hello-recipe.json)

- **Build a new application.** “Build a private quote tracker. Let me add $125 and $75 quotes, accept the first, and see $75 outstanding after restarting.” The API and interface can be separate milestones. One browser check verifies the whole workflow.

- **Extend an existing application.** “Add CSV export of open quotes. Handle client names containing commas and exclude accepted quotes.” Use the project’s usual tests and add a check that parses the export and compares the rows.

Set the preview command to a local app that reads `SWARM_PORT` or `PORT`, uses the available `HOST` value, and stores scratch data in `SWARM_DATA_DIR`. The workbench serves the preview from its own loopback port.

## Browser check for the quote tracker

The example checker imports Playwright from the Pantheon Local source checkout. There, run `npm --prefix web ci` and install Chromium with `npm --prefix web exec -- playwright install chromium`. In the recipe’s `checks` list, use the following argument array with the actual paths. [Quote tracker browser check](quotes-check.mjs)

```json
["node", "/absolute/path/to/pantheon-local/examples/quotes-check.mjs", "/absolute/path/to/chromium"]
```

The checker starts `python3 app.py` with `SWARM_PORT` and `SWARM_DATA_DIR`. It expects **Client** and **Amount** fields, an **Add quote** button, an **Accept** button in each open quote’s row, and a total formatted as **Outstanding: $75.00**. It restarts the app to verify persistence and saves a screenshot and trace in the check’s data directory.
