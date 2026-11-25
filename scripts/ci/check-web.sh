#!/usr/bin/env bash
# Open the production bundle without a daemon and verify React renders its recovery UI.
set -euo pipefail
scratch=$(mktemp -d)
python3 -u -m http.server 0 --bind 127.0.0.1 --directory web/dist >"$scratch/server.log" 2>&1 &
server=$!
trap 'kill "$server" 2>/dev/null || true; rm -rf "$scratch"' EXIT
for ((attempt = 0; attempt < 100; attempt++)); do
  port=$(sed -n 's/.* port \([0-9]*\) .*/\1/p' "$scratch/server.log" | head -1)
  [[ -n "$port" ]] && break
  kill -0 "$server"
  sleep 0.1
done
: "${port:?Static server did not start}"
"${CHROME_BIN:-google-chrome}" --headless --disable-dev-shm-usage --user-data-dir="$scratch/chrome" \
  --virtual-time-budget=5000 --dump-dom "http://127.0.0.1:$port/" >"$scratch/page"
grep -q 'role="alert"' "$scratch/page"
grep -q '>Try again</button>' "$scratch/page"
