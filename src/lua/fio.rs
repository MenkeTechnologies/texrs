//! `fio` and `sio`: LuaTeX's binary readers (luatexdir/lua/liolibext.c),
//! over an open Lua file and over a string.
//!
//! The luaotfload font loader and lualibs read font files with these when
//! they are there, and `luatex-core.lua` takes `fio.readline` for its
//! `io.lines`. Each function is liolibext.c's, quirks included, because the
//! answers are what font code is written against:
//!
//! - a multi-byte read is `nil` only when the LAST byte read is EOF
//!   (`getc` keeps answering EOF once it has, so that is the same as "any");
//! - `readcardinal4`, `readcardinaltable(..., 4)`, `readfixed4` and
//!   `read2dot14` compute in a C `int`, so a value past 2^31 wraps negative,
//!   where the `integer` readers compute in `lua_Integer` and do not;
//! - `readfixed2` is unsigned (`(n>>8) + (n&0xff)/256`), and `sio.readfixed2`
//!   wants four bytes left where it reads two;
//! - a string position is 1-based (`p = luaL_checkinteger - 1`).
//!
//! The functions are C functions on the raw API, as the file readers have to
//! be: a Lua file is a `luaL_Stream` userdata (`LUA_FILEHANDLE`), whose
//! `FILE *` they read with stdio.

use mlua::ffi::{self, lua_State};
use mlua::Lua;
use std::os::raw::c_int;

/// `luaL_Stream` (lauxlib.h): what a Lua file handle's userdata holds.
#[repr(C)]
struct Stream {
    f: *mut libc::FILE,
    /// `NULL` once the file is closed.
    closef: Option<ffi::lua_CFunction>,
}

const EOF: c_int = -1;

/// `tofile`: the open file argument 1 names, or `luaL_error`.
unsafe fn tofile(l: *mut lua_State) -> *mut libc::FILE {
    let p = ffi::luaL_checkudata(l, 1, c"FILE*".as_ptr()) as *mut Stream;
    if (*p).closef.is_none() {
        ffi::luaL_error(l, c"attempt to use a closed file".as_ptr());
    }
    (*p).f
}

/// `n` bytes read with `getc`, most significant first, or `None` when the
/// last one read was EOF.
unsafe fn read_file(f: *mut libc::FILE, n: usize, little_endian: bool) -> Option<[c_int; 4]> {
    let mut bytes = [0; 4];
    for b in bytes.iter_mut().take(n) {
        *b = libc::fgetc(f);
    }
    if bytes[n - 1] == EOF {
        return None;
    }
    if little_endian {
        bytes[..n].reverse();
    }
    Some(bytes)
}

/// `n` bytes of a string from position `p` (0-based), most significant
/// first, or `None` when `p + n - 1` is past the end.
fn read_string(s: &[u8], p: usize, n: usize, little_endian: bool) -> Option<[c_int; 4]> {
    let mut bytes = [0; 4];
    for (i, b) in bytes.iter_mut().take(n).enumerate() {
        *b = c_int::from(*s.get(p.checked_add(i)?)?);
    }
    if little_endian {
        bytes[..n].reverse();
    }
    Some(bytes)
}

/// The value an unsigned read pushes: exact below four bytes, a C `int`
/// sum at four.
fn cardinal(b: [c_int; 4], n: usize) -> i64 {
    let mut v: i32 = 0;
    for &x in &b[..n] {
        v = v.wrapping_mul(0x100).wrapping_add(x);
    }
    i64::from(v)
}

/// The value a signed read pushes: two's complement on `n` bytes, computed
/// exactly (the four-byte readers use `lua_Integer` for the top byte).
fn integer(b: [c_int; 4], n: usize) -> i64 {
    let mut v: i64 = 0;
    for &x in &b[..n] {
        v = v * 0x100 + i64::from(x);
    }
    match b[0] >= 0x80 {
        true => v - (1i64 << (8 * n)),
        false => v,
    }
}

unsafe fn push_or_nil(l: *mut lua_State, v: Option<i64>) -> c_int {
    match v {
        Some(v) => ffi::lua_pushinteger(l, v),
        None => ffi::lua_pushnil(l),
    }
    1
}

unsafe fn push_number_or_nil(l: *mut lua_State, v: Option<f64>) -> c_int {
    match v {
        Some(v) => ffi::lua_pushnumber(l, v),
        None => ffi::lua_pushnil(l),
    }
    1
}

/// The string argument 1 and the 0-based position argument 2.
unsafe fn string_at<'a>(l: *mut lua_State) -> (&'a [u8], usize) {
    let mut len = 0usize;
    let s = ffi::luaL_checklstring(l, 1, &mut len);
    let p = (ffi::luaL_checkinteger(l, 2) as usize).wrapping_sub(1);
    (std::slice::from_raw_parts(s as *const u8, len), p)
}

/// A `fio` reader of `n` bytes.
macro_rules! file_reader {
    ($name:ident, $n:expr, $le:expr, $value:ident) => {
        unsafe extern "C-unwind" fn $name(l: *mut lua_State) -> c_int {
            let f = tofile(l);
            push_or_nil(l, read_file(f, $n, $le).map(|b| $value(b, $n)))
        }
    };
}

/// A `sio` reader of `n` bytes.
macro_rules! string_reader {
    ($name:ident, $n:expr, $le:expr, $value:ident) => {
        unsafe extern "C-unwind" fn $name(l: *mut lua_State) -> c_int {
            let (s, p) = string_at(l);
            push_or_nil(l, read_string(s, p, $n, $le).map(|b| $value(b, $n)))
        }
    };
}

file_reader!(readcardinal1, 1, false, cardinal);
file_reader!(readcardinal2, 2, false, cardinal);
file_reader!(readcardinal3, 3, false, cardinal);
file_reader!(readcardinal4, 4, false, cardinal);
file_reader!(readcardinal2_le, 2, true, cardinal);
file_reader!(readcardinal3_le, 3, true, cardinal);
file_reader!(readcardinal4_le, 4, true, cardinal);
file_reader!(readinteger1, 1, false, integer);
file_reader!(readinteger2, 2, false, integer);
file_reader!(readinteger3, 3, false, integer);
file_reader!(readinteger4, 4, false, integer);
file_reader!(readinteger2_le, 2, true, integer);
file_reader!(readinteger3_le, 3, true, integer);
file_reader!(readinteger4_le, 4, true, integer);
string_reader!(readcardinal1_s, 1, false, cardinal);
string_reader!(readcardinal2_s, 2, false, cardinal);
string_reader!(readcardinal3_s, 3, false, cardinal);
string_reader!(readcardinal4_s, 4, false, cardinal);
string_reader!(readcardinal2_s_le, 2, true, cardinal);
string_reader!(readcardinal3_s_le, 3, true, cardinal);
string_reader!(readcardinal4_s_le, 4, true, cardinal);
string_reader!(readinteger1_s, 1, false, integer);
string_reader!(readinteger2_s, 2, false, integer);
string_reader!(readinteger3_s, 3, false, integer);
string_reader!(readinteger4_s, 4, false, integer);
string_reader!(readinteger2_s_le, 2, true, integer);
string_reader!(readinteger3_s_le, 3, true, integer);
string_reader!(readinteger4_s_le, 4, true, integer);

/// `readcardinaltable(f, n, b)` and `readintegertable`: up to `n` values of
/// `b` bytes, stopping at EOF; any other `b` is an empty table.
unsafe fn file_table(l: *mut lua_State, value: fn([c_int; 4], usize) -> i64) -> c_int {
    let f = tofile(l);
    let n = ffi::lua_tointeger(l, 2) as c_int;
    let b = ffi::lua_tointeger(l, 3);
    ffi::lua_createtable(l, n.max(0), 0);
    if (1..=4).contains(&b) {
        for i in 1..=n {
            let Some(bytes) = read_file(f, b as usize, false) else {
                break;
            };
            ffi::lua_pushinteger(l, value(bytes, b as usize));
            ffi::lua_rawseti(l, -2, i64::from(i));
        }
    }
    1
}

/// The string twins: `(s, p, n, b)`.
unsafe fn string_table(l: *mut lua_State, value: fn([c_int; 4], usize) -> i64) -> c_int {
    let (s, mut p) = string_at(l);
    let n = ffi::lua_tointeger(l, 3) as c_int;
    let b = ffi::lua_tointeger(l, 4);
    ffi::lua_createtable(l, n.max(0), 0);
    if (1..=4).contains(&b) {
        let b = b as usize;
        for i in 1..=n {
            let Some(bytes) = read_string(s, p, b, false) else {
                break;
            };
            p += b;
            ffi::lua_pushinteger(l, value(bytes, b));
            ffi::lua_rawseti(l, -2, i64::from(i));
        }
    }
    1
}

unsafe extern "C-unwind" fn readcardinaltable(l: *mut lua_State) -> c_int {
    file_table(l, cardinal)
}

unsafe extern "C-unwind" fn readintegertable(l: *mut lua_State) -> c_int {
    file_table(l, integer)
}

unsafe extern "C-unwind" fn readcardinaltable_s(l: *mut lua_State) -> c_int {
    string_table(l, cardinal)
}

unsafe extern "C-unwind" fn readintegertable_s(l: *mut lua_State) -> c_int {
    string_table(l, integer)
}

/// `readfixed2`: an unsigned 8.8 number.
fn fixed2(b: [c_int; 4]) -> f64 {
    let n = 0x100 * b[0] + b[1];
    f64::from(n >> 8) + f64::from(n & 0xff) / 256.0
}

/// `readfixed4`: a 16.16 number, the sum in a C `int`.
fn fixed4(b: [c_int; 4]) -> f64 {
    let n = cardinal(b, 4) as i32;
    f64::from(n >> 16) + f64::from(n & 0xffff) / 65536.0
}

/// `read2dot14`: a 2.14 number, sign-extended by the shifts of an `int`.
fn dot14(b: [c_int; 4]) -> f64 {
    let n: i32 = 0x100 * b[0] + b[1];
    f64::from(n.wrapping_shl(16) >> (16 + 14)) + f64::from(n & 0x3fff) / 16384.0
}

unsafe extern "C-unwind" fn readfixed2(l: *mut lua_State) -> c_int {
    let f = tofile(l);
    push_number_or_nil(l, read_file(f, 2, false).map(fixed2))
}

unsafe extern "C-unwind" fn readfixed4(l: *mut lua_State) -> c_int {
    let f = tofile(l);
    push_number_or_nil(l, read_file(f, 4, false).map(fixed4))
}

unsafe extern "C-unwind" fn read2dot14(l: *mut lua_State) -> c_int {
    let f = tofile(l);
    push_number_or_nil(l, read_file(f, 2, false).map(dot14))
}

unsafe extern "C-unwind" fn readfixed2_s(l: *mut lua_State) -> c_int {
    let (s, p) = string_at(l);
    // `if (p+3 >= l)`: four bytes asked for, two read.
    let v = read_string(s, p, 4, false).map(fixed2);
    push_number_or_nil(l, v)
}

unsafe extern "C-unwind" fn readfixed4_s(l: *mut lua_State) -> c_int {
    let (s, p) = string_at(l);
    push_number_or_nil(l, read_string(s, p, 4, false).map(fixed4))
}

unsafe extern "C-unwind" fn read2dot14_s(l: *mut lua_State) -> c_int {
    let (s, p) = string_at(l);
    push_number_or_nil(l, read_string(s, p, 2, false).map(dot14))
}

unsafe extern "C-unwind" fn getposition(l: *mut lua_State) -> c_int {
    let f = tofile(l);
    let p = libc::ftell(f);
    push_or_nil(l, (p >= 0).then_some(p as i64))
}

unsafe extern "C-unwind" fn setposition(l: *mut lua_State) -> c_int {
    let f = tofile(l);
    let p = ffi::lua_tointeger(l, 2) as libc::c_long;
    let r = libc::fseek(f, p, libc::SEEK_SET);
    push_or_nil(l, (r >= 0).then_some(i64::from(r)))
}

unsafe extern "C-unwind" fn skipposition(l: *mut lua_State) -> c_int {
    let f = tofile(l);
    let p = ffi::lua_tointeger(l, 2) as libc::c_long;
    let r = libc::fseek(f, libc::ftell(f) + p, libc::SEEK_SET);
    push_or_nil(l, (r >= 0).then_some(i64::from(r)))
}

/// `readbytetable(f, n)`: up to `n` bytes as a table, stopping at EOF.
unsafe extern "C-unwind" fn readbytetable(l: *mut lua_State) -> c_int {
    let f = tofile(l);
    let n = ffi::lua_tointeger(l, 2) as c_int;
    ffi::lua_createtable(l, n.max(0), 0);
    for i in 1..=n {
        let a = libc::fgetc(f);
        if a == EOF {
            break;
        }
        ffi::lua_pushinteger(l, i64::from(a));
        ffi::lua_rawseti(l, -2, i64::from(i));
    }
    1
}

/// `readbytes(f, n)`: up to `n` bytes as that many results.
unsafe extern "C-unwind" fn readbytes(l: *mut lua_State) -> c_int {
    let f = tofile(l);
    let n = ffi::lua_tointeger(l, 2) as c_int;
    for i in 1..=n {
        let a = libc::fgetc(f);
        if a == EOF {
            return i - 1;
        }
        ffi::lua_pushinteger(l, i64::from(a));
    }
    n.max(0)
}

/// The bytes `sio.readbytetable` and `sio.readbytes` read: `n` from `p`, cut
/// at the end of the string, or `None` from past it.
unsafe fn string_bytes<'a>(l: *mut lua_State) -> Option<&'a [u8]> {
    let (s, p) = string_at(l);
    let n = ffi::lua_tointeger(l, 3) as c_int;
    if p >= s.len() {
        return None;
    }
    let n = match p as i64 + i64::from(n) >= s.len() as i64 {
        true => s.len() - p,
        false => n.max(0) as usize,
    };
    Some(&s[p..p + n])
}

unsafe extern "C-unwind" fn readbytetable_s(l: *mut lua_State) -> c_int {
    let Some(bytes) = string_bytes(l) else {
        ffi::lua_pushnil(l);
        return 1;
    };
    ffi::lua_createtable(l, bytes.len() as c_int, 0);
    for (i, b) in bytes.iter().enumerate() {
        ffi::lua_pushinteger(l, i64::from(*b));
        ffi::lua_rawseti(l, -2, i as i64 + 1);
    }
    1
}

unsafe extern "C-unwind" fn readbytes_s(l: *mut lua_State) -> c_int {
    let Some(bytes) = string_bytes(l) else {
        return 0;
    };
    for b in bytes {
        ffi::lua_pushinteger(l, i64::from(*b));
    }
    bytes.len() as c_int
}

/// `readline(f)`: the next line without its end, which is `\n`, `\r` or
/// `\r\n`; `nil` at the end of the file when nothing is left.
unsafe extern "C-unwind" fn readline(l: *mut lua_State) -> c_int {
    let f = tofile(l);
    let mut line = Vec::new();
    loop {
        let c = libc::fgetc(f);
        match c {
            EOF => {
                if line.is_empty() {
                    ffi::lua_pushnil(l);
                    return 1;
                }
                break;
            }
            0x0A => break,
            0x0D => {
                let d = libc::fgetc(f);
                if d != EOF && d != 0x0A {
                    libc::ungetc(d, f);
                }
                break;
            }
            c => line.push(c as u8),
        }
    }
    ffi::lua_pushlstring(l, line.as_ptr().cast(), line.len());
    1
}

type Reader = unsafe extern "C-unwind" fn(*mut lua_State) -> c_int;

/// `fiolib`, in liolibext.c's order.
const FIO: &[(&str, Reader)] = &[
    ("readcardinal1", readcardinal1),
    ("readcardinal2", readcardinal2),
    ("readcardinal3", readcardinal3),
    ("readcardinal4", readcardinal4),
    ("readcardinal1le", readcardinal1),
    ("readcardinal2le", readcardinal2_le),
    ("readcardinal3le", readcardinal3_le),
    ("readcardinal4le", readcardinal4_le),
    ("readcardinaltable", readcardinaltable),
    ("readinteger1", readinteger1),
    ("readinteger2", readinteger2),
    ("readinteger3", readinteger3),
    ("readinteger4", readinteger4),
    ("readinteger1le", readinteger1),
    ("readinteger2le", readinteger2_le),
    ("readinteger3le", readinteger3_le),
    ("readinteger4le", readinteger4_le),
    ("readintegertable", readintegertable),
    ("readfixed2", readfixed2),
    ("readfixed4", readfixed4),
    ("read2dot14", read2dot14),
    ("setposition", setposition),
    ("getposition", getposition),
    ("skipposition", skipposition),
    ("readbytes", readbytes),
    ("readbytetable", readbytetable),
    ("readline", readline),
];

/// `siolib`.
const SIO: &[(&str, Reader)] = &[
    ("readcardinal1", readcardinal1_s),
    ("readcardinal2", readcardinal2_s),
    ("readcardinal3", readcardinal3_s),
    ("readcardinal4", readcardinal4_s),
    ("readcardinal1le", readcardinal1_s),
    ("readcardinal2le", readcardinal2_s_le),
    ("readcardinal3le", readcardinal3_s_le),
    ("readcardinal4le", readcardinal4_s_le),
    ("readcardinaltable", readcardinaltable_s),
    ("readinteger1", readinteger1_s),
    ("readinteger2", readinteger2_s),
    ("readinteger3", readinteger3_s),
    ("readinteger4", readinteger4_s),
    ("readinteger1le", readinteger1_s),
    ("readinteger2le", readinteger2_s_le),
    ("readinteger3le", readinteger3_s_le),
    ("readinteger4le", readinteger4_s_le),
    ("readintegertable", readintegertable_s),
    ("readfixed2", readfixed2_s),
    ("readfixed4", readfixed4_s),
    ("read2dot14", read2dot14_s),
    ("readbytes", readbytes_s),
    ("readbytetable", readbytetable_s),
];

/// `luaopen_fio`: `fio` and `sio` as globals and in `package.loaded`, as
/// `luaL_openlib` registers them.
pub(super) fn open(lua: &Lua) -> mlua::Result<()> {
    let loaded: mlua::Table = lua.globals().get::<mlua::Table>("package")?.get("loaded")?;
    for (name, funcs) in [("fio", FIO), ("sio", SIO)] {
        let t = lua.create_table()?;
        for (key, f) in funcs {
            // SAFETY: every reader checks its arguments with the auxiliary
            // library before it touches them, and holds nothing that needs
            // dropping when one of those checks raises a Lua error, except
            // `readline`'s buffer, which is built after its only check.
            t.set(*key, unsafe { lua.create_c_function(*f)? })?;
        }
        loaded.set(name, t.clone())?;
        lua.globals().set(name, t)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_byte_cardinal_wraps_as_a_c_int_and_the_integer_does_not() {
        // liolibext.c sums `readcardinal4` in an `int` and `readinteger4` in
        // a `lua_Integer`: 0xFFFFFFFE is -2 both ways, 0x80000000 is
        // -2147483648 as a cardinal and as an integer.
        assert_eq!(cardinal([0xFF, 0xFF, 0xFF, 0xFE], 4), -2);
        assert_eq!(integer([0xFF, 0xFF, 0xFF, 0xFE], 4), -2);
        assert_eq!(cardinal([0x7F, 0xFF, 0xFF, 0xFF], 4), 0x7FFF_FFFF);
        assert_eq!(cardinal([0x01, 0x02, 0x03, 0], 3), 0x010203);
        assert_eq!(integer([0x80, 0x00, 0, 0], 2), -32768);
    }

    #[test]
    fn the_fixed_readers_are_liolibexts() {
        // 0x0180 is 1.5 as 8.8; 0xFF80 is 255.5 (unsigned); 0xC000 is -1.0
        // as 2.14 and 0x7000 is 1.75.
        assert_eq!(fixed2([0x01, 0x80, 0, 0]), 1.5);
        assert_eq!(fixed2([0xFF, 0x80, 0, 0]), 255.5);
        assert_eq!(dot14([0xC0, 0x00, 0, 0]), -1.0);
        assert_eq!(dot14([0x70, 0x00, 0, 0]), 1.75);
        assert_eq!(fixed4([0xFF, 0xFF, 0x80, 0x00]), -0.5);
    }
}
