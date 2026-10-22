#!/usr/bin/env bash
# Builds AWorld.
#   ./build.sh          the desktop app            -> target/release/aworld
#   ./build.sh web      the web package (wasm)     -> dist/web/  (with a tiny web server)
#   ./build.sh web run  same, then serves it on http://localhost:8765
set -euo pipefail
cd "$(dirname "$0")"

mode="${1:-app}"
case "$mode" in
  app)
    cargo build --release --bin aworld
    echo "OK : target/release/aworld"
    ;;
  web)
    rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
    cargo build --release --target wasm32-unknown-unknown --bin aworld
    cargo build --release --bin aworld-serve
    out=dist/web
    rm -rf "$out"
    mkdir -p "$out/site"
    cp web/index.html web/mq_js_bundle.js "$out/site/"
    cp target/wasm32-unknown-unknown/release/aworld.wasm "$out/site/"
    cp target/release/aworld-serve "$out/"
    cat > "$out/serve.sh" <<'SERVE'
#!/usr/bin/env bash
# Serves the game on http://localhost:${1:-8765}/
cd "$(dirname "$0")"
exec ./aworld-serve site "${1:-8765}"
SERVE
    chmod +x "$out/serve.sh"
    echo "OK : $out  (lance $out/serve.sh puis ouvre http://localhost:8765/)"
    if [ "${2:-}" = "run" ]; then
      exec "$out/serve.sh"
    fi
    ;;
  *)
    echo "usage : $0 [app|web [run]]" >&2
    exit 1
    ;;
esac
