#!/usr/bin/env bash
# package-status.sh — does each package a real document uses load, and does
# the page say what lualatex's says?
#
#   bash scripts/package-status.sh               # print the table
#   bash scripts/package-status.sh --write       # and put it into BUGS.md
#   TEXRS_BIN=target/release/texrs bash ...      # which texrs (default: debug)
#   TEXRS_TIMEOUT=300 bash ...                   # per document (default 120s)
#
# For every package below, one document:
#
#   \documentclass{article}\usepackage{X}\begin{document}x\end{document}
#
# (for a class, `\documentclass{X}` alone), run through `texrs --pdf` and
# through `lualatex`, and both PDFs through `pdftotext`. "Completes" is texrs's
# exit status, and "loads" whether it read the package without reporting it
# as refused (a refused package is named on stderr and the run goes on
# without it); "text" is whether the two texts agree once runs of white space
# are one space; "what refused it" is that report. The
# table in BUGS.md is this script's output between its two markers, so it is
# a measurement and never typed by hand.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WRITE=0
if [ "${PKG_ONE:-0}" != "1" ]; then
  for a in "$@"; do
    case "$a" in
      --write) WRITE=1 ;;
      *) echo "package-status: unknown option $a" >&2; exit 2 ;;
    esac
  done
fi

# The classes first, then the packages, in the order the mission lists them.
ITEMS=(
  class:article class:report class:book
  amsmath amssymb amsthm graphicx xcolor hyperref geometry fontspec
  unicode-math babel polyglossia enumitem booktabs longtable array tabularx
  caption fancyhdr microtype csquotes biblatex listings tikz expl3
)

# One item, in a directory of its own: prints `item<TAB>completes<TAB>text<TAB>stopped by`.
if [ "${PKG_ONE:-0}" = "1" ]; then
  item="$1"
  dir="$PKG_DIR/${item/:/-}"
  mkdir -p "$dir"
  case "$item" in
    class:*) head="\\documentclass{${item#class:}}" ;;
    *) head="\\documentclass{article}\\usepackage{$item}" ;;
  esac
  printf '%s\n\\begin{document}\nx\n\\end{document}\n' "$head" > "$dir/doc.tex"
  ( cd "$dir" && timeout "${TEXRS_TIMEOUT:-120}" "$TEXRS_BIN" --pdf --no-cache \
      -interaction=nonstopmode doc.tex >/dev/null 2>stderr.txt )
  rc=$?
  # A package texrs could not read is reported by name and the run goes on
  # without it, so the exit status alone does not say it loaded.
  refusal=$(grep -m1 -E '^texrs: package .* (is not loadable|needs )' "$dir/stderr.txt" |
    tr -d '\t' | cut -c1-120)
  loads="yes"
  [ -n "$refusal" ] && loads="no"
  text="-"
  if [ "$rc" = 0 ] && [ -f "$dir/doc.pdf" ] && command -v lualatex >/dev/null; then
    ( cd "$dir" && timeout 300 lualatex -interaction=nonstopmode -jobname=ref doc.tex \
        >/dev/null 2>&1 )
    if [ -f "$dir/ref.pdf" ]; then
      ours=$(pdftotext "$dir/doc.pdf" - 2>/dev/null | tr -s ' \n\f\t' ' ')
      theirs=$(pdftotext "$dir/ref.pdf" - 2>/dev/null | tr -s ' \n\f\t' ' ')
      if [ "$ours" = "$theirs" ]; then text="same"; else text="differs"; fi
    fi
  fi
  case "$rc" in
    0) done="yes" ;;
    124) done="timeout" ;;
    *) done="no" ;;
  esac
  printf '%s\t%s\t%s\t%s\t%s\n' "$item" "$loads" "$done" "$text" "$refusal"
  exit 0
fi

BIN="${TEXRS_BIN:-$ROOT/target/debug/texrs}"
[ -x "$BIN" ] || { echo "package-status: no texrs binary at $BIN" >&2; exit 1; }
BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"
DIR="$(mktemp -d "${TMPDIR:-/tmp}/texrs-packages.XXXXXX")"
trap 'rm -rf "$DIR"' EXIT

rows=$(printf '%s\n' "${ITEMS[@]}" |
  PKG_ONE=1 PKG_DIR="$DIR" TEXRS_BIN="$BIN" \
    xargs -P "${TEXRS_JOBS:-4}" -I{} bash "$0" {} |
  sort -t$'\t' -k1,1)

# Back into the order of ITEMS, which is the order a reader looks for them in.
table="| item | loads | run completes | text vs lualatex | what refused it |
|---|---|---|---|---|"
loaded=0
completed=0
same=0
for item in "${ITEMS[@]}"; do
  row=$(printf '%s\n' "$rows" | awk -F'\t' -v k="$item" '$1==k')
  IFS=$'\t' read -r name loads done text said <<<"$row"
  [ "$loads" = "yes" ] && loaded=$((loaded + 1))
  [ "$done" = "yes" ] && completed=$((completed + 1))
  [ "$text" = "same" ] && same=$((same + 1))
  said=${said//|/\\|}
  table="$table
| \`${name#class:}\`$( [ "${name%%:*}" = class ] && printf ' (class)') | $loads | $done | $text | ${said:-} |"
done
summary="$loaded of ${#ITEMS[@]} load; $completed of ${#ITEMS[@]} runs complete; $same of ${#ITEMS[@]} set the same text as lualatex."
out="$summary

$table"
printf '%s\n' "$out"

if [ "$WRITE" = 1 ]; then
  file="$ROOT/BUGS.md"
  begin='<!-- package-status:begin -->'
  end='<!-- package-status:end -->'
  grep -q "$begin" "$file" || { echo "package-status: no $begin marker in BUGS.md" >&2; exit 1; }
  BLOCK="$out" perl -0 -i -pe 's/(\Q'"$begin"'\E\n).*?(\Q'"$end"'\E)/$1$ENV{BLOCK}\n$2/s' "$file"
  echo "package-status: wrote BUGS.md"
fi
