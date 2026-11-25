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

| Change | Checks |
| --- | --- |
| Documentation or `.gitignore` only | Selection job only |
| React source, styles, or web dependencies | Linux frontend job builds with Vite and opens the production bundle in headless Chrome. The Rust matrix is skipped. |
| Rust source or shared fixtures | Linux and macOS Clippy and tests, Linux formatting and browser tests |
| One integration test file | Clippy and that test target on both platforms, formatting on Linux. Workbench changes also run the browser suite. |
| Packaging scripts or release workflow | Build, extract, and test the archives on Linux and macOS |
| CI workflow, toolchain, or unclassified build input | Full set |

For mixed changes, CI combines the affected checks. Each selected check compares its Git inputs, workflow, runner image, architecture, and tool versions with successful steps from the last 20 CI runs. A matching step from a repository branch can cover a PR update, merge, or retry. A later failure or missing evidence makes the check run. The selection log shows which results were reused. GitHub caches the built web assets separately.

For a release tag, the selector reuses green main CI for the tagged commit or waits up to ten minutes for a known active run. Tags qualify the full daemon-backed browser suite, including changes previously checked by the frontend job alone. An unverified tag also runs the missing Rust checks. Release tags build fresh archives and run the packaged fixture suites on all three release platforms before publication. To exercise the selection rules locally, run `cargo test --locked --test ci` with Python 3 and Node.js available.
