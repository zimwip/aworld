#!/usr/bin/env bash
# Builds AWorld, then starts it.
#   ./run.sh                the desktop app (extra arguments go to the game, e.g. --mute)
#   ./run.sh web [port]     the web version: builds the package and serves it on http://localhost:8765/
set -euo pipefail
cd "$(dirname "$0")"

if [ "${1:-}" = "web" ]; then
  shift
  ./build.sh web
  echo "AWorld web : http://localhost:${1:-8765}/   (Ctrl+C pour arrêter)"
  exec dist/web/serve.sh "${1:-8765}"
fi

./build.sh app
exec ./target/release/aworld "$@"
