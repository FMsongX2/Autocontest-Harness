#!/usr/bin/env bash
# Install `ach`, the harness core that its agents call, into this checkout.
#
#   scripts/install.sh                 prebuilt binary from the GitHub release matching Cargo.toml's
#                                      version, checked against SHA256SUMS (falls back to building)
#   scripts/install.sh --from-source   build with cargo instead
#
# The binary goes to <checkout>/bin/ach and nowhere else: it is not put on your PATH. Sessions
# the harness starts get it on theirs. Run it again after `git pull` to upgrade.
set -euo pipefail

REPO="FMsongX2/Autocontest-Harness"
ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
BIN_DIR="${BIN_DIR:-$HOME/.local/bin}"   # only to remove the PATH link older versions made
FROM_SOURCE=0
case "${1:-}" in
  --from-source) FROM_SOURCE=1 ;;
  "") ;;
  -h|--help) sed -n '2,9p' "$0"; exit 0 ;;
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
  asset="ach-$VERSION-$tgt.tar.gz"
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
  install -m 0755 "$TMP/ach" "$ROOT/bin/ach"
}

build() {
  command -v cargo >/dev/null 2>&1 || { echo "cargo not found: install Rust from https://rustup.rs" >&2; exit 1; }
  echo "building from source"
  (cd "$ROOT" && cargo build --release --locked)
  mkdir -p "$ROOT/bin"
  install -m 0755 "$ROOT/target/release/ach" "$ROOT/bin/ach"
}

if [ "$FROM_SOURCE" = 1 ]; then
  build
elif ! download; then
  echo "falling back to a source build"
  build
fi

# Up to 0.5 the binary was `bin/harness`, linked as $BIN_DIR/harness. Remove both if they are ours.
if [ -L "$BIN_DIR/harness" ] && [ "$(readlink "$BIN_DIR/harness")" = "$ROOT/bin/harness" ]; then
  rm "$BIN_DIR/harness"
  echo "removed the old link $BIN_DIR/harness"
fi
rm -f "$ROOT/bin/harness"

echo "installed: $ROOT/bin/ach"
"$ROOT/bin/ach" --version
echo "next: open your coding agent in $ROOT and ask it to read INIT.md and set up the harness"
