#!/usr/bin/env bash
# fuzz_parity.sh — differential fuzzer: texrs against the real `tex` binary.
#
# Generates a seeded corpus of random TeX programs (scripts/fuzz/gen.pl), runs
# every one under BOTH engines, and reports every program whose `\message`
# stream differs. Each divergence is then REDUCED — statements are dropped for
# as long as the divergence survives — so what lands in target/fuzz/diverge/ is
# small enough to read and to commit to tests/cases as a regression case.
#
#   bash scripts/fuzz_parity.sh                  # 100 programs, seed 1
#   bash scripts/fuzz_parity.sh -n 2000 -s 42    # bigger corpus, new seed
#   bash scripts/fuzz_parity.sh -c DIR           # re-check an existing corpus
#   bash scripts/fuzz_parity.sh -1 case.tex      # one file, verbose
#
#   -n N       corpus size (default 100)
#   -s SEED    PRNG seed (default 1); same seed => same corpus, so a divergence
#              reproduces exactly on any machine
#   -d DEPTH   max nesting of conditionals and groups (default 2)
#   -j JOBS    parallel cases (default: the machine's CPU count)
#   -t SECS    per-engine timeout for one case (default 10)
#   -c DIR     use an existing corpus directory instead of generating one
#   -1 FILE    check exactly one file and print both streams
#   -R         do not reduce divergences (faster; the raw case is kept)
#   -q         summary only
#
# Exit status is the number of diverging programs, capped at 250. Artifacts land
# in target/fuzz/ (gitignored).
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
. "$ROOT/scripts/lib.sh"

N=100 SEED=1 DEPTH=2 JOBS= CORPUS= ONE= REDUCE=1 QUIET=0
while [ $# -gt 0 ]; do
  case "$1" in
    -n) N="$2"; shift 2 ;;
    -s) SEED="$2"; shift 2 ;;
    -d) DEPTH="$2"; shift 2 ;;
    -j) JOBS="$2"; shift 2 ;;
    -t) CASE_TIMEOUT="$2"; shift 2 ;;
    -c) CORPUS="$2"; shift 2 ;;
    -1) ONE="$2"; shift 2 ;;
    -R) REDUCE=0; shift ;;
    -q) QUIET=1; shift ;;
    # Only the header block: the body's explanatory comments are not usage text.
    -h|--help) perl -ne 'next if $. == 1; last unless /^#/; s/^# ?//; print' "$0"; exit 0 ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
done
export CASE_TIMEOUT="${CASE_TIMEOUT:-10}"
: "${JOBS:=$( (command -v nproc >/dev/null && nproc) || sysctl -n hw.ncpu 2>/dev/null || echo 4 )}"

if [ -t 1 ]; then C=$'\033[36m'; G=$'\033[32m'; R=$'\033[31m'; D=$'\033[2m'; Z=$'\033[0m'; else C= G= R= D= Z=; fi
say() { [ "$QUIET" = 1 ] || printf '%s==>%s %s\n' "$C" "$Z" "$1"; }

oracle_tex || exit 2
subject_bin || exit 2
export TEX BIN

# ---- one case --------------------------------------------------------------
# Prints nothing when the two engines agree; otherwise the case name and both
# streams. Used directly by -1 and, through the worker below, by the corpus run.
check() { # check FILE -> 0 parity, 1 diverges
  local f="$1" want got
  want=$(reference "$f"); got=$(subject "$f")
  [ "$want" = "$got" ] && return 0
  printf '%s\n  tex   : %s\n  texrs : %s\n' "$f" "$want" "$got"
  return 1
}

if [ -n "$ONE" ]; then
  printf 'oracle: tex %s at %s\n' "$ORACLE_VERSION" "$ORACLE_PATH"
  if check "$ONE"; then printf '%sPARITY%s %s\n' "$G" "$Z" "$ONE"; exit 0; fi
  exit 1
fi

OUT="$ROOT/target/fuzz"
CASES="$OUT/cases"
DIVERGE="$OUT/diverge"
command rm -rf "$DIVERGE"; mkdir -p "$OUT" "$DIVERGE"

# ---- corpus ----------------------------------------------------------------
if [ -n "$CORPUS" ]; then
  CASES="$CORPUS"
else
  command rm -rf "$CASES"; mkdir -p "$CASES"
  say "generating $N programs (seed $SEED, depth $DEPTH)"
  perl "$ROOT/scripts/fuzz/gen.pl" "$SEED" "$N" "$CASES" "$DEPTH" || exit 2
fi
TOTAL=$(find "$CASES" -name '*.tex' | wc -l | tr -d ' ')
[ "$TOTAL" -gt 0 ] || { echo "empty corpus" >&2; exit 2; }

# ---- run both engines ------------------------------------------------------
# One process per case, $JOBS at a time. Cases are independent, and tex is the
# slow half — a serial run of a few thousand programs is minutes of wall clock
# that the machine has no reason to spend.
say "checking $TOTAL programs against tex $ORACLE_VERSION ($JOBS jobs, ${CASE_TIMEOUT}s each)"
export ROOT
worker() {
  # A subshell per case, because `check' needs the sourced library.
  . "$ROOT/scripts/lib.sh"
  want=$(reference "$1"); got=$(subject "$1")
  [ "$want" = "$got" ] && exit 0
  printf '%s\t%s\t%s\n' "$1" "$want" "$got"
}
export -f worker
find "$CASES" -name '*.tex' | sort | xargs -P "$JOBS" -I{} bash -c 'worker "$@"' _ {} > "$OUT/diverge.tsv"
BAD=$(grep -c . "$OUT/diverge.tsv" || true)

if [ "${BAD:-0}" -eq 0 ]; then
  printf '%sPARITY: %d/%d generated programs agree with tex.%s\n' "$G" "$TOTAL" "$TOTAL" "$Z"
  exit 0
fi

# ---- reduce ----------------------------------------------------------------
# A generated program is 5-10 statements and most of them have nothing to do
# with the divergence. Dropping one statement at a time, keeping the drop only
# while the two engines still disagree, leaves the smallest program that still
# shows the bug — which is what gets committed to tests/cases.
# A reduction step must stay in the SAME divergence class. Dropping a `\def'
# turns the program into an `Undefined control sequence' error, which is also a
# divergence -- so an unconstrained reducer walks every finding into the same
# uninteresting shape and throws away the bug it started from. Whether real tex
# errored is the class marker.
errored() { case "$1" in *'! '*) echo 1 ;; *) echo 0 ;; esac; }

reduce() { # reduce FILE OUTFILE
  local f="$1" out="$2" tmp n i kept class
  tmp=$(mktemp -d); cp "$f" "$tmp/cur.tex"
  class=$(errored "$(reference "$f")")
  n=$(grep -c '' "$tmp/cur.tex")
  i=2   # never drop line 1 (the catcode preamble)
  while [ "$i" -le "$n" ]; do
    # \end must stay: without it the program has no terminator to compare at.
    if [ "$(perl -ne "print if \$. == $i" "$tmp/cur.tex")" = '\end' ]; then i=$((i+1)); continue; fi
    perl -ne "print unless \$. == $i" "$tmp/cur.tex" > "$tmp/try.tex"
    if check "$tmp/try.tex" >/dev/null 2>&1 || [ "$(errored "$(reference "$tmp/try.tex")")" != "$class" ]; then
      i=$((i+1))                       # still agrees without it: the line mattered
    else
      mv "$tmp/try.tex" "$tmp/cur.tex" # still diverges: the line was noise
      n=$((n-1))
    fi
  done
  kept=$(grep -c '' "$tmp/cur.tex")
  cp "$tmp/cur.tex" "$out"
  command rm -rf "$tmp"
  printf '%s' "$kept"
}

say "reducing $BAD divergence(s)"
i=0
while IFS=$'\t' read -r f want got; do
  [ -n "$f" ] || continue
  i=$((i+1))
  dst="$DIVERGE/$(printf 'd%03d' "$i").tex"
  if [ "$REDUCE" = 1 ]; then
    lines=$(reduce "$f" "$dst")
    want=$(reference "$dst"); got=$(subject "$dst")
  else
    cp "$f" "$dst"; lines=$(grep -c '' "$dst")
  fi
  printf '%s%s%s  %s(%s lines)%s\n  tex   : [%s]\n  texrs : [%s]\n' \
    "$R" "$(basename "$dst")" "$Z" "$D" "$lines" "$Z" "$want" "$got"
  [ "$QUIET" = 1 ] || perl -ne 'print "    $_"' "$dst"
done < "$OUT/diverge.tsv"

printf '\n%s%d/%d generated programs diverge from tex%s %s(%s)%s\n' \
  "$R" "$BAD" "$TOTAL" "$Z" "$D" "$DIVERGE" "$Z"
echo "commit a reduced case into tests/cases to keep it from coming back"
[ "$BAD" -gt 250 ] && BAD=250
exit "$BAD"
