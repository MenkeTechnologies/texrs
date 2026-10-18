-- What luaotfload's harf plugin makes of a list, for tests/harf_parity.rs.
--
-- Loaded by scripts/harf-freeze.sh into luahbtex. A case calls
-- `harfdump.box(label)` after `\setbox0\hbox{...}`: the list the box was
-- given BEFORE any font handler ran is caught by an hpack_filter ordered
-- ahead of luaotfload's node processor (`declare_callback_rule`), and the
-- list it ended up with is read out of the box. Both are written in one plain line-per-node format, so the
-- replay needs no Lua and no TeX.
--
--   glyph CHAR FONT [XOFF YOFF W H D]   (hex char; metrics only after)
--   glue SUBTYPE WIDTH STRETCH SHRINK
--   kern SUBTYPE KERN
--   disc SUBTYPE {   ...pre...   |   ...post...   |   ...replace...   }
--   dir DIRECTION CANCEL
--   localpar DIRECTION
--   literal DATA
--   other ID

harfdump = harfdump or {}

local direct = node.direct
local todirect = direct.todirect
local getid, getsubtype, getnext = direct.getid, direct.getsubtype, direct.getnext
local getchar, getfont, getdisc = direct.getchar, direct.getfont, direct.getdisc
local getoffsets, getwidth, getheight, getdepth =
  direct.getoffsets, direct.getwidth, direct.getheight, direct.getdepth
local getkern, getglue, getdirection = direct.getkern, direct.getglue, direct.getdirection

local glyph_t, glue_t, kern_t, disc_t, dir_t, localpar_t, whatsit_t =
  node.id"glyph", node.id"glue", node.id"kern", node.id"disc",
  node.id"dir", node.id"local_par", node.id"whatsit"
local pdfliteral_t = node.subtype"pdf_literal"

local out = {}
local function emit(s) out[#out + 1] = s end

local function write_list(n, metrics, depth)
  local pad = string.rep("  ", depth)
  while n do
    local id = getid(n)
    if id == glyph_t then
      if metrics then
        local x, y = getoffsets(n)
        emit(string.format("%sglyph %X %d %d %d %d %d %d", pad, getchar(n), getfont(n),
          x, y, getwidth(n), getheight(n), getdepth(n)))
      else
        emit(string.format("%sglyph %X %d", pad, getchar(n), getfont(n)))
      end
    elseif id == glue_t then
      local w, st, sh = getglue(n)
      emit(string.format("%sglue %d %d %d %d", pad, getsubtype(n), w, st, sh))
    elseif id == kern_t then
      emit(string.format("%skern %d %d", pad, getsubtype(n), getkern(n)))
    elseif id == disc_t then
      local pre, post, rep = getdisc(n)
      emit(string.format("%sdisc %d {", pad, getsubtype(n)))
      write_list(pre, metrics, depth + 1)
      emit(pad .. "|")
      write_list(post, metrics, depth + 1)
      emit(pad .. "|")
      write_list(rep, metrics, depth + 1)
      emit(pad .. "}")
    elseif id == dir_t then
      local d, cancel = getdirection(n)
      emit(string.format("%sdir %d %d", pad, d, cancel and 1 or 0))
    elseif id == localpar_t then
      emit(string.format("%slocalpar %d", pad, (getdirection(n))))
    elseif id == whatsit_t and getsubtype(n) == pdfliteral_t then
      emit(string.format("%sliteral %s", pad, direct.getdata(n)))
    else
      emit(string.format("%sother %d", pad, id))
    end
    n = getnext(n)
  end
end

local fonts_seen = {}
local function note_fonts(n)
  while n do
    local id = getid(n)
    if id == glyph_t then
      fonts_seen[getfont(n)] = true
    elseif id == disc_t then
      local pre, post, rep = getdisc(n)
      note_fonts(pre) note_fonts(post) note_fonts(rep)
    end
    n = getnext(n)
  end
end

local before
luatexbase.add_to_callback("hpack_filter", function(head)
  before = {}
  local saved = out
  out = before
  write_list(todirect(head), false, 0)
  out = saved
  note_fonts(todirect(head))
  return true
end, "harfdump.before")
luatexbase.declare_callback_rule("hpack_filter", "harfdump.before", "before",
  "luaotfload.node_processor")
-- A rule reorders the whole chain, so the order luaotfload's two filters
-- have without one (shaping, then the /ActualText pass) is stated too.
luatexbase.declare_callback_rule("hpack_filter", "luaotfload.node_processor",
  "before", "luaotfload.harf.finalize_hlist")

function harfdump.box(label)
  local box = tex.box[0]
  emit("=== " .. label)
  local ids = {}
  for id in pairs(fonts_seen) do ids[#ids + 1] = id end
  table.sort(ids)
  for _, id in ipairs(ids) do
    local f = font.getfont(id)
    if f then
      emit(string.format("font %d %d %s", id, f.size, f.name))
    end
  end
  fonts_seen = {}
  emit("in")
  for _, l in ipairs(before) do emit(l) end
  emit("out")
  write_list(todirect(box.head), true, 0)
end

function harfdump.finish(path)
  local f = assert(io.open(path, "a"))
  f:write(table.concat(out, "\n"), "\n")
  f:close()
  out = {}
end
