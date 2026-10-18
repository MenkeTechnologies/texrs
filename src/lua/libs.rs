//! The Lua that LuaTeX's interpreter starts with, before any TeX library.
//!
//! `luainterpreter()` (luatexdir/lua/luastuff.c) builds the state in a fixed
//! order: Lua's own libraries and the extra C libraries from one table
//! (`lualibs[]`: `_G package table io os string math debug lpeg bit32 utf8
//! coroutine unicode zip md5 sha2 lfs ffi`), then `open_oslibext` and
//! `open_strlibext`, then the TeX libraries, and last `luatex-core.lua`, which
//! fills gaps and applies the file-system policy. This module is everything in
//! that list that is not about TeX:
//!
//! - **The C libraries**, compiled from LuaTeX's own copies by `build.rs`:
//!   `lpeg` (LPeg 1.0.1), `unicode` (slnunicode), `lfs` (LuaFileSystem) and
//!   `bit32` (Lua 5.3.6's lbitlib.c), plus the LUA_COMPAT_5_2/LUA_COMPAT_MODULE
//!   functions TeX Live's Lua is built with (`module`, `package.seeall`,
//!   `math.pow` and the rest, `__ipairs`).
//! - **`md5` and `sha2`**, in Rust: their answers are RFC 1321's and FIPS
//!   180-4's, and `md5.exor`, `md5.crypt` and `md5.decrypt` are ports of
//!   luamd5/md5lib.c's.
//! - **`string`'s and `os`'s additions**, ports of lstrlibext.c and
//!   loslibext.c.
//! - **`luatex-core.lua`'s gap-filling**: `lfs.mkdirp`, `lfs.isfile`,
//!   `lfs.isdir`, `lfs.shortname`, `lfs.readlink`, `md5.sumhexa`,
//!   `md5.sumHEXA`, `unpack`, `loadstring`, `package.loaders`, and the debug
//!   library cut down to `traceback`.
//!
//! What is NOT here, and why:
//!
//! - **`zlib` and `gzip`**, luazlib's lzlib.c and lgzip.c over zlib itself;
//!   lualibs reads zip archives with `zlib.decompress` and indexes it at load.
//! - **`zip`, `ffi`, `socket`, `mime`, `fio`, `sio`.** Not
//!   opened. Every package this engine is measured against tests for them
//!   before use -- luaotfload's `fontloader-util-fil.lua` reads `if fio and
//!   fio.readcardinal1 then` and otherwise uses its own Lua readers -- so an
//!   absent library takes the package's own fallback, where a half-built one
//!   would take the fast path into a function that is not there.
//! - **The shell.** texrs runs no shell commands, so this state is LuaTeX's
//!   with shell escape DISABLED (`luatex -no-shell-escape`): `os.execute()`
//!   answers 0, `os.execute(cmd)` and `os.kpsepopen` refuse with LuaTeX's own
//!   messages, and `io.popen` is `os.kpsepopen`, as `luatex-core.lua` makes it.

use mlua::{Function, Lua, Table, Value, Variadic};
// Named so the static zlib that `lzlib.c` and `lgzip.c` call into is linked:
// nothing in Rust calls it, and an unreferenced crate is not.
use libz_sys as _;
use std::cell::Cell;
use std::os::raw::c_int;
use std::rc::Rc;

extern "C-unwind" {
    fn luaopen_lpeg(l: *mut mlua::ffi::lua_State) -> c_int;
    fn luaopen_bit32(l: *mut mlua::ffi::lua_State) -> c_int;
    fn luaopen_unicode(l: *mut mlua::ffi::lua_State) -> c_int;
    fn luaopen_lfs(l: *mut mlua::ffi::lua_State) -> c_int;
    fn luaopen_zlib(l: *mut mlua::ffi::lua_State) -> c_int;
    fn luaopen_gzip(l: *mut mlua::ffi::lua_State) -> c_int;
    fn texrs_open_compat(l: *mut mlua::ffi::lua_State);
}

/// `status.shell_escape`: 0 is "disabled" (lstatslib.c `shell_escape_state`).
pub(super) const SHELL_ESCAPE: i64 = 0;

/// A new state, with everything luastuff.c opens before the TeX libraries.
///
/// `unsafe_new_with` rather than `Lua::new`: mlua's safe mode refuses binary
/// chunks and the `debug` library, and LuaTeX has both (`\luabytecode` and
/// `lua.setbytecode` load binary chunks; `debug.traceback` survives
/// `luatex-core.lua`). Nothing is loaded from a C module either way -- the C
/// searchers are removed below, as `luatex-core.lua` removes them.
pub(super) fn new_state() -> mlua::Result<Lua> {
    // SAFETY: the state is used only through mlua's API and the C libraries
    // compiled by build.rs against the same headers; no C module is loaded
    // from disk (searchers 3 and 4 are removed in `core`).
    let lua = unsafe { Lua::unsafe_new_with(mlua::StdLib::ALL, mlua::LuaOptions::default()) };
    open_c(&lua, "lpeg", luaopen_lpeg)?;
    open_c(&lua, "bit32", luaopen_bit32)?;
    open_c(&lua, "unicode", luaopen_unicode)?;
    open_md5(&lua)?;
    open_sha2(&lua)?;
    open_c(&lua, "lfs", luaopen_lfs)?;
    // SAFETY: `texrs_open_compat` only registers C functions into tables the
    // standard libraries have already created.
    unsafe { lua.exec_raw::<()>((), |l| texrs_open_compat(l))? };
    open_oslibext(&lua)?;
    open_strlibext(&lua)?;
    errors_read_as_strings(&lua)?;
    // `luaopen_zlib(L); luaopen_gzip(L);`, called bare: each registers its
    // own global and `package.loaded` entry through `luaL_openlib`.
    for opener in [
        luaopen_zlib as unsafe extern "C-unwind" fn(_) -> _,
        luaopen_gzip,
    ] {
        // SAFETY: as for `open_c`.
        let f = unsafe { lua.create_c_function(opener)? };
        f.call::<()>(())?;
    }
    Ok(lua)
}

/// Make an error raised by a Rust library function read as the string LuaTeX's
/// C function would have raised.
///
/// A C library function fails with `luaL_error`, whose value is a STRING --
/// `"file.lua:12: bad argument..."` -- and Lua code relies on that: it
/// concatenates the message, matches it, prints it. mlua raises a Rust
/// function's error as a userdata of its own instead, whose `tostring` is
/// `runtime error: ...` followed by a traceback. Every such userdata shares one
/// metatable, so that metatable is given what makes the value behave as the
/// string: `__tostring` answers the message alone, `__concat` concatenates
/// it, and `__index` answers the string methods over it, so `err:match(...)`
/// works as on a string. `type(err)` still says `userdata`, which is the one
/// difference a chunk can still see.
fn errors_read_as_strings(lua: &Lua) -> mlua::Result<()> {
    let raise = lua.create_function(|_, ()| Err::<(), _>(mlua::Error::runtime("probe")))?;
    lua.load(ERRORS_AS_STRINGS)
        .set_name("=errors")
        .call::<()>(raise)
}

const ERRORS_AS_STRINGS: &str = r#"
local raise = ...
local _, sample = pcall(raise)
local mt = debug.getmetatable(sample)
local original = mt.__tostring
local function text(e)
    local s = original(e)
    s = s:gsub("^runtime error: ", ""):gsub("^callback error: ", "")
    s = s:gsub("\nstack traceback:.*$", "")
    return s
end
mt.__tostring = text
mt.__concat = function(a, b)
    if type(a) == "userdata" then a = text(a) end
    if type(b) == "userdata" then b = text(b) end
    return a .. b
end
mt.__index = function(e, k)
    local f = string[k]
    if f then return function(self, ...) return f(text(self), ...) end end
end
"#;

/// `Luas_open(name, luaopen_lib)` in its Lua 5.3 spelling,
/// `luaL_requiref(L, name, luaopen_lib, 1)`: run the opener with the name,
/// keep what it returns as `package.loaded[name]` and as the global.
fn open_c(
    lua: &Lua,
    name: &str,
    opener: unsafe extern "C-unwind" fn(*mut mlua::ffi::lua_State) -> c_int,
) -> mlua::Result<()> {
    // SAFETY: each opener is a `luaopen_*` from build.rs, compiled against the
    // headers this Lua was built from.
    let f = unsafe { lua.create_c_function(opener)? };
    let module: Value = f.call(name)?;
    let loaded: Table = lua.globals().get::<Table>("package")?.get("loaded")?;
    loaded.set(name, module.clone())?;
    lua.globals().set(name, module)
}

/// Register a table as a library: `package.loaded[name]` and the global.
fn register(lua: &Lua, name: &str, t: &Table) -> mlua::Result<()> {
    let loaded: Table = lua.globals().get::<Table>("package")?.get("loaded")?;
    loaded.set(name, t.clone())?;
    lua.globals().set(name, t.clone())
}

// ── md5 and sha2 ────────────────────────────────────────────────────────

fn md5_digest(data: &[u8]) -> [u8; 16] {
    use md5::Digest;
    md5::Md5::digest(data).into()
}

/// luamd5/md5lib.c: `sum`, `exor`, `crypt`, `decrypt`. (`sumhexa` and
/// `sumHEXA` are `luatex-core.lua`'s, added in `core`.)
fn open_md5(lua: &Lua) -> mlua::Result<()> {
    let t = lua.create_table()?;
    t.set(
        "sum",
        lua.create_function(|lua, s: mlua::LuaString| {
            lua.create_string(md5_digest(&s.as_bytes()))
        })?,
    )?;
    // "X-Or. Does a bit-a-bit exclusive-or of two strings" -- with
    // `luaL_argcheck(L, l1 == l2, 2, "lengths must be equal")`.
    t.set(
        "exor",
        lua.create_function(|lua, (a, b): (mlua::LuaString, mlua::LuaString)| {
            let (a, b) = (a.as_bytes(), b.as_bytes());
            if a.len() != b.len() {
                return Err(mlua::Error::runtime(
                    "bad argument #2 to 'exor' (lengths must be equal)",
                ));
            }
            let x: Vec<u8> = a.iter().zip(b.iter()).map(|(p, q)| p ^ q).collect();
            lua.create_string(x)
        })?,
    )?;
    t.set(
        "crypt",
        lua.create_function(
            |lua, (msg, key, seed): (mlua::LuaString, mlua::LuaString, Option<mlua::LuaString>)| {
                // `checkseed`: no seed means the bytes of `time(NULL)`.
                let seed = match seed {
                    Some(s) => s.as_bytes().to_vec(),
                    None => {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs() as libc::time_t)
                            .unwrap_or(0);
                        now.to_ne_bytes().to_vec()
                    }
                };
                if seed.len() > BLOCKSIZE {
                    return Err(mlua::Error::runtime(format!(
                        "seed too long (> {BLOCKSIZE})"
                    )));
                }
                let mut out = vec![seed.len() as u8];
                out.extend_from_slice(&seed);
                let mut block = init_block(&seed, &key.as_bytes())?;
                out.extend(code_stream(&msg.as_bytes(), &mut block, false));
                lua.create_string(out)
            },
        )?,
    )?;
    t.set(
        "decrypt",
        lua.create_function(|lua, (cypher, key): (mlua::LuaString, mlua::LuaString)| {
            let c = cypher.as_bytes();
            let lseed = c.first().copied().unwrap_or(0) as usize;
            if c.len() < lseed + 1 || lseed > BLOCKSIZE {
                return Err(mlua::Error::runtime(
                    "bad argument #1 to 'decrypt' (invalid cyphered string)",
                ));
            }
            let seed = &c[1..=lseed];
            let mut block = init_block(seed, &key.as_bytes())?;
            lua.create_string(code_stream(&c[lseed + 1..], &mut block, true))
        })?,
    )?;
    register(lua, "md5", &t)
}

/// md5lib.c's `BLOCKSIZE` and `MAXKEY`.
const BLOCKSIZE: usize = 16;
const MAXKEY: usize = 256;

/// `initblock`: the seed zero-padded to a block, then the key.
fn init_block(seed: &[u8], key: &[u8]) -> mlua::Result<Vec<u8>> {
    if key.len() > MAXKEY {
        return Err(mlua::Error::runtime(format!("key too long (> {MAXKEY})")));
    }
    let mut block = vec![0u8; BLOCKSIZE];
    block[..seed.len()].copy_from_slice(seed);
    block.extend_from_slice(key);
    Ok(block)
}

/// `codestream` and `decodestream`: MD5 in cipher-feedback mode. The two differ
/// only in which bytes feed back into the seed -- the cyphertext either way,
/// which is what `crypt` produces and `decrypt` consumes.
fn code_stream(input: &[u8], block: &mut [u8], decoding: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    for chunk in input.chunks(BLOCKSIZE) {
        let mut code = md5_digest(block);
        for (c, m) in code.iter_mut().zip(chunk) {
            *c ^= m;
        }
        let n = chunk.len();
        out.extend_from_slice(&code[..n]);
        let feedback = match decoding {
            true => chunk,
            false => &code[..n],
        };
        block[..n].copy_from_slice(feedback);
    }
    out
}

/// md5lib.c's `sha2lib`: `digest256`, `digest384`, `digest512`, each the raw
/// digest of a string argument and NO VALUE for anything else (`return 0`).
fn open_sha2(lua: &Lua) -> mlua::Result<()> {
    let t = lua.create_table()?;
    fn digest_fn<D: sha2::Digest>(lua: &Lua) -> mlua::Result<Function> {
        lua.create_function(|lua, v: Value| match v {
            Value::String(s) => Ok(Variadic::from_iter([Value::String(
                lua.create_string(D::digest(&*s.as_bytes()))?,
            )])),
            _ => Ok(Variadic::new()),
        })
    }
    t.set("digest256", digest_fn::<sha2::Sha256>(lua)?)?;
    t.set("digest384", digest_fn::<sha2::Sha384>(lua)?)?;
    t.set("digest512", digest_fn::<sha2::Sha512>(lua)?)?;
    register(lua, "sha2", &t)
}

// ── string: lstrlibext.c ────────────────────────────────────────────────

/// An iterator over a string, as lstrlibext.c builds them: a closure whose
/// two upvalues are the string and the byte offset reached so far.
fn iterator(
    lua: &Lua,
    s: mlua::LuaString,
    step: impl Fn(&Lua, &[u8], usize) -> mlua::Result<Option<(usize, Variadic<Value>)>> + 'static,
) -> mlua::Result<Function> {
    let at = Rc::new(Cell::new(0usize));
    lua.create_function(move |lua, _: Variadic<Value>| {
        let bytes = s.as_bytes();
        match step(lua, &bytes, at.get())? {
            Some((next, out)) => {
                at.set(next);
                Ok(out)
            }
            None => Ok(Variadic::new()),
        }
    })
}

fn int(n: i64) -> Value {
    Value::Integer(n)
}

/// `utf_failed`: U+FFFD as three bytes, and the offset to resume at.
fn utf_failed(lua: &Lua, resume: usize) -> mlua::Result<Option<(usize, Variadic<Value>)>> {
    let fffd = lua.create_string([0xEF, 0xBF, 0xBD])?;
    Ok(Some((resume, Variadic::from_iter([Value::String(fffd)]))))
}

/// `utfvalues_aux`'s and `str_utfvalue`'s decoder: the code point at `ind` and
/// how many bytes it took, `None` when the lead byte promised more
/// continuation bytes than follow. A continuation byte is tested only for
/// `>= 0x80`, exactly as the C does; it is not checked to be `< 0xC0`.
fn utf_decode(s: &[u8], ind: usize) -> (Option<u32>, usize) {
    let ls = s.len();
    let i = s[ind] as u32;
    let cont = |k: usize| s[ind + k] >= 0x80;
    let low = |k: usize| (s[ind + k].wrapping_sub(128)) as u32;
    if i < 0x80 {
        (Some(i), 1)
    } else if i >= 0xF0 {
        match ind + 3 < ls && cont(1) && cont(2) && cont(3) {
            true => (
                Some((((i - 0xF0) * 64 + low(1)) * 64 + low(2)) * 64 + low(3)),
                4,
            ),
            false => (None, 4),
        }
    } else if i >= 0xE0 {
        match ind + 2 < ls && cont(1) && cont(2) {
            true => (Some(((i - 0xE0) * 64 + low(1)) * 64 + low(2)), 3),
            false => (None, 3),
        }
    } else if i >= 0xC0 {
        match ind + 1 < ls && cont(1) {
            true => (Some((i - 0xC0) * 64 + low(1)), 2),
            false => (None, 2),
        }
    } else {
        (None, 1)
    }
}

/// `utf8_enco`: a code point as UTF-8, silently nothing above U+10FFFF.
fn utf8_encode(out: &mut Vec<u8>, c: u32) {
    if c > 0x10FFFF {
        return;
    }
    if c < 0x80 {
        out.push(c as u8);
        return;
    }
    if c < 0x800 {
        out.push((0xC0 | (c >> 6)) as u8);
    } else {
        if c < 0x10000 {
            out.push((0xE0 | (c >> 12)) as u8);
        } else {
            out.push((0xF0 | (c >> 18)) as u8);
            out.push((0x80 | (0x3F & (c >> 12))) as u8);
        }
        out.push((0x80 | (0x3F & (c >> 6))) as u8);
    }
    out.push((0x80 | (0x3F & c)) as u8);
}

/// `str_split`, which is `string.explode(s [, joiner])`.
///
/// The joiner's first byte is the separator, and a `+` after it means runs of
/// it count as one and leading ones are skipped. The default is `" +"`.
fn explode(s: &[u8], joiner: &[u8]) -> Vec<Vec<u8>> {
    if s.is_empty() {
        return vec![s.to_vec()];
    }
    let Some(&sep) = joiner.first() else {
        return s.iter().map(|&b| vec![b]).collect();
    };
    let mult = joiner.get(1) == Some(&b'+');
    let mut p = s;
    if mult {
        while p.first() == Some(&sep) {
            p = &p[1..];
        }
    }
    let l = p.len();
    let mut out = Vec::new();
    let mut q = 0usize;
    let mut i = 0usize;
    while i < l {
        if p[i] == sep {
            out.push(p[q..i].to_vec());
            if mult {
                while i + 1 < l && p[i + 1] == sep {
                    i += 1;
                }
            }
            q = i + 1;
        }
        i += 1;
    }
    if mult && q == l {
        return out;
    }
    if q <= l {
        out.push(p[q..].to_vec());
    }
    out
}

fn open_strlibext(lua: &Lua) -> mlua::Result<()> {
    let string: Table = lua.globals().get("string")?;

    string.set(
        "bytes",
        lua.create_function(|lua, s: mlua::LuaString| {
            iterator(lua, s, |_, s, ind| {
                Ok((ind < s.len()).then(|| (ind + 1, Variadic::from_iter([int(s[ind] as i64)]))))
            })
        })?,
    )?;
    // Two bytes at a time; an odd length ends with a nil second byte.
    string.set(
        "bytepairs",
        lua.create_function(|lua, s: mlua::LuaString| {
            iterator(lua, s, |_, s, ind| {
                if ind >= s.len() {
                    return Ok(None);
                }
                let two = ind + 1 < s.len();
                let second = match two {
                    true => int(s[ind + 1] as i64),
                    false => Value::Nil,
                };
                let next = if two { ind + 2 } else { ind + 1 };
                Ok(Some((
                    next,
                    Variadic::from_iter([int(s[ind] as i64), second]),
                )))
            })
        })?,
    )?;
    string.set(
        "characters",
        lua.create_function(|lua, s: mlua::LuaString| {
            iterator(lua, s, |lua, s, ind| {
                if ind >= s.len() {
                    return Ok(None);
                }
                let c = lua.create_string([s[ind]])?;
                Ok(Some((ind + 1, Variadic::from_iter([Value::String(c)]))))
            })
        })?,
    )?;
    // Two characters at a time; an odd length ends with an empty string.
    string.set(
        "characterpairs",
        lua.create_function(|lua, s: mlua::LuaString| {
            iterator(lua, s, |lua, s, ind| {
                if ind >= s.len() {
                    return Ok(None);
                }
                let two = ind + 1 < s.len();
                let first = lua.create_string([s[ind]])?;
                let second = match two {
                    true => lua.create_string([s[ind + 1]])?,
                    false => lua.create_string("")?,
                };
                let next = if two { ind + 2 } else { ind + 1 };
                Ok(Some((
                    next,
                    Variadic::from_iter([Value::String(first), Value::String(second)]),
                )))
            })
        })?,
    )?;
    // `utfcharacters_aux`: one UTF-8 sequence at a time, by the lead byte's
    // mask; a bad sequence is U+FFFD, resuming at the first byte that did not
    // fit (or at the end, when the sequence ran past it).
    string.set(
        "utfcharacters",
        lua.create_function(|lua, s: mlua::LuaString| {
            iterator(lua, s, |lua, s, ind| {
                const MASK: [u8; 4] = [0x80, 0xE0, 0xF0, 0xF8];
                const MEQU: [u8; 4] = [0x00, 0xC0, 0xE0, 0xF0];
                let ls = s.len();
                if ind >= ls {
                    return Ok(None);
                }
                let c = s[ind];
                for j in 0..4 {
                    if c & MASK[j] == MEQU[j] {
                        if ind + 1 + j > ls {
                            return utf_failed(lua, ls);
                        }
                        for k in 1..=j {
                            if s[ind + k] & 0xC0 != 0x80 {
                                return utf_failed(lua, ind + k);
                            }
                        }
                        let ch = lua.create_string(&s[ind..ind + 1 + j])?;
                        return Ok(Some((
                            ind + 1 + j,
                            Variadic::from_iter([Value::String(ch)]),
                        )));
                    }
                }
                utf_failed(lua, ind + 1)
            })
        })?,
    )?;
    // `utfvalues_aux`: a bad sequence answers 0xFFFD and advances ONE byte.
    string.set(
        "utfvalues",
        lua.create_function(|lua, s: mlua::LuaString| {
            iterator(lua, s, |_, s, ind| {
                if ind >= s.len() {
                    return Ok(None);
                }
                let (v, n) = match utf_decode(s, ind) {
                    (Some(v), n) => (v, n),
                    (None, _) => (0xFFFD, 1),
                };
                Ok(Some((ind + n, Variadic::from_iter([int(v as i64)]))))
            })
        })?,
    )?;
    string.set(
        "bytetable",
        lua.create_function(|lua, s: mlua::LuaString| {
            lua.create_sequence_from(s.as_bytes().iter().map(|&b| b as i64))
        })?,
    )?;
    string.set(
        "explode",
        lua.create_function(
            |lua, (s, joiner): (mlua::LuaString, Option<mlua::LuaString>)| {
                let joiner = joiner
                    .map(|j| j.as_bytes().to_vec())
                    .unwrap_or_else(|| b" +".to_vec());
                let parts = explode(&s.as_bytes(), &joiner);
                let t = lua.create_table_with_capacity(parts.len(), 0)?;
                for (i, p) in parts.into_iter().enumerate() {
                    t.raw_set(i + 1, lua.create_string(p)?)?;
                }
                Ok(t)
            },
        )?,
    )?;
    // `str_character`: every argument as UTF-8, through `lua_tointeger`, so a
    // non-number is code point 0.
    string.set(
        "utfcharacter",
        lua.create_function(|lua, args: Variadic<Value>| {
            let mut out = Vec::new();
            for v in args.iter() {
                let n = match v {
                    Value::Integer(n) => *n,
                    Value::Number(f) if f.fract() == 0.0 => *f as i64,
                    Value::String(s) => s
                        .to_str()
                        .ok()
                        .and_then(|s| s.trim().parse().ok())
                        .unwrap_or(0),
                    _ => 0,
                };
                utf8_encode(&mut out, n as u32);
            }
            lua.create_string(out)
        })?,
    )?;
    // `str_utfvalue`: every code point, as separate results. A bad sequence
    // contributes NOTHING and skips the length its lead byte promised.
    string.set(
        "utfvalue",
        lua.create_function(|_, s: mlua::LuaString| {
            let s = s.as_bytes();
            let mut out = Variadic::new();
            let mut ind = 0;
            while ind < s.len() {
                let (v, n) = utf_decode(&s, ind);
                if let Some(v) = v {
                    out.push(int(v as i64));
                }
                ind += n;
            }
            Ok(out)
        })?,
    )?;
    // `str_utflength`: lead bytes counted by their promised length, a stray
    // continuation byte counting as one character ("stupid recovery").
    string.set(
        "utflength",
        lua.create_function(|_, s: mlua::LuaString| {
            let s = s.as_bytes();
            let (mut ind, mut num) = (0usize, 0i64);
            while ind < s.len() {
                let i = s[ind];
                ind += match i {
                    0..=0x7F => 1,
                    0xF0..=0xFF => 4,
                    0xE0..=0xEF => 3,
                    0xC0..=0xDF => 2,
                    _ => 1,
                };
                num += 1;
            }
            Ok(num)
        })?,
    )?;
    Ok(())
}

// ── os: loslibext.c ─────────────────────────────────────────────────────

/// loslibext.c's `OS_PLATNAME` for the platforms texrs builds on.
fn os_name() -> &'static str {
    if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "freebsd") {
        "freebsd"
    } else if cfg!(target_os = "openbsd") {
        "openbsd"
    } else if cfg!(target_os = "macos") {
        "macosx"
    } else if cfg!(windows) {
        "windows"
    } else {
        "generic"
    }
}

fn open_oslibext(lua: &Lua) -> mlua::Result<()> {
    let os: Table = lua.globals().get("os")?;
    // `find_env`: the environment as a table, read once.
    let env = lua.create_table()?;
    for (k, v) in std::env::vars_os() {
        env.set(
            lua.create_string(k.as_encoded_bytes())?,
            lua.create_string(v.as_encoded_bytes())?,
        )?;
    }
    os.set("env", env)?;
    // `ex_sleep(interval [, units])`: `usleep(1e6 * interval / units)`.
    os.set(
        "sleep",
        lua.create_function(|_, (interval, units): (f64, Option<f64>)| {
            let us = 1e6 * interval / units.unwrap_or(1.0);
            if us > 0.0 {
                std::thread::sleep(std::time::Duration::from_micros(us as u64));
            }
            Ok(())
        })?,
    )?;
    os.set("type", if cfg!(windows) { "windows" } else { "unix" })?;
    os.set("name", os_name())?;
    os.set(
        "uname",
        lua.create_function(|lua, ()| {
            // SAFETY: `uname` fills the zeroed struct it is given.
            let mut u: libc::utsname = unsafe { std::mem::zeroed() };
            if unsafe { libc::uname(&mut u) } < 0 {
                return Ok(Value::Nil);
            }
            let field = |f: &[libc::c_char]| {
                // SAFETY: utsname fields are NUL-terminated C strings.
                unsafe { std::ffi::CStr::from_ptr(f.as_ptr()) }
                    .to_string_lossy()
                    .into_owned()
            };
            let t = lua.create_table()?;
            t.set("sysname", field(&u.sysname))?;
            t.set("machine", field(&u.machine))?;
            t.set("release", field(&u.release))?;
            t.set("version", field(&u.version))?;
            t.set("nodename", field(&u.nodename))?;
            Ok(Value::Table(t))
        })?,
    )?;
    os.set(
        "times",
        lua.create_function(|lua, ()| {
            // SAFETY: `times` fills the zeroed struct it is given.
            let mut r: libc::tms = unsafe { std::mem::zeroed() };
            unsafe { libc::times(&mut r) };
            let tck = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
            let t = lua.create_table()?;
            t.set("utime", r.tms_utime as f64 / tck)?;
            t.set("stime", r.tms_stime as f64 / tck)?;
            t.set("cutime", r.tms_cutime as f64 / tck)?;
            t.set("cstime", r.tms_cstime as f64 / tck)?;
            Ok(t)
        })?,
    )?;
    let now = || {
        let d = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        d.as_secs() as f64 + d.subsec_micros() as f64 / 1_000_000.0
    };
    os.set("gettimeofday", lua.create_function(move |_, ()| Ok(now()))?)?;
    os.set(
        "socketgettime",
        lua.create_function(move |_, ()| Ok(now()))?,
    )?;
    os.set(
        "socketsleep",
        lua.create_function(|_, n: f64| {
            let n = n.clamp(0.0, i32::MAX as f64);
            std::thread::sleep(std::time::Duration::from_secs_f64(n));
            Ok(())
        })?,
    )?;
    // `os_setenv(key [, value])`: no value unsets.
    os.set(
        "setenv",
        lua.create_function(|_, (key, val): (Option<String>, Option<String>)| {
            if let Some(key) = key {
                match val {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
            Ok(true)
        })?,
    )?;
    // `os_tmpdir([template])`: the template must end in six X's.
    os.set(
        "tmpdir",
        lua.create_function(|lua, tmpl: Option<String>| {
            let tmpl = tmpl.unwrap_or_else(|| "luatex.XXXXXX".to_string());
            if tmpl.len() < 6 || !tmpl.ends_with("XXXXXX") {
                return Ok((
                    Value::Nil,
                    Some("Invalid argument to os.tmpdir()".to_string()),
                ));
            }
            let c = std::ffi::CString::new(tmpl).map_err(mlua::Error::external)?;
            let raw = c.into_raw();
            // SAFETY: `mkdtemp` rewrites the template it owns in place; it is
            // taken back into a CString straight after.
            let made = unsafe { libc::mkdtemp(raw) };
            let back = unsafe { std::ffi::CString::from_raw(raw) };
            if made.is_null() {
                let e = std::io::Error::last_os_error();
                return Ok((Value::Nil, Some(e.to_string())));
            }
            Ok((Value::String(lua.create_string(back.as_bytes())?), None))
        })?,
    )?;
    // Shell escape is disabled (see the module docs). `os_execute(nil)` is
    // "pretend we are \pdfshellescape", which answers 0.
    os.set(
        "execute",
        lua.create_function(|lua, cmd: Option<mlua::LuaString>| match cmd {
            None => Ok(Variadic::from_iter([Value::Integer(SHELL_ESCAPE)])),
            Some(_) => Ok(Variadic::from_iter([
                Value::Nil,
                Value::String(lua.create_string("All command execution disabled.")?),
            ])),
        })?,
    )?;
    os.set(
        "kpsepopen",
        lua.create_function(|_, _: Variadic<Value>| {
            Ok((Value::Nil, "all command execution is disabled"))
        })?,
    )?;
    for name in ["exec", "spawn"] {
        os.set(
            name,
            lua.create_function(|_, _: Variadic<Value>| {
                Ok((Value::Nil, "All command execution disabled."))
            })?,
        )?;
    }
    // `prepare_cmdline`: `os.selfdir` is the directory of the executable.
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_string_lossy().into_owned()))
    {
        os.set("selfdir", dir)?;
    }
    Ok(())
}

/// `luatex-core.lua`, the part that is not about permissions.
///
/// Run after the TeX libraries are in place, as `load_luatex_core_lua` is
/// (luainit.c), because it reads `status`. The policy half -- `io.open`
/// and `lfs` wrapped in `kpse.in_name_ok_silent_extended` checks -- is in
/// `super::kpse`, which owns those checks.
pub(super) fn core(lua: &Lua) -> mlua::Result<()> {
    lua.load(CORE).set_name("=luatex-core").exec()
}

/// The statements of `luatex-core.lua` (LuaTeX 1.24, LUATEXCOREVERSION 1.255)
/// that fill gaps, in its order. The `bit32` fallbacks are not here: `bit32`
/// is always the C library in this state.
const CORE: &str = r#"
LUATEXCOREVERSION = 1.255
local gmatch = string.gmatch
function lfs.mkdirp(path)
    local full = ""
    local r1, r2, r3
    for sub in gmatch(path, "(/*[^\\/]+)") do
        full = full .. sub
        r1, r2, r3 = lfs.mkdir(full)
    end
    return r1, r2, r3
end
-- The debug library is enabled only when shell escape permits everything
-- (luainit.c), so here it is cut down to traceback.
for k, _ in pairs(package.loaded.debug) do
    if not (k == 'traceback') then package.loaded.debug[k] = nil end
end
for k, v in pairs(debug) do
    if not (k == 'traceback') then debug[k] = nil end
end
-- shell_escape ~= 1: no C modules, no loadlib.
package.loadlib = function() end
package.searchers[4] = nil
package.searchers[3] = nil
if md5 then
    local sum, gsub, format, byte = md5.sum, string.gsub, string.format, string.byte
    if not md5.sumhexa then
        function md5.sumhexa(k)
            return (gsub(sum(k), ".", function(c) return format("%02x", byte(c)) end))
        end
    end
    if not md5.sumHEXA then
        function md5.sumHEXA(k)
            return (gsub(sum(k), ".", function(c) return format("%02X", byte(c)) end))
        end
    end
end
if not unpack then unpack = table.unpack end
if not package.loaders then package.loaders = package.searchers end
if not loadstring then loadstring = load end
do
    local loaded = package.loaded
    if not loaded.lfs then loaded.lfs = lfs end
end
do
    local lfsattributes     = lfs.attributes
    local symlinkattributes = lfs.symlinkattributes
    if not lfs.isfile then
        function lfs.isfile(name)
            local m = lfsattributes(name, "mode")
            return m == "file" or m == "link"
        end
    end
    if not lfs.isdir then
        function lfs.isdir(name)
            local m = lfsattributes(name, "mode")
            return m == "directory"
        end
    end
    if not lfs.shortname then
        function lfs.shortname(name) return name end
    end
    if not lfs.readlink then
        function lfs.readlink(name)
            return symlinkattributes(name, "target") or nil
        end
    end
end
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explode_is_str_split() {
        let parts = |s: &str, j: &str| {
            explode(s.as_bytes(), j.as_bytes())
                .into_iter()
                .map(|p| String::from_utf8(p).unwrap())
                .collect::<Vec<_>>()
        };
        // The default `" +"`: runs collapse, leading separators go, and a
        // trailing one leaves nothing behind.
        assert_eq!(parts("  a  b c ", " +"), ["a", "b", "c"]);
        // A single separator keeps the empty fields, the last one included.
        assert_eq!(parts("a,,b,", ","), ["a", "", "b", ""]);
        assert_eq!(parts("", ","), [""]);
        assert_eq!(parts("abc", ""), ["a", "b", "c"]);
    }

    #[test]
    fn a_bad_utf8_sequence_decodes_as_lstrlibext_does() {
        // A lead byte promising three bytes with only one following.
        assert_eq!(utf_decode(&[0xE2, 0x82], 0), (None, 3));
        assert_eq!(utf_decode("€".as_bytes(), 0), (Some(0x20AC), 3));
        assert_eq!(utf_decode(&[0x80], 0), (None, 1));
    }

    #[test]
    fn crypt_and_decrypt_are_inverses_under_a_fixed_seed() {
        let mut block = init_block(b"seed", b"key").unwrap();
        let c = code_stream(b"the quick brown fox jumps", &mut block, false);
        let mut block = init_block(b"seed", b"key").unwrap();
        assert_eq!(
            code_stream(&c, &mut block, true),
            b"the quick brown fox jumps"
        );
    }
}
