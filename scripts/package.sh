#!/bin/sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
case "$(uname -s)" in
  Darwin) os=darwin ;;
  Linux) os=linux ;;
  *) echo "Packaging supports macOS and Linux." >&2; exit 1 ;;
esac
case "$(uname -m)" in
  arm64|aarch64) arch=arm64 ;;
  x86_64) arch=amd64 ;;
  *) arch=$(uname -m) ;;
esac
name="pantheon-local-${os}-${arch}"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT HUP INT TERM
mkdir -p "$stage/$name" bin
npm --prefix web ci
npm --prefix web run build
cargo build --locked --release --bin swarm --examples --target-dir "$root/target"
cargo test --locked --release --no-run --test e2e --test experiments --target-dir "$root/target" --message-format=json > "$stage/tests.json"
cp target/release/swarm "$stage/$name/swarm"
cp LICENSE NOTICE README.md CONTRIBUTING.md SECURITY.md "$stage/$name/"
cp -R docs examples "$stage/$name/"
mkdir -p "$stage/$name/tests/fixtures"
cp tests/e2e.rs tests/experiments.rs "$stage/$name/tests/"
cp -R tests/common tests/browser "$stage/$name/tests/"
cp target/release/examples/hello-check target/release/examples/quotes-check "$stage/$name/examples/"
for fixture in fixture-agent fixture-release experiment-grader; do
  cp "target/release/examples/$fixture" "$stage/$name/tests/fixtures/"
done
for suite in e2e experiments; do
  executable=$(jq --raw-output --exit-status --arg suite "$suite" 'select(.reason=="compiler-artifact" and .target.kind==["test"] and .target.name==$suite) | .executable' "$stage/tests.json")
  cp "$executable" "$stage/$name/tests/$suite"
done
sed -e 's|](../README.md|](README.md|' -e 's|](model-routing.md)|](docs/model-routing.md)|' -e 's|](experiments.md)|](docs/experiments.md)|' docs/getting-started.md > "$stage/$name/START-HERE.md"
if [ "$os" = darwin ]; then
  cp scripts/open-pantheon-local.command "$stage/$name/Open Pantheon Local.command"
fi
COPYFILE_DISABLE=1 tar -C "$stage" -czf "bin/$name.tar.gz" "$name"
(
  cd bin
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$name.tar.gz"
  else
    shasum -a 256 "$name.tar.gz"
  fi > "$name.tar.gz.sha256"
)
printf '%s\n' "bin/$name.tar.gz"
