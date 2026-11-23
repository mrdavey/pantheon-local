# Contributing

For a bug, open an issue with your OS, agent CLI version and steps to reproduce. Remove tokens, prompts and private repo paths from logs.

Build the workbench and CLI using the README. Then run:

```sh
go test -race ./...
go vet ./...
python3 tests/e2e.py
python3 tests/experiments.py
npm --prefix web exec -- playwright install chromium
npm --prefix web test
```

The tests use disposable Git repos and fixture agents. The native smoke tests in `tests/` use your signed-in agent accounts and run only when called explicitly. Use the [adapter contract](docs/architecture.md#native-adapters) if you’re adding a harness.
