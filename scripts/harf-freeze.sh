#!/usr/bin/env bash
# Record what LuaHBTeX's harf mode makes of every list in tests/harf_cases,
# for tests/harf_parity.rs to replay without TeX.
#
# The dump is written next to the case, relative: luatex's default
# `openout_any = p` refuses to open an absolute path for writing.
#
# Each case is run by luahbtex with luaotfload loaded and scripts/harf-dump.lua
# watching hpack_filter; the before and after of every box it dumps are
# appended to tests/data/harf_expected.txt, under a header naming the engine,
# luaotfload and the HarfBuzz it links -- the three things a different answer
# could come from.
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
out="$repo/tests/data/harf_expected.txt"
work=$(mktemp -d)
trap 'command rm -rf "$work"' EXIT

engine=$(luahbtex --version | head -1)
{
  echo "# $engine"
} >"$work/expected.txt"

for case in "$repo"/tests/harf_cases/*.tex; do
  name=$(basename "$case")
  cp "$case" "$work/$name"
  cat >"$work/run.tex" <<EOF
\\input luaotfload.sty
\\directlua{dofile("$repo/scripts/harf-dump.lua")}
\\directlua{texio.write_nl("term and log", "HARFDUMP luaotfload " .. luaotfload.version .. " harfbuzz " .. require"luaharfbuzz".version())}
\\directlua{harfdump.case = "$name"}
\\input $name
\\directlua{harfdump.finish("$name.dump")}
\\bye
EOF
  (cd "$work" && luahbtex -interaction=nonstopmode -halt-on-error run.tex </dev/null >"$work/$name.log" 2>&1) || {
    echo "luahbtex failed on $name:" >&2
    tail -20 "$work/$name.log" >&2
    exit 1
  }
  if ! grep -q 'HARFDUMP' "$work/$name.log"; then
    echo "no HARFDUMP line from $name" >&2
    exit 1
  fi
  versions=$(grep -o 'HARFDUMP .*' "$work/$name.log" | head -1)
  if ! grep -q '^# luaotfload' "$work/expected.txt"; then
    echo "# ${versions#HARFDUMP }" >>"$work/expected.txt"
  fi
  echo "##### $name" >>"$work/expected.txt"
  cat "$work/$name.dump" >>"$work/expected.txt"
done

cp "$work/expected.txt" "$out"
echo "wrote $out"
