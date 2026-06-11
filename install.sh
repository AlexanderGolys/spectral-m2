#!/bin/bash
set -euo pipefail

REPO_DIR="$(cd "$(dirname "$0")" && pwd)"
BIN_DIR="${HOME}/.local/bin"
BIN_NAME="spectral-m2"
ALIAS_NAME="sp2"

echo "=== spectral-m2 installer ==="

if ! command -v cargo &>/dev/null; then
    echo "error: 'cargo' not found. Install Rust first: https://rustup.rs"
    exit 1
fi
if ! command -v M2 &>/dev/null; then
    echo "error: 'M2' not found. Install Macaulay2 first."
    exit 1
fi

echo "prerequisites: OK (cargo $(cargo --version | awk '{print $2}'), M2 $(M2 --version 2>&1 | head -1 | awk '{print $2}'))"

cd "$REPO_DIR"

# Quality gates — any failure aborts the install (set -e).
echo "checking formatting..."
cargo fmt --all --check

echo "running clippy..."
cargo clippy --all-targets -- -D warnings

echo "running tests..."
cargo test

echo "building (release)..."
cargo build --release

mkdir -p "$BIN_DIR"
install -m 755 target/release/spectral-m2 "$BIN_DIR/$BIN_NAME"
# short alias for convenience: `sp2` -> spectral-m2
ln -sf "$BIN_NAME" "$BIN_DIR/$ALIAS_NAME"

echo "installed: $BIN_DIR/$BIN_NAME (alias: $ALIAS_NAME)"
echo ""
echo "Run 'spectral-m2' (or 'sp2') to start the Macaulay2 highlighted REPL."
