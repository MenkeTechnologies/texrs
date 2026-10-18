//! The node processors luaotfload runs around the fontloader's features,
//! in the order a node-mode font has them (`shared.processes`, as luahbtex
//! reports it): `szss`, `normalize`, `amtra`, `analyze`, `features`,
//! `notdef`, `invisible`. `normalize` is `super::normalize`, `analyze`
//! `super::ota`, `features` `super::ots`; the others are here.
//!
//! - `szss` (luaotfload-szss.lua:28-77): a capital sharp s the font lacks
//!   becomes `SS`.
//! - `amtra` (luaotfload-arabic.lua:42-143): Unicode's Arabic mark
//!   transient reordering. Its initializer tests an undefined name, so the
//!   feature stays `auto` -- on -- for every font.
//! - `notdef` (luaotfload-notdef.lua:93-165): a character the font does not
//!   have is followed by the font's `.notdef` glyph.
//! - `invisible` (:167-371): default-ignorable characters become the
//!   font's space with a kern taking its width back (or are removed).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use super::nodes::{Ix, Kind, List, GLYPH_PROTECTED};
use super::normalize::Tables;
use super::Face;

/// `szssprocessor` for a font without U+1E9E.
pub fn szss(list: &mut List, head: Option<Ix>, font: usize) -> Option<Ix> {
    let mut head = head;
    let mut n = head;
    while let Some(cur) = n {
        match list.ischar(Some(cur), Some(font)) {
            Ok(Some(0x1E9E)) => {
                list.set_char(cur, 0x53);
                let c = list.copy(cur);
                let (h, nn) = list.insert_after(head, Some(cur), c);
                head = h;
                n = nn;
            }
            Err(()) if list.is_disc(Some(cur)) => {
                let (pre, post, replace) = list.disc(cur);
                let pre = szss(list, pre, font);
                let post = szss(list, post, font);
                let replace = szss(list, replace, font);
                list.set_disc(cur, pre, post, replace);
            }
            _ => {}
        }
        n = n.and_then(|x| list.nodes[x].next);
    }
    head
}

/// luaotfload-arabic.lua:25-40.
const MCM: [u32; 14] = [
    0x0654, 0x0655, 0x0658, 0x06DC, 0x06E3, 0x06E7, 0x06E8, 0x08CA, 0x08CB, 0x08CD, 0x08CE, 0x08CF,
    0x08D3, 0x08F3,
];

/// `reorder_amtra` (luaotfload-arabic.lua:45-116), including its last
/// branch, which reads two names local to another branch and so stops the
/// pass at a run of ccc 230 marks without a modifier combining mark.
pub fn amtra(list: &mut List, head: Option<Ix>, font: usize, t: &Tables) {
    let ccc_of = |list: &List, n: Option<Ix>| {
        list.ischar(n, Some(font))
            .ok()
            .flatten()
            .and_then(|c| t.ccc(c))
    };
    let is_mcm = |list: &List, n: Option<Ix>| {
        list.ischar(n, Some(font))
            .ok()
            .flatten()
            .is_some_and(|c| MCM.contains(&c))
    };
    let mut n = head;
    while let Some(start) = n {
        let mut base = start;
        let mut prev = Some(start);
        n = list.nodes[start].next;
        loop {
            let Some(this) = ccc_of(list, n) else {
                break;
            };
            if this == 33 || this == 220 || this == 230 {
                let mut after = n;
                let mut tail;
                let mut found = false;
                loop {
                    tail = after;
                    after = list.next(tail);
                    if is_mcm(list, after) {
                        found = true;
                    }
                    if ccc_of(list, after) != Some(this) {
                        break;
                    }
                }
                if this == 33 || found {
                    list.link(prev, after);
                    // base's next as it is now, after the first setlink
                    let base_next = list.nodes[base].next;
                    list.link(tail, base_next);
                    list.link(Some(base), n);
                    if prev == Some(base) {
                        prev = tail;
                    }
                    n = after;
                    if this == 220 {
                        base = tail.unwrap();
                    }
                } else if this == 220 {
                    prev = tail;
                    n = after;
                } else {
                    prev = None;
                    n = None;
                }
            } else {
                prev = n;
                n = list.next(n);
            }
        }
    }
}

/// `setnotdef` (luaotfload-notdef.lua:93-137): the slot of glyph 0.
pub fn notdef_code(face: &Face) -> Option<u32> {
    let index0 = |c: u32| face.descriptions.get(&c).is_some_and(|d| d.index == 0);
    if index0(0xF0000) {
        return Some(0xF0000);
    }
    if let Some(c) = (0xFFFC..=0xFFFF).find(|c| index0(*c)) {
        return Some(c);
    }
    if let Some(&c) = face.unicodes.get(".notdef") {
        if index0(c) {
            return Some(c);
        }
    }
    // the Lua's `pairs` takes whichever comes first; the lowest here
    face.descriptions
        .iter()
        .find(|(_, d)| d.index == 0)
        .map(|(c, _)| *c)
}

/// `donotdef` (:140-153). `has` says whether the font has a character.
pub fn notdef(
    list: &mut List,
    head: Option<Ix>,
    font: usize,
    code: Option<u32>,
    has: &dyn Fn(u32) -> bool,
) {
    let Some(code) = code else {
        return;
    };
    let ignorable = ignorables();
    let mut n = head;
    while let Some(cur) = n {
        n = list.nodes[cur].next;
        let Ok(Some(c)) = list.ischar(Some(cur), None) else {
            continue;
        };
        if list.font(cur) != Some(font) {
            continue;
        }
        // (a character the font lacks is zero by zero by zero, so the Lua's
        // getwhd test adds nothing to `not chars[cid]`)
        if !has(c) && !ignorable.contains(&c) {
            let g = list.alloc(Kind::Glyph {
                char: code,
                font,
                subtype: GLYPH_PROTECTED,
                components: None,
                xoffset: 0,
                yoffset: 0,
            });
            list.insert_after(Some(cur), Some(cur), g);
            protect(list, cur);
            n = list.nodes[g].next;
        }
    }
}

/// `protect_glyph`: a character or ligature becomes protected (256).
fn protect(list: &mut List, n: Ix) {
    if let Kind::Glyph { subtype, .. } = &mut list.nodes[n].kind {
        if *subtype <= 256 {
            *subtype = GLYPH_PROTECTED;
        }
    }
}

/// The default-ignorable code points of luaotfload-notdef.lua:53-91, from
/// TeX Live's UnicodeData.txt and PropList.txt.
pub fn ignorables() -> &'static BTreeSet<u32> {
    static SET: OnceLock<BTreeSet<u32>> = OnceLock::new();
    SET.get_or_init(|| {
        let read = |name: &str| {
            crate::kpse::locate(name)
                .and_then(|p| std::fs::read_to_string(p).ok())
                .unwrap_or_default()
        };
        ignorables_from(&read("UnicodeData.txt"), &read("PropList.txt"))
    })
}

fn ignorables_from(unicodedata: &str, proplist: &str) -> BTreeSet<u32> {
    let mut set = BTreeSet::new();
    for line in unicodedata.lines() {
        let f: Vec<&str> = line.splitn(4, ';').collect();
        if f.len() >= 3 && f[2] == "Cf" && f[0].len() >= 4 {
            if let Ok(c) = u32::from_str_radix(f[0], 16) {
                set.insert(c);
            }
        }
    }
    for line in proplist.lines() {
        let Some((range, rest)) = line.split_once(';') else {
            continue;
        };
        let rest = rest.trim_start();
        let Some((prop, comment)) = rest.split_once(" # ") else {
            continue;
        };
        let value = match prop.trim_end() {
            "Other_Default_Ignorable_Code_Point" | "Variation_Selector" => true,
            "White_Space" | "Prepended_Concatenation_Mark" => false,
            _ => continue,
        };
        // the grammar wants a character other than the start of "Lo" there
        if comment.starts_with("Lo") || comment.is_empty() {
            continue;
        }
        let range = range.trim();
        let (a, b) = match range.split_once("..") {
            Some((a, b)) => (a, b),
            None => (range, range),
        };
        let (Ok(a), Ok(b)) = (u32::from_str_radix(a, 16), u32::from_str_radix(b, 16)) else {
            continue;
        };
        for c in a..=b {
            if value {
                set.insert(c);
            } else {
                set.remove(&c);
            }
        }
    }
    for c in (0xFFF9..=0xFFFB).chain(0x13430..=0x13438) {
        set.remove(&c);
    }
    set.remove(&0xAD);
    set
}

/// `ignorablehandler` (:313-341) with the feature's value: replace by the
/// space and take its width back with a kern, or remove when the value is
/// `remove` or the font has no space. In discretionaries the lists are
/// processed in place: the Lua's `setdisc` there is given the pre-break
/// list instead of the disc, so a removed first node is never unhooked.
pub fn invisible(
    list: &mut List,
    head: Option<Ix>,
    font: usize,
    replacement: Option<(u32, f64)>,
) -> Option<Ix> {
    let ignorable = ignorables();
    let mut head = head;
    let mut n = head;
    while let Some(cur) = n {
        n = list.nodes[cur].next;
        let Ok(Some(c)) = list.ischar(Some(cur), None) else {
            continue;
        };
        if list.font(cur) != Some(font) || !ignorable.contains(&c) {
            continue;
        }
        match replacement {
            Some((r, width)) => {
                list.set_char(cur, r);
                let k = list.alloc(Kind::Kern {
                    subtype: 0,
                    kern: super::nodes::round(-width),
                });
                let (h, _) = list.insert_after(head, Some(cur), k);
                head = h;
            }
            None => {
                let (h, _) = list.remove(head, Some(cur));
                head = h;
            }
        }
    }
    let mut d = head;
    while let Some(x) = d {
        if list.is_disc(Some(x)) {
            let (pre, post, replace) = list.disc(x);
            invisible(list, pre, font, replacement);
            invisible(list, post, font, replacement);
            invisible(list, replace, font, replacement);
        }
        d = list.nodes[x].next;
    }
    head
}

/// The `invisible` value's replacement (`font_invisible_replacement`,
/// :167-183): the character (32 unless a number says otherwise) and its
/// width, or none for `remove` or a font without it.
pub fn invisible_replacement(
    value: Option<&super::ots::FeatureValue>,
    widths: &BTreeMap<u32, f64>,
) -> Option<(u32, f64)> {
    use super::ots::FeatureValue;
    let r = match value {
        Some(FeatureValue::Text(t)) if t == "remove" => return None,
        Some(FeatureValue::Number(x)) => *x as u32,
        Some(FeatureValue::Text(t)) => t.parse::<f64>().map_or(32, |x| x as u32),
        _ => 32,
    };
    widths.get(&r).map(|w| (r, *w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignorables_follow_luaotfloads_grammar() {
        let ud = "00AD;SOFT HYPHEN;Cf;0;BN;;;;;N;;;;;\n200B;ZERO WIDTH SPACE;Cf;0;BN;;;;;N;;;;;\n0600;ARABIC NUMBER SIGN;Cf;0;AN;;;;;N;;;;;\nFFF9;INTERLINEAR ANNOTATION ANCHOR;Cf;0;ON;;;;;N;;;;;\n0041;LATIN CAPITAL LETTER A;Lu;0;L;;;;;N;;;;0061;\n";
        let pl = "0600..0605    ; Prepended_Concatenation_Mark # Cf   [6] ARABIC NUMBER SIGN..ARABIC NUMBER MARK ABOVE\n034F          ; Other_Default_Ignorable_Code_Point # Mn       COMBINING GRAPHEME JOINER\n3164          ; Other_Default_Ignorable_Code_Point # Lo       HANGUL FILLER\nFE00..FE0F    ; Variation_Selector # Mn  [16] VARIATION SELECTOR-1..VARIATION SELECTOR-16\n";
        let set = ignorables_from(ud, pl);
        assert!(set.contains(&0x200B) && set.contains(&0x034F) && set.contains(&0xFE0F));
        // soft hyphen, prepended concatenation marks, interlinear
        // annotation and Lo fillers are not ignorable
        for c in [0xAD, 0x600, 0xFFF9, 0x3164, 0x41] {
            assert!(!set.contains(&c), "{c:X}");
        }
    }
}
