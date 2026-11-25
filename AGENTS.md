# Project instructions

## Pantheon Local

Pantheon Local coordinates native coding agents on the developer’s computer. See the [architecture](docs/architecture.md) and [experiment guide](docs/experiments.md). Use ordinary Git commits for milestones and preserve the developer’s index and checkout.

Install rustup (see rust-toolchain.toml) and Git 2.39 or newer. Run `cargo test --locked`, `cargo clippy --locked --all-targets -- -D warnings`, and `git diff --check` from this repository. Tests use temporary Git repos and must never depend on a developer's project or model quota.

## Engineering and documentation

Include meaningful regression tests with every feature and run the affected checks. Maintain CI for relevant tests, linting, type checks, and builds using verified project commands; prefer existing CI, or GitHub Actions for new GitHub projects.
Keep documentation and this AGENTS.md current with implementation. Periodically compare documentation with code at major milestones and before releases. Maintain a README for setup, running, testing, and deployment as applicable, a safe .env.example when environment configuration is required, and architecture decision documentation when complexity warrants it. Reuse existing equivalents and avoid empty scaffolding.
For interfaces, extend existing design systems and shared components before adding competing patterns or dependencies. Verify main flows in a real browser at desktop and mobile sizes, including keyboard navigation and relevant loading, empty, error, and success states. Fix observed issues and report verification blockers.

## Completion and commits

Use Git for project version control and GitHub as the default remote host. During local project setup, initialize Git at the active project root if it is outside an existing repository. Respect repository and worktree boundaries, avoid nested repositories, and keep secrets and generated artifacts out of version control.

Complete the requested behavior through integration and relevant validation. Fix failures caused by the change and rerun affected checks; repeat passing checks only when new changes or unresolved concerns justify it. Report concrete blockers and continue independent work where possible.
After every completed feature integration, create a local Git commit unless the user explicitly asks to defer it. Review the diff and stage only the intended completed code, tests, and documentation. Preserve unrelated, pre-staged, incomplete, or concurrently edited work. Use the git-commit-complete-changes skill when available.
Use a relevant imperative subject and a body explaining what changed, why, validation results, and material limitations. Report the commit hash and subject, or the concrete blocker. This rule does not authorize pushing or deployment.
Bump the package version in `Cargo.toml` (and sync `Cargo.lock`) according to Semantic Versioning for completed release-worthy changes—patch (`x.y.Z`) for bug fixes and internal hardening, minor (`x.Y.0`) for backward-compatible features or CLI/API additions, and major (`X.0.0`) for breaking changes—and create a matching annotated local Git tag (`vX.Y.Z`) after validation passes unless the user asks to defer versioning or tagging. Pushing a `vX.Y.Z` tag triggers `.github/workflows/release.yml` to build, verify, and publish platform archives to GitHub Releases; never push commits or tags unless authorized.

Build the React interface with `npm --prefix web ci` and `npm --prefix web run build`, then build the Rust CLI with `cargo build --locked`. Write tests and example checkers in Rust. Use the [contributor checks](CONTRIBUTING.md) for the daemon and real browser, and keep native account checks separate from fixtures. See [architecture](docs/architecture.md) for process, Git and adapter boundaries.
