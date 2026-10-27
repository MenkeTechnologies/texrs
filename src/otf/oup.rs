//! `fontloader-font-oup.lua` and `fontloader-font-map.lua`: the glyphs keyed
//! by Unicode instead of by index, the code points a glyph shares with
//! others copied out, the ones a glyph has not got guessed from its name,
//! and each glyph's height and depth taken from its box.
//!
//! The fontloader iterates hash tables with `next` in several of these
//! passes; where the order could change an answer (two lookups giving one
//! private glyph two different Unicodes) the order here is ascending, which
//! is a determinism the Lua does not promise and is said so where it is used.

use std::collections::{BTreeMap, BTreeSet};

use super::otr::{FontData, Glyph, PRIVATE};

/// What a glyph stands for: luaotfload's `unicode` field is a number or a
/// list of them.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Unicode {
    One(u32),
    Many(Vec<u32>),
}

/// A `descriptions[u]` entry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Description {
    pub index: u32,
    pub name: Option<String>,
    pub unicode: Option<Unicode>,
    pub width: Option<f64>,
    pub boundingbox: Option<[f64; 4]>,
    pub class: Option<String>,
    pub height: Option<f64>,
    pub depth: Option<f64>,
    /// The MATH table's data for the glyph, its glyph references rekeyed
    /// like everything else (font-oup.lua:916-938).
    pub math: Option<super::dsp::MathGlyph>,
}

impl From<&Glyph> for Description {
    fn from(g: &Glyph) -> Description {
        Description {
            index: g.index,
            name: g.name.clone(),
            unicode: g.unicode.map(Unicode::One),
            width: g.width,
            boundingbox: g.boundingbox,
            class: g.class.clone(),
            height: None,
            depth: None,
            math: None,
        }
    }
}

const PUA_FIRST: u32 = 0xE000;
const PUA_LAST: u32 = 0xF8FF;

/// `unifyglyphs` (font-oup.lua:774-964), the `trace_unicodes == false`
/// branch: the description of each glyph goes under its own Unicode, or the
/// next private slot when it has none, sits in the plane-15 private area, or
/// a glyph before it already took that Unicode. Returns `indices`, glyph
/// index to its key, and the next free private slot.
pub fn unify_glyphs(fontdata: &mut FontData) -> (BTreeMap<u32, Description>, Vec<u32>, u32) {
    let mut private = PRIVATE;
    let mut descriptions: BTreeMap<u32, Description> = BTreeMap::new();
    if fontdata.glyphs.is_empty() {
        fontdata.glyph(0);
    }
    let mut indices = vec![0u32; fontdata.glyphs.len()];
    let zerocode = match fontdata.glyphs[0].unicode {
        Some(u) => u,
        None => {
            let z = private;
            fontdata.glyphs[0].unicode = Some(z);
            private += 1;
            z
        }
    };
    descriptions.insert(zerocode, Description::from(&fontdata.glyphs[0]));
    indices[0] = zerocode;
    // `nofglyphs = #glyphs`: the glyphs are dense from 0, so the border is
    // the last index.
    for (index, glyph) in fontdata.glyphs.iter().enumerate().skip(1) {
        let unicode = match glyph.unicode {
            None => {
                private += 1;
                private - 1
            }
            Some(u) if u >= PRIVATE => {
                private += 1;
                private - 1
            }
            Some(u) if (PUA_FIRST..=PUA_LAST).contains(&u) => u,
            Some(u) if descriptions.contains_key(&u) => {
                private += 1;
                private - 1
            }
            Some(u) => u,
        };
        indices[index] = unicode;
        descriptions.insert(unicode, Description::from(glyph));
    }
    (descriptions, indices, private)
}

/// `copyduplicates` (font-oup.lua:454-538) with `check_soft_hyphen` off, as
/// it is outside ConTeXt: every code point `cmap` gave a glyph after its
/// first gets a copy of that glyph's description, unless something else
/// already has the code point.
pub fn copy_duplicates(
    descriptions: &mut BTreeMap<u32, Description>,
    duplicates: &BTreeMap<u32, BTreeSet<u32>>,
) {
    for (u, d) in duplicates {
        let Some(du) = descriptions.get(u).cloned() else {
            continue;
        };
        for &v in d {
            descriptions.entry(v).or_insert_with(|| {
                let mut c = du.clone();
                c.unicode = Some(Unicode::One(v));
                c
            });
        }
    }
}

/// `readers.expand`'s first loop (font-oup.lua:3141-3170): a positive top of
/// the box is the height, a positive bottom below the baseline the depth;
/// anything else is left unset, which is how a hyphen ends up with no depth.
pub fn expand_dimensions(descriptions: &mut BTreeMap<u32, Description>, defaultwidth: f64) {
    for d in descriptions.values_mut() {
        if d.width.is_none() {
            d.width = Some(defaultwidth);
        }
        if let Some(bb) = d.boundingbox {
            let ht = bb[3];
            let dp = -bb[1];
            if ht > 0.0 {
                d.height = Some(ht);
            }
            if dp > 0.0 {
                d.depth = Some(dp);
            }
        }
    }
}

/// `p_crappyname` (font-oup.lua:976-1002): names the fontloader drops as
/// carrying no information (`uni0041`, `glyph123`, `a1`, `_x`, ...).
/// `p_done` always matches, so a prefix decides; the trie of
/// `utfchartabletopattern` takes the longest of its words and is never
/// retried shorter once a later element fails, which is why `uacute` (a `u`
/// and the hex `ac`) is one of them.
pub fn crappy_name(name: &str) -> bool {
    let b = name.as_bytes();
    let hex = |c: u8| c.is_ascii_hexdigit();
    let digit = |c: u8| c.is_ascii_digit();
    let count =
        |from: usize, f: &dyn Fn(u8) -> bool| b[from..].iter().take_while(|&&c| f(c)).count();
    let word = |from: usize, words: &[&str]| -> Option<usize> {
        // the longest word matching case-insensitively, as the trie does
        let mut best: Option<usize> = None;
        for w in words {
            let n = w.len();
            if b.len() >= from + n && b[from..from + n].eq_ignore_ascii_case(w.as_bytes()) {
                best = Some(best.map_or(n, |m: usize| m.max(n)));
            }
        }
        best
    };
    // `uni`/`u` * S"Xx_"^0 * hex^1
    if let Some(n) = word(0, &["uni", "u"]) {
        let mut at = n;
        at += b[at..]
            .iter()
            .take_while(|&&c| matches!(c, b'X' | b'x' | b'_'))
            .count();
        if count(at, &hex) >= 1 {
            return true;
        }
    }
    // `identity`/`glyph`/`jamo` * hex^1
    if let Some(n) = word(0, &["identity", "glyph", "jamo"]) {
        if count(n, &hex) >= 1 {
            return true;
        }
    }
    // `index`/`afii` * digit^1
    if let Some(n) = word(0, &["index", "afii"]) {
        if count(n, &digit) >= 1 {
            return true;
        }
    }
    // digit * hex^3
    if !b.is_empty() && digit(b[0]) && count(1, &hex) >= 3 {
        return true;
    }
    // alpha * digit^1
    if !b.is_empty() && b[0].is_ascii_alphabetic() && count(1, &digit) >= 1 {
        return true;
    }
    // "aj" * digit^1
    if b.starts_with(b"aj") && count(2, &digit) >= 1 {
        return true;
    }
    // "eh_" * (digit^1 + ALPHA * digit^1)
    if b.starts_with(b"eh_")
        && (count(3, &digit) >= 1
            || (b.len() > 3 && b[3].is_ascii_uppercase() && count(4, &digit) >= 1))
    {
        return true;
    }
    // (1-"_")^1 * "_uni" * hex^1: the greedy run stops at the first "_"
    let run = b.iter().take_while(|&&c| c != b'_').count();
    if run >= 1 && b[run..].starts_with(b"_uni") && count(run + 4, &hex) >= 1 {
        return true;
    }
    // "_" * P(1)^1
    b.len() >= 2 && b[0] == b'_'
}

/// `stripredundant` outside ConTeXt (font-oup.lua:1150-1173): drop the `base`
/// class, which is what an unclassed glyph is anyway, and -- unless
/// `keepnames` is on -- the uninformative names. luaotfload turns
/// `keepnames` on by default (luaotfload-configuration.lua:235, applied by
/// luaotfload-init.lua:554-557), so names normally stay.
pub fn strip_redundant(descriptions: &mut BTreeMap<u32, Description>, keepnames: bool) {
    for d in descriptions.values_mut() {
        if !keepnames && d.name.as_deref().is_some_and(crappy_name) {
            d.name = None;
        }
        if d.class.as_deref() == Some("base") {
            d.class = None;
        }
    }
}

/// `readers.addunicodetable` (font-oup.lua:1311-1327): name to Unicode.
pub fn unicode_table(descriptions: &BTreeMap<u32, Description>) -> BTreeMap<String, u32> {
    let mut unicodes = BTreeMap::new();
    for (&u, d) in descriptions {
        if let Some(n) = &d.name {
            unicodes.insert(n.clone(), u);
        }
    }
    unicodes
}

/// luaotfload's glyph list (`luaotfload-glyphlist.lua`, from the Adobe Glyph
/// List), which `fonts.encodings.agl.unicodes` is (luaotfload-init.lua:520):
/// read through kpse when it is installed, empty when it is not.
pub fn glyph_list() -> &'static BTreeMap<String, u32> {
    static LIST: std::sync::OnceLock<BTreeMap<String, u32>> = std::sync::OnceLock::new();
    LIST.get_or_init(|| {
        let mut list = BTreeMap::new();
        let Some(path) = crate::kpse::locate("luaotfload-glyphlist.lua") else {
            return list;
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return list;
        };
        // ` ["name"]=number,`
        for line in text.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("[\"") else {
                continue;
            };
            let Some((name, value)) = rest.split_once("\"]=") else {
                continue;
            };
            let value = value.trim_end_matches(',');
            if let Ok(v) = value.parse::<u32>() {
                list.insert(name.to_string(), v);
            }
        }
        list
    })
}

/// The overloads `addtounicode` consults first (`mappings.overloads`), which
/// luaotfload replaces with its own table (luaotfload-tounicode.lua): glyph
/// names whose Unicode the Adobe list gets wrong or does not have -- small
/// capitals, old-style figures, `dotlessj` -- and, from ConTeXt, the
/// ligatures, under their names and their presentation-form code points.
fn overload(key_name: Option<&str>, key_code: Option<u32>) -> Option<Unicode> {
    enum O {
        One(u32),
        Many(&'static [u32]),
    }
    use O::*;
    let by_name = |n: &str| -> Option<O> {
        Some(match n {
            "longst" => Many(&[0x017F, 0x0074]),
            "st" => Many(&[0x0073, 0x0074]),
            "AEsmall" => One(0x00E6),
            "Aacutesmall" => One(0x00E1),
            "Acircumflexsmall" => One(0x00E2),
            "Acute" => One(0x00B4),
            "Acutesmall" => One(0x00B4),
            "Adieresissmall" => One(0x00E4),
            "Agravesmall" => One(0x00E0),
            "Aringsmall" => One(0x00E5),
            "Asmall" => One(0x0061),
            "Atildesmall" => One(0x00E3),
            "Brevesmall" => One(0x02D8),
            "Bsmall" => One(0x0062),
            "Caron" => One(0x02C7),
            "Caronsmall" => One(0x02C7),
            "Ccedillasmall" => One(0x00E7),
            "Cedillasmall" => One(0x00B8),
            "Circumflexsmall" => One(0x02C6),
            "Csmall" => One(0x0063),
            "Dieresis" => One(0x00A8),
            "DieresisAcute" => One(0xF6CC),
            "DieresisGrave" => One(0xF6CD),
            "Dieresissmall" => One(0x00A8),
            "Dotaccentsmall" => One(0x02D9),
            "Dsmall" => One(0x0064),
            "Eacutesmall" => One(0x00E9),
            "Ecircumflexsmall" => One(0x00EA),
            "Edieresissmall" => One(0x00EB),
            "Egravesmall" => One(0x00E8),
            "Esmall" => One(0x0065),
            "Ethsmall" => One(0x00F0),
            "Fsmall" => One(0x0066),
            "Grave" => One(0x0060),
            "Gravesmall" => One(0x0060),
            "Gsmall" => One(0x0067),
            "Hsmall" => One(0x0068),
            "Hungarumlaut" => One(0x02DD),
            "Hungarumlautsmall" => One(0x02DD),
            "Iacutesmall" => One(0x00ED),
            "Icircumflexsmall" => One(0x00EE),
            "Idieresissmall" => One(0x00EF),
            "Igravesmall" => One(0x00EC),
            "Ismall" => One(0x0069),
            "Jsmall" => One(0x006A),
            "Ksmall" => One(0x006B),
            "LL" => Many(&[0x004C, 0x004C]),
            "Lslashsmall" => One(0x0142),
            "Lsmall" => One(0x006C),
            "Macron" => One(0x00AF),
            "Macronsmall" => One(0x00AF),
            "Msmall" => One(0x006D),
            "Nsmall" => One(0x006E),
            "Ntildesmall" => One(0x00F1),
            "OEsmall" => One(0x0153),
            "Oacutesmall" => One(0x00F3),
            "Ocircumflexsmall" => One(0x00F4),
            "Odieresissmall" => One(0x00F6),
            "Ogoneksmall" => One(0x02DB),
            "Ogravesmall" => One(0x00F2),
            "Oslashsmall" => One(0x00F8),
            "Osmall" => One(0x006F),
            "Otildesmall" => One(0x00F5),
            "Psmall" => One(0x0070),
            "Qsmall" => One(0x0071),
            "Ringsmall" => One(0x02DA),
            "Rsmall" => One(0x0072),
            "Scaronsmall" => One(0x0161),
            "Ssmall" => One(0x0073),
            "Thornsmall" => One(0x00FE),
            "Tildesmall" => One(0x02DC),
            "Tsmall" => One(0x0074),
            "Uacutesmall" => One(0x00FA),
            "Ucircumflexsmall" => One(0x00FB),
            "Udieresissmall" => One(0x00FC),
            "Ugravesmall" => One(0x00F9),
            "Usmall" => One(0x0075),
            "Vsmall" => One(0x0076),
            "Wsmall" => One(0x0077),
            "Xsmall" => One(0x0078),
            "Yacutesmall" => One(0x00FD),
            "Ydieresissmall" => One(0x00FF),
            "Ysmall" => One(0x0079),
            "Zcaronsmall" => One(0x017E),
            "Zsmall" => One(0x007A),
            "ampersandsmall" => One(0x0026),
            "asuperior" => One(0x0061),
            "bsuperior" => One(0x0062),
            "centinferior" => One(0x00A2),
            "centoldstyle" => One(0x00A2),
            "centsuperior" => One(0x00A2),
            "commainferior" => One(0x002C),
            "commasuperior" => One(0x002C),
            "copyrightsans" => One(0x00A9),
            "copyrightserif" => One(0x00A9),
            "cyrBreve" => One(0x02D8),
            "cyrFlex" => Many(&[0x00A0, 0x0311]),
            "cyrbreve" => One(0x02D8),
            "cyrflex" => Many(&[0x00A0, 0x0311]),
            "dblGrave" => Many(&[0x00A0, 0x030F]),
            "dblgrave" => Many(&[0x00A0, 0x030F]),
            "dieresisacute" => Many(&[0x00A0, 0x0308, 0x0301]),
            "dieresisgrave" => Many(&[0x00A0, 0x0308, 0x0300]),
            "dollarinferior" => One(0x0024),
            "dollaroldstyle" => One(0x0024),
            "dollarsuperior" => One(0x0024),
            "dotlessj" => One(0x0237),
            "dsuperior" => One(0x0064),
            "eightoldstyle" => One(0x0038),
            "esuperior" => One(0x0065),
            "exclamdownsmall" => One(0x00A1),
            "exclamsmall" => One(0x0021),
            "fiveoldstyle" => One(0x0035),
            "fouroldstyle" => One(0x0034),
            "hypheninferior" => One(0x002D),
            "hyphensuperior" => One(0x002D),
            "isuperior" => One(0x0069),
            "ll" => Many(&[0x006C, 0x006C]),
            "lsuperior" => One(0x006C),
            "msuperior" => One(0x006D),
            "nineoldstyle" => One(0x0039),
            "onefitted" => One(0x0031),
            "oneoldstyle" => One(0x0031),
            "osuperior" => One(0x006F),
            "periodinferior" => One(0x002E),
            "periodsuperior" => One(0x002E),
            "questiondownsmall" => One(0x00BF),
            "questionsmall" => One(0x003F),
            "registersans" => One(0x00AE),
            "registerserif" => One(0x00AE),
            "rsuperior" => One(0x0072),
            "rupiah" => One(0x20A8),
            "sevenoldstyle" => One(0x0037),
            "sixoldstyle" => One(0x0036),
            "ssuperior" => One(0x0073),
            "threeoldstyle" => One(0x0033),
            "trademarksans" => One(0x2122),
            "trademarkserif" => One(0x2122),
            "tsuperior" => One(0x0074),
            "twooldstyle" => One(0x0032),
            "zerooldstyle" => One(0x0030),
            "FFsmall" => Many(&[0x0066, 0x0066]),
            "FFIsmall" => Many(&[0x0066, 0x0066, 0x0069]),
            "FFLsmall" => Many(&[0x0066, 0x0066, 0x006C]),
            "FIsmall" => Many(&[0x0066, 0x0069]),
            "FLsmall" => Many(&[0x0066, 0x006C]),
            "Germandblssmall" => Many(&[0x0073, 0x0073]),
            "SSsmall" => Many(&[0x0073, 0x0073]),
            "IJ" | "I_J" => Many(&[0x49, 0x4A]),
            "ij" | "i_j" => Many(&[0x69, 0x6A]),
            "ff" | "f_f" => Many(&[0x66, 0x66]),
            "fi" | "f_i" => Many(&[0x66, 0x69]),
            "fl" | "f_l" => Many(&[0x66, 0x6C]),
            "ffi" | "f_f_i" => Many(&[0x66, 0x66, 0x69]),
            "ffl" | "f_f_l" => Many(&[0x66, 0x66, 0x6C]),
            "fj" | "f_j" => Many(&[0x66, 0x6A]),
            "fk" | "f_k" => Many(&[0x66, 0x6B]),
            _ => return None,
        })
    };
    let by_code = |c: u32| -> Option<O> {
        Some(match c {
            0x0132 => Many(&[0x49, 0x4A]),
            0x0133 => Many(&[0x69, 0x6A]),
            0xFB00 => Many(&[0x66, 0x66]),
            0xFB01 => Many(&[0x66, 0x69]),
            0xFB02 => Many(&[0x66, 0x6C]),
            0xFB03 => Many(&[0x66, 0x66, 0x69]),
            0xFB04 => Many(&[0x66, 0x66, 0x6C]),
            _ => return None,
        })
    };
    key_name
        .and_then(by_name)
        .or_else(|| key_code.and_then(by_code))
        .map(|o| match o {
            One(u) => Unicode::One(u),
            Many(l) => Unicode::Many(l.to_vec()),
        })
}

/// `lpegmatch(namesplitter, name)` (font-map.lua:170-172): the parts of a
/// name between underscores, stopping at the first period.
fn split_name(name: &str) -> Vec<&str> {
    let base = name.split('.').next().unwrap_or("");
    let mut parts = Vec::new();
    for (k, p) in base.split('_').enumerate() {
        if p.is_empty() {
            // `C((1 - sep)^1)` needs at least one character: an empty part
            // ends the capture there (and none at all if it is the first).
            if k == 0 {
                return Vec::new();
            }
            break;
        }
        parts.push(p);
    }
    parts
}

/// The `uni`/`u`/`index` name parser (font-map.lua:33-42): one code point
/// (`uni0041`, `u1D400`) or several (`uni00410042`), or an index.
fn parse_uni_name(name: &str) -> Option<(Unicode, bool)> {
    let hex_run = |s: &str, min: usize, max: usize| -> Option<(u32, usize)> {
        let n = s
            .bytes()
            .take_while(|c| c.is_ascii_hexdigit())
            .count()
            .min(max);
        (n >= min).then(|| (u32::from_str_radix(&s[..n], 16).unwrap_or(0), n))
    };
    let parse = |rest: &str, min: usize, max: usize| -> Option<(Unicode, bool)> {
        // `hexfour * (period + P(-1)) * Cc(false)`
        if let Some((v, n)) = hex_run(rest, min, max) {
            let after = &rest[n..];
            if after.is_empty() || after.starts_with('.') {
                return Some((Unicode::One(v), false));
            }
        }
        // `Ct(hexfour^1) * Cc(true)`: as many groups as there are
        let mut list = Vec::new();
        let mut s = rest;
        while let Some((v, n)) = hex_run(s, min, max) {
            list.push(v);
            s = &s[n..];
        }
        (!list.is_empty()).then_some((Unicode::Many(list), true))
    };
    if let Some(rest) = name
        .strip_prefix("uni")
        .or_else(|| name.strip_prefix("UNI"))
    {
        return parse(rest, 2, 4);
    }
    if let Some(rest) = name.strip_prefix('u').or_else(|| name.strip_prefix('U')) {
        return parse(rest, 2, 6);
    }
    if let Some(rest) = name.strip_prefix("index") {
        let n = rest.bytes().take_while(|c| c.is_ascii_digit()).count();
        if n > 0 {
            return Some((Unicode::One(rest[..n].parse().unwrap_or(0)), false));
        }
    }
    None
}

/// `mappings.addtounicode` (font-map.lua:226-504) over descriptions keyed by
/// Unicode, with `unicodes` the name table `unifymissing` builds and `lookups`
/// the GSUB data `checklookups` and `getcomponents` read.
pub fn add_tounicode(
    descriptions: &mut BTreeMap<u32, Description>,
    unicodes: &mut BTreeMap<String, u32>,
    lookups: &Lookups,
) {
    unicodes.entry("space".into()).or_insert(32);
    unicodes.entry("hyphen".into()).or_insert(45);
    unicodes.entry("zwj".into()).or_insert(0x200D);
    unicodes.entry("zwnj".into()).or_insert(0x200C);
    let agl = glyph_list();
    let mut missing: BTreeMap<u32, bool> = BTreeMap::new();
    let mut nofmissing: i64 = 0;
    let keys: Vec<u32> = descriptions.keys().copied().collect();
    for &du in &keys {
        let glyph = descriptions.get_mut(&du).expect("listed");
        let Some(name) = glyph.name.clone() else {
            if let Some(o) = overload(None, Some(du)) {
                glyph.unicode = Some(o);
            } else if glyph.unicode.is_none() {
                missing.insert(du, true);
                nofmissing += 1;
            }
            continue;
        };
        if let Some(o) = overload(Some(&name), Some(du)) {
            glyph.unicode = Some(o);
            continue;
        }
        let gu = glyph.unicode.clone();
        let wants = gu.is_none()
            || du >= PRIVATE
            || (PUA_FIRST..=PUA_LAST).contains(&du)
            || du == 0xFFFE
            || du == 0xFFFF;
        if !wants {
            continue;
        }
        let mut unicode: Option<Unicode> = agl.get(&name).copied().map(Unicode::One);
        if let Some(u) = &unicode {
            glyph.unicode = Some(u.clone());
        }
        // The CID map heuristics need a cidmap file (font-cid.lua), and a
        // CID font's glyphs are named by number: nothing to resolve by name.
        if unicode.is_none() {
            let split = split_name(&name);
            if split.len() == 1 {
                let base = split[0];
                let u = unicodes
                    .get(base)
                    .copied()
                    .or_else(|| agl.get(base).copied());
                if let Some(u) = u.filter(|u| *u < PRIVATE) {
                    unicode = Some(Unicode::One(u));
                    glyph.unicode = unicode.clone();
                }
            } else if split.len() > 1 {
                let mut t = Vec::new();
                for base in &split {
                    match unicodes
                        .get(*base)
                        .copied()
                        .or_else(|| agl.get(*base).copied())
                    {
                        Some(u) if u < PRIVATE => t.push(u),
                        _ => break,
                    }
                }
                if !t.is_empty() {
                    unicode = Some(match t.len() {
                        1 => Unicode::One(t[0]),
                        _ => Unicode::Many(t),
                    });
                    glyph.unicode = unicode.clone();
                }
            }
        }
        if unicode.is_none() {
            if let Some((found, _multiple)) = parse_uni_name(&name) {
                glyph.unicode = Some(found.clone());
                unicode = Some(found);
            }
        }
        if let Some(Unicode::One(c)) = &unicode {
            if let Some(o) = overload(None, Some(*c)) {
                unicode = Some(o.clone());
                glyph.unicode = Some(o);
            }
        }
        if unicode.is_none() {
            missing.insert(du, true);
            nofmissing += 1;
        }
    }
    check_lookups(descriptions, &mut missing, &mut nofmissing, lookups);
    // getcomponents: a ligature glyph without a Unicode stands for its parts.
    let collected = components(lookups);
    for &du in &keys {
        if du >= PRIVATE || (PUA_FIRST..=PUA_LAST).contains(&du) {
            let glyph = descriptions.get_mut(&du).expect("listed");
            if glyph.class.as_deref() == Some("ligature") && glyph.unicode.is_none() {
                if let Some(u) = collected.get(&du) {
                    if !u.is_empty() && u.iter().all(|c| *c <= PRIVATE) {
                        glyph.unicode = Some(match u.len() {
                            1 => Unicode::One(u[0]),
                            _ => Unicode::Many(u.clone()),
                        });
                    }
                }
            }
        }
    }
}

/// The GSUB lookups the Unicode guessing reads: single substitutions,
/// alternates and the ligature trees, keyed by description (Unicode) after
/// `unifyresources`.
#[derive(Clone, Debug, Default)]
pub struct Lookups {
    pub singles: Vec<BTreeMap<u32, u32>>,
    pub alternates: Vec<BTreeMap<u32, Vec<u32>>>,
    /// Each ligature lookup's tree: first component -> rest.
    pub ligatures: Vec<BTreeMap<u32, LigatureTree>>,
}

/// A ligature tree node: the ligature ending here, and the components that
/// may follow.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LigatureTree {
    pub ligature: Option<u32>,
    pub next: BTreeMap<u32, LigatureTree>,
}

fn unicode_of(descriptions: &BTreeMap<u32, Description>, g: u32) -> Option<Unicode> {
    descriptions.get(&g).and_then(|d| d.unicode.clone())
}

/// `checklookups` (font-oup.lua:549-742).
fn check_lookups(
    descriptions: &mut BTreeMap<u32, Description>,
    missing: &mut BTreeMap<u32, bool>,
    nofmissing: &mut i64,
    lookups: &Lookups,
) {
    if *nofmissing <= 0 {
        return;
    }
    let is_missing = |m: &BTreeMap<u32, bool>, g: u32| m.get(&g).copied().unwrap_or(false);
    loop {
        let old = *nofmissing;
        for c in &lookups.singles {
            // `next` order in the Lua; ascending here.
            for (&g1, &g2) in c {
                if is_missing(missing, g1) {
                    if let Some(u2) = unicode_of(descriptions, g2) {
                        missing.insert(g1, false);
                        if let Some(d) = descriptions.get_mut(&g1) {
                            d.unicode = Some(u2);
                        }
                        *nofmissing -= 1;
                    }
                }
                if is_missing(missing, g2) {
                    if let Some(u1) = unicode_of(descriptions, g1) {
                        missing.insert(g2, false);
                        if let Some(d) = descriptions.get_mut(&g2) {
                            d.unicode = Some(u1);
                        }
                        *nofmissing -= 1;
                    }
                }
            }
        }
        for c in &lookups.alternates {
            for (&g1, d1) in c {
                if is_missing(missing, g1) {
                    for &g2 in d1 {
                        if let Some(u2) = unicode_of(descriptions, g2) {
                            missing.insert(g1, false);
                            if let Some(d) = descriptions.get_mut(&g1) {
                                d.unicode = Some(u2);
                            }
                            *nofmissing -= 1;
                        }
                    }
                }
                if !is_missing(missing, g1) {
                    for &g2 in d1 {
                        if is_missing(missing, g2) {
                            if let Some(u1) = unicode_of(descriptions, g1) {
                                missing.insert(g2, false);
                                if let Some(d) = descriptions.get_mut(&g2) {
                                    d.unicode = Some(u1);
                                }
                                *nofmissing -= 1;
                            }
                        }
                    }
                }
            }
        }
        if *nofmissing <= 0 {
            return;
        }
        if old == *nofmissing {
            break;
        }
    }
    // The ligature trees: a missing ligature glyph stands for the Unicodes
    // of the path to it.
    fn recurse(
        tree: &BTreeMap<u32, LigatureTree>,
        path: &mut Vec<Unicode>,
        descriptions: &mut BTreeMap<u32, Description>,
        missing: &mut BTreeMap<u32, bool>,
        nofmissing: &mut i64,
    ) {
        for (&g, node) in tree {
            let Some(u) = unicode_of(descriptions, g) else {
                continue;
            };
            path.push(u);
            if let Some(lig) = node.ligature {
                if missing.get(&lig).copied().unwrap_or(false) {
                    let mut l = Vec::new();
                    for u in path.iter() {
                        match u {
                            Unicode::One(c) => l.push(*c),
                            Unicode::Many(cs) => l.extend(cs),
                        }
                    }
                    missing.insert(lig, false);
                    if let Some(d) = descriptions.get_mut(&lig) {
                        d.unicode = Some(Unicode::Many(l));
                    }
                    *nofmissing -= 1;
                }
            }
            recurse(&node.next, path, descriptions, missing, nofmissing);
            path.pop();
        }
    }
    loop {
        let old = *nofmissing;
        for tree in &lookups.ligatures {
            let mut path = Vec::new();
            recurse(tree, &mut path, descriptions, missing, nofmissing);
        }
        if *nofmissing <= 0 || old == *nofmissing {
            break;
        }
    }
}

/// `readers.getcomponents` (font-oup.lua:1181-1271): each ligature glyph to
/// the glyphs it is made of, expanded through ligatures of ligatures, with a
/// glyph that contains itself dropped.
fn components(lookups: &Lookups) -> BTreeMap<u32, Vec<u32>> {
    let mut collected: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    fn walk(
        tree: &BTreeMap<u32, LigatureTree>,
        path: &mut Vec<u32>,
        out: &mut BTreeMap<u32, Vec<u32>>,
    ) {
        for (&k, node) in tree {
            path.push(k);
            if let Some(lig) = node.ligature {
                out.insert(lig, path.clone());
            }
            walk(&node.next, path, out);
            path.pop();
        }
    }
    for tree in &lookups.ligatures {
        let mut path = Vec::new();
        walk(tree, &mut path, &mut collected);
    }
    loop {
        let mut done = false;
        let keys: Vec<u32> = collected.keys().copied().collect();
        for k in keys {
            let Some(v) = collected.get(&k).cloned() else {
                continue;
            };
            for (i, &vi) in v.iter().enumerate() {
                if vi == k {
                    collected.remove(&k);
                    break;
                }
                if let Some(c) = collected.get(&vi).cloned() {
                    done = true;
                    let mut t: Vec<u32> = v[..i].to_vec();
                    t.extend(c);
                    t.extend_from_slice(&v[i + 1..]);
                    collected.insert(k, t);
                    break;
                }
            }
        }
        if !done {
            break;
        }
    }
    collected
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pattern decides on a prefix, and a `u` followed by any hex digit
    /// is enough -- including the `ac` of `uacute`.
    #[test]
    fn uninformative_names() {
        for n in [
            "uni0041",
            "u1D400",
            "uniform",
            "uacute",
            "glyph123",
            "index12",
            "afii10017",
            "a1",
            "1abc",
            "_part",
            "foo_uni0041",
            "eh_12",
        ] {
            assert!(crappy_name(n), "{n} should be dropped");
        }
        for n in ["A", "fi", "f_f_i", "union", "eacute", "space", "_", "Aring"] {
            assert!(!crappy_name(n), "{n} should be kept");
        }
    }

    #[test]
    fn name_parts_and_uni_names() {
        assert_eq!(split_name("f_f_i.liga"), vec!["f", "f", "i"]);
        assert_eq!(split_name("a.sc"), vec!["a"]);
        assert!(split_name("_x").is_empty());
        assert_eq!(parse_uni_name("uni0041"), Some((Unicode::One(0x41), false)));
        assert_eq!(
            parse_uni_name("uni00410042"),
            Some((Unicode::Many(vec![0x41, 0x42]), true))
        );
        assert_eq!(
            parse_uni_name("u1D400.alt"),
            Some((Unicode::One(0x1D400), false))
        );
        assert_eq!(parse_uni_name("index42"), Some((Unicode::One(42), false)));
        assert_eq!(parse_uni_name("eacute"), None);
    }

    /// A glyph a duplicate code point reaches is copied under it; the
    /// second glyph to claim a Unicode moves to a private slot.
    #[test]
    fn unicode_keys_and_duplicates() {
        let mut fd = FontData::default();
        fd.glyph(0);
        fd.glyph(1).unicode = Some(0x20);
        fd.glyph(2).unicode = Some(0x41);
        fd.glyph(3).unicode = Some(0x41);
        fd.glyph(4);
        let (mut d, indices, private) = unify_glyphs(&mut fd);
        assert_eq!(indices, vec![PRIVATE, 0x20, 0x41, PRIVATE + 1, PRIVATE + 2]);
        assert_eq!(private, PRIVATE + 3);
        let mut dups = BTreeMap::new();
        dups.insert(0x20, BTreeSet::from([0xA0]));
        copy_duplicates(&mut d, &dups);
        assert_eq!(d[&0xA0].index, 1);
        assert_eq!(d[&0xA0].unicode, Some(Unicode::One(0xA0)));
    }
}

// ---------------------------------------------------------------------------
// The lookups: compact (before the rehash), unify (rekey by Unicode) and
// expand (after loading).
// ---------------------------------------------------------------------------

use super::dsp::{Coverage, Layout, LigTree, Pos, Rule, Sequence, Set};

/// `mergesteps_1` (font-oup.lua:2488-2521): a key the first step lacks is
/// taken from the later steps, first come first served. `strict` refuses
/// steps of different formats.
fn mergesteps_1(lookup: &mut Sequence, strict: bool) -> usize {
    let nofsteps = lookup.steps.len();
    if strict {
        let f = lookup.steps[0].format;
        if lookup.steps.iter().skip(1).any(|s| s.format != f) {
            return 0;
        }
    }
    let rest: Vec<Coverage> = lookup.steps.drain(1..).map(|s| s.coverage).collect();
    let target = &mut lookup.steps[0].coverage;
    for c in rest {
        match (target as &mut Coverage, c) {
            (Coverage::Single(t), Coverage::Single(c)) => {
                for (k, v) in c {
                    t.entry(k).or_insert(v);
                }
            }
            (Coverage::Sequence(t), Coverage::Sequence(c)) => {
                for (k, v) in c {
                    t.entry(k).or_insert(v);
                }
            }
            (Coverage::Single1(t), Coverage::Single1(c)) => {
                // `if not target[k]`: a false value is replaced too
                for (k, v) in c {
                    match t.get(&k) {
                        Some(Some(_)) => {}
                        _ => {
                            t.insert(k, v);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    lookup.merged = true;
    nofsteps - 1
}

/// `mergesteps_2` (font-oup.lua:2523-2566): pairs, per first glyph.
fn mergesteps_2(lookup: &mut Sequence) -> usize {
    let nofsteps = lookup.steps.len();
    let rest: Vec<Coverage> = lookup.steps.drain(1..).map(|s| s.coverage).collect();
    if let Coverage::Pair(t) = &mut lookup.steps[0].coverage {
        for c in rest {
            if let Coverage::Pair(c) = c {
                for (k, v) in c {
                    match t.get_mut(&k) {
                        Some(tk) => {
                            for (kk, vv) in v {
                                tk.entry(kk).or_insert(vv);
                            }
                        }
                        None => {
                            t.insert(k, v);
                        }
                    }
                }
            }
        }
    }
    lookup.merged = true;
    nofsteps - 1
}

/// `mergesteps_3` (font-oup.lua:2571-2614): marks, refused when two steps
/// cover one mark; the classes are renumbered `10*step + class`.
fn mergesteps_3(lookup: &mut Sequence) -> usize {
    let nofsteps = lookup.steps.len();
    let mut seen = BTreeSet::new();
    for s in &lookup.steps {
        if let Coverage::Mark(c) = &s.coverage {
            for k in c.keys() {
                if !seen.insert(*k) {
                    return 0;
                }
            }
        }
    }
    let mut coverage = BTreeMap::new();
    let mut baseclasses = BTreeMap::new();
    for (i, step) in lookup.steps.iter().enumerate() {
        let offset = (i + 1) * 10;
        if let Some(b) = &step.baseclasses {
            for (k, v) in b {
                baseclasses.insert(offset + k, v.clone());
            }
        }
        if let Coverage::Mark(c) = &step.coverage {
            for (k, v) in c {
                let mut v = v.clone();
                v.class += offset;
                coverage.insert(*k, v);
            }
        }
    }
    lookup.steps.truncate(1);
    lookup.steps[0].coverage = Coverage::Mark(coverage);
    lookup.steps[0].baseclasses = Some(baseclasses);
    lookup.merged = true;
    nofsteps - 1
}

/// `nested` (font-oup.lua:2616-2631): merge an old ligature tree into new.
fn nested(old: LigTree, new: &mut LigTree) {
    if new.ligature.is_none() {
        new.ligature = old.ligature;
    }
    for (k, v) in old.next {
        match new.next.get_mut(&k) {
            Some(n) => nested(v, n),
            None => {
                new.next.insert(k, v);
            }
        }
    }
}

/// `mergesteps_4` (font-oup.lua:2633-2657): ligatures. (This one leaves
/// `merged` unset; `compact` sets it from the step count.)
fn mergesteps_4(lookup: &mut Sequence) -> usize {
    let nofsteps = lookup.steps.len();
    let rest: Vec<Coverage> = lookup.steps.drain(1..).map(|s| s.coverage).collect();
    if let Coverage::Ligature(t) = &mut lookup.steps[0].coverage {
        for c in rest {
            if let Coverage::Ligature(c) = c {
                for (k, v) in c {
                    match t.get_mut(&k) {
                        Some(tk) => nested(v, tk),
                        None => {
                            t.insert(k, v);
                        }
                    }
                }
            }
        }
    }
    nofsteps - 1
}

/// `mergesteps_5` (font-oup.lua:2663-2699): cursive entries and exits.
fn mergesteps_5(lookup: &mut Sequence) -> usize {
    let nofsteps = lookup.steps.len();
    let rest: Vec<Coverage> = lookup.steps.drain(1..).map(|s| s.coverage).collect();
    if let Coverage::Cursive(t) = &mut lookup.steps[0].coverage {
        for c in rest {
            if let Coverage::Cursive(c) = c {
                for (k, v) in c {
                    match t.get_mut(&k) {
                        Some(tk) => {
                            if tk.entry.is_none() {
                                tk.entry = v.entry;
                            }
                            if tk.exit.is_none() {
                                tk.exit = v.exit;
                            }
                        }
                        None => {
                            t.insert(k, v);
                        }
                    }
                }
            }
        }
    }
    lookup.merged = true;
    nofsteps - 1
}

/// `checkkerns` (font-oup.lua:2701-2737) would turn a single adjustment that
/// only moves the advance into a `move` table, but it tests
/// `step.format == "pair"` on a gpos_single lookup, whose steps are formatted
/// "single" (font-dsp.lua:1466), so it never changes anything.
fn checkkerns(_lookup: &mut Sequence) {}

/// `checkpairs` (font-oup.lua:2781-2833): a pair table that only moves the
/// first glyph's advance becomes a `move` table of those advances.
fn checkpairs(lookup: &mut Sequence) {
    for step in lookup.steps.iter_mut() {
        if step.format != Some("pair") {
            continue;
        }
        let Coverage::Pair(c) = &step.coverage else {
            continue;
        };
        let only = c.values().all(|d1| {
            d1.values().all(|d2| {
                if d2.second.is_some() {
                    return false;
                }
                match &d2.first {
                    Some(Pos::V(v)) => v[0] == 0.0 && v[1] == 0.0 && v[3] == 0.0,
                    _ => true,
                }
            })
        });
        if !only {
            continue;
        }
        let mut moved = BTreeMap::new();
        for (g1, d1) in c {
            let mut d = BTreeMap::new();
            for (g2, d2) in d1 {
                if let Some(Pos::V(v)) = &d2.first {
                    d.insert(*g2, v[2]);
                }
            }
            moved.insert(*g1, d);
        }
        step.coverage = Coverage::Move2(moved);
        step.format = Some("move");
    }
}

/// `strippairs` (font-oup.lua:2835-2867): a pair whose first record is all
/// zero and has no second record does nothing and goes.
fn strippairs(lookup: &mut Sequence) {
    for step in lookup.steps.iter_mut() {
        if step.format != Some("pair") {
            continue;
        }
        if let Coverage::Pair(c) = &mut step.coverage {
            for d1 in c.values_mut() {
                d1.retain(|_, d2| !(d2.second.is_none() && d2.first == Some(Pos::Zero)));
            }
        }
    }
}

/// `readers.compact` (font-oup.lua:2869-2969).
pub fn compact(layout: &mut Layout) {
    for list in [&mut layout.sequences, &mut layout.sublookups] {
        for lookup in list.iter_mut() {
            let nofsteps = lookup.steps.len();
            if nofsteps > 1 {
                let merged = match lookup.typ {
                    "gsub_single" | "gsub_alternate" | "gsub_multiple" => {
                        mergesteps_1(lookup, false)
                    }
                    "gsub_ligature" => mergesteps_4(lookup),
                    "gpos_single" => {
                        let m = mergesteps_1(lookup, true);
                        checkkerns(lookup);
                        m
                    }
                    "gpos_pair" => {
                        strippairs(lookup);
                        let m = mergesteps_2(lookup);
                        checkpairs(lookup);
                        m
                    }
                    "gpos_cursive" => mergesteps_5(lookup),
                    "gpos_mark2mark" | "gpos_mark2base" | "gpos_mark2ligature" => {
                        mergesteps_3(lookup)
                    }
                    _ => 0,
                };
                if merged > 0 {
                    lookup.merged = true;
                }
            } else if nofsteps == 1 {
                match lookup.typ {
                    "gpos_single" => checkkerns(lookup),
                    "gpos_pair" => checkpairs(lookup),
                    _ => {}
                }
            }
        }
    }
}

/// `unifyresources` (font-oup.lua:74-452): every glyph index in the lookups
/// becomes the Unicode (or private slot) its description is keyed by, with a
/// duplicate code point getting the same substitutions and adjustments as
/// its primary for the lookup types the Lua extends (single, alternate,
/// multiple, single positioning, cursive).
pub fn unify_resources(
    layout: &mut Layout,
    indices: &[u32],
    duplicates: &BTreeMap<u32, BTreeSet<u32>>,
) {
    let ix = |g: u32| -> Option<u32> { indices.get(g as usize).copied() };
    layout.marks = layout.marks.iter().filter_map(|&g| ix(g)).collect();
    for set in layout.markclasses.values_mut() {
        *set = set.iter().filter_map(|&g| ix(g)).collect();
    }
    let dups = |u: u32| -> Vec<u32> {
        duplicates
            .get(&u)
            .map(|d| d.iter().copied().collect())
            .unwrap_or_default()
    };
    let reset = |s: &Set| -> Set { s.iter().filter_map(|&g| ix(g)).collect() };
    fn retree(t: LigTree, ix: &dyn Fn(u32) -> Option<u32>) -> LigTree {
        LigTree {
            ligature: t.ligature.and_then(ix),
            next: t
                .next
                .into_iter()
                .filter_map(|(g, d)| Some((ix(g)?, retree(d, ix))))
                .collect(),
        }
    }
    for list in [&mut layout.sequences, &mut layout.sublookups] {
        for seq in list.iter_mut() {
            let kind = seq.typ;
            for step in seq.steps.iter_mut() {
                let cov = std::mem::replace(&mut step.coverage, Coverage::None);
                step.coverage = match (kind, cov) {
                    ("gsub_single", Coverage::Single(c)) => {
                        let mut t = BTreeMap::new();
                        for (g1, d1) in c {
                            let (Some(ug1), Some(ud1)) = (ix(g1), ix(d1)) else {
                                continue;
                            };
                            t.insert(ug1, ud1);
                            for u in dups(ug1) {
                                t.insert(u, ud1);
                            }
                        }
                        Coverage::Single(t)
                    }
                    ("gpos_pair", Coverage::Pair(c)) => Coverage::Pair(
                        c.into_iter()
                            .filter_map(|(g1, d1)| {
                                Some((
                                    ix(g1)?,
                                    d1.into_iter()
                                        .filter_map(|(g2, v)| Some((ix(g2)?, v)))
                                        .collect(),
                                ))
                            })
                            .collect(),
                    ),
                    ("gpos_pair", Coverage::Move2(c)) => Coverage::Move2(
                        c.into_iter()
                            .filter_map(|(g1, d1)| {
                                Some((
                                    ix(g1)?,
                                    d1.into_iter()
                                        .filter_map(|(g2, v)| Some((ix(g2)?, v)))
                                        .collect(),
                                ))
                            })
                            .collect(),
                    ),
                    ("gpos_pair", Coverage::Kern(c)) => Coverage::Kern(
                        c.into_iter()
                            .filter_map(|(g1, d1)| {
                                Some((
                                    ix(g1)?,
                                    d1.into_iter()
                                        .filter_map(|(g2, v)| Some((ix(g2)?, v)))
                                        .collect(),
                                ))
                            })
                            .collect(),
                    ),
                    ("gsub_ligature", Coverage::Ligature(c)) => Coverage::Ligature(
                        c.into_iter()
                            .filter_map(|(g, d)| Some((ix(g)?, retree(d, &ix))))
                            .collect(),
                    ),
                    ("gsub_alternate" | "gsub_multiple", Coverage::Sequence(c)) => {
                        let mut t = BTreeMap::new();
                        for (g1, d1) in c {
                            let d1: Vec<u32> = d1.iter().map(|&g| ix(g).unwrap_or(g)).collect();
                            let Some(ug1) = ix(g1) else {
                                continue;
                            };
                            t.insert(ug1, d1.clone());
                            for u in dups(ug1) {
                                t.insert(u, d1.clone());
                            }
                        }
                        Coverage::Sequence(t)
                    }
                    ("gpos_single", Coverage::Single1(c)) => {
                        let mut t = BTreeMap::new();
                        for (g1, d1) in c {
                            let Some(ug1) = ix(g1) else {
                                continue;
                            };
                            t.insert(ug1, d1.clone());
                            for u in dups(ug1) {
                                t.insert(u, d1.clone());
                            }
                        }
                        Coverage::Single1(t)
                    }
                    ("gpos_single", Coverage::Move1(c)) => {
                        let mut t = BTreeMap::new();
                        for (g1, d1) in c {
                            let Some(ug1) = ix(g1) else {
                                continue;
                            };
                            t.insert(ug1, d1);
                            for u in dups(ug1) {
                                t.insert(u, d1);
                            }
                        }
                        Coverage::Move1(t)
                    }
                    (
                        "gpos_mark2base" | "gpos_mark2mark" | "gpos_mark2ligature",
                        Coverage::Mark(c),
                    ) => {
                        if let Some(b) = step.baseclasses.as_mut() {
                            for d1 in b.values_mut() {
                                *d1 = std::mem::take(d1)
                                    .into_iter()
                                    .filter_map(|(g, a)| Some((ix(g)?, a)))
                                    .collect();
                            }
                        }
                        Coverage::Mark(
                            c.into_iter()
                                .filter_map(|(g, d)| Some((ix(g)?, d)))
                                .collect(),
                        )
                    }
                    ("gpos_cursive", Coverage::Cursive(c)) => {
                        let mut t = BTreeMap::new();
                        for (g1, d1) in c {
                            let Some(ug1) = ix(g1) else {
                                continue;
                            };
                            t.insert(ug1, d1.clone());
                            for u in dups(ug1) {
                                t.insert(u, d1.clone());
                            }
                        }
                        Coverage::Cursive(t)
                    }
                    (_, other) => other,
                };
                if let Some(rules) = step.rules.as_mut() {
                    for rule in rules.iter_mut() {
                        if let Some(b) = rule.before.as_mut() {
                            for s in b.iter_mut() {
                                *s = reset(s);
                            }
                        }
                        if let Some(a) = rule.after.as_mut() {
                            for s in a.iter_mut() {
                                *s = reset(s);
                            }
                        }
                        for s in rule.current.iter_mut() {
                            *s = reset(s);
                        }
                        if let Some(r) = rule.replacements.as_mut() {
                            *r = std::mem::take(r)
                                .into_iter()
                                .filter_map(|(k, v)| Some((ix(k)?, ix(v)?)))
                                .collect();
                        }
                    }
                }
            }
        }
    }
}

/// A contextual rule as `expandlookups` stores it (font-oup.lua:3268-3277).
#[derive(Clone, Debug, PartialEq)]
pub struct RuleData {
    /// the rule's number within its step, 1-based
    pub rule: usize,
    pub lookuptype: &'static str,
    /// before (reversed) ++ current ++ after
    pub sequence: Vec<Set>,
    /// 1-based positions of the first and last current set
    pub start: usize,
    pub stop: usize,
    /// per current position: sublookup numbers (1-based)
    pub lookups: Option<Vec<Option<Vec<usize>>>>,
    pub replacements: Option<BTreeMap<u32, u32>>,
    /// the type of the first lookup a position names
    pub subtype: Option<&'static str>,
}

/// A step's contextual rules after expansion (the `rulehash` of
/// font-oup.lua:3211); the step's coverage maps a first glyph to indices
/// into it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Expanded {
    /// per sequence, per step
    pub rules: Vec<Vec<Vec<RuleData>>>,
    pub subrules: Vec<Vec<Vec<RuleData>>>,
    /// per sequence: the resolved mark filtering set
    pub markclass: Vec<Option<Set>>,
    pub submarkclass: Vec<Option<Set>>,
}

fn expand_list(
    list: &mut [Sequence],
    sublookups: &[Sequence],
    fallback: Option<&[Sequence]>,
    markclasses: &BTreeMap<usize, Set>,
) -> (Vec<Vec<Vec<RuleData>>>, Vec<Option<Set>>) {
    let mut allrules = Vec::new();
    let mut marks = Vec::new();
    for seq in list.iter_mut() {
        marks.push(
            seq.markclass
                .map(|m| markclasses.get(&m).cloned().unwrap_or_default()),
        );
        let lookuptype = seq.typ;
        let mut perstep = Vec::new();
        for step in seq.steps.iter_mut() {
            let Some(rules) = step.rules.as_ref() else {
                perstep.push(Vec::new());
                continue;
            };
            let mut hash: Vec<RuleData> = Vec::new();
            let mut coverage: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
            for (k, rule) in rules.iter().enumerate() {
                let Rule {
                    before,
                    current,
                    after,
                    lookups,
                    replacements,
                } = rule;
                let mut sequence: Vec<Set> = Vec::new();
                if let Some(b) = before {
                    sequence.extend(b.iter().cloned());
                }
                let start = sequence.len() + 1;
                sequence.extend(current.iter().cloned());
                let stop = sequence.len();
                if let Some(a) = after {
                    sequence.extend(a.iter().cloned());
                }
                let mut subtype = None;
                if let Some(l) = lookups {
                    for v in l.iter().flatten().flatten() {
                        let found = sublookups
                            .get(v - 1)
                            .or_else(|| fallback.and_then(|f| f.get(v - 1)));
                        if let Some(found) = found {
                            if subtype.is_none() {
                                subtype = Some(found.typ);
                            }
                        }
                    }
                }
                if sequence.is_empty() {
                    continue;
                }
                hash.push(RuleData {
                    rule: k + 1,
                    lookuptype,
                    sequence: sequence.clone(),
                    start,
                    stop,
                    lookups: lookups.clone(),
                    replacements: replacements.clone(),
                    subtype,
                });
                let at = hash.len() - 1;
                if let Some(first) = sequence.get(start - 1) {
                    for &u in first {
                        coverage.entry(u).or_default().push(at);
                    }
                }
            }
            step.coverage = Coverage::Rules(coverage);
            perstep.push(hash);
        }
        allrules.push(perstep);
    }
    (allrules, marks)
}

/// `expandlookups` (font-oup.lua:3175-3335) for the sequences, then the
/// sublookups (which may fall back on a sequence).
pub fn expand_lookups(layout: &mut Layout) -> Expanded {
    let subs = layout.sublookups.clone();
    let (rules, markclass) = expand_list(&mut layout.sequences, &subs, None, &layout.markclasses);
    let seqs = layout.sequences.clone();
    let (subrules, submarkclass) = expand_list(
        &mut layout.sublookups,
        &subs,
        Some(&seqs),
        &layout.markclasses,
    );
    Expanded {
        rules,
        subrules,
        markclass,
        submarkclass,
    }
}

/// The GSUB lookups `checklookups` and `getcomponents` read, out of the
/// unified sequences and sublookups.
pub fn lookups_of(layout: &Layout) -> Lookups {
    let mut out = Lookups::default();
    fn convert(t: &LigTree) -> LigatureTree {
        LigatureTree {
            ligature: t.ligature,
            next: t.next.iter().map(|(k, v)| (*k, convert(v))).collect(),
        }
    }
    for list in [&layout.sequences, &layout.sublookups] {
        for seq in list {
            for step in &seq.steps {
                match (seq.typ, &step.coverage) {
                    ("gsub_single", Coverage::Single(c)) => out.singles.push(c.clone()),
                    ("gsub_alternate", Coverage::Sequence(c)) => out.alternates.push(c.clone()),
                    ("gsub_ligature", Coverage::Ligature(c)) => out
                        .ligatures
                        .push(c.iter().map(|(k, v)| (*k, convert(v))).collect()),
                    _ => {}
                }
            }
        }
    }
    out
}
