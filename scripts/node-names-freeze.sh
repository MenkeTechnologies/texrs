#!/usr/bin/env bash
# Record which file luaotfload resolves each font request in
# tests/data/node_names.txt to, for tests/node_names.rs to replay without
# TeX: `file:`, `path`, anonymous and `name:` requests, the last two through
# luaotfload's font names database as it stands on this machine.
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
out="$repo/tests/data/node_names.txt"
work=$(mktemp -d)
trap 'command rm -rf "$work"' EXIT

requests=(
  "file:lmroman10-regular.otf"
  "lmroman10-regular.otf"
  "name:STIX Two Math"
  "STIX Two Math:mode=base;script=math"
  "name:STIX Two Text/B"
  "name:Latin Modern Roman"
  "name:Latin Modern Roman/I"
  "name:Latin Modern Roman/B:optsize=5"
  "name:TeX Gyre Termes/BI"
  "name:Arial"
  "Arial Unicode MS:mode=base;"
  "name:DejaVu Sans Mono/B"
  "name:EB Garamond"
  "name:Noto Serif/I"
)

cat >"$work/dump.lua" <<'LUA'
function dumpfile(cs, request, path)
  local f = font.getfont(font.id(cs))
  local out = assert(io.open(path, "a"))
  out:write(request, "\t", tostring(f.filename), "\t", tostring(f.subfont or ""), "\n")
  out:close()
end
LUA
{
  echo "# $(luahbtex --version | head -1)"
  echo "# request<TAB>file luaotfload resolved it to<TAB>subfont, at 10pt"
} >"$work/expected.txt"
for r in "${requests[@]}"; do
  cat >"$work/run.tex" <<TEX
\\input luaotfload.sty
\\directlua{dofile("dump.lua")}
\\font\\x="$r" at 10pt
\\directlua{dumpfile("x", [==[$r]==], "names.dump")}
\\bye
TEX
  (cd "$work" && luahbtex -interaction=nonstopmode -halt-on-error run.tex </dev/null >"$work/log" 2>&1) || {
    echo "luahbtex failed on $r:" >&2
    tail -20 "$work/log" >&2
    exit 1
  }
done
cat "$work/names.dump" >>"$work/expected.txt"
cp "$work/expected.txt" "$out"
echo "wrote $out"
