//! The half of the `tex` library that reaches TeX's tables rather than its
//! registers: internal parameters, the character-code tables, the hash.
//!
//! Ports of luatexdir/lua/ltexlib.c's `gettex`/`settex` (`tex.get`, `tex.set`
//! and `tex.<name>` through the `tex.meta` metatable), the `get`/`set` pairs
//! for `\catcode`, `\lccode`, `\uccode`, `\sfcode`, `\mathcode` and
//! `\delcode` with their `tex.catcode[...]` table spellings, `hashtokens`,
//! `primitives` and `badness`.
//!
//! An assignment goes through the ENGINE's own assignment, not around it: the
//! value is spelt as the text TeX would scan (`=123 `) and handed to the same
//! `compile_time_*` entry the lowerer calls for `\tolerance=123`, so it is
//! saved for the enclosing group exactly as a TeX assignment is, `"global"`
//! is `\global`, and `\globaldefs` applies. A dimension or glue parameter is a
//! slot of the register file (`crate::params`), and goes the way a register
//! write from Lua already goes (the parent module's bridge); a token parameter
//! is one of the expander's token lists.

use super::*;

/// Run `text` through `assign` as if it stood in the document, with
/// `\global` in front when `global`.
fn assign(
    eng: &mut Engine,
    global: bool,
    text: &str,
    assign: impl FnOnce(&mut Engine, &mut Lexer) -> R<()>,
) -> mlua::Result<()> {
    let mut lx = Lexer::new(text);
    eng.set_global_prefix(global);
    let out = assign(eng, &mut lx);
    eng.set_global_prefix(false);
    out.map_err(|e| mlua::Error::runtime(e.0))
}

/// A leading `"global"`, LuaTeX's `check_item_global`: it counts only when
/// it is the first of more arguments than the call needs.
fn global_prefix(args: &Variadic<Value>, needed: usize) -> (bool, Vec<Value>) {
    let is_global =
        |v: &Value| matches!(v, Value::String(s) if s.to_str().is_ok_and(|s| &*s == "global"));
    match args.first() {
        Some(v) if args.len() > needed && is_global(v) => {
            (true, args.iter().skip(1).cloned().collect())
        }
        _ => (false, args.iter().cloned().collect()),
    }
}

/// What kind of internal quantity a primitive name is.
enum Param {
    Int(usize),
    Dimen(i64),
    Glue { slot: i64 },
    Toks(i64),
}

fn param(name: &str) -> Option<Param> {
    if let Some(i) = crate::intpar::index(name) {
        return Some(Param::Int(i));
    }
    if let Some(slot) = crate::params::register_slot(name) {
        if crate::params::DIMEN_NAMES.contains(&name)
            || crate::params::LUATEX_DIMEN_NAMES.contains(&name)
        {
            return Some(Param::Dimen(slot));
        }
        return Some(Param::Glue { slot });
    }
    crate::params::toks_register(name).map(Param::Toks)
}

/// `gettex` for a parameter name: an integer, scaled points, a `glue_spec`
/// node (or with a trailing boolean, the five numbers or the width alone), or
/// a token list as a string.
fn get_param(
    lua: &Lua,
    bridge: &Rc<RefCell<Bridge>>,
    arena: &Rc<RefCell<node::Arena>>,
    p: &Param,
    all: Option<bool>,
) -> mlua::Result<Variadic<Value>> {
    let one = |v: Value| Ok(Variadic::from_iter([v]));
    match p {
        Param::Int(i) => {
            let v = ctx::with("tex.get", |eng, _| Ok(eng.intpars.get(*i)))?;
            one(Value::Integer(v))
        }
        Param::Dimen(slot) => one(Value::Integer(slot_value(&bridge.borrow(), *slot))),
        Param::Glue { slot, .. } => {
            let g = glue_get(&bridge.borrow(), *slot);
            match all {
                Some(false) => one(Value::Integer(g.0)),
                Some(true) => Ok(Variadic::from_iter(
                    [g.0, g.1, g.2, g.3, g.4].map(Value::Integer),
                )),
                None => one(node::spec_node(lua, arena, g)?),
            }
        }
        Param::Toks(reg) => {
            let text = bridge.borrow().toks.get(reg).cloned().unwrap_or_default();
            one(Value::String(lua.create_string(text)?))
        }
    }
}

/// `settex` for a parameter name.
fn set_param(
    bridge: &Rc<RefCell<Bridge>>,
    arena: &Rc<RefCell<node::Arena>>,
    p: &Param,
    global: bool,
    values: &[Value],
) -> mlua::Result<()> {
    let value = values.first().cloned().unwrap_or(Value::Nil);
    match p {
        Param::Int(i) => {
            let Some(n) = value
                .as_integer()
                .or_else(|| value.as_number().map(|f| f as i64))
            else {
                return Err(mlua::Error::runtime("unsupported value type"));
            };
            let i = *i;
            ctx::with("tex.set", |eng, _| {
                assign(eng, global, &format!("={n} "), |eng, lx| {
                    eng.compile_time_intpar(lx, i)
                })
            })
        }
        Param::Dimen(slot) => {
            let v = scaled_value(Kind::Dimen, &value)
                .map_err(|_| mlua::Error::runtime("unsupported value type"))?;
            let mut b = bridge.borrow_mut();
            b.counts.insert(*slot, v);
            b.counts_written.insert(*slot, v);
            Ok(())
        }
        // "tex.set('baselineskip', width, stretch, shrink, sto, sho)" or a
        // glue_spec node (settex's `is_glue_assign` arm).
        Param::Glue { slot, .. } => {
            let g = match &value {
                Value::Integer(_) | Value::Number(_) => {
                    let n = |i: usize| {
                        values
                            .get(i)
                            .and_then(|v| {
                                v.as_integer()
                                    .or_else(|| v.as_number().map(|f| f.round() as i64))
                            })
                            .unwrap_or(0)
                    };
                    (n(0), n(1), n(2), n(3), n(4))
                }
                other => node::spec_values(arena, other)?,
            };
            glue_set(&mut bridge.borrow_mut(), *slot, g)
        }
        Param::Toks(reg) => {
            let Value::String(s) = &value else {
                return Err(mlua::Error::runtime("unsupported value type"));
            };
            let text = s.to_str()?.to_string();
            let mut b = bridge.borrow_mut();
            b.toks.insert(*reg, text.clone());
            b.toks_written.insert(*reg, text);
            Ok(())
        }
    }
}

/// The read-only `last_item` and `convert` quantities `gettex` answers through
/// `do_lastitem`/`do_convert`, for the ones texrs knows.
fn get_item(lua: &Lua, name: &str) -> mlua::Result<Option<Value>> {
    Ok(Some(match name {
        "inputlineno" => {
            Value::Integer(ctx::with("tex.inputlineno", |_, lx| Ok(lx.line() as i64))?)
        }
        "luatexversion" => Value::Integer(124),
        "eTeXversion" => Value::Integer(2),
        "eTeXminorversion" => Value::Integer(6),
        "luatexrevision" => Value::String(lua.create_string("0")?),
        "eTeXrevision" => Value::String(lua.create_string(".6")?),
        "jobname" => Value::String(lua.create_string(jobname())?),
        "currentgrouplevel" => Value::Integer(ctx::with("tex.currentgrouplevel", |eng, _| {
            Ok(eng.group_level() as i64)
        })?),
        _ => return Ok(None),
    }))
}

/// The six character-code tables, by LuaTeX's names.
#[derive(Clone, Copy)]
enum Code {
    Cat,
    Table(crate::charcodes::Table),
}

/// Install everything above into `tex`.
pub(super) fn install(
    lua: &Lua,
    tex: &Table,
    bridge: &Rc<RefCell<Bridge>>,
    arena: &Rc<RefCell<node::Arena>>,
) -> mlua::Result<()> {
    use crate::charcodes::Table as T;

    // `tex.get(name [, all])`.
    let (b, a) = (Rc::clone(bridge), Rc::clone(arena));
    tex.set(
        "get",
        lua.create_function(move |lua, args: Variadic<Value>| {
            let mut args: Vec<Value> = args.into_iter().collect();
            let all = match args.last() {
                Some(Value::Boolean(v)) if args.len() > 1 => {
                    let v = *v;
                    args.pop();
                    Some(v)
                }
                _ => None,
            };
            let Some(Value::String(name)) = args.last() else {
                return Ok(Variadic::from_iter([Value::Nil]));
            };
            let name = name.to_str()?.to_string();
            if let Some(p) = param(&name) {
                return get_param(lua, &b, &a, &p, all);
            }
            if let Some(v) = get_item(lua, &name)? {
                return Ok(Variadic::from_iter([v]));
            }
            refused(&name).map(|_| Variadic::from_iter([Value::Nil]))
        })?,
    )?;
    // `tex.set(["global",] name, value...)`.
    let (b, a) = (Rc::clone(bridge), Rc::clone(arena));
    tex.set(
        "set",
        // settex reads the name at `top - 1` and the value at `top`, and
        // `"global"` only as the first of exactly three arguments: anything
        // else is quietly nothing, which is what luatex does with
        // `tex.set("baselineskip", 786432, 65536)` -- the name is not where it
        // looks.
        lua.create_function(move |_, args: Variadic<Value>| {
            let n = args.len();
            if n < 2 {
                return Ok(());
            }
            let Value::String(name) = &args[n - 2] else {
                return Ok(());
            };
            let global = n == 3
                && matches!(&args[0], Value::String(s) if s.to_str().is_ok_and(|s| &*s == "global"));
            let name = name.to_str()?.to_string();
            match param(&name) {
                Some(p) => set_param(&b, &a, &p, global, &args[n - 1..]),
                None => refused(&name),
            }
        })?,
    )?;
    // The `tex.meta` metatable: `tex.tolerance` and `tex.tolerance = 200`.
    // A name that is no parameter behaves as an ordinary field ("people may
    // want to add keys that are also primitives ... so creating an error is
    // not right here").
    let mt = lua.create_table()?;
    let (b, a) = (Rc::clone(bridge), Rc::clone(arena));
    mt.set(
        "__index",
        lua.create_function(move |lua, (_, key): (Table, Value)| {
            let Value::String(s) = &key else {
                return Ok(Value::Nil);
            };
            let name = s.to_str()?.to_string();
            if let Some(p) = param(&name) {
                return Ok(get_param(lua, &b, &a, &p, None)?
                    .into_iter()
                    .next()
                    .unwrap_or(Value::Nil));
            }
            if let Some(v) = get_item(lua, &name)? {
                return Ok(v);
            }
            refused(&name).map(|_| Value::Nil)
        })?,
    )?;
    let (b, a) = (Rc::clone(bridge), Rc::clone(arena));
    mt.set(
        "__newindex",
        lua.create_function(move |_, (t, key, value): (Table, Value, Value)| {
            if let Value::String(s) = &key {
                let name = s.to_str()?.to_string();
                if let Some(p) = param(&name) {
                    return set_param(&b, &a, &p, false, &[value]);
                }
                if parameter_reason(&key)?.is_some() {
                    return refused(&name);
                }
            }
            t.raw_set(key, value)
        })?,
    )?;
    tex.set_metatable(Some(mt))?;

    // The character-code tables: `getcatcode([cattable,] c)`,
    // `setcatcode(["global",] [cattable,] c, v)`, and `tex.catcode[c]`. texrs
    // has one catcode table, so a table argument is read and dropped.
    for (name, code) in [
        ("catcode", Code::Cat),
        ("lccode", Code::Table(T::Lower)),
        ("uccode", Code::Table(T::Upper)),
        ("sfcode", Code::Table(T::Space)),
        ("mathcode", Code::Table(T::Math)),
        ("delcode", Code::Table(T::Delimiter)),
    ] {
        let getter = lua.create_function(move |_, args: Variadic<Value>| {
            let Some(c) = args.last().and_then(|v| v.as_integer()) else {
                return Err(mlua::Error::runtime(format!(
                    "bad argument to 'get{name}' (number expected)"
                )));
            };
            let ch = char::from_u32(c as u32)
                .ok_or_else(|| mlua::Error::runtime("invalid character code"))?;
            ctx::with(name, |eng, _| {
                Ok(match code {
                    Code::Cat => eng.cats.get(ch) as i64,
                    Code::Table(t) => eng.charcodes.get(t, ch),
                })
            })
        })?;
        let setter = lua.create_function(move |_, args: Variadic<Value>| {
            let (global, args) = global_prefix(&args, 2);
            let n = args.len();
            let (Some(c), Some(v)) = (
                args.get(n.wrapping_sub(2)).and_then(|v| v.as_integer()),
                args.get(n.wrapping_sub(1)).and_then(|v| v.as_integer()),
            ) else {
                return Err(mlua::Error::runtime(format!(
                    "bad arguments to 'set{name}'"
                )));
            };
            ctx::with(name, |eng, _| {
                let text = format!("{c}={v} ");
                match code {
                    Code::Cat if c > 255 => {
                        // The engine's `\catcode` reads a code of 0..255, so a
                        // wider character is set straight into the table.
                        let ch = char::from_u32(c as u32)
                            .ok_or_else(|| mlua::Error::runtime("invalid character code"))?;
                        let cat = crate::catcode::cat_from_i64(v)
                            .ok_or_else(|| mlua::Error::runtime("invalid catcode"))?;
                        eng.cats.set(ch, cat);
                        Ok(())
                    }
                    Code::Cat => assign(eng, global, &text, |eng, lx| eng.compile_time_catcode(lx)),
                    Code::Table(t) => assign(eng, global, &text, |eng, lx| {
                        eng.compile_time_charcode(lx, t)
                    }),
                }
            })
        })?;
        tex.set(format!("get{name}"), getter.clone())?;
        tex.set(format!("set{name}"), setter.clone())?;
        // make_table: `__index` is the getter, `__newindex` the setter, both
        // called with the table first -- which the getter skips by reading its
        // LAST argument, and the setter by reading its last two.
        let t = lua.create_table()?;
        let mt = lua.create_table()?;
        mt.set("__index", getter)?;
        mt.set("__newindex", setter)?;
        t.set_metatable(Some(mt))?;
        tex.set(name, t)?;
    }

    math_and_delimiter_codes(lua, tex)?;

    // "tex.hashtokens()": every name in the hash -- here, every control
    // sequence the meaning table holds and every primitive luatex has.
    tex.set(
        "hashtokens",
        lua.create_function(|lua, ()| {
            let names = ctx::with("tex.hashtokens", |eng, _| {
                let mut names: Vec<String> =
                    eng.meanings.keys().map(|n| n.name().to_string()).collect();
                names.extend(
                    crate::luatex_primitives::TABLE
                        .iter()
                        .map(|(n, _, _)| n.to_string()),
                );
                names.sort();
                names.dedup();
                Ok(names)
            })?;
            lua.create_sequence_from(names)
        })?,
    )?;
    // "tex.primitives()": every primitive's name.
    tex.set(
        "primitives",
        lua.create_function(|lua, ()| {
            lua.create_sequence_from(crate::luatex_primitives::TABLE.iter().map(|(n, _, _)| *n))
        })?,
    )?;
    // "tex.badness(t, s)": §108's badness.
    tex.set(
        "badness",
        lua.create_function(|_, (t, s): (i64, i64)| Ok(badness(t, s)))?,
    )?;
    Ok(())
}

/// `\mathcode` and `\delcode` as LuaTeX hands them over: in their PARTS.
///
/// `getmathcode(c)` is `{class, family, character}` and `getmathcodes(c)` the
/// same three as values; `getdelcode(c)` is `{small family, small character,
/// large family, large character}` and `getdelcodes(c)` the four values. A
/// `\delcode` of -1 (none, INITEX's default) is `-1, 0, 0, 0`, as luatex
/// 1.24.0 answers it. texrs keeps TeX82's packed codes (`"7141`, `"028300`),
/// so the parts are unpacked from those, and a setter packs them -- which
/// refuses a family past 15 or a character past 255, the `\Umathcode` range
/// TeX82's 15 bits cannot hold.
fn math_and_delimiter_codes(lua: &Lua, tex: &Table) -> mlua::Result<()> {
    use crate::charcodes::Table as T;
    fn code_of(args: &Variadic<Value>, what: &str) -> mlua::Result<char> {
        let c = args.last().and_then(|v| v.as_integer()).ok_or_else(|| {
            mlua::Error::runtime(format!("bad argument to '{what}' (number expected)"))
        })?;
        char::from_u32(c as u32)
            .ok_or_else(|| mlua::Error::runtime(format!("{what}: invalid character code")))
    }
    let math_parts = |v: i64| [v >> 12, (v >> 8) & 15, v & 255];
    let del_parts = |v: i64| match v < 0 {
        true => [-1, 0, 0, 0],
        false => [(v >> 20) & 15, (v >> 12) & 255, (v >> 8) & 15, v & 255],
    };
    let get = move |table: T, args: &Variadic<Value>, what: &str| -> mlua::Result<Vec<i64>> {
        let c = code_of(args, what)?;
        let v = ctx::with(what, |eng, _| Ok(eng.charcodes.get(table, c)))?;
        Ok(match table {
            T::Math => math_parts(v).to_vec(),
            _ => del_parts(v).to_vec(),
        })
    };
    for (name, table) in [("mathcode", T::Math), ("delcode", T::Delimiter)] {
        let getter = lua.create_function(move |lua, args: Variadic<Value>| {
            lua.create_sequence_from(get(table, &args, &format!("get{name}"))?)
        })?;
        tex.set(
            format!("get{name}s"),
            lua.create_function(move |_, args: Variadic<Value>| {
                Ok(Variadic::from_iter(
                    get(table, &args, &format!("get{name}s"))?
                        .into_iter()
                        .map(Value::Integer),
                ))
            })?,
        )?;
        // `setmathcode(["global",] c, class, fam, char)` or with the three in
        // a table; `setdelcode` the same with four.
        let parts = match table {
            T::Math => 3,
            _ => 4,
        };
        let setter = lua.create_function(move |_, args: Variadic<Value>| {
            let mut rest: Vec<Value> = args.into_iter().collect();
            let global = matches!(rest.first(), Some(Value::String(s)) if s.to_str().is_ok_and(|s| &*s == "global"));
            if global {
                rest.remove(0);
            }
            // Called through the `tex.mathcode[c] = {...}` table, the table
            // itself comes first.
            if matches!(rest.first(), Some(Value::Table(_))) && rest.len() > 2 {
                rest.remove(0);
            }
            let bad = || mlua::Error::runtime(format!("Bad arguments for tex.set{name}()"));
            let c = rest.first().and_then(|v| v.as_integer()).ok_or_else(bad)?;
            let vals: Vec<i64> = match rest.get(1) {
                Some(Value::Table(t)) => (1..=parts).map(|i| t.raw_get::<i64>(i)).collect::<mlua::Result<_>>()?,
                Some(_) => rest[1..].iter().take(parts).map(|v| v.as_integer().ok_or_else(bad)).collect::<mlua::Result<_>>()?,
                None => return Err(bad()),
            };
            if vals.len() != parts {
                return Err(bad());
            }
            let packed = match table {
                T::Math if vals[1] < 16 && vals[2] < 256 => vals[0] * 0x1000 + vals[1] * 0x100 + vals[2],
                T::Delimiter if vals[0] < 0 => -1,
                T::Delimiter if vals[0] < 16 && vals[1] < 256 && vals[2] < 16 && vals[3] < 256 => {
                    vals[0] * 0x10_0000 + vals[1] * 0x1000 + vals[2] * 0x100 + vals[3]
                }
                _ => {
                    return Err(mlua::Error::runtime(format!(
                        "tex.set{name}: a family past 15 or a character past 255 is a \\U{name}, \
                         which texrs's TeX82 {name} table cannot hold"
                    )))
                }
            };
            ctx::with(name, |eng, _| {
                assign(eng, global, &format!("{c}={packed} "), |eng, lx| eng.compile_time_charcode(lx, table))
            })
        })?;
        tex.set(format!("get{name}"), getter.clone())?;
        tex.set(format!("set{name}"), setter.clone())?;
        let t = lua.create_table()?;
        let mt = lua.create_table()?;
        mt.set("__index", getter)?;
        mt.set("__newindex", setter)?;
        t.set_metatable(Some(mt))?;
        tex.set(name, t)?;
    }
    Ok(())
}

/// `tex.web` §108: `100(t/s)^3`, approximated as Knuth approximates it.
fn badness(t: i64, s: i64) -> i64 {
    const INF_BAD: i64 = 10000;
    if t == 0 {
        return 0;
    }
    if s <= 0 {
        return INF_BAD;
    }
    let r = if t <= 7_230_584 {
        (t * 297) / s
    } else if s >= 1_663_497 {
        t / (s / 297)
    } else {
        t
    };
    if r > 1290 {
        INF_BAD
    } else {
        (r * r * r + 0x20000) / 0x40000
    }
}

/// A name that is one of the manual's parameters texrs does not hold, refused
/// with the reason; any other name is not an error.
fn refused(name: &str) -> mlua::Result<()> {
    let key_ok = name.to_string();
    let lua_free = parameter_reason_text(&key_ok);
    match lua_free {
        Some(why) => Err(mlua::Error::runtime(why)),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badness_is_tex_webs() {
        // Read off luatex 1.24.0's `tex.badness`: a box stretched by exactly its
        // stretch is 100, by half of it 12, by 1.4 times 273 (the cube is
        // approximated, not computed), and an infinite request is inf_bad.
        assert_eq!(badness(10, 10), 100);
        assert_eq!(badness(5, 10), 12);
        assert_eq!(badness(0, 0), 0);
        assert_eq!(badness(1, 0), 10000);
        assert_eq!(badness(14, 10), 273);
        assert_eq!(badness(100, 3), 10000);
    }
}
