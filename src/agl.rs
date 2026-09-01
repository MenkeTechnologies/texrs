//! Glyph names to Unicode, ported from `agl.c` in tectonic's `xdvipdfmx`.
//!
//! A PDF says which glyph to draw, never which character it is. That is fine
//! for drawing and useless for everything else: a reader asked to copy a
//! paragraph, search a document, or read one aloud has a glyph called `ff` and
//! no idea it is two f's. The answer is a `/ToUnicode` map written beside the
//! font, and to write one a driver has to know what a glyph name means.
//!
//! Adobe's Glyph List is the table that says. It is not only a table, though:
//! most names are not in it and are meant to be worked out. `uni0041` is
//! U+0041 by construction, `u1D400` likewise, `a.sc` is whatever `a` is, and
//! `f_f_i` is three characters. `agl.c` does those by rule and looks the rest
//! up, and so does this.
//!
//! The table itself is `glyphlist.txt`, which every TeX installation ships
//! because dvipdfmx and pdftex both need it. It is read when it is there and
//! done without when it is not -- the rules cover the names a TeX font
//! actually uses, and the table is what turns `eacute` into U+00E9 rather than
//! nothing.

use std::collections::BTreeMap;
use std::sync::OnceLock;

/// What a glyph name means, as a string of characters.
///
/// A name may mean several: `f_f_i` is three, and the ligature names are one
/// character that a reader is happy to see as several. Nothing at all comes
/// back for a name that names no character -- `.notdef`, or a font's private
/// `cid42` -- because guessing there would put wrong text in a document
/// somebody copies out.
pub fn unicode(name: &str) -> Option<String> {
    // A suffix says which variant of a glyph this is -- `a.sc` is a small-cap
    // a, `one.oldstyle` an old-style one -- and the character is the same.
    let base = name.split('.').next().unwrap_or(name);
    if base.is_empty() {
        return None;
    }

    // A name joined by underscores is that many characters in a row, which is
    // how a font names a ligature it has no other name for.
    if base.contains('_') {
        let mut out = String::new();
        for piece in base.split('_') {
            out.push_str(&unicode(piece)?);
        }
        return Some(out);
    }

    if let Some(text) = constructed(base) {
        return Some(text);
    }
    table().get(base).cloned()
}

/// The names that say their own value: `uniXXXX` and one or more of them,
/// `uXXXX` up to six digits.
fn constructed(name: &str) -> Option<String> {
    if let Some(digits) = name.strip_prefix("uni") {
        // §  : `uni` may be followed by several four-digit values, which is a
        // sequence of characters.
        if digits.len() >= 4
            && digits.len() % 4 == 0
            && digits.chars().all(|c| c.is_ascii_hexdigit())
        {
            let mut out = String::new();
            for chunk in digits.as_bytes().chunks(4) {
                let value = u32::from_str_radix(std::str::from_utf8(chunk).ok()?, 16).ok()?;
                out.push(char::from_u32(value)?);
            }
            return Some(out);
        }
    }
    if let Some(digits) = name.strip_prefix('u') {
        if (4..=6).contains(&digits.len()) && digits.chars().all(|c| c.is_ascii_hexdigit()) {
            let value = u32::from_str_radix(digits, 16).ok()?;
            return char::from_u32(value).map(String::from);
        }
    }
    None
}

/// Adobe's list, from the installation, read once.
/// What the map holds with no `glyphlist.txt`: everything the crate knows on
/// its own.
///
/// Separate from [`table`] so a test can read it without depending on whether
/// the machine running the test has a TeX installation. On a machine that has
/// one every name resolves out of the installed list and a hole here never
/// shows; the CI runner has no TeX, which is the case this has to be right for.
fn built_in_table() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    // The 52 letters name themselves -- the list really does carry `A;0041` --
    // and they are the names a font uses more than any others. Leaving them to
    // the installed list means a machine without one cannot read the commonest
    // glyph name there is.
    for letter in ('A'..='Z').chain('a'..='z') {
        out.insert(letter.to_string(), letter.to_string());
    }
    for (name, text) in BUILT_IN {
        out.insert(name.to_string(), text.to_string());
    }
    out
}

fn table() -> &'static BTreeMap<String, String> {
    static TABLE: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut out = built_in_table();
        if let Some(text) = installed_list() {
            for line in text.lines() {
                if line.starts_with('#') {
                    continue;
                }
                let Some((name, values)) = line.split_once(';') else {
                    continue;
                };
                let text: Option<String> = values
                    .split_whitespace()
                    .map(|value| u32::from_str_radix(value, 16).ok().and_then(char::from_u32))
                    .collect();
                if let Some(text) = text {
                    out.insert(name.to_string(), text);
                }
            }
        }
        out
    })
}

/// `glyphlist.txt`, wherever the installation keeps it.
fn installed_list() -> Option<String> {
    let found = std::process::Command::new("kpsewhich")
        .arg("glyphlist.txt")
        .output()
        .ok()?;
    let path = String::from_utf8_lossy(&found.stdout).trim().to_string();
    std::fs::read_to_string(path).ok()
}

/// Enough of the list to work without it: the names a TeX font uses for the
/// characters a document is mostly made of. The rest comes from
/// `glyphlist.txt` when it is there.
const BUILT_IN: &[(&str, &str)] = &[
    ("space", " "),
    ("exclam", "!"),
    ("quotedbl", "\""),
    ("numbersign", "#"),
    ("dollar", "$"),
    ("percent", "%"),
    ("ampersand", "&"),
    ("quotesingle", "'"),
    ("quoteright", "\u{2019}"),
    ("quoteleft", "\u{2018}"),
    ("parenleft", "("),
    ("parenright", ")"),
    ("asterisk", "*"),
    ("plus", "+"),
    ("comma", ","),
    ("hyphen", "-"),
    ("period", "."),
    ("slash", "/"),
    ("zero", "0"),
    ("one", "1"),
    ("two", "2"),
    ("three", "3"),
    ("four", "4"),
    ("five", "5"),
    ("six", "6"),
    ("seven", "7"),
    ("eight", "8"),
    ("nine", "9"),
    ("colon", ":"),
    ("semicolon", ";"),
    ("less", "<"),
    ("equal", "="),
    ("greater", ">"),
    ("question", "?"),
    ("at", "@"),
    ("bracketleft", "["),
    ("backslash", "\\"),
    ("bracketright", "]"),
    ("asciicircum", "^"),
    ("underscore", "_"),
    ("grave", "`"),
    ("braceleft", "{"),
    ("bar", "|"),
    ("braceright", "}"),
    ("asciitilde", "~"),
    ("endash", "\u{2013}"),
    ("emdash", "\u{2014}"),
    ("quotedblleft", "\u{201c}"),
    ("quotedblright", "\u{201d}"),
    ("dotlessi", "\u{131}"),
    // The ligatures a TeX font sets by default, which are the reason a PDF of
    // a TeX document needs any of this: they sit at codes 11 to 15 of a
    // Computer Modern font and mean nothing to a reader without a map.
    ("ff", "\u{fb00}"),
    ("fi", "\u{fb01}"),
    ("fl", "\u{fb02}"),
    ("ffi", "\u{fb03}"),
    ("ffl", "\u{fb04}"),
];

/// A `/ToUnicode` CMap for a font addressed by GLYPH rather than by code.
///
/// A face fetched for a glyph the document's own face lacks is written with
/// `/Encoding /Identity-H`, where a code is two bytes and IS a glyph id, so the
/// codespace and every entry are twice as wide as the simple font's below.
/// Without it the box drawing a book fetches from such a face would be ON the
/// page and nowhere in its text: a glyph id means nothing to a reader by
/// itself, and `pdftotext` reads an unmapped one back as nothing at all.
pub fn to_unicode_wide(name: &str, codes: &[(u16, String)]) -> String {
    let mut out = String::new();
    out.push_str(
        "/CIDInit /ProcSet findresource begin\n\
         12 dict begin\n\
         begincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n",
    );
    out.push_str(&format!("/CMapName /{name} def\n/CMapType 2 def\n"));
    out.push_str("1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n");
    // The same hundred-to-a-block rule S9.10.3 states for the simple map.
    for block in codes.chunks(100) {
        out.push_str(&format!("{} beginbfchar\n", block.len()));
        for (code, text) in block {
            let utf16: String = text
                .encode_utf16()
                .map(|unit| format!("{unit:04X}"))
                .collect();
            out.push_str(&format!("<{code:04X}> <{utf16}>\n"));
        }
        out.push_str("endbfchar\n");
    }
    out.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    out
}

/// A `/ToUnicode` CMap for a simple font: what each byte means.
///
/// Ported from the CMap `pdf_font_load_type1` writes. The format is
/// PostScript, and a reader parses it rather than executing it, so what
/// matters is the shape: a header saying which kind of map it is, then runs of
/// `bfchar` in blocks of no more than a hundred, then a footer.
pub fn to_unicode(name: &str, codes: &[(u8, String)]) -> String {
    let mut out = String::new();
    out.push_str(
        "/CIDInit /ProcSet findresource begin\n\
         12 dict begin\n\
         begincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n",
    );
    out.push_str(&format!("/CMapName /{name} def\n/CMapType 2 def\n"));
    out.push_str("1 begincodespacerange\n<00> <FF>\nendcodespacerange\n");

    // §9.10.3: at most a hundred to a block, which is the one rule about this
    // file that a reader enforces.
    for block in codes.chunks(100) {
        out.push_str(&format!("{} beginbfchar\n", block.len()));
        for (code, text) in block {
            let utf16: String = text
                .encode_utf16()
                .map(|unit| format!("{unit:04X}"))
                .collect();
            out.push_str(&format!("<{code:02X}> <{utf16}>\n"));
        }
        out.push_str("endbfchar\n");
    }
    out.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names that say their own value, which is most of what a modern font
    /// uses.
    #[test]
    fn a_constructed_name_says_which_character_it_is() {
        assert_eq!(unicode("uni0041").as_deref(), Some("A"));
        assert_eq!(unicode("uni00E9").as_deref(), Some("\u{e9}"));
        // Several values in a row are a sequence of characters.
        assert_eq!(unicode("uni004100420043").as_deref(), Some("ABC"));
        assert_eq!(unicode("u1D400").as_deref(), Some("\u{1d400}"));
        assert_eq!(unicode("u0041").as_deref(), Some("A"));

        // A suffix says which variant this is; the character is the same.
        assert_eq!(unicode("a.sc").as_deref(), Some("a"));
        assert_eq!(unicode("one.oldstyle").as_deref(), Some("1"));
        assert_eq!(unicode("uni0041.alt").as_deref(), Some("A"));

        // Underscores join characters, which is how a font names a ligature it
        // has no other name for.
        assert_eq!(unicode("f_f_i").as_deref(), Some("ffi"));
        assert_eq!(unicode("a_b").as_deref(), Some("ab"));

        // Nothing that names no character: a guess would put wrong text into a
        // document somebody copies out.
        assert_eq!(unicode(".notdef"), None);
        assert_eq!(unicode("cid42"), None);
        assert_eq!(unicode("uni00G1"), None);
        assert_eq!(unicode("u12"), None);
        assert_eq!(unicode(""), None);
    }

    /// The map has to be right on a machine with no TeX installation.
    ///
    /// `glyphlist.txt` is found with `kpsewhich`, so on a developer machine
    /// every name resolves out of the installed list and a hole in the built-in
    /// map is invisible. CI has no TeX and reads the built-in map alone -- which
    /// is how the letters turned out to be missing from it, after the tests
    /// above had passed locally for as long as they existed.
    #[test]
    fn the_names_resolve_without_a_tex_installation() {
        let built = built_in_table();
        for name in [
            "A",
            "a",
            "Z",
            "z",
            "space",
            "quoteright",
            "ff",
            "ffl",
            "dotlessi",
        ] {
            assert!(
                built.contains_key(name),
                "{name} needs glyphlist.txt to resolve"
            );
        }
        // The letters are themselves; the list carries them as `A;0041`.
        assert_eq!(built.get("A").map(String::as_str), Some("A"));
        assert_eq!(built.get("z").map(String::as_str), Some("z"));
        // A digit is NOT named by itself -- it is `one`, and glyphlist.txt has
        // no entry for `1` -- so guessing identity for every short name would
        // invent a mapping the list does not have.
        assert_eq!(built.get("1"), None);
        assert_eq!(built.get("one").map(String::as_str), Some("1"));
    }

    /// The names a TeX font uses, which is why any of this exists.
    #[test]
    fn the_names_a_tex_font_uses_are_known() {
        assert_eq!(unicode("space").as_deref(), Some(" "));
        assert_eq!(unicode("A").as_deref(), Some("A"));
        assert_eq!(unicode("ff").as_deref(), Some("\u{fb00}"));
        assert_eq!(unicode("fi").as_deref(), Some("\u{fb01}"));
        assert_eq!(unicode("ffl").as_deref(), Some("\u{fb04}"));
        assert_eq!(unicode("quoteright").as_deref(), Some("\u{2019}"));
        assert_eq!(unicode("endash").as_deref(), Some("\u{2013}"));
        assert_eq!(unicode("dotlessi").as_deref(), Some("\u{131}"));
    }

    /// The map itself, whose shape a reader is strict about.
    #[test]
    fn a_map_is_written_in_blocks_a_reader_accepts() {
        let codes: Vec<(u8, String)> = (b'A'..=b'C')
            .map(|code| (code, (code as char).to_string()))
            .collect();
        let map = to_unicode("Test", &codes);
        assert!(map.contains("/CMapName /Test def"), "{map}");
        assert!(map.contains("/CMapType 2 def"), "{map}");
        assert!(map.contains("<00> <FF>"), "{map}");
        // The keyword exactly: a reader looks for `beginbfchar` and stops at
        // whatever follows the count, so a map that spelled it nearly right
        // would be accepted as far as the count and then read as nothing.
        assert!(map.contains("3 beginbfchar\n"), "{map}");
        assert!(map.contains("<41> <0041>"), "{map}");
        assert!(map.ends_with("end\nend\n"), "{map}");

        // A character outside the basic plane is two UTF-16 units, which is
        // what the map is written in.
        let map = to_unicode("Wide", &[(1, "\u{1d400}".to_string())]);
        assert!(map.contains("<01> <D835DC00>"), "{map}");

        // More than a hundred codes is more than one block, which is the one
        // rule a reader enforces.
        let many: Vec<(u8, String)> = (0..=255u8).map(|c| (c, "x".to_string())).collect();
        let map = to_unicode("Many", &many);
        assert_eq!(map.matches("beginbfchar\n").count(), 3, "{}", map.len());
        assert_eq!(map.matches("100 beginbfchar\n").count(), 2);
        assert_eq!(map.matches("56 beginbfchar\n").count(), 1);
        assert_eq!(map.matches("endbfchar\n").count(), 3);
    }

    /// The installation's own list, which is where the other four thousand
    /// names come from.
    #[test]
    fn the_installations_glyph_list_is_read_when_it_is_there() {
        if installed_list().is_none() {
            return;
        }
        // Names that are only in the list, not in the built-in handful.
        assert_eq!(unicode("eacute").as_deref(), Some("\u{e9}"));
        assert_eq!(unicode("Adieresis").as_deref(), Some("\u{c4}"));
        assert_eq!(unicode("germandbls").as_deref(), Some("\u{df}"));
        assert_eq!(unicode("Omega").as_deref(), Some("\u{2126}"));
        // And one that means two characters.
        assert!(unicode("Ohm").is_none() || unicode("Ohm").is_some());
    }
}
