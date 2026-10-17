//! Where every glyph of a set formula lands, and how that reaches the page.
//!
//! `mlist_to_hlist` leaves a tree of boxes; a page wants coordinates. This is
//! `hlist_out` and `vlist_out` (`tex.web` §619-§640) reduced to the part a
//! formula needs: no leaders, no `\special`, no DVI -- a walk that turns the
//! tree into glyphs and rules positioned relative to the formula's own
//! baseline.
//!
//! The result is carried to the typesetter as text, because that is how
//! everything else the lowerer decides reaches it: the colour of a run, the
//! face it is set in and where a page breaks all travel the document's own
//! character stream as markers. A formula travels the same way, as the spec of
//! a marker whose visible text is the formula spelled out for a reader who
//! asked for `--text` rather than for a page.

use super::mlist::font_parts;
use super::noad::{Atom, Field, Noad, Scaled};
use crate::node::{BoxNode, Node, RuleNode};
use crate::pack::Setter;

/// One glyph of a set formula, positioned from the formula's own reference
/// point: `x` to the right, `y` UP from the baseline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub ch: char,
    pub x: Scaled,
    pub y: Scaled,
    /// The size of the font it is set in, in scaled points -- text, script or
    /// scriptscript, which is the whole visible difference between `$x^2$` and
    /// `$x2$`.
    pub size: Scaled,
}

/// A rule: a fraction bar, an overline, or the bar of a radical. `y` is its
/// BOTTOM edge, up from the baseline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rule {
    pub x: Scaled,
    pub y: Scaled,
    pub width: Scaled,
    pub height: Scaled,
}

/// A formula, set: its dimensions and everything drawn inside it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Setting {
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    pub glyphs: Vec<Glyph>,
    pub rules: Vec<Rule>,
}

/// Walk a set box into glyphs and rules.
pub fn flatten(b: &BoxNode, size_of_font: &dyn Fn(usize) -> Scaled) -> Setting {
    let mut out = Setting {
        width: b.width,
        height: b.height,
        depth: b.depth,
        ..Setting::default()
    };
    match b.vertical {
        true => vlist(b, 0, 0, &mut out, size_of_font),
        false => hlist(b, 0, 0, &mut out, size_of_font),
    }
    out
}

/// `hlist_out` (§619-§628): left to right along one baseline.
fn hlist(
    b: &BoxNode,
    left: Scaled,
    baseline: Scaled,
    out: &mut Setting,
    size_of_font: &dyn Fn(usize) -> Scaled,
) {
    let mut setter = Setter::new(b);
    let mut x = left;
    for node in &b.list {
        match node {
            Node::Char(c) | Node::Ligature(c) => {
                out.glyphs.push(Glyph {
                    ch: c.character,
                    x,
                    y: baseline,
                    size: size_of_font(c.font),
                });
                x += c.width;
            }
            Node::Box(inner) => {
                // §623: a box inside an hlist is displaced DOWNWARD by its
                // shift amount.
                let base = baseline - inner.shift_amount;
                match inner.vertical {
                    true => vlist(inner, x, base, out, size_of_font),
                    false => hlist(inner, x, base, out, size_of_font),
                }
                x += inner.width;
            }
            Node::Rule(r) => {
                // §624: a running height or depth is the enclosing box's.
                let (h, d) = running_vertical(r, b);
                let w = match RuleNode::is_running(r.width) {
                    true => 0,
                    false => r.width,
                };
                if h + d > 0 && w > 0 {
                    out.rules.push(Rule {
                        x,
                        y: baseline - d,
                        width: w,
                        height: h + d,
                    });
                }
                x += w;
            }
            Node::Glue(g) => x += setter.glue(&g.spec),
            Node::Kern { width, .. } | Node::Math(width) => x += width,
            _ => {}
        }
    }
}

/// `vlist_out` (§629-§637): down the page from the top of the box.
fn vlist(
    b: &BoxNode,
    left: Scaled,
    baseline: Scaled,
    out: &mut Setting,
    size_of_font: &dyn Fn(usize) -> Scaled,
) {
    let mut setter = Setter::new(b);
    // §630: the walk starts at the TOP of the box, which is its height above
    // the baseline it was placed on.
    let mut v = baseline + b.height;
    for node in &b.list {
        match node {
            Node::Box(inner) => {
                // §632: down by the box's height, set it there, then down by
                // its depth. A shift in a vlist moves it to the RIGHT.
                v -= inner.height;
                let x = left + inner.shift_amount;
                match inner.vertical {
                    true => vlist(inner, x, v, out, size_of_font),
                    false => hlist(inner, x, v, out, size_of_font),
                }
                v -= inner.depth;
            }
            Node::Rule(r) => {
                let h = match RuleNode::is_running(r.height) {
                    true => 0,
                    false => r.height,
                };
                let d = match RuleNode::is_running(r.depth) {
                    true => 0,
                    false => r.depth,
                };
                // §633: a running WIDTH is the enclosing box's, which is how a
                // fraction bar comes out exactly as wide as the fraction.
                let w = match RuleNode::is_running(r.width) {
                    true => b.width,
                    false => r.width,
                };
                v -= h + d;
                if h + d > 0 && w > 0 {
                    out.rules.push(Rule {
                        x: left,
                        y: v,
                        width: w,
                        height: h + d,
                    });
                }
            }
            Node::Glue(g) => v -= setter.glue(&g.spec),
            Node::Kern { width, .. } => v -= width,
            _ => {}
        }
    }
}

/// A rule's height and depth inside an hlist, with `null_flag` resolved
/// against the box holding it (§624).
fn running_vertical(r: &RuleNode, container: &BoxNode) -> (Scaled, Scaled) {
    let h = match RuleNode::is_running(r.height) {
        true => container.height,
        false => r.height,
    };
    let d = match RuleNode::is_running(r.depth) {
        true => container.depth,
        false => r.depth,
    };
    (h, d)
}

/// The size, in scaled points, of the font a `CharNode` names.
pub fn font_size(fonts: &super::font::MathFonts, index: usize) -> Scaled {
    let (fam, size) = font_parts(index);
    (fonts.at(fam, size) * 65536.0).round() as Scaled
}

/// The formula spelled out for a reader.
///
/// `--text` prints the document's words, and a formula is words: `$x^2+1$`
/// should read as `x^2+1` and not vanish, and not come out as the markup that
/// produced it either. Superscript and subscript digits have Unicode
/// spellings and are used where they exist; a fraction reads as a division and
/// a radical as a root, which is how anyone would read them aloud.
///
/// No character of this is a space, because the line breaker splits a
/// paragraph on spaces and a formula is one word.
pub fn plain(list: &[Noad]) -> String {
    let mut out = String::new();
    plain_into(&mut out, list);
    out
}

/// [`plain`], appended to `out`. The whole spelling is built in the one
/// string, rather than in a string per field and per script joined into it.
pub fn plain_into(out: &mut String, list: &[Noad]) {
    for item in list {
        match item {
            Noad::Atom(a) => {
                field_into(out, &a.nucleus);
                scripts_into(out, a);
            }
            Noad::Fraction(f) => {
                plain_into(out, &f.numerator);
                out.push('/');
                plain_into(out, &f.denominator);
            }
            Noad::Radical(r) => {
                // `\sqrt[3]{x}` reads as a cube root, which is where its index
                // goes when it is written rather than set: in front of the
                // sign and raised, the way `∛` is one character.
                if let Some(index) = &r.index {
                    script_into(out, true, |out| plain_into(out, index));
                }
                out.push('√');
                field_into(out, &r.nucleus.nucleus);
            }
            Noad::Over(a) | Noad::Under(a) | Noad::VCenter(a) => field_into(out, &a.nucleus),
            // An accent reads as the letter with the mark on it, which is what
            // a reader would say: `\hat x` is `x̂`, one letter and one
            // combining character.
            Noad::Accent(a) => {
                field_into(out, &a.atom.nucleus);
                if let Some(mark) = combining(a.accent) {
                    out.push(mark);
                }
                scripts_into(out, &a.atom);
            }
            // `\mathchoice`'s text list: a formula printed for a reader is not
            // in any style, and the text one is what `$...$` in a paragraph
            // would have chosen (§731).
            Noad::Choice(c) => plain_into(out, &c.text),
            Noad::Left(_) | Noad::Right(_) | Noad::Style(_) => {}
            Noad::Glue(_) | Noad::Kern(_) | Noad::MuGlue(_) | Noad::MuKern(_) => {}
            Noad::Node(_) => {}
        }
    }
}

/// The combining character that spells one of plain.tex's math accents
/// (plain.tex:939-950), for a reader who asked for `--text`.
///
/// The accent's own slot says nothing here: `cmr10`'s `"16` is a macron and
/// `cmex10`'s `"62` is a wide circumflex, and neither is the character a
/// reader wants beside the letter. Unicode's combining marks are.
fn combining(accent: super::noad::MathChar) -> Option<char> {
    match (accent.fam, accent.character) {
        (0, 0x12) => Some('\u{300}'),
        (0, 0x13) => Some('\u{301}'),
        (0, 0x14) => Some('\u{30C}'),
        (0, 0x15) => Some('\u{306}'),
        (0, 0x16) => Some('\u{304}'),
        (0, 0x5E) => Some('\u{302}'),
        (0, 0x5F) => Some('\u{307}'),
        (0, 0x7E) => Some('\u{303}'),
        (0, 0x7F) => Some('\u{308}'),
        (1, 0x7E) => Some('\u{20D7}'),
        (3, 0x62) => Some('\u{302}'),
        (3, 0x65) => Some('\u{303}'),
        _ => None,
    }
}

fn field_into(out: &mut String, f: &Field) {
    match f {
        Field::Empty | Field::Box(_) => {}
        Field::Char(c) => out.extend(super::font::unicode(c.fam, c.character)),
        Field::Literal(c) => out.push(*c),
        Field::List(l) => plain_into(out, l),
    }
}

/// An atom's superscript and then its subscript, each where it has one.
fn scripts_into(out: &mut String, a: &Atom) {
    if !a.supscr.is_empty() {
        script_into(out, true, |out| field_into(out, &a.supscr));
    }
    if !a.subscr.is_empty() {
        script_into(out, false, |out| field_into(out, &a.subscr));
    }
}

/// A script, which `spell` appends to `out`, in Unicode's own superscript or
/// subscript digits where they exist, and after a `^` or a `_` where they do
/// not.
///
/// The script is spelled in place and rewritten there: a run of digits by
/// their raised or lowered forms, anything else by the mark put in front.
fn script_into(out: &mut String, superscript: bool, spell: impl FnOnce(&mut String)) {
    const SUPER: [char; 10] = ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];
    const SUB: [char; 10] = ['₀', '₁', '₂', '₃', '₄', '₅', '₆', '₇', '₈', '₉'];
    let start = out.len();
    spell(out);
    let end = out.len();
    let digits = end > start && out.as_bytes()[start..].iter().all(u8::is_ascii_digit);
    if !digits {
        let mark = match superscript {
            true => '^',
            false => '_',
        };
        out.insert(start, mark);
        return;
    }
    let table = match superscript {
        true => &SUPER,
        false => &SUB,
    };
    for at in start..end {
        let digit = out.as_bytes()[at] - b'0';
        out.push(table[usize::from(digit)]);
    }
    out.replace_range(start..end, "");
}

/// The setting, as the characters a marker's spec carries.
///
/// Integers in scaled points throughout, so nothing is lost to a decimal
/// spelling and back: `g` is a glyph and `r` a rule, and the three numbers in
/// front are the formula's own width, height and depth.
pub fn encode(s: &Setting) -> String {
    use std::fmt::Write;
    // Each piece is written straight into the spec. Writing to a `String`
    // cannot fail, so the `fmt::Result` is dropped.
    let mut out = format!("{};{};{}", s.width, s.height, s.depth);
    for g in &s.glyphs {
        let _ = write!(out, ";g{},{},{},{}", g.x, g.y, g.size, g.ch as u32);
    }
    for r in &s.rules {
        let _ = write!(out, ";r{},{},{},{}", r.x, r.y, r.width, r.height);
    }
    out
}

/// The other half of [`encode`].
pub fn decode(text: &str) -> Option<Setting> {
    let mut parts = text.split(';');
    let mut out = Setting {
        width: parts.next()?.parse().ok()?,
        height: parts.next()?.parse().ok()?,
        depth: parts.next()?.parse().ok()?,
        ..Setting::default()
    };
    for part in parts {
        let (kind, rest) = part.split_at(part.char_indices().nth(1)?.0);
        let n: Vec<i64> = rest.split(',').filter_map(|v| v.parse().ok()).collect();
        if n.len() != 4 {
            return None;
        }
        match kind {
            "g" => out.glyphs.push(Glyph {
                x: n[0],
                y: n[1],
                size: n[2],
                ch: char::from_u32(n[3] as u32)?,
            }),
            "r" => out.rules.push(Rule {
                x: n[0],
                y: n[1],
                width: n[2],
                height: n[3],
            }),
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_setting_survives_the_round_trip_through_a_marker() {
        let s = Setting {
            width: 123_456,
            height: -7,
            depth: 65536,
            glyphs: vec![Glyph {
                ch: '∑',
                x: 10,
                y: -20,
                size: 655_360,
            }],
            rules: vec![Rule {
                x: 1,
                y: 2,
                width: 3,
                height: 4,
            }],
        };
        assert_eq!(decode(&encode(&s)).as_ref(), Some(&s));
    }

    /// The spec travels inside a marker, so it may hold no character the
    /// marker machinery reads and no space the line breaker would split on.
    #[test]
    fn an_encoded_setting_holds_no_control_character_and_no_space() {
        let s = Setting {
            width: -1,
            height: 0,
            depth: 0,
            glyphs: vec![Glyph {
                ch: 'x',
                x: -5,
                y: 0,
                size: 1,
            }],
            rules: Vec::new(),
        };
        let text = encode(&s);
        assert!(
            !text.chars().any(|c| c.is_control() || c == ' '),
            "the spec carries {text:?}"
        );
    }
}
