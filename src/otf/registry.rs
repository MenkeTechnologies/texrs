//! Fonts luaotfload's Lua definer made, as the PDF writer needs them: the
//! file, the subfont, the PostScript name, and each character's glyph index
//! and `/ToUnicode` from the table the definer returned. The Lua side
//! registers one when `define_font` gives the stomach an OpenType or
//! TrueType font; the writer and the subsetters look it up by the stomach's
//! font id.
//!
//! A registered font is processed by luaotfload's own list callbacks, so the
//! native node-mode processing (`super::ots::process`) is not to be run on
//! it: [`is_lua_font`] says which fonts those are.
//!
//! The registry is per thread -- a document is lowered and run on one
//! thread, and several documents run at once in one process, each
//! numbering its fonts from 1 -- and keyed by font id; [`clear_lua_fonts`]
//! empties it for the thread's next run.

use std::cell::RefCell;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LuaFont {
    pub filename: String,
    pub subfont: i64,
    pub psname: Option<String>,
    /// character code -> (glyph index, `tounicode` hex)
    pub glyphs: BTreeMap<u32, (u32, Option<String>)>,
}

thread_local! {
    static FONTS: RefCell<BTreeMap<u16, LuaFont>> = const { RefCell::new(BTreeMap::new()) };
}

/// Record the font luaotfload's definer made for stomach font `id`.
/// `glyphs` are (character code, glyph index, tounicode).
pub fn register_lua_font(
    id: u16,
    filename: &str,
    subfont: i64,
    psname: Option<&str>,
    glyphs: &[(u32, u32, Option<String>)],
) {
    let font = LuaFont {
        filename: filename.to_string(),
        subfont,
        psname: psname.map(str::to_string),
        glyphs: glyphs
            .iter()
            .map(|(c, g, t)| (*c, (*g, t.clone())))
            .collect(),
    };
    FONTS.with(|m| m.borrow_mut().insert(id, font));
}

/// The font registered for `id`, if any.
pub fn lua_font(id: u16) -> Option<LuaFont> {
    FONTS.with(|m| m.borrow().get(&id).cloned())
}

/// Whether font `id` came from luaotfload's definer (and so is processed
/// by its callbacks, not natively).
pub fn is_lua_font(id: u16) -> bool {
    FONTS.with(|m| m.borrow().contains_key(&id))
}

/// Forget every registered font.
pub fn clear_lua_fonts() {
    FONTS.with(|m| m.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registered_font_is_found_by_id_and_cleared() {
        register_lua_font(
            4711,
            "/x/y.otf",
            1,
            Some("Y-Regular"),
            &[(65, 36, Some("0041".into())), (66, 37, None)],
        );
        assert!(is_lua_font(4711));
        let f = lua_font(4711).unwrap();
        assert_eq!(f.glyphs[&65], (36, Some("0041".into())));
        assert_eq!(
            (f.filename.as_str(), f.subfont, f.psname.as_deref()),
            ("/x/y.otf", 1, Some("Y-Regular"))
        );
        assert!(!is_lua_font(4712));
    }
}
