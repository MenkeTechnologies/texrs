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
use super::noad::{Field, Noad, Scaled};
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
    for item in list {
        match item {
            Noad::Atom(a) => {
                out.push_str(&plain_field(&a.nucleus));
                if !a.supscr.is_empty() {
                    out.push_str(&script(&plain_field(&a.supscr), true));
                }
                if !a.subscr.is_empty() {
                    out.push_str(&script(&plain_field(&a.subscr), false));
                }
            }
            Noad::Fraction(f) => {
                out.push_str(&plain(&f.numerator));
                out.push('/');
                out.push_str(&plain(&f.denominator));
            }
            Noad::Radical(r) => {
                out.push('√');
                out.push_str(&plain_field(&r.nucleus.nucleus));
            }
            Noad::Over(a) | Noad::Under(a) => out.push_str(&plain_field(&a.nucleus)),
            Noad::Left(_) | Noad::Right(_) | Noad::Style(_) => {}
            Noad::Glue(_) | Noad::Kern(_) | Noad::Node(_) => {}
        }
    }
    out
}

fn plain_field(f: &Field) -> String {
    match f {
        Field::Empty | Field::Box(_) => String::new(),
        Field::Char(c) => super::font::unicode(c.fam, c.character)
            .map(String::from)
            .unwrap_or_default(),
        Field::Literal(c) => c.to_string(),
        Field::List(l) => plain(l),
    }
}

/// A script in Unicode's own superscript or subscript letters where they
/// exist, and after a `^` or a `_` where they do not.
fn script(text: &str, superscript: bool) -> String {
    const SUPER: &str = "⁰¹²³⁴⁵⁶⁷⁸⁹";
    const SUB: &str = "₀₁₂₃₄₅₆₇₈₉";
    let table: Vec<char> = match superscript {
        true => SUPER.chars().collect(),
        false => SUB.chars().collect(),
    };
    let digits: Option<String> = text
        .chars()
        .map(|c| c.to_digit(10).map(|d| table[d as usize]))
        .collect();
    match digits {
        Some(s) if !s.is_empty() => s,
        _ => {
            let mark = match superscript {
                true => '^',
                false => '_',
            };
            format!("{mark}{text}")
        }
    }
}

/// The setting, as the characters a marker's spec carries.
///
/// Integers in scaled points throughout, so nothing is lost to a decimal
/// spelling and back: `g` is a glyph and `r` a rule, and the three numbers in
/// front are the formula's own width, height and depth.
pub fn encode(s: &Setting) -> String {
    let mut out = format!("{};{};{}", s.width, s.height, s.depth);
    for g in &s.glyphs {
        out.push_str(&format!(";g{},{},{},{}", g.x, g.y, g.size, g.ch as u32));
    }
    for r in &s.rules {
        out.push_str(&format!(";r{},{},{},{}", r.x, r.y, r.width, r.height));
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
