//! The `pdf` library: what a chunk tells the PDF backend (luatexdir/lua/lpdflib.c).
//!
//! Three kinds of thing pass through it, each into `crate::backend`, the
//! state `\pdfextension` and `\pdfvariable` keep, so the file is written with
//! them:
//!
//! - **Strings for the file's dictionaries** -- `pdf.setcatalog`, `setinfo`,
//!   `setnames`, `settrailer`, `setpageresources`, `setpageattributes`,
//!   `setpagesattributes`, `settrailerid` -- kept apart from `\pdfextension`'s
//!   as luatex keeps them (`Backend::lua`), a string per key (a value that is
//!   not a string is ignored, `l_set_pdf_value`), and read back by the
//!   matching getter. `setxformresources` and `setxformattributes` are kept
//!   and read back here; no form is written from them yet.
//! - **The backend's variables** -- `compresslevel`, `objcompresslevel`,
//!   `decimaldigits`, the PDF version and the rest -- which are `\pdfvariable`'s
//!   entries, read and stored as luatex reads and stores them, so the writer,
//!   `\the\pdfvariable` and Lua agree.
//! - **Objects** -- `pdf.obj`, `pdf.immediateobj`, `pdf.reserveobj` -- read
//!   with luatex's argument checking and numbered, held and written by the
//!   backend as `\pdfextension obj` is.
//!
//! What has no meaning before shipout returns what luatex returns then: the
//! position functions answer 0, the matrix is the identity.

use super::*;
use crate::backend::{Backend, RawObject};

/// What a chunk set that the backend does not hold.
#[derive(Clone, Debug, Default)]
pub struct State {
    /// `xformresources` and `xformattributes`.
    pub xform: BTreeMap<&'static str, Vec<u8>>,
    /// The object numbers a chunk has given contents to: a second
    /// `pdf.obj(n, ...)` is "object in use".
    pub filled: std::collections::BTreeSet<u32>,
    /// `pdf.mapfile`/`pdf.mapline` lines, in order.
    pub map_lines: Vec<String>,
    /// `setfontattributes(font, attrs)`.
    pub font_attributes: BTreeMap<i64, String>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    /// The chunk's view of the registers, for the dimension variables.
    static BRIDGE: RefCell<Option<Rc<RefCell<Bridge>>>> = const { RefCell::new(None) };
}

/// What the chunks told the backend that it does not hold itself.
pub fn state() -> State {
    STATE.with(|s| s.borrow().clone())
}

pub(super) fn reset() {
    STATE.with(|s| *s.borrow_mut() = State::default());
}

/// The dictionary keys `l_set_pdf_value` takes.
const KEYS: &[&str] = &[
    "catalog",
    "info",
    "names",
    "trailer",
    "pageresources",
    "pageattributes",
    "pagesattributes",
    "xformresources",
    "xformattributes",
    "trailerid",
];

/// The backend's string for `key`, or `None` for the two it does not hold.
fn lua_string<'a>(s: &'a mut crate::backend::LuaStrings, key: &str) -> Option<&'a mut String> {
    Some(match key {
        "catalog" => &mut s.catalog,
        "info" => &mut s.info,
        "names" => &mut s.names,
        "trailer" => &mut s.trailer,
        "pageresources" => &mut s.page_resources,
        "pageattributes" => &mut s.page_attributes,
        "pagesattributes" => &mut s.pages_attributes,
        "trailerid" => &mut s.trailer_id,
        _ => return None,
    })
}

const VARIABLES: &[&str] = &[
    "compresslevel",
    "objcompresslevel",
    "recompress",
    "decimaldigits",
    "suppressoptionalinfo",
    "majorversion",
    "minorversion",
    "imageresolution",
    "threadmargin",
    "destmargin",
    "linkmargin",
    "xformmargin",
    "inclusionerrorlevel",
    "ignoreunknownimages",
    "gentounicode",
    "omitcidset",
    "omitcharset",
    "omitinfo",
    "omitmediabox",
    "omitprocset",
    "ptexprefix",
];

/// The `\pdfvariable` key a getter or setter reads: `getomitinfo` is
/// `pdf_omit_infodict`, the rest are spelled as their key.
fn key(name: &str) -> &str {
    match name {
        "omitinfo" => "omitinfodict",
        other => other,
    }
}

/// A backend variable as luatex reads it, the eqtb entry `\pdfvariable`
/// names (`get_tex_extension_count_register` and its dimension twin): what
/// the document assigned, by `\pdfvariable` or from Lua, else the value the
/// format leaves it at (`crate::backend::format_default`), which is what the
/// writer reads too. A dimension assigned zero cannot be told from one never
/// assigned and reads the format's value.
fn var(name: &str) -> i64 {
    let key = key(name);
    let parameter = crate::backend::parameter(key);
    if let Some(i) = crate::intpar::index(&parameter) {
        if ctx::depth() > 0 {
            if let Ok(Some(v)) = ctx::with("pdf", |eng, _| Ok(eng.intpars.assigned(i))) {
                return v;
            }
        }
        return crate::backend::format_default(key);
    }
    if let Some(slot) = crate::params::register_slot(&parameter) {
        let v = BRIDGE.with(|b| b.borrow().as_ref().map(|b| slot_value(&b.borrow(), slot)));
        if let Some(v) = v.filter(|v| *v != 0) {
            return v;
        }
    }
    crate::backend::format_default(key)
}

/// `set_tex_extension_count_register` and its dimension twin: a plain store
/// into the entry, not saved for the enclosing group (`int_par(...) = d`).
fn set_var(name: &str, v: i64) {
    let parameter = crate::backend::parameter(key(name));
    if let Some(i) = crate::intpar::index(&parameter) {
        if ctx::depth() > 0 {
            let _ = ctx::with("pdf", |eng, _| {
                eng.store_intpar(i, v);
                Ok(())
            });
        }
        return;
    }
    if let Some(slot) = crate::params::register_slot(&parameter) {
        BRIDGE.with(|b| {
            if let Some(b) = b.borrow().as_ref() {
                let mut b = b.borrow_mut();
                b.counts.insert(slot, v);
                b.counts_written.insert(slot, v);
            }
        });
    }
}

fn bytes(v: &Value) -> mlua::Result<Option<Vec<u8>>> {
    Ok(match v {
        Value::String(s) => Some(s.as_bytes().to_vec()),
        Value::Integer(n) => Some(n.to_string().into_bytes()),
        Value::Number(f) => Some(f.to_string().into_bytes()),
        _ => None,
    })
}

/// An object's contents as `pdf.obj` and its siblings read them.
struct Contents {
    stream: bool,
    file: bool,
    data: Vec<u8>,
    attr: Option<Vec<u8>>,
    /// `objcompression = false`: never into an object stream.
    uncompressed: bool,
}

impl Contents {
    /// The kinds the positional forms name: `"file"`, `"stream"`,
    /// `"streamfile"`, or none for a raw object's text.
    fn of(kind: Option<&str>, data: Vec<u8>, attr: Option<Vec<u8>>) -> Contents {
        Contents {
            stream: matches!(kind, Some("stream" | "streamfile")),
            file: matches!(kind, Some("file" | "streamfile")),
            data,
            attr,
            uncompressed: false,
        }
    }

    fn raw(self) -> RawObject {
        let text = |b: Vec<u8>| String::from_utf8_lossy(&b).into_owned();
        RawObject {
            stream: self.stream,
            attr: self.attr.map(text),
            file: self.file,
            uncompressed: self.uncompressed,
            data: self.data,
        }
    }
}

/// An object number a chunk passed in: it must exist, be a raw object's
/// (`check_obj_exists`, `check_obj_type`) and not be filled yet.
fn reuse(eng: &Engine, k: i64, func: &str) -> mlua::Result<u32> {
    if k < 0 || k > i64::from(eng.backend.objects()) {
        return Err(mlua::Error::runtime("cannot find referenced object"));
    }
    let k = k as u32;
    match eng.backend.object_kind(k) {
        Some("obj") => {}
        kind => {
            return Err(mlua::Error::runtime(format!(
                "referenced object has wrong type {}; should be obj",
                kind.unwrap_or("others")
            )))
        }
    }
    if eng.backend.raw.contains_key(&k) || STATE.with(|s| s.borrow().filled.contains(&k)) {
        return Err(mlua::Error::runtime(format!("pdf.{func}() object in use")));
    }
    Ok(k)
}

/// Hand `contents` to the backend as object `number` (a new one when
/// `None`), written now when `immediate`: `Backend::lua_obj`.
fn put(number: Option<i64>, func: &str, contents: Contents, immediate: bool) -> mlua::Result<u32> {
    ctx::with("pdf", |eng, _| {
        let number = match number {
            Some(k) => Some(reuse(eng, k, func)?),
            None => None,
        };
        let k = Backend::lua_obj(eng, number, contents.raw(), immediate);
        STATE.with(|s| s.borrow_mut().filled.insert(k));
        Ok(k)
    })
}

/// The leading object number of the positional forms, if there is one.
fn leading_number(args: &Variadic<Value>) -> (Option<i64>, &[Value]) {
    match args.first() {
        Some(Value::Integer(n)) => (Some(*n), &args[1..]),
        _ => (None, &args[..]),
    }
}

/// The positional forms `orig_obj` and `l_immediateobj` share:
/// `([n,] text)`, `([n,] "file", name)`, `([n,] "stream"|"streamfile",
/// data [, attr])`, checked with `func`'s messages.
fn positional(rest: &[Value], func: &str) -> mlua::Result<Contents> {
    let err = |what: &str| mlua::Error::runtime(format!("pdf.{func}() {what}"));
    match rest.len() {
        0 => Err(err("needs at least one argument")),
        1 => {
            let data = bytes(&rest[0])?.ok_or_else(|| err("1st argument must be string"))?;
            Ok(Contents::of(None, data, None))
        }
        2 | 3 => {
            let Value::String(kind) = &rest[0] else {
                return Err(err("1st argument must be string"));
            };
            let kind = kind.to_str()?.to_string();
            let data = bytes(&rest[1])?.ok_or_else(|| err("2nd argument must be string"))?;
            if kind == "file" && rest.len() == 3 {
                return Err(err("3rd argument forbidden in file mode"));
            }
            if !matches!(kind.as_str(), "file" | "stream" | "streamfile") {
                return Err(err("invalid argument"));
            }
            let attr = match rest.get(2) {
                Some(v) => Some(bytes(v)?.ok_or_else(|| err("3rd argument must be string"))?),
                None => None,
            };
            Ok(Contents::of(Some(&kind), data, attr))
        }
        _ => Err(err("allows max. 3 arguments")),
    }
}

/// `pdf.immediateobj`.
fn immediateobj(args: &Variadic<Value>) -> mlua::Result<u32> {
    let (number, rest) = leading_number(args);
    let contents = positional(rest, "immediateobj")?;
    put(number, "immediateobj", contents, true)
}

/// `orig_obj`: the positional form of `pdf.obj`, held for a page.
fn orig_obj(args: &Variadic<Value>) -> mlua::Result<u32> {
    let (number, rest) = leading_number(args);
    let contents = positional(rest, "obj")?;
    put(number, "obj", contents, false)
}

/// `pdf.obj{...}`: `table_obj`'s keys and checks.
fn table_obj(t: &Table) -> mlua::Result<u32> {
    let err = |what: &str| mlua::Error::runtime(format!("pdf.obj(): {what}"));
    let kind = match t.raw_get::<Value>("type")? {
        Value::Nil => return Err(err("object \"type\" missing")),
        Value::String(s) => s.to_str()?.to_string(),
        _ => return Err(err("object \"type\" must be string")),
    };
    if kind != "raw" && kind != "stream" {
        return Err(err(&format!("\"{kind}\" is not a valid object type")));
    }
    let stream = kind == "stream";
    let immediate = match t.raw_get::<Value>("immediate")? {
        Value::Nil => false,
        Value::Boolean(b) => b,
        _ => return Err(err("\"immediate\" must be boolean")),
    };
    let number = match t.raw_get::<Value>("objnum")? {
        Value::Nil => None,
        Value::Integer(n) => Some(n),
        _ => return Err(err("\"objnum\" must be integer")),
    };
    let attr = match t.raw_get::<Value>("attr")? {
        Value::Nil => None,
        v => {
            if !stream {
                return Err(err("\"attr\" key not allowed for non-stream object"));
            }
            Some(bytes(&v)?.ok_or_else(|| err("object \"attr\" must be string"))?)
        }
    };
    // A stream's own `compresslevel` is checked as luatex checks it; the
    // writer compresses every stream at `\pdfvariable compresslevel`.
    match t.raw_get::<Value>("compresslevel")? {
        Value::Nil => {}
        Value::Integer(n) => {
            if !stream {
                return Err(err("\"compresslevel\" key not allowed for raw object"));
            }
            if n > 9 {
                return Err(err("\"compresslevel\" must be <= 9"));
            }
            if n < 0 {
                return Err(err("\"compresslevel\" must be >= 0"));
            }
        }
        _ => return Err(err("\"compresslevel\" must be integer")),
    }
    let uncompressed = match t.raw_get::<Value>("objcompression")? {
        Value::Nil => false,
        _ if stream => return Err(err("\"objcompression\" key not allowed for stream object")),
        Value::Boolean(b) => !b,
        _ => return Err(err("\"objcompression\" must be boolean")),
    };
    let (string, file) = (t.raw_get::<Value>("string")?, t.raw_get::<Value>("file")?);
    let (data, from_file) = match (&string, &file) {
        (Value::Nil, Value::Nil) => return Err(err("no \"string\" or \"file\" given")),
        (Value::Nil, f) => (bytes(f)?, true),
        (s, Value::Nil) => (bytes(s)?, false),
        _ => return Err(err("\"string\" and \"file\" must not be given together")),
    };
    let data = data.ok_or_else(|| match (stream, from_file) {
        (_, true) => err(&format!("\"file\" name must be string for {kind} object")),
        (_, false) => err(&format!("\"string\" must be string for {kind} object")),
    })?;
    let contents = Contents {
        stream,
        file: from_file,
        data,
        attr,
        uncompressed,
    };
    put(number, "obj", contents, immediate)
}

/// The `pdf` table.
pub(super) fn table(lua: &Lua, bridge: &Rc<RefCell<Bridge>>) -> mlua::Result<Table> {
    reset();
    BRIDGE.with(|b| *b.borrow_mut() = Some(bridge.clone()));
    let t = lua.create_table()?;
    for key in KEYS {
        let key: &'static str = key;
        t.set(
            format!("set{key}"),
            lua.create_function(move |_, args: Variadic<Value>| {
                // `l_set_pdf_value`: a string is kept, anything else ignored.
                let Some(Value::String(s)) = args.last() else {
                    return Ok(());
                };
                let v = s.as_bytes().to_vec();
                ctx::with("pdf", |eng, _| {
                    match lua_string(&mut eng.backend.lua, key) {
                        Some(field) => *field = String::from_utf8_lossy(&v).into_owned(),
                        None => {
                            STATE.with(|st| st.borrow_mut().xform.insert(key, v));
                        }
                    }
                    Ok(())
                })
            })?,
        )?;
        t.set(
            format!("get{key}"),
            lua.create_function(move |lua, ()| {
                // Nothing set reads `nil`, as an absent `pdf.data` key does.
                let v = ctx::with("pdf", |eng, _| {
                    Ok(match lua_string(&mut eng.backend.lua, key) {
                        Some(field) => (!field.is_empty()).then(|| field.clone().into_bytes()),
                        None => STATE.with(|st| st.borrow().xform.get(key).cloned()),
                    })
                })?;
                match v {
                    Some(v) => Ok(Value::String(lua.create_string(v)?)),
                    None => Ok(Value::Nil),
                }
            })?,
        )?;
    }
    for name in VARIABLES {
        let name: &'static str = name;
        t.set(
            format!("get{name}"),
            lua.create_function(move |_, ()| Ok(var(name)))?,
        )?;
        t.set(
            format!("set{name}"),
            lua.create_function(move |_, v: Value| {
                // Each setter takes only a number, and some bound it as
                // their C does: the compression levels to 0..9, the digits
                // to at least 0, the version to the PDF versions there are.
                // The margins are dimensions, rounded (`lua_roundnumber`);
                // the rest are integers (`lua_tointeger`).
                let n = match v {
                    Value::Integer(n) => n,
                    Value::Number(f) if name.ends_with("margin") => f.round() as i64,
                    Value::Number(f) if f.fract() == 0.0 => f as i64,
                    Value::Number(_) => 0,
                    _ => return Ok(()),
                };
                match name {
                    "compresslevel" | "objcompresslevel" | "recompress" => {
                        set_var(name, n.clamp(0, 9))
                    }
                    "decimaldigits" => set_var(name, n.max(0)),
                    "majorversion" if (1..=2).contains(&n) => set_var(name, n),
                    "minorversion" if (0..=9).contains(&n) => set_var(name, n),
                    "majorversion" | "minorversion" => {}
                    _ => set_var(name, n),
                }
                Ok(())
            })?,
        )?;
    }
    // `getpkresolution` answers the resolution and whether it is fixed; the
    // setter bounds the resolution to 72..8000 and, given a second number,
    // sets the fixed flag from the FIRST argument, as `l_set_pk_resolution`
    // does.
    t.set(
        "getpkresolution",
        lua.create_function(|_, ()| Ok((var("pkresolution"), var("pkfixeddpi"))))?,
    )?;
    t.set(
        "setpkresolution",
        lua.create_function(|_, (r, fixed): (Value, Value)| {
            if let Some(r) = r.as_integer() {
                set_var("pkresolution", r.clamp(72, 8000));
                if fixed.is_number() || fixed.is_integer() {
                    set_var("pkfixeddpi", r);
                }
            }
            Ok(())
        })?,
    )?;
    t.set(
        "obj",
        lua.create_function(|_, args: Variadic<Value>| match &args[..] {
            [Value::Table(t)] => table_obj(t),
            _ => orig_obj(&args),
        })?,
    )?;
    t.set(
        "immediateobj",
        lua.create_function(|_, args: Variadic<Value>| immediateobj(&args))?,
    )?;
    t.set(
        "reserveobj",
        lua.create_function(|_, args: Variadic<Value>| match &args[..] {
            [] => ctx::with("pdf", |eng, _| Ok(Backend::reserve(eng))),
            [Value::String(s)] if &*s.to_str()? == "annot" => {
                ctx::with("pdf", |eng, _| Ok(Backend::reserve_annot(eng)))
            }
            [Value::String(_)] => Err(mlua::Error::runtime(
                "pdf.reserveobj() optional string must be \"annot\"",
            )),
            [_] => Err(mlua::Error::runtime(
                "pdf.reserveobj() optional argument must be string",
            )),
            _ => Err(mlua::Error::runtime(
                "pdf.reserveobj() allows max. 1 argument",
            )),
        })?,
    )?;
    t.set(
        "refobj",
        lua.create_function(|_, args: Variadic<Value>| {
            if args.len() != 1 {
                return Err(mlua::Error::runtime(
                    "pdf.refobj() needs exactly 1 argument",
                ));
            }
            Ok(())
        })?,
    )?;
    t.set(
        "registerannot",
        lua.create_function(|_, _: Variadic<Value>| {
            Err::<(), _>(mlua::Error::runtime(
                "pdf.registerannot() can only be used in late lua",
            ))
        })?,
    )?;
    // What the backend counts and remembers.
    let getter = |lua: &Lua, f: fn(&crate::backend::Backend) -> i64| {
        lua.create_function(move |_, ()| ctx::with("pdf", |eng, _| Ok(f(&eng.backend))))
    };
    t.set("getmaxobjnum", getter(lua, |b| i64::from(b.objects()))?)?;
    t.set("maxobjnum", getter(lua, |b| i64::from(b.objects()))?)?;
    t.set("getlastobj", getter(lua, |b| i64::from(b.last_obj))?)?;
    t.set("getlastannot", getter(lua, |b| i64::from(b.last_annot))?)?;
    t.set("getlastlink", getter(lua, |b| i64::from(b.last_link))?)?;
    t.set("getretval", getter(lua, |b| b.retval)?)?;
    t.set(
        "getnofobjects",
        lua.create_function(|_, ()| {
            ctx::with("pdf", |eng, _| Ok(eng.backend.written_and_dropped()))
        })?,
    )?;
    for name in ["getobjtype", "objtype"] {
        t.set(
            name,
            lua.create_function(|_, k: i64| {
                ctx::with("pdf", |eng, _| {
                    Ok(u32::try_from(k)
                        .ok()
                        .and_then(|k| eng.backend.object_kind(k)))
                })
            })?,
        )?;
    }
    // Before shipout there is no position and no matrix (`getpos` is 0, 0 in
    // luatex 1.24.0 when nothing is being shipped).
    t.set("getpos", lua.create_function(|_, ()| Ok((0, 0)))?)?;
    t.set("gethpos", lua.create_function(|_, ()| Ok(0))?)?;
    t.set("getvpos", lua.create_function(|_, ()| Ok(0))?)?;
    t.set("hasmatrix", lua.create_function(|_, ()| Ok(false))?)?;
    t.set(
        "getmatrix",
        lua.create_function(|_, ()| Ok((1, 0, 0, 1, 0, 0)))?,
    )?;
    t.set(
        "getorigin",
        lua.create_function(|_, ()| Ok((var("horigin"), var("vorigin"))))?,
    )?;
    t.set(
        "setorigin",
        // `setpdforigin`: both 0 unless the first is a number; the second
        // defaults to the first.
        lua.create_function(|_, (h, v): (Option<f64>, Option<f64>)| {
            let (h, v) = match h {
                Some(h) => (
                    h.round() as i64,
                    v.map_or(h.round() as i64, |v| v.round() as i64),
                ),
                None => (0, 0),
            };
            set_var("horigin", h);
            set_var("vorigin", v);
            Ok(())
        })?,
    )?;
    t.set(
        "getcreationdate",
        lua.create_function(|_, ()| Ok(crate::pdf::pdf_date(crate::pdf::stamp())))?,
    )?;
    for name in ["mapfile", "mapline"] {
        t.set(
            name,
            lua.create_function(move |_, s: String| {
                let line = match name {
                    "mapfile" => format!("file {s}"),
                    _ => s,
                };
                STATE.with(|st| st.borrow_mut().map_lines.push(line));
                Ok(())
            })?,
        )?;
    }
    // `newpdfcolorstack(init [, mode [, page]])`: the backend's new stack,
    // the one `\pdffeedback colorstackinit` makes.
    t.set(
        "newcolorstack",
        lua.create_function(|_, args: Variadic<Value>| {
            use crate::pdfpage::LiteralMode;
            let Some(Value::String(init)) = args.first() else {
                return Err(mlua::Error::runtime(
                    "pdf.newcolorstack() expects a string as first argument",
                ));
            };
            let init = init.to_str()?.to_string();
            let mode = match args.get(1) {
                Some(Value::String(m)) => match &*m.to_str()? {
                    "origin" => LiteralMode::SetOrigin,
                    "page" => LiteralMode::DirectPage,
                    "text" => LiteralMode::DirectText,
                    "direct" => LiteralMode::DirectAlways,
                    "raw" => LiteralMode::DirectRaw,
                    _ => {
                        return Err(mlua::Error::runtime(
                            "invalid literal mode in pdf.newcolorstack()",
                        ))
                    }
                },
                _ => LiteralMode::SetOrigin,
            };
            let page_start = matches!(args.get(2), Some(Value::Boolean(true)));
            ctx::with("pdf", |eng, _| {
                Ok(Backend::new_color_stack(eng, mode, page_start, init) as i64)
            })
        })?,
    )?;
    t.set(
        "setfontattributes",
        lua.create_function(|_, (f, attrs): (i64, String)| {
            STATE.with(|s| s.borrow_mut().font_attributes.insert(f, attrs));
            Ok(())
        })?,
    )?;
    // Writing into the page stream, and the questions about fonts and pages
    // that only a backend that is writing can answer.
    for (name, why) in [
        (
            "print",
            "writes into the page stream being built, which only exists during shipout",
        ),
        ("getpageref", "a page's object number is the writer's"),
        ("pageref", "a page's object number is the writer's"),
        ("getfontname", "a font's resource name is the writer's"),
        ("fontname", "a font's resource name is the writer's"),
        ("getfontobjnum", "a font's object number is the writer's"),
        ("fontobjnum", "a font's object number is the writer's"),
        ("getfontsize", "a font's size in the PDF is the writer's"),
        ("fontsize", "a font's size in the PDF is the writer's"),
        ("getxformname", "an xform's resource name is the writer's"),
        ("xformname", "an xform's resource name is the writer's"),
        ("includechar", "a font's glyph subset is the writer's"),
        ("includefont", "a font's inclusion is the writer's"),
        ("includeimage", "an image's inclusion is the writer's"),
    ] {
        t.set(
            name,
            lua.create_function(move |_, _: Variadic<Value>| {
                Err::<(), _>(mlua::Error::runtime(format!(
                    "pdf.{name}: {why}, and texrs's writer does not answer it to Lua yet"
                )))
            })?,
        )?;
    }
    for name in ["setforcefile", "settypeonewidemode", "setdviencoding"] {
        t.set(name, lua.create_function(|_, _: Variadic<Value>| Ok(()))?)?;
    }
    Ok(t)
}
