# Contributing

For a bug, open an issue with your OS, agent CLI version and steps to reproduce. Remove tokens, prompts and private repo paths from logs.

Build the React assets as described in the README. Run the Rust checks from the repository root.

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --examples
cargo test --locked
```

The default suite uses temporary Git repos and compiled fixture agents. To run the desktop and mobile workbench tests, install Chrome or Chromium with a matching ChromeDriver and run the command below. Set `CHROME_BIN` and `CHROMEDRIVER` when the executables are outside their standard locations.

```sh
cargo test --locked --test workbench -- --ignored --test-threads=1
```

For live qualification, set `SWARM_NATIVE_PROFILES` to your profiles file, `SWARM_NATIVE_COORDINATOR` to a profile name and `SWARM_NATIVE_WORKERS` to comma-separated worker names. Run `cargo test --locked --test native native_smoke -- --ignored --nocapture` to use those accounts. Set `SWARM_NATIVE_BROWSER` to the Chrome executable for the quote tracker scenario. [Adapter contract](docs/architecture.md#native-adapters)

Build a local release archive with `scripts/package.sh` (requires jq). In the extracted archive, run `./tests/e2e` and `./tests/experiments` for the same fixture checks. Pushing a `vX.Y.Z` tag matching `Cargo.toml` triggers `.github/workflows/release.yml` to build, verify, and publish the macOS and Linux archives on GitHub Releases.

## CI selection

Pull requests and pushes to `main` start a small selection job. It compares the checked-out tree (the merge result for a PR) with a successfully checked base on `main`. If that base is pending or failed, CI includes changes since the most recent verified ancestor it can find. Without a verified comparison it selects the full set. New commits to a PR cancel its superseded run.

CI discovers Cargo tests and examples in both standard layouts (`tests/name.rs` or `tests/name/main.rs`, and the corresponding `examples/` paths). Files under a target's directory belong to that target. The ownership and dependency rules in `scripts/ci/targets.py` also describe shared Rust modules, the daemon, fixture executables, the frontend, and platform-specific packaging.

| Changed input | What runs |
| --- | --- |
| Documentation or repository metadata | Selection only |
| Example recipe or profile JSON | Parse the changed sample |
| React source, styles, or web dependencies | Vite production build and headless Chrome smoke check on Linux |
| One or more integration tests | Format, lint and run those targets. Build the fixtures they launch. |
| Standalone Cargo example | Format, lint, build and test the example, plus any suites that use it |
| Shared test module or executable fixture | Check the targets that compile the module or launch the fixture |
| Production daemon source | Build and lint the binary and run its unit, integration, and workbench consumers |
| macOS launcher | Build, extract and verify the macOS archive |
| Shared packaging or release workflow | Verify archives on Linux and macOS |
| CI policy, removed Cargo target, or unknown input | Full coverage |

Mixed edits take the union of the affected targets on each platform. Node setup and embedded web assets are selected separately. For example, changes to `tests/service.rs` and `tests/e2e.rs` run those two suites with the fixture agent. The workbench tests also need a built web bundle. Signed-in native qualification stays opt-in.

To inspect the selection for committed changes without contacting GitHub, run `python3 scripts/ci/checks.py explain BASE` from the repository root, replacing `BASE` with a comparison commit. The Actions selection log shows the same target lists. When adding cross-directory source imports or changing which fixture a suite launches, update `SHARED` or `FIXTURES` in the target model and cover the relationship in `tests/ci.rs`. New integration targets conservatively depend on all example executables until their fixture requirements are declared.

CI reuses a successful command from the last 20 runs when its selected targets, input files, workflow, runner and tool versions match. A later failure or missing evidence runs the command. A fresh runner builds fixtures needed by a pending test even if the earlier build passed. GitHub caches the web bundle separately.

For a release tag, the selector reuses green main CI for the tagged commit or waits up to ten minutes for a known active run. Tags qualify the full daemon-backed browser suite, including changes previously checked by the frontend job alone. An unverified tag also runs the missing Rust checks. Release tags build fresh archives and run the packaged fixture suites on all three release platforms before publication. To exercise the selection rules locally, run `cargo test --locked --test ci` with Python 3 and Node.js available.
