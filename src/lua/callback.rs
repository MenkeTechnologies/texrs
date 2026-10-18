//! The `callback` library: where a document's Lua takes over a step of TeX.
//!
//! A port of luatexdir/lua/lcallbacklib.c. The registry is LuaTeX's: one slot
//! per callback NAME, numbered as `luatexcallbackids.h` numbers them, holding
//! a function, `false`, or nothing; and the per-slot state `callback_set[]`
//! that the engine tests before it builds a callback's arguments, which is
//! what makes an unregistered callback cost one load and one branch.
//!
//! The engine's side is [`state`] and [`function`]: each hook point in texrs
//! asks whether its callback is set, and if it is, calls the function with the
//! arguments LuaTeX passes and reads back what LuaTeX reads back. Which hook
//! points exist, and which still wait on the stomach that would call them, is
//! recorded beside each name in [`NAMES`].

use mlua::{Function, Lua, Table, Value, Variadic};
use std::cell::RefCell;

/// `callbacknames[]`, index for index: slot 0 is "empty on purpose", and the
/// rest are in `callback_callback_types` order (luatexcallbackids.h), which is
/// the number `callback.register` answers with.
pub const NAMES: &[&str] = &[
    "",
    "find_write_file",
    "find_output_file",
    "find_image_file",
    "find_format_file",
    "find_read_file",
    "open_read_file",
    "find_vf_file",
    "read_vf_file",
    "find_data_file",
    "read_data_file",
    "find_font_file",
    "read_font_file",
    "find_map_file",
    "read_map_file",
    "find_enc_file",
    "read_enc_file",
    "find_type1_file",
    "read_type1_file",
    "find_truetype_file",
    "read_truetype_file",
    "find_opentype_file",
    "read_opentype_file",
    "find_cidmap_file",
    "read_cidmap_file",
    "find_pk_file",
    "read_pk_file",
    "show_error_hook",
    "process_input_buffer",
    "process_output_buffer",
    "process_jobname",
    "start_page_number",
    "stop_page_number",
    "start_run",
    "stop_run",
    "define_font",
    "pre_output_filter",
    "buildpage_filter",
    "hpack_filter",
    "vpack_filter",
    "glyph_not_found",
    "glyph_info",
    "hyphenate",
    "ligaturing",
    "kerning",
    "pre_linebreak_filter",
    "linebreak_filter",
    "post_linebreak_filter",
    "append_to_vlist_filter",
    "mlist_to_hlist",
    "finish_pdffile",
    "finish_pdfpage",
    "pre_dump",
    "start_file",
    "stop_file",
    "show_error_message",
    "show_lua_error_hook",
    "show_ignored_error_message",
    "show_warning_message",
    "hpack_quality",
    "vpack_quality",
    "process_rule",
    "insert_local_par",
    "contribute_filter",
    "call_edit",
    "build_page_insert",
    "glyph_stream_provider",
    "font_descriptor_objnum_provider",
    "finish_synctex",
    "wrapup_run",
    "new_graf",
    "page_order_index",
    "make_extensible",
    "process_pdf_image_content",
    "provide_charproc_data",
    "input_level_string",
    "dvi_output",
];

/// A callback's slot number, `luatexcallbackids.h`'s enumerator.
pub fn id(name: &str) -> Option<usize> {
    NAMES.iter().skip(1).position(|n| *n == name).map(|i| i + 1)
}

/// What a slot holds, which is `callback_set[i]`: the slot's own number for a
/// function, −1 for `false`, 0 for nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Unset,
    /// Registered as `false`: "the callback is disabled", which for the file
    /// discovery callbacks and a few others means TeX does nothing at all
    /// where it would have called it.
    Disabled,
    Function,
}

thread_local! {
    /// `callback_set[]`. Per thread because the Lua state is (see the parent
    /// module's `RUNTIME`); cleared with it.
    static SET: RefCell<Vec<State>> = RefCell::new(vec![State::Unset; NAMES.len()]);
}

/// Where the functions are kept: one table in the Lua registry, indexed by
/// slot, as `callback_callbacks_id` is in LuaTeX.
const REGISTRY_KEY: &str = "texrs.callbacks";

/// Forget every registration, for a fresh Lua state.
pub(super) fn reset() {
    SET.with(|s| s.borrow_mut().iter_mut().for_each(|v| *v = State::Unset));
}

/// `callback_defined(i)`, by name: what the engine asks before it does
/// anything else at a hook point.
pub fn state(name: &str) -> State {
    match id(name) {
        Some(i) => SET.with(|s| s.borrow()[i]),
        None => State::Unset,
    }
}

/// The function registered for `name`, when there is one.
pub fn function(lua: &Lua, name: &str) -> Option<Function> {
    let i = id(name)?;
    if SET.with(|s| s.borrow()[i]) != State::Function {
        return None;
    }
    let t: Table = lua.named_registry_value(REGISTRY_KEY).ok()?;
    t.raw_get::<Option<Function>>(i).ok().flatten()
}

/// The `callback` table: `register`, `find`, `list` -- the three `luatex`
/// 1.24.0 has (`listidx` is in the trunk source but not in that binary).
pub(super) fn table(lua: &Lua) -> mlua::Result<Table> {
    lua.set_named_registry_value(REGISTRY_KEY, lua.create_table()?)?;
    let t = lua.create_table()?;
    // `callback_register(name, f)`: nil-and-a-message for a bad first
    // argument, a bad second one or an unknown name; the slot number on
    // success.
    t.set(
        "register",
        lua.create_function(|lua, (name, f): (Value, Value)| {
            let fail = |msg: &str| -> mlua::Result<Variadic<Value>> {
                Ok(Variadic::from_iter([
                    Value::Nil,
                    Value::String(lua.create_string(msg)?),
                ]))
            };
            let Value::String(name) = name else {
                return fail(
                    "Invalid arguments to callback.register, first argument must be string.",
                );
            };
            // `(t2 != LUA_TFUNCTION) && (t2 != LUA_TNIL) && ((t2 != LUA_TBOOLEAN)
            // && (lua_toboolean(L, 2) == 0))`: as written, only a non-function,
            // non-nil, non-boolean value that is also falsy is refused -- and
            // nothing but `nil` and `false` is falsy, so in practice every
            // value is accepted and a non-function counts as a function slot.
            let state = match &f {
                Value::Function(_) => State::Function,
                Value::Boolean(_) => State::Disabled,
                Value::Nil => State::Unset,
                _ => State::Function,
            };
            let Some(i) = id(&name.to_str()?) else {
                return fail("No such callback exists.");
            };
            SET.with(|s| s.borrow_mut()[i] = state);
            let reg: Table = lua.named_registry_value(REGISTRY_KEY)?;
            reg.raw_set(i, f)?;
            Ok(Variadic::from_iter([Value::Integer(i as i64)]))
        })?,
    )?;
    t.set(
        "find",
        lua.create_function(|lua, name: Value| {
            let fail = |msg: &str| -> mlua::Result<Variadic<Value>> {
                Ok(Variadic::from_iter([
                    Value::Nil,
                    Value::String(lua.create_string(msg)?),
                ]))
            };
            let Value::String(name) = name else {
                return fail("Invalid arguments to callback.find.");
            };
            let Some(i) = id(&name.to_str()?) else {
                return fail("No such callback exists.");
            };
            let reg: Table = lua.named_registry_value(REGISTRY_KEY)?;
            Ok(Variadic::from_iter([reg.raw_get::<Value>(i)?]))
        })?,
    )?;
    // `callback_listf`: every name, `true` when its slot is set to anything
    // -- a function or `false`.
    t.set(
        "list",
        lua.create_function(|lua, ()| {
            let t = lua.create_table()?;
            SET.with(|s| {
                let s = s.borrow();
                for (i, name) in NAMES.iter().enumerate().skip(1) {
                    t.raw_set(*name, s[i] != State::Unset)?;
                }
                Ok::<_, mlua::Error>(())
            })?;
            Ok(t)
        })?,
    )?;
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slot_numbers_are_luatexs() {
        // luatexcallbackids.h: `find_write_file_callback = 1`, and the order
        // after it. `callback.register` hands these numbers back, and ltluatex
        // keys its own tables on the names, so both have to be exact.
        assert_eq!(id("find_write_file"), Some(1));
        assert_eq!(id("find_read_file"), Some(5));
        assert_eq!(id("define_font"), Some(35));
        assert_eq!(id("pre_linebreak_filter"), Some(45));
        assert_eq!(id("dvi_output"), Some(NAMES.len() - 1));
        assert_eq!(id(""), None);
        assert_eq!(id("no_such_callback"), None);
    }
}
