# Run a product experiment

Start a run from an empty folder or an existing project. Pick a coordinator and describe the outcome. Open **Run options** and choose **Compare approaches against a baseline** if you want competing implementations measured against the starting revision. For a first build, leave this off and approve checks for the first working user journey.

Use **Needs you** when the coordinator asks for a decision. It can answer worker questions on its own, assign more work and save examples as eval cases. A milestone delegated to a child inherits the parent’s allowed files, prerequisites and experiment variant. The parent gets the child commit refs when its children finish. Other ready milestones can continue. Add feedback in **Experiment** when you notice an edge case worth keeping. Open **Project eval cases** to see what the coordinator saved and why. Draft cases become part of a run's active suite when you approve a suite that references them.

## Decide what qualifies

An acceptance recipe can still contain just `checks`, a list of executable argument arrays. Add optional named `cases` and a `rule` to measure an improvement. Each case uses a trusted standalone driver. Supply its absolute path in `driver`, or put the source in `program` and select an `interpreter` such as `python3`. The coordinator can propose an evaluator this way. Review its source and criteria before approval. Pantheon Local saves a fixed copy. The driver starts in the candidate checkout and reads a JSON object with `input`, `expected`, `sample`, and `seed` on stdin. Write one JSON result to stdout. Here is a sample.

```json
{"pass":true,"observed":"Expected note ranked first","metrics":{"relevance":1,"latency":23.4}}
```

Use stderr for diagnostics. A required case must pass every sample. A missing, duplicate, or mismatched sample makes the comparison inconclusive and holds automatic selection. Declared metrics have a unit, a direction of `higher` or `lower`, and an aggregation of `mean`, `median`, or `p95`. Set `min_samples` and, for noisy results, a minimum improvement over the baseline and a minimum lead over the next candidate. Set hard limits using the metric's direction. With a held-back partition, the rule uses that partition for scores and still enforces all required checks. Keep private data and expected answers in the driver input. Pass only the product input to candidate code.

For an example, see [the comparison fixture](../tests/experiments.py). To compare previews using human judgment, set a `rubric` and a `preview` command in the recipe. Start **Blind review** in the experiment, open both previews, then record a preference, tie or skip. A rubric calls for a human selection.

Use **Eval suite** to approve a case or rule revision. Results from prior candidates remain available. A threshold change can reuse matching samples. Changes to what was measured require new evidence. The timing chart shows ranges and sample counts. Use **Compare case results** to inspect observations and errors behind the scores.

## Choose and refine

Open the **Experiment** view to see the recommendation and which implementation landed. **Select** lets you choose another eligible candidate. **Keep current version** records a decision to retain the target. To borrow a strong feature from another candidate, choose **Combine aspects**, select the starting implementation and donors, and describe what should carry over. The resulting commit has its own checks and measurements.

After automatic landing, use **Improve this version** to create a follow-up on the latest target and reuse alternatives from earlier rounds. **Start another round** sends a new hypothesis or goal through the coordinator. Open an earlier round to see its measurements and the decision recorded for that round and suite revision. After keeping the baseline, you can select an eligible implementation if you change your mind. The **Work** view retains the milestone logs, diffs and preview controls.

Set **Delivery** to **Auto-land** to let the approved rule or a single passing implementation advance a clean target. Use **Hold auto-land** before landing or **Require review** to lower authority for the current run. An unresolved conflict stays in a private checkout. If the target moves, Pantheon Local rebuilds and checks against the new revision.

## Deploy through a project adapter

Create `destinations.json` in your Pantheon Local state directory. Run `swarm doctor` to locate it. Each name identifies one release destination. Choose one canonical name per destination.

```json
{
  "production": {
    "executable": "/absolute/path/to/release-adapter",
    "environment": ["MY_DEPLOY_TOKEN"],
    "rollback": true
  }
}
```

The executable takes one argument and reads a JSON request with `operation`, `destination`, `commit`, `source`, `artifact`, `digest`, and `previous` as applicable.

| Argument | Behavior |
| --- | --- |
| `build` | Build from the supplied verified source directory. Write one regular file at the artifact path. |
| `verify` | Check that exact artifact. Exit successfully to allow promotion. |
| `status` | Print the current artifact identity and health as JSON, such as `{"digest":"sha256:…","healthy":true}`. |
| `deploy` | Promote the supplied artifact and digest. Use the operation ID for idempotency. Check the expected previous digest when the provider supports conditional updates. |
| `rollback` | Restore the previous known digest when supported. Use the operation ID and verify the expected current digest. |

Only the status, deploy and rollback phases receive the configured environment variables. Keep the adapter outside the worker checkout and make its behavior independent of mutable candidate scripts during credentialed phases. Test the adapter with a disposable destination. The [filesystem fixture](../tests/fixture-release.py) shows the protocol.

Choose **Auto-land and deploy** and the destination when starting a run. Use **Hold release** before promotion. Status must return both fields. Use an empty digest for a destination with no current release. If the verified artifact is already live and healthy, Pantheon Local records the match and skips promotion. After a promotion it reads the live digest and health again. A failed health check can restore the previous digest when the adapter supports it. If status is unavailable, **Check live state** reconciles the recorded attempt without starting another deployment.

From the terminal, use `swarm start --compare --recipe recipe.json --delivery auto` with the usual project and profile arguments. Add `--delivery deploy --destination production` to release. Use `swarm status`, `swarm cases`, `swarm evidence`, and `swarm help` to find the same controls and records.
