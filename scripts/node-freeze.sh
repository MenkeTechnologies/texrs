#!/usr/bin/env bash
# Record what luaotfload's node mode makes of every list in tests/node_cases,
# for tests/node_parity.rs to replay without TeX.
#
# The same harness as scripts/harf-freeze.sh: each case is run by luahbtex
# with luaotfload loaded and scripts/harf-dump.lua watching hpack_filter, and
# the before and after of every box it dumps are appended to
# tests/data/node_expected.txt under a header naming the engine and the
# luaotfload whose fontloader did the processing.
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
out="$repo/tests/data/node_expected.txt"
work=$(mktemp -d)
trap 'command rm -rf "$work"' EXIT

{
  echo "# $(luahbtex --version | head -1)"
} >"$work/expected.txt"

# the publication corpus's variable fonts, when that checkout is next to
# this one (tests/node_cases/variable.tex uses them)
corpus="$repo/../MenkeTechnologiesPublications/strykelang/docs/.fonts"
if [ -d "$corpus" ]; then cp "$corpus"/*.ttf "$work/"; fi

for case in "$repo"/tests/node_cases/*.tex; do
  name=$(basename "$case")
  cp "$case" "$work/$name"
  cat >"$work/run.tex" <<TEX
\\input luaotfload.sty
\\directlua{dofile("$repo/scripts/harf-dump.lua")}
\\directlua{texio.write_nl("term and log", "NODEDUMP luaotfload " .. luaotfload.version)}
\\input $name
\\directlua{harfdump.finish("$name.dump")}
\\bye
TEX
  (cd "$work" && luahbtex -interaction=nonstopmode -halt-on-error run.tex </dev/null >"$work/$name.log" 2>&1) || {
    echo "luahbtex failed on $name:" >&2
    tail -20 "$work/$name.log" >&2
    exit 1
  }
  versions=$(grep -o 'NODEDUMP .*' "$work/$name.log" | head -1)
  if ! grep -q '^# luaotfload' "$work/expected.txt"; then
    echo "# ${versions#NODEDUMP }" >>"$work/expected.txt"
  fi
  echo "##### $name" >>"$work/expected.txt"
  cat "$work/$name.dump" >>"$work/expected.txt"
done

cp "$work/expected.txt" "$out"
echo "wrote $out"
