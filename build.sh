#!/usr/bin/env bash
# Builds AWorld.
#   ./build.sh          the desktop app            -> target/release/aworld
#   ./build.sh web      the web package (wasm)     -> dist/web/  (with a tiny web server)
# To build and start it at once, use ./run.sh. To put a shortcut on the desktop, ./install.sh.
set -euo pipefail
cd "$(dirname "$0")"

need() {
  command -v "$1" >/dev/null 2>&1 || { echo "Il manque « $1 » : installe Rust (https://rustup.rs) puis relance." >&2; exit 1; }
}

mode="${1:-app}"
case "$mode" in
  app)
    need cargo
    cargo build --release --bin aworld
    echo "OK : target/release/aworld"
    ;;
  web)
    need cargo
    if ! command -v rustup >/dev/null 2>&1; then
      echo "Il manque « rustup » : la version web a besoin de la cible wasm32-unknown-unknown de Rust (https://rustup.rs)." >&2
      exit 1
    fi
    if ! rustup target list --installed 2>/dev/null | grep -q '^wasm32-unknown-unknown$'; then
      echo "Installation de la cible wasm32-unknown-unknown..."
      rustup target add wasm32-unknown-unknown || { echo "Échec : lance « rustup target add wasm32-unknown-unknown »." >&2; exit 1; }
    fi
    cargo build --release --target wasm32-unknown-unknown --bin aworld
    cargo build --release --bin aworld-serve
    out=dist/web
    rm -rf "$out"
    mkdir -p "$out/site"
    cp web/index.html web/mq_js_bundle.js "$out/site/"
    cp target/wasm32-unknown-unknown/release/aworld.wasm "$out/site/"
    cp target/release/aworld-serve "$out/"
    cp web/serve.sh web/serve.bat "$out/"
    chmod +x "$out/serve.sh" "$out/aworld-serve"
    echo "OK : $out"
    echo "Pour le lancer : ./run.sh web   (ou $out/serve.sh puis http://localhost:8765/)"
    ;;
  *)
    echo "usage : $0 [app|web]" >&2
    exit 1
    ;;
esac
