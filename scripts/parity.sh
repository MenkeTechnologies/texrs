#!/usr/bin/env bash
# parity.sh — differential harness: texrs against the real `tex` binary.
#
# Milestone 1's contract is the \message stream. Real tex prints it inline
# between the opened filename and the closing paren:
#
#   This is TeX, Version 3.141592653 (TeX Live 2026) (preloaded format=tex)
#   (./t.tex HELLO-WORLD count=12 )
#
# so the comparison extracts exactly that and diffs it. The banner line carries
# the engine version and is deliberately NOT compared -- texrs is not claiming
# to be TeX Live. Everything between the filename and the `)` is.
#
#   bash scripts/parity.sh                 # run the committed corpus
#   bash scripts/parity.sh case.tex        # one ad-hoc case, verbose
set -u
BIN="${TEXRS:-$(dirname "$0")/../target/debug/texrs}"
TEX="${TEX_ORACLE:-tex}"
command -v "$TEX" >/dev/null || { echo "no reference tex on PATH"; exit 2; }

# The message stream real tex produced for $1, or the marker ERROR.
reference() {
  local f="$1" d out
  d=$(mktemp -d)
  cp "$f" "$d/case.tex"
  out=$( cd "$d" && "$TEX" -interaction=nonstopmode case.tex 2>&1 )
  command rm -rf "$d"
  # The line that opens the file; strip `(./case.tex` and the trailing `)`.
  printf '%s\n' "$out" | perl -ne '
      next unless s{^\(\./case\.tex}{};
      s{\s*\)\s*$}{};
      s{^\s+}{};
      print; exit' 
}

subject() {
  local out
  out=$("$BIN" "$1" 2>&1) || { printf 'ERROR'; return; }
  printf '%s\n' "$out" | perl -ne '
      next unless s{^\(\./[^ )]*}{};
      s{\s*\)\s*$}{};
      s{^\s+}{};
      print; exit'
}

run_case() {
  local f="$1" want got
  want=$(reference "$f"); got=$(subject "$f")
  if [ "$want" = "$got" ]; then
    printf 'PARITY   %s\n' "$(basename "$f")"; return 0
  fi
  printf 'DIVERGES %s\n  tex   : [%s]\n  texrs : [%s]\n' "$(basename "$f")" "$want" "$got"; return 1
}

if [ $# -gt 0 ]; then run_case "$1"; exit $?; fi

fail=0; total=0
for f in "$(dirname "$0")"/../tests/cases/*.tex; do
  [ -e "$f" ] || continue
  total=$((total+1)); run_case "$f" || fail=$((fail+1))
done
printf '\n%d/%d cases in parity\n' "$((total-fail))" "$total"
[ "$fail" -eq 0 ]
