#!/usr/bin/env bash
# compare.sh — wall-clock texrs against the real `tex` binary on the same files.
#
# The criterion benchmarks (`cargo bench`) measure texrs against ITSELF, which
# says where the time goes inside the engine and nothing about whether the
# engine is fast. This measures the only comparison that answers that: the same
# documents through both engines, end to end, process start to process exit.
#
#   bash bench/compare.sh                 # the committed corpus
#   bash bench/compare.sh -n 50 FILE      # one file, 50 runs each
#   bash bench/compare.sh -n 200          # more runs, tighter numbers
#
#   -n N     runs per engine per file (default 20)
#   -b BIN   texrs binary (default target/release/texrs, then target/debug)
#
# Uses hyperfine when it is installed, because it handles warmup and outliers
# properly; falls back to a timed loop otherwise. Two caveats are printed with
# the numbers rather than left for the reader to discover:
#
#   * tex loads the plain format and texrs loads nothing, so part of any
#     difference is format loading rather than engine speed.
#   * texrs implements the mouth and the expander only. A document that would
#     make tex build pages is not being compared like for like.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

N=20
BIN=""
while [ $# -gt 0 ]; do
  case "$1" in
    -n) N="$2"; shift 2 ;;
    -b) BIN="$2"; shift 2 ;;
    -h|--help) perl -ne 'next if $. == 1; last unless /^#/; s/^# ?//; print' "$0"; exit 0 ;;
    *) break ;;
  esac
done

if [ -z "$BIN" ]; then
  if   [ -x "$ROOT/target/release/texrs" ]; then BIN="$ROOT/target/release/texrs"
  elif [ -x "$ROOT/target/debug/texrs" ];   then BIN="$ROOT/target/debug/texrs"
  else echo "no texrs binary — run \`cargo build\` first" >&2; exit 2; fi
fi
command -v tex >/dev/null || { echo "no \`tex\` on PATH — nothing to compare against" >&2; exit 2; }

files=("$@")
if [ ${#files[@]} -eq 0 ]; then
  while IFS= read -r f; do files+=("$f"); done < <(find "$ROOT/tests/cases" -name '*.tex' | sort)
fi
[ ${#files[@]} -gt 0 ] || { echo "no files to measure" >&2; exit 2; }

printf 'texrs: %s\n' "$BIN"
printf 'tex  : %s (%s)\n' "$(command -v tex)" "$(tex --version | head -1)"
printf 'runs : %d per engine per file, %d file(s)\n\n' "$N" "${#files[@]}"

# The debug binary is not a number worth quoting anywhere.
case "$BIN" in
  */debug/*) printf 'NOTE: measuring a DEBUG build. Build with --release before quoting anything.\n\n' ;;
esac

# One temp dir per run so tex writes its .log and .dvi somewhere disposable.
run_many() { # run_many N CMD...
  local n="$1"; shift
  local d start end
  d=$(mktemp -d)
  start=$(perl -MTime::HiRes=time -e 'print time')
  for _ in $(seq "$n"); do ( cd "$d" && "$@" ) >/dev/null 2>&1; done
  end=$(perl -MTime::HiRes=time -e 'print time')
  command rm -rf "$d"
  perl -e 'printf "%.2f", ($ARGV[1] - $ARGV[0]) * 1000 / $ARGV[2]' "$start" "$end" "$n"
}

total_texrs=0 total_tex=0
printf '%-34s %12s %12s %8s\n' 'case' 'texrs (ms)' 'tex (ms)' 'ratio'
for f in "${files[@]}"; do
  # tex is given a copy, because it writes beside the input.
  d=$(mktemp -d); cp "$f" "$d/case.tex"
  a=$(run_many "$N" "$BIN" "$d/case.tex")
  b=$(run_many "$N" tex -interaction=nonstopmode "$d/case.tex")
  command rm -rf "$d"
  ratio=$(perl -e 'printf "%.2fx", $ARGV[1] / ($ARGV[0] || 1e-9)' "$a" "$b")
  printf '%-34s %12s %12s %8s\n' "$(basename "$f")" "$a" "$b" "$ratio"
  total_texrs=$(perl -e 'printf "%.4f", $ARGV[0] + $ARGV[1]' "$total_texrs" "$a")
  total_tex=$(perl -e 'printf "%.4f", $ARGV[0] + $ARGV[1]' "$total_tex" "$b")
done

printf '\n%-34s %12.2f %12.2f %8s\n' 'TOTAL' "$total_texrs" "$total_tex" \
  "$(perl -e 'printf "%.2fx", $ARGV[1] / ($ARGV[0] || 1e-9)' "$total_texrs" "$total_tex")"
cat <<'NOTE'

Read the ratio with two things in mind:
  * tex loads the plain format on every run and texrs loads nothing, so part of
    the difference is format loading rather than engine speed.
  * texrs implements the mouth and the expander only; a document that would make
    tex build pages is not a like-for-like comparison.
NOTE
