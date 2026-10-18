#!/usr/bin/env bash
# Record the characters table luaotfload's node mode builds for variable
# fonts at an instance, for tests/node_var.rs to replay without TeX.
#
# Each font is loaded by luahbtex in `mode=node` at 10pt with `axis=` values
# (as fontspec's RawFeature={axis={...}} asks), and every slot's width,
# height, depth, glyph index and /ToUnicode is written with the font's
# parameters, the way scripts/node-chars-freeze.sh writes them, as a SHA-256
# of the lines. PennstanderVF is TeX Live's; Arimo-VF is the corpus's, found
# next to this repository when the publications checkout is there.
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
out="$repo/tests/data/node_var.txt"
work=$(mktemp -d)
trap 'command rm -rf "$work"' EXIT
corpus="$repo/../MenkeTechnologiesPublications/strykelang/docs/.fonts"

cases=(
  "PennstanderVF.ttf|"
  "PennstanderVF.ttf|wght=700"
  "PennstanderVF.ttf|wght=250"
  "Arimo-VF.ttf|wght=400"
  "Arimo-VF.ttf|wght=700"
  "Arimo-Italic-VF.ttf|wght=700"
  "Orbitron-VF.ttf|wght=900"
)

cat >"$work/dump.lua" <<'LUA'
local function r(x) if x == nil then return "nil" end return string.format("%d", math.floor(x + 0.5)) end
function dumpchars(csname, path)
  local f = font.getfont(font.id(csname))
  local out = assert(io.open(path, "w"))
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
  echo "# file|axis sha256 lines, at 10pt in mode=node"
} >"$work/expected.txt"
for c in "${cases[@]}"; do
  file=${c%%|*}
  axis=${c#*|}
  if [ -f "$corpus/$file" ]; then cp "$corpus/$file" "$work/"; fi
  if [ -n "$axis" ]; then req="file:$file:mode=node;axis={$axis}"; else req="file:$file:mode=node"; fi
  cat >"$work/run.tex" <<TEX
\\input luaotfload.sty
\\directlua{dofile("dump.lua")}
\\font\\x="$req" at 10pt
\\directlua{dumpchars("x", "font.dump")}
\\bye
TEX
  (cd "$work" && luahbtex -interaction=nonstopmode -halt-on-error run.tex </dev/null >"$work/log" 2>&1) || {
    echo "luahbtex failed on $c:" >&2
    tail -20 "$work/log" >&2
    exit 1
  }
  echo "digest $file|$axis $(shasum -a 256 "$work/font.dump" | cut -d' ' -f1) $(wc -l <"$work/font.dump" | tr -d ' ')" >>"$work/expected.txt"
  cp "$work/font.dump" "$work/${file%.ttf}-${axis:-default}.dump"
done
cp "$work/expected.txt" "$out"
mkdir -p "${NODE_VAR_KEEP:-$work}"
cp "$work"/*.dump "${NODE_VAR_KEEP:-$work}/" 2>/dev/null || true
echo "wrote $out"
