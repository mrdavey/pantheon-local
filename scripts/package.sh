#!/bin/sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
os=$(go env GOOS)
arch=$(go env GOARCH)
name="pantheon-local-${os}-${arch}"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT HUP INT TERM
mkdir -p "$stage/$name" bin
npm --prefix web ci
npm --prefix web run build
go build -trimpath -o "$stage/$name/swarm" ./cmd/swarm
cp LICENSE NOTICE README.md CONTRIBUTING.md SECURITY.md "$stage/$name/"
cp -R docs examples "$stage/$name/"
mkdir "$stage/$name/tests"
cp tests/experiments.py tests/experiment-grader.py tests/fixture-release.py tests/e2e.py tests/fixture-agent.py "$stage/$name/tests/"
sed -e 's|](../README.md|](README.md|' -e 's|](model-routing.md)|](docs/model-routing.md)|' -e 's|](experiments.md)|](docs/experiments.md)|' docs/getting-started.md > "$stage/$name/START-HERE.md"
if [ "$os" = darwin ]; then
  cp scripts/open-pantheon-local.command "$stage/$name/Open Pantheon Local.command"
fi
tar -C "$stage" -czf "bin/$name.tar.gz" "$name"
printf '%s\n' "bin/$name.tar.gz"
