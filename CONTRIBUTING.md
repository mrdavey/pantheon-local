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
