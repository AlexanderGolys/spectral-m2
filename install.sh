#!/bin/bash
set -euo pipefail

REPO_DIR="$(cd "$(dirname "$0")" && pwd)"

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

echo "installing through Cargo..."
cargo install --path "$REPO_DIR" --locked --force

echo "installed: spectral-m2"
echo ""
echo "Run 'spectral-m2' to start the Macaulay2 highlighted REPL."
