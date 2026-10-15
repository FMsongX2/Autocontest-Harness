#!/usr/bin/env bash
# Everything a pull request or a release must pass. There is no CI: run this locally.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== cargo fmt --check"
cargo fmt --check
echo "== cargo clippy"
cargo clippy --all-targets --locked -- -D warnings
echo "== cargo test"
cargo test --locked
if command -v cargo-audit >/dev/null 2>&1; then
  echo "== cargo audit"
  cargo audit --deny warnings
else
  echo "== cargo audit: not installed (cargo install cargo-audit --locked); skipped"
fi
echo "ok"
