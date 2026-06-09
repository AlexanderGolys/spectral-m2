#!/bin/bash
set -euo pipefail

REPO_DIR="$(cd "$(dirname "$0")" && pwd)"
BIN_DIR="${HOME}/.local/bin"
BIN_NAME="m2sh"

echo "=== pretty-shell installer ==="

if ! command -v cargo &>/dev/null; then
    echo "error: 'cargo' not found. Install Rust first: https://rustup.rs"
    exit 1
fi
if ! command -v M2 &>/dev/null; then
    echo "error: 'M2' not found. Install Macaulay2 first."
    exit 1
fi

echo "prerequisites: OK (cargo $(cargo --version | awk '{print $2}'), M2 $(M2 --version 2>&1 | head -1 | awk '{print $2}'))"

echo "building..."
cd "$REPO_DIR"
cargo build --release

mkdir -p "$BIN_DIR"
install -m 755 target/release/m2sh "$BIN_DIR/$BIN_NAME"

echo "installed: $BIN_DIR/$BIN_NAME"
echo ""
echo "Run 'm2sh' to start the Macaulay2 highlighted REPL."