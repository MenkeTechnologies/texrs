#!/usr/bin/env bash
# parity.sh — differential harness: texrs against the real `tex` binary.
#
# The contract for this milestone is the \message stream. Real tex prints it
# between the opened filename and the closing paren:
#
#   This is TeX, Version 3.141592653 (TeX Live 2026) (preloaded format=tex)
#   (./t.tex HELLO-WORLD count=12 )
#
# so the comparison extracts exactly that and diffs it. The banner line carries
# the engine version and is deliberately NOT compared -- texrs is not claiming to
# be TeX Live. Everything between the filename and the `)` is.
#
#   bash scripts/parity.sh                 # the committed corpus
#   bash scripts/parity.sh case.tex        # one ad-hoc case
#
# The extraction and the oracle version gate live in scripts/lib.sh, shared with
# scripts/fuzz_parity.sh: two harnesses that extract differently are asking the
# oracle two different questions.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
. "$ROOT/scripts/lib.sh"

oracle_tex || exit 2
subject_bin || exit 2

run_case() {
  local f="$1" want got
  want=$(reference "$f"); got=$(subject "$f")
  if [ "$want" = "$got" ]; then
    printf 'PARITY   %s\n' "$(basename "$f")"; return 0
  fi
  printf 'DIVERGES %s\n  tex   : [%s]\n  texrs : [%s]\n' "$(basename "$f")" "$want" "$got"; return 1
}

if [ $# -gt 0 ]; then run_case "$1"; exit $?; fi

printf 'oracle: tex %s at %s\n\n' "$ORACLE_VERSION" "$ORACLE_PATH"
fail=0; total=0
for f in "$ROOT"/tests/cases/*.tex; do
  [ -e "$f" ] || continue
  total=$((total+1)); run_case "$f" || fail=$((fail+1))
done
printf '\n%d/%d cases in parity\n' "$((total-fail))" "$total"
[ "$fail" -eq 0 ]
