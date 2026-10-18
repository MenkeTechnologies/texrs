//! The `font` library: fonts as Lua tables (luatexdir/lua/lfontlib.c and
//! the `font_from_lua`/`font_to_lua` halves of font/luafont.c).
//!
//! A font LuaTeX defines from Lua is a table: `name`, `size`, `designsize`,
//! `parameters`, and `characters`, each character a table of `width`,
//! `height`, `depth`, `italic`, `kerns`, `ligatures` and the rest. luaotfload
//! builds exactly that for an OpenType font and hands it to `font.define` (or
//! returns it from its `define_font` callback); `font.read_tfm` builds the
//! same for a TFM. This module keeps those tables by font id, answers
//! `font.getfont`/`font.fonts[id]` with the table that was defined -- luatex
//! answers the stored table itself, not a copy, measured -- and reads each one
//! into a [`LuaFont`], the shape the engine can set type from.
//!
//! ## Where the ids come from
//!
//! In LuaTeX one table numbers every font, `\font`'s and `font.define`'s
//! alike. Here `\font` numbers its fonts in `crate::stomach::font::Table`,
//! which has no entry point yet for a font that did not come from a TFM, so
//! the fonts defined from Lua are numbered in this module's own table,
//! starting after the ones `\font` had defined when the first was made. Where
//! the engine reads a Lua-defined font back -- the `define_font` callback's
//! answer, a glyph node's `font` -- is the stomach's to wire (BUGS.md).

use super::*;
use std::collections::BTreeMap;
use std::sync::Arc;

/// One character of a Lua font, in scaled points at the font's size.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LuaChar {
    pub width: i64,
    pub height: i64,
    pub depth: i64,
    pub italic: i64,
    /// The glyph index in the font file, when the table names one.
    pub index: Option<i64>,
    /// `tounicode`: the hex string of what the glyph stands for.
    pub tounicode: Option<String>,
    /// `kerns[next] = amount`.
    pub kerns: BTreeMap<i64, i64>,
    /// `ligatures[next] = { char = c, type = t }`.
    pub ligatures: BTreeMap<i64, (i64, i64)>,
    /// `next`: the next larger character of a math charlist.
    pub next: Option<i64>,
}

/// A font as `font.define` was given it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LuaFont {
    pub name: String,
    pub fullname: Option<String>,
    pub psname: Option<String>,
    pub filename: Option<String>,
    pub size: i64,
    pub designsize: i64,
    /// `real` or `virtual`.
    pub kind: String,
    /// `type1`, `opentype`, `truetype` or `unknown`.
    pub format: String,
    pub encodingbytes: i64,
    /// `\fontdimen1..n`: `slant`, `space`, `space_stretch`, `space_shrink`,
    /// `x_height`, `quad`, `extra_space`, then the numbered rest.
    pub parameters: Vec<i64>,
    pub characters: BTreeMap<i64, LuaChar>,
}

/// font/luafont.c's `font_param_names`: the parameter keys, in `\fontdimen`
/// order.
const PARAM_NAMES: [&str; 7] = [
    "slant",
    "space",
    "space_stretch",
    "space_shrink",
    "x_height",
    "quad",
    "extra_space",
];

fn int_field(t: &Table, k: &str) -> mlua::Result<Option<i64>> {
    Ok(match t.raw_get::<Value>(k)? {
        Value::Integer(n) => Some(n),
        Value::Number(f) => Some(f.round() as i64),
        _ => None,
    })
}

fn str_field(t: &Table, k: &str) -> mlua::Result<Option<String>> {
    Ok(match t.raw_get::<Value>(k)? {
        Value::String(s) => Some(s.to_str()?.to_string()),
        _ => None,
    })
}

/// `font_from_lua`: the fields the engine reads, from a font table.
fn from_lua(t: &Table) -> mlua::Result<LuaFont> {
    let mut f = LuaFont {
        name: str_field(t, "name")?.unwrap_or_default(),
        fullname: str_field(t, "fullname")?,
        psname: str_field(t, "psname")?,
        filename: str_field(t, "filename")?,
        size: int_field(t, "size")?.unwrap_or(655_360),
        designsize: int_field(t, "designsize")?.unwrap_or(655_360),
        kind: str_field(t, "type")?.unwrap_or_else(|| "real".to_string()),
        format: str_field(t, "format")?.unwrap_or_else(|| "unknown".to_string()),
        encodingbytes: int_field(t, "encodingbytes")?.unwrap_or(0),
        ..LuaFont::default()
    };
    if let Value::Table(p) = t.raw_get::<Value>("parameters")? {
        // Named keys first, then numeric ones from 8 on (luafont.c reads
        // `parameters[i]` for i past the seven named ones).
        let mut params = Vec::new();
        for (i, name) in PARAM_NAMES.iter().enumerate() {
            let named = int_field(&p, name)?;
            let numbered = match p.raw_get::<Value>(i as i64 + 1)? {
                Value::Integer(n) => Some(n),
                Value::Number(x) => Some(x.round() as i64),
                _ => None,
            };
            params.push(named.or(numbered).unwrap_or(0));
        }
        let mut i = 8;
        while let Some(v) = p.raw_get::<Option<i64>>(i).ok().flatten() {
            params.push(v);
            i += 1;
        }
        f.parameters = params;
    }
    if let Value::Table(chars) = t.raw_get::<Value>("characters")? {
        for pair in chars.pairs::<Value, Value>() {
            let (k, v) = pair?;
            let (Some(code), Value::Table(c)) = (k.as_integer(), v) else {
                continue;
            };
            let mut ch = LuaChar {
                width: int_field(&c, "width")?.unwrap_or(0),
                height: int_field(&c, "height")?.unwrap_or(0),
                depth: int_field(&c, "depth")?.unwrap_or(0),
                italic: int_field(&c, "italic")?.unwrap_or(0),
                index: int_field(&c, "index")?,
                tounicode: str_field(&c, "tounicode")?,
                next: int_field(&c, "next")?,
                ..LuaChar::default()
            };
            if let Value::Table(k) = c.raw_get::<Value>("kerns")? {
                for pair in k.pairs::<i64, Value>() {
                    let (next, amount) = pair?;
                    if let Some(a) = amount
                        .as_integer()
                        .or_else(|| amount.as_number().map(|x| x.round() as i64))
                    {
                        ch.kerns.insert(next, a);
                    }
                }
            }
            if let Value::Table(l) = c.raw_get::<Value>("ligatures")? {
                for pair in l.pairs::<i64, Table>() {
                    let (next, lig) = pair?;
                    let target = int_field(&lig, "char")?.unwrap_or(0);
                    let kind = int_field(&lig, "type")?.unwrap_or(0);
                    ch.ligatures.insert(next, (target, kind));
                }
            }
            f.characters.insert(code, ch);
        }
    }
    Ok(f)
}

/// One font of this module's table.
struct Defined {
    table: mlua::RegistryKey,
    font: Arc<LuaFont>,
}

thread_local! {
    /// The fonts defined from Lua, by id; slot 0 is `\nullfont`'s and empty.
    static FONTS: RefCell<Vec<Option<Defined>>> = RefCell::new(vec![None]);
    /// `font.current(id)` when a chunk set it.
    static CURRENT: Cell<i64> = const { Cell::new(0) };
}

/// Forget every font, for a fresh state.
pub(super) fn reset() {
    FONTS.with(|f| *f.borrow_mut() = vec![None]);
    crate::otf::clear_lua_fonts();
    CURRENT.with(|c| c.set(0));
}

/// The Lua-defined font `id`, for the engine to set type from.
pub fn get(id: usize) -> Option<Arc<LuaFont>> {
    FONTS.with(|f| {
        f.borrow()
            .get(id)
            .and_then(|d| d.as_ref())
            .map(|d| Arc::clone(&d.font))
    })
}

/// The first id no font has: past every Lua font, and past the fonts `\font`
/// had defined when this is asked, so the two tables do not hand out the
/// same number.
fn next_free() -> usize {
    let tex_fonts = ctx::with("font", |eng, _| {
        let mut n = 1u16;
        while eng.fonts.get_mut(n).is_some() {
            n += 1;
        }
        Ok(usize::from(n))
    })
    .unwrap_or(1);
    FONTS.with(|f| f.borrow().len()).max(tex_fonts)
}

/// `deffont`'s second half: store `t` as font `id`.
fn store(lua: &Lua, id: usize, t: Table) -> mlua::Result<()> {
    let font = Arc::new(from_lua(&t)?);
    let table = lua.create_registry_value(t)?;
    FONTS.with(|f| {
        let mut f = f.borrow_mut();
        if f.len() <= id {
            f.resize_with(id + 1, || None);
        }
        f[id] = Some(Defined { table, font });
    });
    Ok(())
}

/// `define_font`'s answer made into a font of the stomach's, kept here as
/// font `id` too, so `font.getfont(id)` answers the table the callback gave:
/// luatex `font.define`s a callback's table under the id `\font` assigns.
/// The hyphen and skew characters are the table's, else `\defaulthyphenchar`
/// and `\defaultskewchar` (luafont.c `font_from_lua`).
pub(super) fn define_for_tex(
    lua: &Lua,
    id: usize,
    t: Table,
) -> mlua::Result<crate::stomach::font::Font> {
    use crate::stomach::font::{Font, Metrics};
    let hyphen = int_field(&t, "hyphenchar")?;
    let skew = int_field(&t, "skewchar")?;
    let subfont = int_field(&t, "subfont")?.unwrap_or(0);
    store(lua, id, t)?;
    let f = get(id).ok_or_else(|| mlua::Error::runtime("font creation failed, error in table"))?;
    let chars = f.characters.iter().filter_map(|(code, c)| {
        let code = u32::try_from(*code).ok()?;
        Some((
            code,
            Metrics {
                width: c.width,
                height: c.height,
                depth: c.depth,
                italic: c.italic,
            },
        ))
    });
    let mut font = Font::from_metrics(
        &f.name,
        "",
        f.size,
        f.designsize,
        f.parameters.clone(),
        chars,
    );
    let (default_hyphen, default_skew) = ctx::with("font.define", |eng, _| {
        Ok((
            eng.intpar_named("defaulthyphenchar"),
            eng.intpar_named("defaultskewchar"),
        ))
    })?;
    font.hyphen_char = hyphen.unwrap_or(default_hyphen);
    font.skew_char = skew.unwrap_or(default_skew);
    // An OpenType or TrueType font is drawn from the file the table names, by
    // each character's glyph `index` (`crate::otf::register_lua_font`, which
    // the writer and the subsetters read), and is processed by the definer's
    // own callbacks rather than texrs's node mode.
    if let (Some(file), "opentype" | "truetype") = (&f.filename, f.format.as_str()) {
        let glyphs: Vec<(u32, u32, Option<String>)> = f
            .characters
            .iter()
            .filter_map(|(code, c)| {
                Some((
                    u32::try_from(*code).ok()?,
                    u32::try_from(c.index?).ok()?,
                    c.tounicode.clone(),
                ))
            })
            .collect();
        if let Ok(id) = u16::try_from(id) {
            crate::otf::register_lua_font(id, file, subfont, f.psname.as_deref(), &glyphs);
        }
    }
    Ok(font)
}

fn valid(id: i64) -> bool {
    id > 0 && FONTS.with(|f| f.borrow().get(id as usize).is_some_and(|d| d.is_some()))
}

/// The table font `id` was defined with.
fn table_of(lua: &Lua, id: i64) -> mlua::Result<Value> {
    if !valid(id) {
        return Ok(Value::Nil);
    }
    FONTS.with(|f| match &f.borrow()[id as usize] {
        Some(d) => lua.registry_value::<Value>(&d.table),
        None => Ok(Value::Nil),
    })
}

/// `font.read_tfm(name, size)`: the table `font_to_lua` builds for a TFM,
/// key for key as luatex 1.24.0 answers `font.read_tfm("cmr10", 655360)`.
fn read_tfm(lua: &Lua, name: &str, size: i64) -> mlua::Result<Table> {
    let path = crate::typeset::find_font(name)
        .ok_or_else(|| mlua::Error::runtime("font loading failed"))?;
    let tfm =
        crate::tfm::Tfm::open(&path).map_err(|_| mlua::Error::runtime("font loading failed"))?;
    let designsize = ((tfm.design_size * f64::from(1 << 20)).round() as i64) / 16;
    // A negative size is `scaled`: `-1000` is the design size (§568).
    let size = match size < 0 {
        true => crate::dimen::xn_over_d(designsize, -size, 1000),
        false => size,
    };
    let s = |v: f64| crate::tfm::store_scaled(v, size);
    let t = lua.create_table()?;
    t.set("name", name)?;
    t.set("size", size)?;
    t.set("designsize", designsize)?;
    t.set("checksum", i64::from(tfm.checksum))?;
    for (k, v) in [
        ("direction", 0i64),
        ("encodingbytes", 0),
        ("extend", 1000),
        ("mode", 0),
        ("slant", 0),
        ("squeeze", 1000),
        ("streamprovider", 0),
        ("subfont", 0),
        ("tounicode", 0),
        ("units_per_em", 0),
        ("width", 0),
    ] {
        t.set(k, v)?;
    }
    for (k, v) in [
        ("embedding", "unknown"),
        ("format", "unknown"),
        ("identity", "unknown"),
        ("type", "unknown"),
        ("writingmode", "unknown"),
    ] {
        t.set(k, v)?;
    }
    t.set("oldmath", false)?;
    t.set("used", false)?;
    t.set("MathConstants", lua.create_table()?)?;
    let params = lua.create_table()?;
    for (i, name) in PARAM_NAMES.iter().enumerate() {
        // `slant` is not a dimension: §558 does not scale \fontdimen1.
        let raw = tfm.raw_params.get(i).copied().unwrap_or(0.0);
        let v = match i {
            0 => (raw * 65536.0).round() as i64,
            _ => s(raw),
        };
        params.set(*name, v)?;
    }
    for (i, raw) in tfm.raw_params.iter().enumerate().skip(7) {
        params.set(i as i64 + 1, s(*raw))?;
    }
    t.set("parameters", params)?;
    let chars = lua.create_table()?;
    for code in tfm.codes() {
        let Some(m) = tfm.char(code) else { continue };
        let c = lua.create_table()?;
        c.set("width", s(m.width))?;
        c.set("height", s(m.height))?;
        c.set("depth", s(m.depth))?;
        if m.italic != 0.0 {
            c.set("italic", s(m.italic))?;
        }
        if let crate::tfm::Tag::List = tfm.tag(code) {
            c.set("next", i64::from(m.remainder))?;
        }
        // The lig/kern program, walked as §545 walks it.
        if let Some(mut k) = tfm.lig_kern_start(u16::from(code)) {
            let (kerns, ligs) = (lua.create_table()?, lua.create_table()?);
            let (mut any_kern, mut any_lig) = (false, false);
            let mut seen = std::collections::HashSet::new();
            while let Some([skip, next, op, rem]) = tfm.lig_kern_word(k) {
                if skip <= 128 && seen.insert(next) {
                    if op >= 128 {
                        kerns.set(i64::from(next), s(tfm.kern_value(op, rem)))?;
                        any_kern = true;
                    } else {
                        let l = lua.create_table()?;
                        l.set("char", i64::from(rem))?;
                        l.set("type", i64::from(op))?;
                        ligs.set(i64::from(next), l)?;
                        any_lig = true;
                    }
                }
                if skip >= 128 {
                    break;
                }
                k += skip as usize + 1;
            }
            if any_kern {
                c.set("kerns", kerns)?;
            }
            if any_lig {
                c.set("ligatures", ligs)?;
            }
        }
        chars.set(i64::from(code), c)?;
    }
    t.set("characters", chars)?;
    Ok(t)
}

/// The `font` table.
pub(super) fn table(lua: &Lua) -> mlua::Result<Table> {
    reset();
    let t = lua.create_table()?;
    t.set(
        "read_tfm",
        lua.create_function(|lua, (name, size): (Value, Value)| {
            let Value::String(name) = name else {
                return Err(mlua::Error::runtime("expected tfm name as first argument"));
            };
            let Some(size) = size
                .as_integer()
                .or_else(|| size.as_number().map(|f| f.round() as i64))
            else {
                return Err(mlua::Error::runtime(
                    "expected an integer size as second argument",
                ));
            };
            let name = name.to_str()?.to_string();
            if name.is_empty() {
                return Err(mlua::Error::runtime("expected tfm name as first argument"));
            }
            read_tfm(lua, &name, size)
        })?,
    )?;
    // "font.define([id,] table)".
    t.set(
        "define",
        lua.create_function(|lua, args: Variadic<Value>| {
            let (id, tbl) = match &args[..] {
                [Value::Table(t)] => (next_free(), t.clone()),
                [id, Value::Table(t)] => {
                    let id = id.as_integer().unwrap_or(0);
                    if !valid(id) {
                        return Err(mlua::Error::runtime(
                            "font creation failed, invalid id passed",
                        ));
                    }
                    (id as usize, t.clone())
                }
                _ => {
                    return Err(mlua::Error::runtime(
                        "font creation failed, no table passed",
                    ))
                }
            };
            store(lua, id, tbl)
                .map_err(|_| mlua::Error::runtime("font creation failed, error in table"))?;
            Ok(id as i64)
        })?,
    )?;
    // "font.nextid([keep])": the id the next `define` would take; with `true`
    // it is reserved, as luatex reserves it.
    t.set(
        "nextid",
        lua.create_function(|lua, keep: Option<bool>| {
            let id = next_free();
            if keep.unwrap_or(false) {
                store(lua, id, lua.create_table()?)?;
            }
            Ok(id as i64)
        })?,
    )?;
    t.set(
        "max",
        lua.create_function(|_, ()| {
            Ok(FONTS.with(|f| f.borrow().iter().rposition(|d| d.is_some()).unwrap_or(0)) as i64)
        })?,
    )?;
    t.set(
        "current",
        lua.create_function(|_, id: Option<i64>| match id {
            Some(i) if i > 0 => {
                if !valid(i) {
                    return Err(mlua::Error::runtime("expected a valid font id"));
                }
                CURRENT.with(|c| c.set(i));
                Ok(None)
            }
            _ => {
                let lua_font = CURRENT.with(Cell::get);
                let tex_font =
                    ctx::with("font.current", |eng, _| Ok(i64::from(eng.cur_font))).unwrap_or(0);
                Ok(Some(if lua_font > 0 { lua_font } else { tex_font }))
            }
        })?,
    )?;
    // getfont answers the stored table; getcopy is `font_to_lua(L, i, 0)`,
    // which luatex builds afresh -- a shallow copy here.
    t.set(
        "getfont",
        lua.create_function(|lua, args: Variadic<Value>| {
            let id = args.last().and_then(|v| v.as_integer()).unwrap_or(0);
            table_of(lua, id)
        })?,
    )?;
    t.set(
        "getcopy",
        lua.create_function(|lua, id: i64| {
            let Value::Table(orig) = table_of(lua, id)? else {
                return Ok(Value::Nil);
            };
            let copy = lua.create_table()?;
            for pair in orig.pairs::<Value, Value>() {
                let (k, v) = pair?;
                copy.raw_set(k, v)?;
            }
            Ok(Value::Table(copy))
        })?,
    )?;
    t.set(
        "getparameters",
        lua.create_function(|lua, id: i64| {
            let Some(f) = (valid(id)).then(|| get(id as usize)).flatten() else {
                return Ok(Value::Nil);
            };
            let p = lua.create_table()?;
            for (i, v) in f.parameters.iter().enumerate() {
                match PARAM_NAMES.get(i) {
                    Some(name) => p.set(*name, *v)?,
                    None => p.set(i as i64 + 1, *v)?,
                }
            }
            Ok(Value::Table(p))
        })?,
    )?;
    // "setfont(id, table)": allowed until the font has been used, which a font
    // defined from Lua here never is -- nothing sets type from it yet.
    t.set(
        "setfont",
        lua.create_function(|lua, args: Variadic<Value>| {
            let id = args.first().and_then(|v| v.as_integer()).unwrap_or(0);
            if id == 0 {
                return Ok(());
            }
            let Some(Value::Table(tbl)) = args.last() else {
                return Err(mlua::Error::runtime(
                    "bad argument to 'setfont' (table expected)",
                ));
            };
            if !valid(id) {
                return Err(mlua::Error::runtime("that integer id is not a valid font"));
            }
            store(lua, id as usize, tbl.clone())
        })?,
    )?;
    // "addcharacters(id, { characters = ... })": the new characters merged
    // into the stored table, then the font read again.
    t.set(
        "addcharacters",
        lua.create_function(|lua, (id, more): (i64, Table)| {
            let Value::Table(orig) = table_of(lua, id)? else {
                return Err(mlua::Error::runtime("that integer id is not a valid font"));
            };
            if let Value::Table(add) = more.raw_get::<Value>("characters")? {
                let chars = match orig.raw_get::<Value>("characters")? {
                    Value::Table(c) => c,
                    _ => {
                        let c = lua.create_table()?;
                        orig.raw_set("characters", c.clone())?;
                        c
                    }
                };
                for pair in add.pairs::<Value, Value>() {
                    let (k, v) = pair?;
                    chars.raw_set(k, v)?;
                }
            }
            store(lua, id as usize, orig)
        })?,
    )?;
    t.set(
        "frozen",
        lua.create_function(|_, id: i64| Ok((valid(id)).then_some(false)))?,
    )?;
    // "font.id(csname)": the number a `\font` identifier selects, -1 and a
    // message when the name is not one.
    t.set(
        "id",
        lua.create_function(|lua, name: Value| {
            let Value::String(name) = name else {
                return Err(mlua::Error::runtime(
                    "expected font csname string as argument",
                ));
            };
            let name = name.to_str()?.to_string();
            let f = ctx::with("font.id", |eng, _| {
                Ok(match eng.meanings.get(&crate::token::CsId::intern(&name)) {
                    Some(Meaning::Font(f)) => Some(i64::from(*f)),
                    _ if name == "nullfont" => Some(0),
                    _ => None,
                })
            })?;
            Ok(match f {
                Some(f) => Variadic::from_iter([Value::Integer(f)]),
                None => Variadic::from_iter([
                    Value::String(lua.create_string("not a valid font csname")?),
                    Value::Integer(-1),
                ]),
            })
        })?,
    )?;
    // "font.each()": id and table of every defined font.
    t.set(
        "each",
        lua.create_function(|lua, ()| {
            let ids: Vec<i64> = FONTS.with(|f| {
                f.borrow()
                    .iter()
                    .enumerate()
                    .filter(|(_, d)| d.is_some())
                    .map(|(i, _)| i as i64)
                    .collect()
            });
            let at = Rc::new(Cell::new(0usize));
            lua.create_function(move |lua, _: Variadic<Value>| {
                let i = at.get();
                at.set(i + 1);
                match ids.get(i) {
                    Some(&id) => Ok(Variadic::from_iter([
                        Value::Integer(id),
                        table_of(lua, id)?,
                    ])),
                    None => Ok(Variadic::from_iter([Value::Nil])),
                }
            })
        })?,
    )?;
    // Expansion and tounicode are backend settings of a used font; nothing
    // sets type from a Lua font yet, so they are stored nowhere.
    t.set(
        "setexpansion",
        lua.create_function(|_, _: Variadic<Value>| Ok(()))?,
    )?;
    t.set(
        "settounicode",
        lua.create_function(|_, _: Variadic<Value>| Ok(()))?,
    )?;
    t.set(
        "read_vf",
        lua.create_function(|_, _: Variadic<Value>| {
            Err::<(), _>(mlua::Error::runtime(
                "font.read_vf reads a virtual font's packets into Lua, which texrs's vf reader \
                 does not hand out",
            ))
        })?,
    )?;
    // make_table(L, "fonts", "tex.fonts", "getfont", "setfont").
    let fonts = lua.create_table()?;
    let mt = lua.create_table()?;
    mt.set("__index", t.get::<mlua::Function>("getfont")?)?;
    mt.set("__newindex", t.get::<mlua::Function>("setfont")?)?;
    fonts.set_metatable(Some(mt))?;
    t.set("fonts", fonts)?;
    Ok(t)
}
