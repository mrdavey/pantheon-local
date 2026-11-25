#!/bin/sh
set -eu
set -- bin/pantheon-local-*.tar.gz
[ "$#" -eq 1 ]
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT HUP INT TERM
tar -xzf "$1" -C "$stage"
cd "$stage/$(basename "$1" .tar.gz)"
export SWARM_UPDATE_URL=off
./tests/e2e
./tests/experiments
