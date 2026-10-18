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
//! The registry is per process and keyed by font id, the way one run's
//! fonts are numbered; [`clear_lua_fonts`] empties it for the next run.

use std::collections::BTreeMap;
use std::sync::RwLock;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LuaFont {
    pub filename: String,
    pub subfont: i64,
    pub psname: Option<String>,
    /// character code -> (glyph index, `tounicode` hex)
    pub glyphs: BTreeMap<u32, (u32, Option<String>)>,
}

static FONTS: RwLock<BTreeMap<u16, LuaFont>> = RwLock::new(BTreeMap::new());

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
    if let Ok(mut m) = FONTS.write() {
        m.insert(id, font);
    }
}

/// The font registered for `id`, if any.
pub fn lua_font(id: u16) -> Option<LuaFont> {
    FONTS.read().ok()?.get(&id).cloned()
}

/// Whether font `id` came from luaotfload's definer (and so is processed
/// by its callbacks, not natively).
pub fn is_lua_font(id: u16) -> bool {
    FONTS.read().is_ok_and(|m| m.contains_key(&id))
}

/// Forget every registered font.
pub fn clear_lua_fonts() {
    if let Ok(mut m) = FONTS.write() {
        m.clear();
    }
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
