#!/usr/bin/env bash
# Build release binaries and publish a GitHub release (maintainers; there is no CI).
#
#   scripts/release.sh            build, check and package into dist/ only
#   scripts/release.sh --publish  also tag v<version>, push the tag and create the GitHub release
#
# Requirements: a clean tree on main whose Cargo.toml version has a `## <version>` section in
# CHANGELOG.md; rustup targets aarch64-apple-darwin, x86_64-apple-darwin,
# x86_64-unknown-linux-musl, aarch64-unknown-linux-musl; zig + cargo-zigbuild for the Linux
# targets; gh logged in for --publish.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$ROOT"
PUBLISH=0
[ "${1:-}" = "--publish" ] && PUBLISH=1

VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"
TAG="v$VERSION"
MAC_TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
LINUX_TARGETS=(x86_64-unknown-linux-musl aarch64-unknown-linux-musl)

fail() { echo "release: $*" >&2; exit 1; }

[ -z "$(git status --porcelain)" ] || fail "working tree is not clean"
[ "$(git rev-parse --abbrev-ref HEAD)" = main ] || fail "not on main"
grep -q "^## $VERSION" CHANGELOG.md || fail "CHANGELOG.md has no '## $VERSION' section"
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then fail "tag $TAG already exists"; fi
command -v cargo-zigbuild >/dev/null 2>&1 || fail "cargo-zigbuild not installed (brew install zig && cargo install cargo-zigbuild --locked)"

scripts/check.sh

SHA="$(git rev-parse --short=12 HEAD)"
BUILD="$(mktemp -d /tmp/autocontest-harness-release.XXXXXX)"
trap 'rm -rf "$BUILD"' EXIT
# Build from an exported tree at a neutral path, and strip local paths from the binaries.
git archive --format=tar HEAD | tar -x -C "$BUILD"
export HARNESS_GIT_SHA="$SHA"
export RUSTFLAGS="--remap-path-prefix=$BUILD=/build --remap-path-prefix=$HOME/.cargo=/cargo --remap-path-prefix=$HOME/.rustup=/rustup --remap-path-prefix=$HOME=/home"
export CARGO_TARGET_DIR="$BUILD/target"

(cd "$BUILD" && for t in "${MAC_TARGETS[@]}"; do cargo build --release --locked --target "$t"; done)
(cd "$BUILD" && for t in "${LINUX_TARGETS[@]}"; do cargo zigbuild --release --locked --target "$t"; done)

DIST="$ROOT/dist/$TAG"
rm -rf "$DIST"
mkdir -p "$DIST"
for t in "${MAC_TARGETS[@]}" "${LINUX_TARGETS[@]}"; do
  bin="$CARGO_TARGET_DIR/$t/release/harness"
  [ -x "$bin" ] || fail "missing binary for $t"
  # A published binary must not carry the builder's username or home directory.
  if LC_ALL=C grep -a -q -e "$HOME" -e "/$(id -un)/" "$bin"; then fail "$t binary contains a local path"; fi
  stage="$BUILD/stage-$t"
  mkdir -p "$stage"
  cp "$bin" "$stage/harness"
  cp LICENSE "$stage/"
  tar -czf "$DIST/harness-$VERSION-$t.tar.gz" -C "$stage" harness LICENSE
done
(cd "$DIST" && shasum -a 256 harness-*.tar.gz > SHA256SUMS)
"$CARGO_TARGET_DIR/aarch64-apple-darwin/release/harness" --version 2>/dev/null || true
echo "packaged into $DIST:"
ls -l "$DIST"

if [ "$PUBLISH" = 1 ]; then
  notes="$BUILD/notes.md"
  awk -v v="## $VERSION" 'index($0, v) == 1 {on = 1; next} /^## / && on {exit} on' CHANGELOG.md > "$notes"
  git tag -a "$TAG" -m "Autocontest-Harness $VERSION"
  git push origin "$TAG"
  gh release create "$TAG" --title "Autocontest-Harness $VERSION" --notes-file "$notes" "$DIST"/*
  echo "published $TAG"
fi
