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
    for index in 1..fontdata.glyphs.len() {
        let glyph = &fontdata.glyphs[index];
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
            if !descriptions.contains_key(&v) {
                let mut c = du.clone();
                c.unicode = Some(Unicode::One(v));
                descriptions.insert(v, c);
            }
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

/// `stripredundant` outside ConTeXt (font-oup.lua:1150-1173) with
/// `keepnames = false`: drop the uninformative names and the `base` class,
/// which is what an unclassed glyph is anyway.
pub fn strip_redundant(descriptions: &mut BTreeMap<u32, Description>) {
    for d in descriptions.values_mut() {
        if d.name.as_deref().is_some_and(crappy_name) {
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

/// `overloads` (font-map.lua:193-206): the ligatures whose Unicode
/// presentation forms a font should not be trusted to mean.
fn overload(key_name: Option<&str>, key_code: Option<u32>) -> Option<Unicode> {
    let by_name = |n: &str| -> Option<Vec<u32>> {
        Some(match n {
            "IJ" | "I_J" => vec![0x49, 0x4A],
            "ij" | "i_j" => vec![0x69, 0x6A],
            "ff" | "f_f" => vec![0x66, 0x66],
            "fi" | "f_i" => vec![0x66, 0x69],
            "fl" | "f_l" => vec![0x66, 0x6C],
            "ffi" | "f_f_i" => vec![0x66, 0x66, 0x69],
            "ffl" | "f_f_l" => vec![0x66, 0x66, 0x6C],
            "fj" | "f_j" => vec![0x66, 0x6A],
            "fk" | "f_k" => vec![0x66, 0x6B],
            _ => return None,
        })
    };
    let by_code = |c: u32| -> Option<Vec<u32>> {
        Some(match c {
            0x0132 => vec![0x49, 0x4A],
            0x0133 => vec![0x69, 0x6A],
            0xFB00 => vec![0x66, 0x66],
            0xFB01 => vec![0x66, 0x69],
            0xFB02 => vec![0x66, 0x6C],
            0xFB03 => vec![0x66, 0x66, 0x69],
            0xFB04 => vec![0x66, 0x66, 0x6C],
            _ => return None,
        })
    };
    key_name
        .and_then(by_name)
        .or_else(|| key_code.and_then(by_code))
        .map(Unicode::Many)
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
