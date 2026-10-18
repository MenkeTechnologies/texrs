#!/usr/bin/env bash
# Record the characters table luaotfload's node mode builds for a few fonts,
# for tests/node_chars.rs to replay without TeX.
#
# Each font is loaded by luahbtex with `mode=node` at 10pt and every
# character's slot, width, height, depth, glyph index and /ToUnicode is
# written, the dimensions rounded the way the engine rounds a Lua float
# (floor(x + 0.5)), together with the font's parameters.
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
out="$repo/tests/data/node_chars.txt"
work=$(mktemp -d)
trap 'command rm -rf "$work"' EXIT

fonts=(lmroman10-regular.otf Arimo-Regular.ttf STIXTwoMath-Regular.otf)

cat >"$work/dump.lua" <<'LUA'
local function r(x) if x == nil then return "nil" end return string.format("%d", math.floor(x + 0.5)) end
function dumpchars(csname, file, path)
  local f = font.getfont(font.id(csname))
  local out = assert(io.open(path, "w"))
  out:write("##### ", file, "\n")
  local p = f.parameters
  for _, k in ipairs{"slant", "space", "space_stretch", "space_shrink", "x_height", "quad", "extra_space"} do
    out:write(string.format("param %s %s\n", k, r(p[k])))
  end
  out:write(string.format("param 8 %s\n", r(p[8])))
  local keys = {} for k in pairs(f.characters) do keys[#keys + 1] = k end table.sort(keys)
  for _, k in ipairs(keys) do
    local c = f.characters[k]
    out:write(string.format("%X %s %s %s %d %s\n", k, r(c.width), r(c.height), r(c.depth), c.index, tostring(c.tounicode)))
  end
  out:close()
end
LUA

{
  echo "# $(luahbtex --version | head -1)"
  echo "# slot width height depth index tounicode, at 10pt in mode=node"
} >"$work/expected.txt"
for f in "${fonts[@]}"; do
  cat >"$work/run.tex" <<TEX
\\input luaotfload.sty
\\directlua{dofile("dump.lua")}
\\font\\x="file:$f:mode=node" at 10pt
\\directlua{dumpchars("x", "$f", "font.dump")}
\\bye
TEX
  (cd "$work" && luahbtex -interaction=nonstopmode -halt-on-error run.tex </dev/null >"$work/log" 2>&1) || {
    echo "luahbtex failed on $f:" >&2
    tail -20 "$work/log" >&2
    exit 1
  }
  cat "$work/font.dump" >>"$work/expected.txt"
done
cp "$work/expected.txt" "$out"
echo "wrote $out"
