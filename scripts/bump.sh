#!/usr/bin/env bash
# bump.sh — set the texrs version everywhere, verify, commit, tag, push, publish.
#
#   bash scripts/bump.sh patch    # 0.4.0 -> 0.4.1
#   bash scripts/bump.sh minor    # 0.4.0 -> 0.5.0
#   bash scripts/bump.sh major    # 0.4.0 -> 1.0.0
#   bash scripts/bump.sh 1.2.3    # exactly that
#   bash scripts/bump.sh patch --dry-run    # say what would change, change nothing
#
# The version lives in six tracked files, and nothing in a build or a test run
# notices when they disagree: the code compiles, the pages render, the man page
# formats. That is how v0.1.0 sat in the docs through v0.3.0 and the man pages
# fell three versions behind. `tests/version_sync.rs` is the gate that makes the
# drift a failure; this is the one command that avoids causing it.
#
# Two of the six are GENERATED rather than stamped — docs/reference.html and the
# Emacs primitive table — so they are rebuilt from the corpus here rather than
# text-substituted, which also picks up any primitive added since the last bump.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DRY=0
for a in "$@"; do
  [ "$a" = "--dry-run" ] && DRY=1
done

CURRENT=$(perl -ne 'if (/^version\s*=\s*"(\d+\.\d+\.\d+)"/) { print $1; exit }' Cargo.toml)
[ -n "$CURRENT" ] || { echo "bump: no version in Cargo.toml" >&2; exit 1; }

MAJOR=${CURRENT%%.*}; _r=${CURRENT#*.}; MINOR=${_r%%.*}; PATCH=${_r#*.}
case "${1:-patch}" in
  patch) PATCH=$((PATCH + 1)) ;;
  minor) MINOR=$((MINOR + 1)); PATCH=0 ;;
  major) MAJOR=$((MAJOR + 1)); MINOR=0; PATCH=0 ;;
  [0-9]*.[0-9]*.[0-9]*) MAJOR=${1%%.*}; _r=${1#*.}; MINOR=${_r%%.*}; PATCH=${_r#*.} ;;
  *) echo "usage: bump.sh [patch|minor|major|X.Y.Z] [--dry-run]" >&2; exit 1 ;;
esac
NEW="${MAJOR}.${MINOR}.${PATCH}"
TODAY=$(date +%Y-%m-%d)

echo "bumping $CURRENT -> $NEW"
if [ "$DRY" = 1 ]; then
  echo "  (dry run: nothing is written)"
  exit 0
fi

# The manifest, and only its package version -- an anchored match, so a
# dependency that happens to be at the same number is untouched.
perl -i -pe "s/^version = \"\Q$CURRENT\E\"/version = \"$NEW\"/" Cargo.toml

# The two hand-written docs pages carry a build line reading `texrs vX.Y.Z · …`.
#
# Only the BRANDED form is stamped, never a bare `vX.Y.Z`. A sentence dating a
# change ("the expander gained \futurelet in v0.3.1") is true and must survive a
# bump, and a stamp that rewrote every version-shaped string would falsify it --
# the same carelessness that turned an `0.0.0.0` IP address into `0.18.1.0` in a
# sibling's docs during a sweep today. tests/version_sync.rs enforces the same
# split from the other side.
for f in docs/index.html docs/report.html; do
  perl -i -pe "s/texrs v\Q$CURRENT\E/texrs v$NEW/g" "$f"
done

# The man pages carry `.TH NAME 1 "DATE" "texrs X.Y.Z" "User Commands"`. The
# date moves with the version: a page stamped with the version of today's
# release and the date of a release three weeks ago is worse than no date.
for f in man/man1/*.1; do
  perl -i -pe "s/^(\.TH \S+ 1 )\"[^\"]*\" \"texrs [0-9.]+\"/\${1}\"$TODAY\" \"texrs $NEW\"/" "$f"
done

# The IntelliJ plugin speaks LSP and DAP to this binary; its version tracks the
# crate's so a published zip names the engine it was built against.
if [ -f editors/intellij/gradle.properties ]; then
  perl -i -pe "s/^pluginVersion=\Q$CURRENT\E\$/pluginVersion=$NEW/" editors/intellij/gradle.properties
fi

# The generated pair, rebuilt rather than substituted.
cargo run -q --bin gen-docs
cargo run -q --bin gen-emacs-stdlib

echo "  Cargo.toml, docs/index.html, docs/report.html: $NEW"
echo "  man/man1/*.1: $NEW ($TODAY)"
echo "  editors/intellij/gradle.properties: $NEW"
echo "  docs/reference.html, editors/emacs/texrs-stdlib.el: regenerated"

# A bump that ships a broken tree is worse than a bump that never happened, and
# tests/version_sync.rs is in here: it fails if any of the six was missed.
echo ""
echo "verifying..."
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
echo "verify ok"

# `-f` on the docs: they are tracked, but the global gitignore carries `docs/`,
# and a plain `git add docs/...` refuses -- which `set -e` would turn into an
# abort after the files were already rewritten.
git add Cargo.toml Cargo.lock man/man1/*.1 editors/emacs/texrs-stdlib.el
[ -f editors/intellij/gradle.properties ] && git add editors/intellij/gradle.properties
git add -f docs/index.html docs/report.html docs/reference.html
git commit -m "bump v$NEW"
git tag "v$NEW"
git push origin HEAD
git push origin "v$NEW"

echo ""
echo "publishing texrs v$NEW to crates.io..."
cargo publish

echo ""
echo "done: texrs v$NEW stamped in six files, verified, tagged, pushed, published"
