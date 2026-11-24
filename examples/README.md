# Examples

Build the checkers with `cargo build --locked --release --example hello-check --example quotes-check`. The executables are in `target/release/examples`. Release archives include both under `examples`.

## First run

Ask for `hello.txt` containing `hello` and one newline. The [hello recipe](hello-recipe.json) runs a Rust checker that compares the entire file. Replace `hello-check` in the recipe with the absolute path to the executable, or add its directory to your PATH before starting Pantheon Local.

## Quote tracker

Ask the team to build a small Rust quote tracker. Let the API and interface workers agree on how quotes are added, accepted and persisted. The [browser checker](quotes-check.rs) compiles the candidate’s standalone `app.rs`, starts it on `SWARM_PORT` with storage in `SWARM_DATA_DIR`, and drives Chrome through a matching ChromeDriver.

Use this recipe with absolute paths. Use `rustup which rustc` to find the compiler.

```json
{"checks":[["/path/to/quotes-check","/path/to/chrome","/path/to/chromedriver","/path/to/rustc"]]}
```

The checker fills **Client** and **Amount**, adds $125 and $75 quotes, and accepts the first. It expects **Outstanding: $75.00** before and after restarting the app, with **Accepted** in the first quote’s row. It saves a screenshot and browser performance log in the check’s data directory.

On macOS, Chrome launched by this approved check inherits Pantheon’s filesystem sandbox. Chrome’s nested sandbox is disabled for that launch because macOS rejects a second profile. Standalone browser tests use Chrome’s default sandbox.
