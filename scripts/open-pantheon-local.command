#!/bin/sh
set -eu

cd -- "$(dirname -- "$0")"
swarm="$PWD/swarm"
cd /
# Load the login shell's usual PATH, including agents installed through Node.
exec /bin/zsh -ilc 'exec "$1" open' local-swarm "$swarm"
