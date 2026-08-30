#!/usr/bin/env bash
# publications.sh — run texrs over a real LaTeX corpus and report what it did.
#
#   bash scripts/publications.sh                    # ../MenkeTechnologiesPublications
#   bash scripts/publications.sh /path/to/corpus    # somewhere else
#   bash scripts/publications.sh --gate             # exit non-zero if any document failed
#   TEXRS_JOBS=8 bash scripts/publications.sh       # how many at once (default: one per core)
#
# The tests in `tests/` pin behaviour a sentence at a time and the corpus in
# `tests/cases` pins byte-for-byte parity with real tex. Neither says whether a
# 16,000-line book compiles, and that is the question the LaTeX layer exists to
# answer: every fault it has found so far — the arm of a decided conditional
# that must not run, an optional argument declared and never matched, a stub
# whose arity does not match the package's — was invisible to both and obvious
# here within one sweep.
#
# The README states a number from this sweep. It is a MEASUREMENT, so it has to
# be re-measurable: this is the command that produces it, rather than a figure
# someone remembers. Nothing here is an allowlist — every document that fails is
# printed, whatever the reason, and a corpus that vendors texrs's own fixtures
# will show the ones written to be refused among them.
#
# "Runs" is the exact claim, and it is what this measures: the mouth and the
# expander read the whole document and produce what its text says. Nothing is
# typeset. The bytes of text are reported next to the count because a document
# that "runs" and says nothing is not the same result as one that says a book.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

GATE=0
CORPUS=""
for a in "$@"; do
  case "$a" in
    --gate) GATE=1 ;;
    -*) echo "publications: unknown option $a" >&2; exit 2 ;;
    *) CORPUS="$a" ;;
  esac
done
: "${CORPUS:=$ROOT/../MenkeTechnologiesPublications}"

# One document, run from its own directory so a relative \input or \include
# resolves the way it does for the build that produced the .pdf next to it.
# Prints one TSV row: path, exit status, bytes of text, first line of stderr.
if [ "${TEXRS_ONE:-0}" = "1" ]; then
  file="$1"
  # The text is measured rather than kept: a book is megabytes, and `wc -c`
  # counts the bytes a shell variable would have counted as characters.
  bytes=$(cd "$(dirname "$file")" && timeout "${TEXRS_TIMEOUT:-120}" "$TEXRS_BIN" \
          --text --no-cache -interaction=nonstopmode "$(basename "$file")" \
          2>"$TEXRS_ERR.$$" | wc -c)
  rc=${PIPESTATUS[0]}
  said=$(head -1 "$TEXRS_ERR.$$" | tr -d '\t')
  rm -f "$TEXRS_ERR.$$"
  printf '%s\t%s\t%s\t%s\n' "$file" "$rc" "$(echo "$bytes" | tr -d ' ')" "$said"
  exit 0
fi

[ -d "$CORPUS" ] || { echo "publications: no corpus at $CORPUS" >&2; exit 1; }
CORPUS="$(cd "$CORPUS" && pwd)"

BIN="${TEXRS_BIN:-$ROOT/target/debug/texrs}"
if [ ! -x "$BIN" ]; then
  ( cd "$ROOT" && cargo build ) || exit 1
fi
[ -x "$BIN" ] || { echo "publications: no texrs binary at $BIN" >&2; exit 1; }

JOBS="${TEXRS_JOBS:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)}"
LOG="${TEXRS_LOG:-$ROOT/target/publications.tsv}"
mkdir -p "$(dirname "$LOG")"

echo "corpus  $CORPUS"
echo "engine  $BIN"
echo "jobs    $JOBS"

# The bytecode cache is keyed on the file, so a sweep that used it would
# measure the last build rather than this one. --no-cache above, and each
# document is compiled from source.
export TEXRS_BIN="$BIN" TEXRS_ONE=1 TEXRS_ERR="${TMPDIR:-/tmp}/texrs-publications-err"
export TEXRS_TIMEOUT="${TEXRS_TIMEOUT:-120}"
find "$CORPUS" -name '*.tex' -not -path '*/.git/*' -print0 \
  | sort -z \
  | xargs -0 -P "$JOBS" -n 1 "$0" \
  | sort > "$LOG"

total=$(wc -l < "$LOG" | tr -d ' ')
[ "$total" -gt 0 ] || { echo "publications: no .tex under $CORPUS" >&2; exit 1; }
ok=$(awk -F'\t' '$2 == 0' "$LOG" | wc -l | tr -d ' ')
bytes=$(awk -F'\t' '$2 == 0 { n += $3 } END { print n + 0 }' "$LOG")

echo
printf '%s of %s documents run to completion, and say %s bytes of text.\n' \
  "$ok" "$total" "$bytes"
echo "  $LOG"

if [ "$ok" != "$total" ]; then
  echo
  echo "did not finish:"
  # The prefix goes before the padding, or the column does not line up.
  perl -pe "s{\Q$CORPUS\E/}{}" "$LOG" \
    | awk -F'\t' '$2 != 0 { printf "  %-52s rc=%-3s %s\n", $1, $2, $4 }'
fi

[ "$GATE" = "1" ] && [ "$ok" != "$total" ] && exit 1
exit 0
