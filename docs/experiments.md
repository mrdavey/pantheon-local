# Run a product experiment

Use a comparison when you can describe what would make one implementation better. In **Run options**, select **Compare approaches against a baseline**. The starting Git revision is the baseline. Pantheon Local runs the approved checks against it and the alternatives.

For example, the search fixture rewards relevant hits and sets a 5 ms latency limit. One implementation finds 10 hits but exceeds the limit. Another finds 7, stays within the limit and improves on the baseline’s 3. The second qualifies under that rule. [Comparison fixture](../tests/experiments.rs)

## Decide what qualifies

For pass/fail checks, put each command and its arguments in an array under `checks`. To compare measurements, add named `cases` and a `rule`. Each case uses a trusted standalone driver. Supply its absolute path in `driver`, or put the source in `program` and select its `interpreter`. The coordinator can propose an evaluator this way. Review its source and criteria before approval. Pantheon Local saves a fixed copy. The driver starts in the candidate checkout and reads a JSON object with `input`, `expected`, `sample`, and `seed` on stdin. Write one JSON result to stdout.

```json
{"pass":true,"observed":"Expected note ranked first","metrics":{"relevance":1,"latency":23.4}}
```

Use stderr for diagnostics. A required case must pass every sample. A missing, duplicate, or mismatched sample makes the comparison inconclusive and holds automatic selection. Declared metrics have a unit, a direction of `higher` or `lower`, and an aggregation of `mean`, `median`, or `p95`. Set `min_samples` and, for noisy results, a minimum improvement over the baseline and a minimum lead over the next candidate. Set hard limits using the metric's direction. With a held-back partition, the rule uses that partition for scores and still enforces all required checks. Keep private data and expected answers in the driver input. Pass only the product input to candidate code.

To compare previews using human judgment, set a `rubric` and a `preview` command in the recipe. Start **Blind review**, open both previews, then record a preference, tie or skip. A rubric calls for a human selection.

Use **Eval suite** to approve a case or rule revision. If you revise the suite from the CLI while review is paused, checks wait until you resume. A threshold change can reuse matching samples. Changes to what was measured require new evidence. **Measurements** shows how metrics vary across samples or rounds. Use **Compare case results** to inspect observations and errors behind the scores.

When you spot a missed edge case, send feedback in **Experiment**. The coordinator can save it under **Project eval cases**. It becomes part of a run’s checks when you approve a suite that references it.

## Choose and refine

Open the **Experiment** view to see the recommendation and which implementation landed. **Select** lets you choose another eligible candidate. **Keep current version** records a decision to retain the target. To borrow a strong feature from another candidate, choose **Combine aspects**, select the starting implementation and donors, and describe what should carry over. The resulting commit has its own checks and measurements.

After automatic landing, use **Improve this version** to create a follow-up on the latest target and reuse alternatives from earlier rounds. **Start another round** sends a new hypothesis or goal through the coordinator. Repairs and requested changes apply to the current round. Earlier rounds keep their results and decisions. After keeping the baseline, you can select an eligible implementation if you change your mind. The **Work** view retains the milestone logs, diffs and preview controls.

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

Only the status, deploy and rollback phases receive the configured environment variables. Keep the adapter outside the worker checkout and make its behavior independent of mutable candidate scripts during credentialed phases. Test the adapter with a disposable destination. The [filesystem fixture](../examples/fixture-release.rs) shows the protocol.

Choose **Auto-land and deploy** and the destination when starting a run. Use **Hold release** before promotion. Status must return both fields. Use an empty digest for a destination with no current release. If the verified artifact is already live and healthy, Pantheon Local records the match and skips promotion. After a promotion it reads the live digest and health again. A failed health check can restore the previous digest when the adapter supports it. If status is unavailable, **Check live state** reconciles the recorded attempt without starting another deployment.

For a CLI comparison, pass `--compare --recipe recipe.json` to `swarm start`. Choose `--delivery auto` for auto-land or `--delivery deploy --destination production` for deployment. Use `swarm help` for command syntax.
