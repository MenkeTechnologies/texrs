#!/usr/bin/env bash
# Record the math luaotfload gives an OpenType math font, for
# tests/node_math.rs to replay without TeX.
#
# Each font is loaded by luahbtex as unicode-math loads one -- `mode=base`,
# `script=math` -- at 10pt, and what the engine then holds is written: every
# MathConstants entry, and for every character that has any, its next larger
# variant, italic correction, top accent position, extensible recipe and
# math kerns, the dimensions rounded the way the engine rounds a Lua float.
# Latin Modern Math is recorded in full, the others as a SHA-256 of the same
# lines.
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
out="$repo/tests/data/node_math.txt"
work=$(mktemp -d)
trap 'command rm -rf "$work"' EXIT

full=(latinmodern-math.otf)
digested=(STIXTwoMath-Regular.otf texgyretermes-math.otf texgyrepagella-math.otf)

cat >"$work/dump.lua" <<'LUA'
local function r(x) if x == nil then return "nil" end return string.format("%d", math.floor(x + 0.5)) end
function dumpmath(csname, path)
  local f = font.getfont(font.id(csname))
  local out = assert(io.open(path, "w"))
  local mc = f.MathConstants or {}
  local keys = {} for k in pairs(mc) do keys[#keys + 1] = k end table.sort(keys)
  for _, k in ipairs(keys) do out:write(string.format("const %s %s\n", k, r(mc[k]))) end
  local cs = {} for k in pairs(f.characters) do cs[#cs + 1] = k end table.sort(cs)
  for _, k in ipairs(cs) do
    local c = f.characters[k]
    local parts = {}
    if c.next then parts[#parts + 1] = string.format("next %X", c.next) end
    if c.italic then parts[#parts + 1] = "italic " .. r(c.italic) end
    if c.top_accent then parts[#parts + 1] = "accent " .. r(c.top_accent) end
    for _, w in ipairs{"vert_variants", "horiz_variants"} do
      local v = c[w]
      if v then
        local s = {}
        for _, p in ipairs(v) do
          s[#s + 1] = string.format("%X:%s:%s:%s:%s", p.glyph, r(p.start), r(p["end"]), r(p.advance), tostring(p.extender))
        end
        parts[#parts + 1] = w .. " " .. table.concat(s, ",")
      end
    end
    if c.mathkern then
      for _, w in ipairs{"top_right", "top_left", "bottom_right", "bottom_left"} do
        local v = c.mathkern[w]
        if v then
          local s = {}
          for _, p in ipairs(v) do s[#s + 1] = r(p.height) .. "/" .. r(p.kern) end
          parts[#parts + 1] = w .. " " .. table.concat(s, ",")
        end
      end
    end
    if #parts > 0 then out:write(string.format("%X %s\n", k, table.concat(parts, " "))) end
  end
  out:close()
end
LUA

dump() {
  cat >"$work/run.tex" <<TEX
\\input luaotfload.sty
\\directlua{dofile("dump.lua")}
\\font\\x="file:$1:mode=base;script=math" at 10pt
\\directlua{dumpmath("x", "font.dump")}
\\bye
TEX
  (cd "$work" && luahbtex -interaction=nonstopmode -halt-on-error run.tex </dev/null >"$work/log" 2>&1) || {
    echo "luahbtex failed on $1:" >&2
    tail -20 "$work/log" >&2
    exit 1
  }
}

{
  echo "# $(luahbtex --version | head -1)"
  echo "# MathConstants and per-character math, at 10pt in mode=base;script=math"
} >"$work/expected.txt"
for f in "${full[@]}"; do
  dump "$f"
  echo "##### $f" >>"$work/expected.txt"
  cat "$work/font.dump" >>"$work/expected.txt"
done
for f in "${digested[@]}"; do
  dump "$f"
  echo "digest $f $(shasum -a 256 "$work/font.dump" | cut -d' ' -f1) $(wc -l <"$work/font.dump" | tr -d ' ')" >>"$work/expected.txt"
done
cp "$work/expected.txt" "$out"
echo "wrote $out"
