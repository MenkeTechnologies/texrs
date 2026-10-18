//! The `lua` table: bytecode registers, chunk names, the function table.
//!
//! A port of luatexdir/lua/llualib.c. A bytecode register holds a function as
//! `lua_dump` wrote it -- `lua.setbytecode(n, f [, strip])` dumps, and
//! `lua.getbytecode(n)` loads it again, once, keeping the loaded function in
//! a shadow table so a second get answers the same function. `\luabytecode n`
//! and `\luabytecodecall n` run register `n` with `n` as its argument
//! (`luabytecodecall`). `lua.name[n]` is the name `\directlua n {...}` reports
//! its chunk under.

use mlua::{Function, Lua, Table, Value};
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    /// `lua_bytecode_registers`: the dumped bytes, by register.
    static BYTECODES: RefCell<HashMap<i64, Vec<u8>>> = RefCell::new(HashMap::new());
    /// `luanames`: register 0..65535 to a chunk name.
    static NAMES: RefCell<HashMap<i64, String>> = RefCell::new(HashMap::new());
}

/// Where `lua.getbytecode` keeps what it loaded (`lua.bytecodes.indirect`).
const SHADOW: &str = "lua.bytecodes.indirect";
/// Where `lua.get_functions_table()` keeps its table (`lua_functions`).
pub(super) const FUNCTIONS: &str = "lua.functions";

/// Forget the registers, for a fresh state.
pub(super) fn reset() {
    BYTECODES.with(|b| b.borrow_mut().clear());
    NAMES.with(|n| n.borrow_mut().clear());
}

/// `get_luaname(n)`: the name a `\directlua n` chunk runs under.
pub(super) fn chunk_name(n: i64) -> Option<String> {
    NAMES.with(|m| m.borrow().get(&n).cloned())
}

/// `get_bytecode`: the function in register `k`, loading it the first time.
fn get(lua: &Lua, k: i64) -> mlua::Result<Value> {
    if k < 0 {
        return Ok(Value::Nil);
    }
    let shadow: Table = lua.named_registry_value(SHADOW)?;
    if let Value::Function(f) = shadow.raw_get::<Value>(k)? {
        return Ok(Value::Function(f));
    }
    let Some(bytes) = BYTECODES.with(|b| b.borrow().get(&k).cloned()) else {
        return Ok(Value::Nil);
    };
    let f = lua
        .load(bytes)
        .set_name("bytecode")
        .set_mode(mlua::chunk::ChunkMode::Binary)
        .into_function()
        .map_err(|_| mlua::Error::runtime("bad bytecode register"))?;
    shadow.raw_set(k, f.clone())?;
    Ok(Value::Function(f))
}

/// `set_bytecode(k, f [, strip])`: `nil` empties the register.
fn set(lua: &Lua, k: i64, f: Value, strip: bool) -> mlua::Result<()> {
    if k < 0 {
        return Err(mlua::Error::runtime("negative values not allowed"));
    }
    let shadow: Table = lua.named_registry_value(SHADOW)?;
    shadow.raw_set(k, Value::Nil)?;
    match f {
        Value::Nil => {
            BYTECODES.with(|b| b.borrow_mut().remove(&k));
        }
        Value::Function(f) => {
            let bytes = f.dump(strip);
            BYTECODES.with(|b| b.borrow_mut().insert(k, bytes));
        }
        _ => return Err(mlua::Error::runtime("unsupported type")),
    }
    Ok(())
}

/// `luabytecodecall(slot)`: the errors of llualib.c, then the call.
pub(super) fn call(lua: &Lua, slot: i64) -> mlua::Result<()> {
    if slot < 0 || BYTECODES.with(|b| b.borrow().keys().max().is_none_or(|m| slot > *m)) {
        return Err(mlua::Error::runtime("bytecode register out of range"));
    }
    if !BYTECODES.with(|b| b.borrow().contains_key(&slot)) {
        return Err(mlua::Error::runtime("undefined bytecode register"));
    }
    let bytes = BYTECODES
        .with(|b| b.borrow().get(&slot).cloned())
        .unwrap_or_default();
    let f: Function = lua
        .load(bytes)
        .set_name("bytecode")
        .set_mode(mlua::chunk::ChunkMode::Binary)
        .into_function()
        .map_err(|_| mlua::Error::runtime("bytecode register doesn't load well"))?;
    f.call::<()>(slot)
}

/// A table whose metatable's `__index` IS the getter and `__newindex` IS the
/// setter: luastuff.c's `make_table`, which stores the two library functions
/// themselves, so they are called with the table as their first argument.
/// That is why `lua.bytecode[n]` works -- `getbytecode` reads its LAST
/// argument -- and why `lua.name[n]` does not: `getluaname` checks its first,
/// which is the table, and `setluaname` acts only on exactly two arguments.
/// Both measured in luatex 1.24.0, and both kept.
fn proxy(lua: &Lua, lib: &Table, name: &str, getter: &str, setter: &str) -> mlua::Result<()> {
    let t = lua.create_table()?;
    let mt = lua.create_table()?;
    mt.set("__name", format!("tex.{name}"))?;
    mt.set("__index", lib.get::<Function>(getter)?)?;
    mt.set("__newindex", lib.get::<Function>(setter)?)?;
    t.set_metatable(Some(mt))?;
    lib.set(name, t)
}

/// The `lua` table.
pub(super) fn table(lua: &Lua) -> mlua::Result<Table> {
    reset();
    lua.set_named_registry_value(SHADOW, lua.create_table()?)?;
    lua.set_named_registry_value(FUNCTIONS, lua.create_table()?)?;
    let t = lua.create_table()?;
    // `get_luaname`: `luaL_checkinteger(L, 1)`.
    t.set(
        "getluaname",
        lua.create_function(|_, args: mlua::Variadic<Value>| {
            let first = args.first().cloned().unwrap_or(Value::Nil);
            let Some(k) = first.as_integer() else {
                return Err(mlua::Error::runtime(format!(
                    "bad argument #1 to 'getluaname' (number expected, got {})",
                    first.type_name()
                )));
            };
            Ok(chunk_name(k))
        })?,
    )?;
    // `set_luaname(k, name)`: exactly two arguments, a string name or anything
    // else to clear it, and nothing at all for a register out of range.
    t.set(
        "setluaname",
        lua.create_function(|_, args: mlua::Variadic<Value>| {
            if let [k, v] = &args[..] {
                let k = k.as_integer().unwrap_or(-1);
                if (0..=65535).contains(&k) {
                    NAMES.with(|m| match v {
                        Value::String(s) => {
                            m.borrow_mut().insert(k, s.to_string_lossy());
                        }
                        _ => {
                            m.borrow_mut().remove(&k);
                        }
                    });
                }
            }
            Ok(())
        })?,
    )?;
    // `get_bytecode`: `luaL_checkinteger(L, -1)`, the LAST argument.
    t.set(
        "getbytecode",
        lua.create_function(|lua, args: mlua::Variadic<Value>| {
            let last = args.last().cloned().unwrap_or(Value::Nil);
            let Some(k) = last.as_integer() else {
                return Err(mlua::Error::runtime(format!(
                    "bad argument #-1 to 'getbytecode' (number expected, got {})",
                    last.type_name()
                )));
            };
            get(lua, k)
        })?,
    )?;
    t.set(
        "setbytecode",
        lua.create_function(|lua, args: mlua::Variadic<Value>| {
            let mut args: Vec<Value> = args.into_iter().collect();
            let strip = match args.last() {
                Some(Value::Boolean(b)) => {
                    let b = *b;
                    args.pop();
                    b
                }
                _ => false,
            };
            let n = args.len();
            if n < 2 {
                return Err(mlua::Error::runtime("bad argument to 'setbytecode'"));
            }
            let k = args[n - 2].as_integer().ok_or_else(|| {
                mlua::Error::runtime("bad argument #1 to 'setbytecode' (number expected)")
            })?;
            set(lua, k, args[n - 1].clone(), strip)
        })?,
    )?;
    t.set(
        "newtable",
        lua.create_function(|lua, (i, h): (usize, usize)| lua.create_table_with_capacity(i, h))?,
    )?;
    t.set(
        "get_functions_table",
        lua.create_function(|lua, ()| lua.named_registry_value::<Table>(FUNCTIONS))?,
    )?;
    // `get_stack_top`: the C function's own stack, which holds its arguments.
    t.set(
        "getstacktop",
        lua.create_function(|_, args: mlua::Variadic<Value>| Ok(args.len() as i64))?,
    )?;
    // `lua_active`: how many TeX-to-Lua calls are running, this one included.
    t.set(
        "getcalllevel",
        lua.create_function(|_, ()| Ok(super::ctx::depth() as i64))?,
    )?;
    // "getcodepage": the Windows code pages; elsewhere `false, false`.
    t.set(
        "getcodepage",
        lua.create_function(|_, ()| Ok((false, false)))?,
    )?;
    proxy(lua, &t, "bytecode", "getbytecode", "setbytecode")?;
    proxy(lua, &t, "name", "getluaname", "setluaname")?;
    // "<string> s = lua.version — This returns the Lua version identifier
    // string." `LUA_VERSION` is the interpreter's own answer, so this cannot
    // drift from the interpreter that is actually running.
    t.set("version", lua.globals().get::<Value>("_VERSION")?)?;
    Ok(t)
}
