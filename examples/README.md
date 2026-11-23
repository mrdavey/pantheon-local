# Examples

The smallest complete change adds `hello.txt` containing `hello` and a final newline. Pass `examples/hello-recipe.json` as the accepted recipe. The checker lives in the command itself, so it is independent of files the agent writes.

For a new application, use “Build a private quote tracker. Let me add $125 and $75 quotes, accept the first, and see $75 outstanding after restarting.” Select an advisor if you want help refining the brief. Review the acceptance recipe before approving it. A browser check can be an executable script that starts the candidate, opens a browser, tests the workflow, and exits nonzero if an assertion fails.

For an existing application, use “Add CSV export of open quotes. Verify names containing commas and exclude quotes in every other status.” Start from the repository’s current committed revision and use its established test commands. The accepted recipe can include a separate CLI assertion that parses the exported CSV.

Set the preview command to a local app that reads `SWARM_PORT` or `PORT`, uses the available `HOST` value, and stores scratch data in `SWARM_DATA_DIR`. The workbench serves the preview from its own loopback port.

## Browser check for the quote tracker

`node /absolute/path/to/examples/quotes-check.mjs` is a reusable independent check. Install the web dependencies and Playwright Chromium first. Add it to the accepted eval as an argument list. The example brief needs an `app.py` start command, `SWARM_PORT` and `SWARM_DATA_DIR` support, **Client** and **Amount** field labels, an **Add quote** button, table rows with **Accept** actions, and text formatted as **Outstanding: $75.00**. An optional argument specifies a Chromium executable. The check exercises a restart and keeps a screenshot and trace under the eval’s data directory.
