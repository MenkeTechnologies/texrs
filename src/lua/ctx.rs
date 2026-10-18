//! The engine a running chunk reaches, for exactly as long as it runs.
//!
//! LuaTeX's libraries are C functions over TeX's globals: `token.scan_int`
//! moves `cur_input`, `tex.print` writes the current input level, and a
//! function a package stored in a local at load time -- ltluatex.lua's
//! `local tex_setcount = tex.setcount`, luaotfload's `local scan_int =
//! token.scan_int` -- reaches the same globals whenever it is later called. So
//! the library functions here are built ONCE, with the state, and find the
//! engine through this module rather than capturing it.
//!
//! The engine is texrs's `&mut Engine` and `&mut Lexer`, owned by the caller
//! of a chunk. [`enter`] lends them to the thread for the length of one call
//! into Lua and takes them back when it returns -- on unwind too -- restoring
//! whatever was lent before, so a chunk that runs TeX that runs a chunk nests
//! correctly. [`with`] is the borrow a library function makes. Outside any
//! call there is no engine, and a library function that needs one says so
//! rather than reading freed memory: that is the case of a function called
//! from a hook that fires with no TeX input behind it.

use crate::expand::Engine;
use crate::lexer::Lexer;
use std::cell::Cell;

/// One lent engine. Raw pointers because the lender's lifetimes cannot be
/// named in a `thread_local!`; [`enter`]'s guard is what bounds them.
#[derive(Clone, Copy)]
struct Frame {
    eng: *mut Engine,
    lx: *mut Lexer,
}

thread_local! {
    static CURRENT: Cell<Option<Frame>> = const { Cell::new(None) };
    /// How many lent frames are open: llualib.c's `lua_active`.
    static DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// How many calls from TeX into Lua are running (`lua.getcalllevel()`).
pub(super) fn depth() -> usize {
    DEPTH.with(Cell::get)
}

/// Run `body` with `eng` and `lx` reachable through [`with`].
pub(super) fn enter<T>(eng: &mut Engine, lx: &mut Lexer, body: impl FnOnce() -> T) -> T {
    struct Restore(Option<Frame>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CURRENT.with(|c| c.set(self.0));
            DEPTH.with(|d| d.set(d.get() - 1));
        }
    }
    DEPTH.with(|d| d.set(d.get() + 1));
    let frame = Frame {
        eng: eng as *mut Engine,
        lx: lx as *mut Lexer,
    };
    let _restore = Restore(CURRENT.with(|c| c.replace(Some(frame))));
    body()
}

/// The lent engine and input, for one library call.
///
/// `what` names the function, for the error a call made with nothing lent
/// gets.
pub(super) fn with<T>(
    what: &str,
    f: impl FnOnce(&mut Engine, &mut Lexer) -> mlua::Result<T>,
) -> mlua::Result<T> {
    let Some(frame) = CURRENT.with(|c| c.get()) else {
        return Err(mlua::Error::runtime(format!(
            "{what} reads TeX's input, and it was called when no TeX input \
             was running it"
        )));
    };
    // SAFETY: the frame was set by `enter` from two exclusive borrows that
    // outlive the call into Lua it guards, and it is cleared before they end.
    // The lender does not touch either while Lua runs; a nested `enter` lends
    // borrows derived from these, and restores this frame when it returns.
    let (eng, lx) = unsafe { (&mut *frame.eng, &mut *frame.lx) };
    f(eng, lx)
}
