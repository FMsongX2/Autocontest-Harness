#!/usr/bin/env bash
# Install the `harness` binary for this checkout.
#
#   scripts/install.sh                 prebuilt binary from the GitHub release matching Cargo.toml's
#                                      version, checked against SHA256SUMS (falls back to building)
#   scripts/install.sh --from-source   build with cargo instead
#
# The binary goes to <checkout>/bin/harness and a symlink to it is placed in $BIN_DIR
# (default ~/.local/bin). Through that symlink the binary finds this checkout, so prompts,
# agents and data/ are read from here. Run it again after `git pull` to upgrade.
set -euo pipefail

REPO="FMsongX2/Autocontest-Harness"
ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
BIN_DIR="${BIN_DIR:-$HOME/.local/bin}"
FROM_SOURCE=0
case "${1:-}" in
  --from-source) FROM_SOURCE=1 ;;
  "") ;;
  -h|--help) sed -n '2,11p' "$0"; exit 0 ;;
  *) echo "unknown option: $1" >&2; exit 2 ;;
esac

VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$ROOT/Cargo.toml" | head -n 1)"
[ -n "$VERSION" ] || { echo "cannot read the version from $ROOT/Cargo.toml" >&2; exit 1; }

target() {
  local os arch
  os="$(uname -s)"; arch="$(uname -m)"
  case "$os/$arch" in
    Darwin/arm64) echo aarch64-apple-darwin ;;
    Darwin/x86_64) echo x86_64-apple-darwin ;;
    Linux/x86_64) echo x86_64-unknown-linux-musl ;;
    Linux/aarch64|Linux/arm64) echo aarch64-unknown-linux-musl ;;
    *) return 1 ;;
  esac
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

download() {
  local tgt asset base want got
  tgt="$(target)" || { echo "no prebuilt binary for $(uname -s)/$(uname -m)"; return 1; }
  asset="harness-$VERSION-$tgt.tar.gz"
  base="https://github.com/$REPO/releases/download/v$VERSION"
  echo "downloading $asset (v$VERSION)"
  curl -fsSL --retry 2 -o "$TMP/$asset" "$base/$asset" || { echo "no release asset $base/$asset"; return 1; }
  curl -fsSL --retry 2 -o "$TMP/SHA256SUMS" "$base/SHA256SUMS" || { echo "no SHA256SUMS in release v$VERSION"; return 1; }
  want="$(grep " $asset\$" "$TMP/SHA256SUMS" | cut -d' ' -f1 || true)"
  got="$(sha256 "$TMP/$asset")"
  if [ -z "$want" ] || [ "$want" != "$got" ]; then
    echo "checksum mismatch for $asset (expected ${want:-none}, got $got)" >&2
    exit 1
  fi
  tar -xzf "$TMP/$asset" -C "$TMP"
  mkdir -p "$ROOT/bin"
  install -m 0755 "$TMP/harness" "$ROOT/bin/harness"
}

build() {
  command -v cargo >/dev/null 2>&1 || { echo "cargo not found: install Rust from https://rustup.rs" >&2; exit 1; }
  echo "building from source"
  (cd "$ROOT" && cargo build --release --locked)
  mkdir -p "$ROOT/bin"
  install -m 0755 "$ROOT/target/release/harness" "$ROOT/bin/harness"
}

if [ "$FROM_SOURCE" = 1 ]; then
  build
elif ! download; then
  echo "falling back to a source build"
  build
fi

mkdir -p "$BIN_DIR"
ln -sfn "$ROOT/bin/harness" "$BIN_DIR/harness"
echo "installed: $BIN_DIR/harness -> $ROOT/bin/harness"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo "note: $BIN_DIR is not on PATH; add it to your shell profile" ;;
esac
"$BIN_DIR/harness" --version
echo "next: harness doctor"
