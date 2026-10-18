//! The `pdf` library: what a chunk tells the PDF backend (luatexdir/lua/lpdflib.c).
//!
//! Three kinds of thing pass through it:
//!
//! - **Strings for the file's dictionaries** -- `pdf.setcatalog`, `setinfo`,
//!   `setnames`, `settrailer`, `setpageresources`, `setpageattributes`,
//!   `setpagesattributes`, `setxformresources`, `setxformattributes`,
//!   `settrailerid` -- kept as luatex keeps them, a string per key (a value
//!   that is not a string is ignored, `l_set_pdf_value`), and read back by the
//!   matching getter.
//! - **The backend's variables** -- `compresslevel`, `objcompresslevel`,
//!   `decimaldigits`, the PDF version and the rest -- which are `\pdfvariable`'s
//!   entries (`crate::backend`), read and stored as luatex reads and stores
//!   them, so the writer, `\the\pdfvariable` and Lua agree.
//! - **Objects** -- `pdf.obj`, `pdf.immediateobj`, `pdf.reserveobj` -- which
//!   are numbered and their contents kept, with luatex's argument checking.
//!
//! [`state`] is what the writer would read for the other two. It does not read
//! it yet -- the dictionaries and objects a chunk sets are kept and answered
//! back, and not yet written into the file (BUGS.md); the object numbers are
//! this table's own, counted from 1 as a fresh luatex counts them.
//!
//! What has no meaning before shipout returns what luatex returns then: the
//! position functions answer 0, the matrix is the identity.

use super::*;

/// One object a chunk made.
#[derive(Clone, Debug, PartialEq)]
pub struct Obj {
    /// `raw`, `stream`, `file` (raw contents read from a file) or `streamfile`.
    pub kind: String,
    /// The text, or the file name for the `file` kinds.
    pub data: Option<Vec<u8>>,
    /// A stream's extra dictionary entries.
    pub attr: Option<Vec<u8>>,
    pub immediate: bool,
    pub compress_level: Option<i64>,
}

/// Everything a document's Lua told the backend.
#[derive(Clone, Debug, Default)]
pub struct State {
    /// `catalog`, `info`, `names`, `trailer`, `pageresources`,
    /// `pageattributes`, `pagesattributes`, `xformresources`,
    /// `xformattributes`, `trailerid`.
    pub strings: BTreeMap<&'static str, Vec<u8>>,
    /// Every object number handed out, from 1: `obj_tab`'s type and what
    /// was put in it.
    pub table: Vec<Slot>,
    /// The object stream immediate objects are collecting in, 0 for none
    /// (`os->cur_objstm`), and how many it holds (`os->idx`).
    pub cur_objstm: i64,
    pub os_idx: usize,
    /// The `objcompresslevel` the file was opened with, fixed then
    /// (`init_pdf_outputparameters`): bounded to 0..3, and 0 below PDF 1.5.
    pub os_level: Option<i64>,
    pub last_obj: i64,
    pub last_annot: i64,
    /// `pdf.mapfile`/`pdf.mapline` lines, in order.
    pub map_lines: Vec<String>,
    /// `newcolorstack(init, mode, page)`'s stacks.
    pub color_stacks: Vec<(String, String, bool)>,
    /// `setfontattributes(font, attrs)`.
    pub font_attributes: BTreeMap<i64, String>,
}

/// One object number: its `pdf_obj_typenames` type, whether it is in the
/// file yet (`is_obj_written`), and the contents a chunk gave it.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub kind: &'static str,
    pub written: bool,
    pub obj: Option<Obj>,
}

/// `OBJSTM_ALWAYS` and `OBJSTM_NEVER` (`pdftypes.h`): an immediate object
/// goes into an object stream when `objcompresslevel` reaches its threshold.
const OBJSTM_ALWAYS: i64 = 1;
const OBJSTM_NEVER: i64 = 4;
/// `PDF_OS_MAX_OBJS`: an object stream is written once it holds this many.
const OS_MAX_OBJS: usize = 100;

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    /// The chunk's view of the registers, for the dimension variables.
    static BRIDGE: RefCell<Option<Rc<RefCell<Bridge>>>> = const { RefCell::new(None) };
}

/// What the backend has been told, for the writer.
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

/// Open the file if it is not open yet (`ensure_output_state`), which fixes
/// the object-stream level it is written with; that level.
fn open_output() -> i64 {
    if let Some(level) = STATE.with(|s| s.borrow().os_level) {
        return level;
    }
    let level = var("objcompresslevel").clamp(0, 3);
    let streams = var("majorversion") > 1 || var("minorversion") >= 5;
    let level = if streams { level } else { 0 };
    STATE.with(|s| s.borrow_mut().os_level = Some(level));
    level
}

/// A new object number of type `kind` (`pdf_create_obj`).
fn create(s: &mut State, kind: &'static str) -> i64 {
    s.table.push(Slot {
        kind,
        written: false,
        obj: None,
    });
    s.table.len() as i64
}

/// A new raw object's number, which becomes `getlastobj`'s
/// (`obj_count++; k = pdf_create_obj(...); pdf_last_obj = k`).
fn new_obj() -> i64 {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let k = create(&mut s, "obj");
        s.last_obj = k;
        k
    })
}

/// An object number a chunk passed in: it must exist, be a raw object's
/// (`check_obj_exists`, `check_obj_type`) and not be filled yet.
fn reuse(k: i64, func: &str) -> mlua::Result<i64> {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if k < 0 || k > s.table.len() as i64 {
            return Err(mlua::Error::runtime("cannot find referenced object"));
        }
        let Some(slot) = s.table.get((k - 1) as usize) else {
            return Err(mlua::Error::runtime("cannot find referenced object"));
        };
        if slot.kind != "obj" {
            return Err(mlua::Error::runtime(format!(
                "referenced object has wrong type {}; should be obj",
                slot.kind
            )));
        }
        if slot.obj.is_some() || slot.written {
            return Err(mlua::Error::runtime(format!("pdf.{func}() object in use")));
        }
        s.last_obj = k;
        Ok(k)
    })
}

/// Give object `k` its contents, and when it is immediate write it out now
/// at `threshold`: into the current object stream when `objcompresslevel`
/// reaches it, a stream being opened for the first (`pdf_prepare_obj`) and
/// written once it holds `PDF_OS_MAX_OBJS` (`pdf_end_obj`).
fn store(k: i64, obj: Obj, threshold: Option<i64>) {
    let level = open_output();
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.table[(k - 1) as usize].obj = Some(obj);
        let Some(threshold) = threshold else { return };
        if level >= threshold {
            if s.cur_objstm == 0 {
                s.cur_objstm = create(&mut s, "objstm");
                s.os_idx = 0;
            }
            s.os_idx += 1;
            if s.os_idx == OS_MAX_OBJS {
                let stm = s.cur_objstm;
                s.table[(stm - 1) as usize].written = true;
                s.cur_objstm = 0;
            }
        }
        s.table[(k - 1) as usize].written = true;
    })
}

/// `pdf.immediateobj([n,] text)`, `([n,] "file", name)`,
/// `([n,] "stream"|"streamfile", data [, attr])`.
fn immediateobj(args: &Variadic<Value>) -> mlua::Result<i64> {
    open_output();
    let mut first = 0;
    let k = match args.first() {
        Some(Value::Integer(n)) => {
            first = 1;
            reuse(*n, "immediateobj")?
        }
        _ => new_obj(),
    };
    let rest = &args[first..];
    let obj = match rest.len() {
        0 => {
            return Err(mlua::Error::runtime(
                "pdf.immediateobj() needs at least one argument",
            ))
        }
        1 => Obj {
            kind: "raw".into(),
            data: Some(bytes(&rest[0])?.ok_or_else(|| {
                mlua::Error::runtime("pdf.immediateobj() 1st argument must be string")
            })?),
            attr: None,
            immediate: true,
            compress_level: None,
        },
        2 | 3 => {
            let Value::String(kind) = &rest[0] else {
                return Err(mlua::Error::runtime(
                    "pdf.immediateobj() 1st argument must be string",
                ));
            };
            let kind = kind.to_str()?.to_string();
            let data = bytes(&rest[1])?.ok_or_else(|| {
                mlua::Error::runtime("pdf.immediateobj() 2nd argument must be string")
            })?;
            if kind == "file" && rest.len() == 3 {
                return Err(mlua::Error::runtime(
                    "pdf.immediateobj() 3rd argument forbidden in file mode",
                ));
            }
            if !matches!(kind.as_str(), "file" | "stream" | "streamfile") {
                return Err(mlua::Error::runtime("pdf.immediateobj() invalid argument"));
            }
            let attr = match rest.get(2) {
                Some(v) => Some(bytes(v)?.ok_or_else(|| {
                    mlua::Error::runtime("pdf.immediateobj() 3rd argument must be string")
                })?),
                None => None,
            };
            Obj {
                kind,
                data: Some(data),
                attr,
                immediate: true,
                compress_level: None,
            }
        }
        _ => {
            return Err(mlua::Error::runtime(
                "pdf.immediateobj() allows max. 3 arguments",
            ))
        }
    };
    let threshold = match obj.kind.as_str() {
        "raw" | "file" => OBJSTM_ALWAYS,
        _ => OBJSTM_NEVER,
    };
    store(k, obj, Some(threshold));
    Ok(k)
}

/// `pdf.obj{...}`: `table_obj`'s keys and checks.
fn table_obj(t: &Table) -> mlua::Result<i64> {
    let kind = match t.raw_get::<Value>("type")? {
        Value::Nil => return Err(mlua::Error::runtime("pdf.obj(): object \"type\" missing")),
        Value::String(s) => s.to_str()?.to_string(),
        _ => {
            return Err(mlua::Error::runtime(
                "pdf.obj(): object \"type\" must be string",
            ))
        }
    };
    if kind != "raw" && kind != "stream" {
        return Err(mlua::Error::runtime(format!(
            "pdf.obj(): \"{kind}\" is not a valid object type"
        )));
    }
    let immediate = match t.raw_get::<Value>("immediate")? {
        Value::Nil => false,
        Value::Boolean(b) => b,
        _ => {
            return Err(mlua::Error::runtime(
                "pdf.obj(): \"immediate\" must be boolean",
            ))
        }
    };
    let k = match t.raw_get::<Value>("objnum")? {
        Value::Nil => new_obj(),
        Value::Integer(n) => reuse(n, "obj")?,
        _ => {
            return Err(mlua::Error::runtime(
                "pdf.obj(): \"objnum\" must be integer",
            ))
        }
    };
    let attr =
        match t.raw_get::<Value>("attr")? {
            Value::Nil => None,
            v => {
                if kind != "stream" {
                    return Err(mlua::Error::runtime(
                        "pdf.obj(): \"attr\" key not allowed for non-stream object",
                    ));
                }
                Some(bytes(&v)?.ok_or_else(|| {
                    mlua::Error::runtime("pdf.obj(): object \"attr\" must be string")
                })?)
            }
        };
    let compress_level = match t.raw_get::<Value>("compresslevel")? {
        Value::Nil => None,
        Value::Integer(n) => {
            if kind == "raw" {
                return Err(mlua::Error::runtime(
                    "pdf.obj(): \"compresslevel\" key not allowed for raw object",
                ));
            }
            if n > 9 {
                return Err(mlua::Error::runtime(
                    "pdf.obj(): \"compresslevel\" must be <= 9",
                ));
            }
            if n < 0 {
                return Err(mlua::Error::runtime(
                    "pdf.obj(): \"compresslevel\" must be >= 0",
                ));
            }
            Some(n)
        }
        _ => {
            return Err(mlua::Error::runtime(
                "pdf.obj(): \"compresslevel\" must be integer",
            ))
        }
    };
    // Whether a raw object may go into an object stream.
    let threshold = match t.raw_get::<Value>("objcompression")? {
        Value::Nil => OBJSTM_ALWAYS,
        Value::Boolean(b) => {
            if kind == "stream" {
                return Err(mlua::Error::runtime(
                    "pdf.obj(): \"objcompression\" key not allowed for stream object",
                ));
            }
            if b {
                OBJSTM_ALWAYS
            } else {
                OBJSTM_NEVER
            }
        }
        _ => {
            if kind == "stream" {
                return Err(mlua::Error::runtime(
                    "pdf.obj(): \"objcompression\" key not allowed for stream object",
                ));
            }
            return Err(mlua::Error::runtime(
                "pdf.obj(): \"objcompression\" must be boolean",
            ));
        }
    };
    let threshold = if kind == "stream" {
        OBJSTM_NEVER
    } else {
        threshold
    };
    let (string, file) = (t.raw_get::<Value>("string")?, t.raw_get::<Value>("file")?);
    let (data, from_file) = match (&string, &file) {
        (Value::Nil, Value::Nil) => {
            return Err(mlua::Error::runtime(
                "pdf.obj(): no \"string\" or \"file\" given",
            ))
        }
        (Value::Nil, f) => (bytes(f)?, true),
        (s, Value::Nil) => (bytes(s)?, false),
        _ => {
            return Err(mlua::Error::runtime(
                "pdf.obj(): \"string\" and \"file\" must not be given together",
            ))
        }
    };
    let kind = match (kind.as_str(), from_file) {
        ("raw", true) => "file",
        ("stream", true) => "streamfile",
        (k, _) => k,
    };
    store(
        k,
        Obj {
            kind: kind.to_string(),
            data,
            attr,
            immediate,
            compress_level,
        },
        immediate.then_some(threshold),
    );
    Ok(k)
}

/// `orig_obj`: the positional form of `pdf.obj`.
fn orig_obj(args: &Variadic<Value>) -> mlua::Result<i64> {
    let mut first = 0;
    let k = match args.first() {
        Some(Value::Integer(n)) => {
            first = 1;
            reuse(*n, "obj")?
        }
        _ => new_obj(),
    };
    let rest = &args[first..];
    let (kind, data, attr) = match rest.len() {
        0 => {
            return Err(mlua::Error::runtime(
                "pdf.obj() needs at least one argument",
            ))
        }
        1 => (
            "raw".to_string(),
            bytes(&rest[0])?
                .ok_or_else(|| mlua::Error::runtime("pdf.obj() 1st argument must be string"))?,
            None,
        ),
        2 | 3 => {
            let Value::String(kind) = &rest[0] else {
                return Err(mlua::Error::runtime(
                    "pdf.obj() 1st argument must be string",
                ));
            };
            let kind = kind.to_str()?.to_string();
            let data = bytes(&rest[1])?
                .ok_or_else(|| mlua::Error::runtime("pdf.obj() 2nd argument must be string"))?;
            if kind == "file" && rest.len() == 3 {
                return Err(mlua::Error::runtime(
                    "pdf.obj() 3rd argument forbidden in file mode",
                ));
            }
            if !matches!(kind.as_str(), "file" | "stream" | "streamfile") {
                return Err(mlua::Error::runtime("pdf.obj() invalid argument"));
            }
            let attr = match rest.get(2) {
                Some(v) => Some(bytes(v)?.ok_or_else(|| {
                    mlua::Error::runtime("pdf.obj() 3rd argument must be string")
                })?),
                None => None,
            };
            (kind, data, attr)
        }
        _ => return Err(mlua::Error::runtime("pdf.obj() allows max. 3 arguments")),
    };
    store(
        k,
        Obj {
            kind,
            data: Some(data),
            attr,
            immediate: false,
            compress_level: None,
        },
        None,
    );
    Ok(k)
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
                if let Some(Value::String(s)) = args.last() {
                    let v = s.as_bytes().to_vec();
                    STATE.with(|st| st.borrow_mut().strings.insert(key, v));
                }
                Ok(())
            })?,
        )?;
        t.set(
            format!("get{key}"),
            lua.create_function(move |lua, ()| {
                match STATE.with(|st| st.borrow().strings.get(key).cloned()) {
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
        lua.create_function(|_, args: Variadic<Value>| {
            open_output();
            match &args[..] {
                [Value::Table(t)] => table_obj(t),
                _ => orig_obj(&args),
            }
        })?,
    )?;
    t.set(
        "immediateobj",
        lua.create_function(|_, args: Variadic<Value>| immediateobj(&args))?,
    )?;
    t.set(
        "reserveobj",
        lua.create_function(|_, args: Variadic<Value>| match &args[..] {
            [] => Ok(new_obj()),
            [Value::String(s)] if &*s.to_str()? == "annot" => Ok(STATE.with(|st| {
                let mut st = st.borrow_mut();
                let k = create(&mut st, "annot");
                st.last_annot = k;
                k
            })),
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
    let getter = |lua: &Lua, f: fn(&State) -> i64| {
        lua.create_function(move |_, ()| Ok(STATE.with(|s| f(&s.borrow()))))
    };
    t.set("getmaxobjnum", getter(lua, |s| s.table.len() as i64)?)?;
    t.set("maxobjnum", getter(lua, |s| s.table.len() as i64)?)?;
    t.set("getlastobj", getter(lua, |s| s.last_obj)?)?;
    t.set("getlastannot", getter(lua, |s| s.last_annot)?)?;
    t.set("getlastlink", lua.create_function(|_, ()| Ok(0))?)?;
    t.set("getretval", lua.create_function(|_, ()| Ok(0))?)?;
    t.set(
        "getnofobjects",
        lua.create_function(|_, ()| {
            Ok(STATE.with(|s| {
                let s = s.borrow();
                let written = s.table.iter().filter(|slot| slot.written).count();
                (written, s.table.len() - written)
            }))
        })?,
    )?;
    for name in ["getobjtype", "objtype"] {
        t.set(
            name,
            lua.create_function(|_, k: i64| {
                Ok(STATE.with(|s| {
                    let s = s.borrow();
                    (k > 0)
                        .then(|| s.table.get((k - 1) as usize).map(|slot| slot.kind))
                        .flatten()
                }))
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
    // `newcolorstack(init, mode, page)`: the stack's number, from 1 (0 is the
    // default colour stack luatex makes for itself).
    t.set(
        "newcolorstack",
        lua.create_function(
            |_, (init, mode, page): (String, Option<String>, Option<bool>)| {
                let mode = mode.unwrap_or_else(|| "origin".to_string());
                if !matches!(mode.as_str(), "origin" | "page" | "direct") {
                    return Err(mlua::Error::runtime(
                        "invalid literal mode in pdf.newcolorstack()",
                    ));
                }
                Ok(STATE.with(|s| {
                    let mut s = s.borrow_mut();
                    s.color_stacks.push((init, mode, page.unwrap_or(false)));
                    s.color_stacks.len() as i64
                }))
            },
        )?,
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
