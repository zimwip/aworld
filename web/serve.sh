#!/usr/bin/env bash
# Serves the game on http://localhost:${1:-8765}/ (Ctrl+C to stop).
cd "$(dirname "$0")"
exec ./aworld-serve site "${1:-8765}"
