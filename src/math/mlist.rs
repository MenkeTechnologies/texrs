//! `mlist_to_hlist`: turning a formula into boxes.
//!
//! `tex.web` §719-§767, with the subroutines of §704-§717 it rests on. This is
//! the routine that decides everything a reader recognises as mathematics: how
//! far a superscript rises, how thick a fraction bar is and where it sits, how
//! tall a `\left(` grows, and how much room goes between a variable and the
//! operator beside it.
//!
//! It makes two passes (§725). The first turns every noad into an hlist and
//! records the tallest and deepest of them, because a `\left(` cannot be sized
//! until the whole formula's height is known. The second throws the noads away
//! and puts the inter-element spacing of §764 between what is left.
//!
//! The boxes are `crate::node`'s, and they are packaged by `crate::pack`'s
//! `hpack` and `vpack` -- §649 and §668 -- rather than by a second copy of
//! them written here.

use super::font::{CharDim, MathFont, MathFonts, TEXT_SIZE};
use super::noad::*;
use crate::glue::Glue;
use crate::node::{BoxNode, CharNode, GlueNode, Node, RuleNode};
use crate::pack::{hpack, vpack, Spec, Tolerances, NATURAL};

/// `half(x)` (§100), which is not `x/2`: an odd number rounds AWAY from zero
/// upward, and half of every shift in this file is computed with it.
fn half(x: Scaled) -> Scaled {
    match x % 2 == 0 {
        true => x / 2,
        false => (x + 1) / 2,
    }
}

/// `x_over_n(x,n)` (§106): integer division truncating toward zero, which is
/// what `cur_mu` is computed with.
fn x_over_n(x: Scaled, n: i64) -> Scaled {
    match n {
        0 => 0,
        n => x / n,
    }
}

/// `xn_over_d(x,n,d)` (§107): $xn/d$, truncated toward zero.
///
/// §107 does it in two halves to stay inside a 32-bit word; `i128` here
/// computes the same quotient without the split.
fn xn_over_d(x: Scaled, n: i64, d: i64) -> Scaled {
    match d {
        0 => 0,
        d => ((x as i128 * n as i128) / d as i128) as i64,
    }
}

/// The font index a `CharNode` carries: family and size packed into one
/// number, the way `fam_fnt` packs them (§699).
pub fn font_index(fam: usize, size: usize) -> usize {
    fam * 3 + size
}

/// The family and size a font index names.
pub fn font_parts(index: usize) -> (usize, usize) {
    (index / 3, index % 3)
}

/// `\hss` — the glue `rebox` centres with (§715).
fn ss_glue() -> Glue {
    Glue {
        natural: 0,
        stretch: 65536,
        stretch_order: 1,
        shrink: 65536,
        shrink_order: 1,
    }
}

fn kern(width: Scaled) -> Node {
    Node::Kern {
        width,
        explicit: false,
    }
}

/// The state `mlist_to_hlist` keeps in globals (§719): the fonts, the style,
/// the size that style implies, and the math unit at that size.
pub struct Converter<'a> {
    pub fonts: &'a MathFonts,
    style: Style,
    size: usize,
    mu: Scaled,
}

/// Which noad a `r_type` names during the first pass. `left_noad` and
/// `right_noad` are not spacing classes, and §728 tests for them by name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RType {
    Class(Class),
    Left,
    Right,
}

/// One entry of the list the first pass builds and the second pass reads.
struct Translated {
    /// The spacing class, once the Bin-to-Ord rewrites of §728 have settled.
    class: Option<RType>,
    /// `new_hlist(q)` (§725).
    hlist: Vec<Node>,
    /// A style node, which the second pass obeys and then deletes (§763).
    style: Option<Style>,
    /// The delimiter a `left_noad` or `right_noad` will be grown into once
    /// `max_h` and `max_d` are known (§762).
    delimiter: Option<Delimiter>,
}

impl Translated {
    fn plain(node: Node) -> Translated {
        Translated {
            class: None,
            hlist: vec![node],
            style: None,
            delimiter: None,
        }
    }
}

impl<'a> Converter<'a> {
    pub fn new(fonts: &'a MathFonts) -> Converter<'a> {
        let mut c = Converter {
            fonts,
            style: TEXT_STYLE,
            size: TEXT_SIZE,
            mu: 0,
        };
        c.set_style(TEXT_STYLE);
        c
    }

    /// "Set up the values of `cur_size` and `cur_mu`, based on `cur_style`"
    /// (§703).
    fn set_style(&mut self, style: Style) {
        self.style = style;
        self.size = size_of(style);
        self.mu = x_over_n(self.fonts.math_quad(self.size), 18);
    }

    // ── the parameters, at the current size ──────────────────────────────

    fn default_rule_thickness(&self) -> Scaled {
        self.fonts.default_rule_thickness(self.size)
    }
    fn axis_height(&self) -> Scaled {
        self.fonts.axis_height(self.size)
    }
    fn math_x_height(&self) -> Scaled {
        self.fonts.math_x_height(self.size)
    }

    /// `fetch(a)` (§722): the font and the metrics of a `math_char`.
    fn fetch(&self, c: MathChar, size: usize) -> Option<(&'a MathFont, CharDim)> {
        let f = self.fonts.font(c.fam, size)?;
        let m = f.metrics(c.character)?;
        Some((f, m))
    }

    /// A character node, measured in the font the family and size name.
    fn char_node(&self, c: MathChar, size: usize) -> Option<Node> {
        let (_, m) = self.fetch(c, size)?;
        let character = super::font::unicode(c.fam, c.character)?;
        Some(Node::Char(CharNode {
            font: font_index(c.fam, size),
            character,
            width: m.width,
            height: m.height,
            depth: m.depth,
        }))
    }

    /// `char_box(f,c)` (§709): a box holding one character, whose width
    /// INCLUDES the italic correction.
    fn char_box(&self, fam: usize, size: usize, code: u8) -> BoxNode {
        let c = MathChar {
            fam,
            character: code,
        };
        let Some((_, m)) = self.fetch(c, size) else {
            return BoxNode::null();
        };
        let list = match self.char_node(c, size) {
            Some(n) => vec![n],
            None => Vec::new(),
        };
        BoxNode {
            width: m.width + m.italic,
            height: m.height,
            depth: m.depth,
            list,
            ..BoxNode::null()
        }
    }

    /// `fraction_rule(t)` (§704): a rule `t` thick, running the width of
    /// whatever holds it.
    fn fraction_rule(&self, t: Scaled) -> Node {
        Node::Rule(RuleNode {
            width: crate::node::NULL_FLAG,
            height: t,
            depth: 0,
        })
    }

    /// `overbar(b,k,t)` (§705): `b`, a kern of `k`, a rule of `t`, and `t`
    /// more of space above it.
    fn overbar(&self, b: BoxNode, k: Scaled, t: Scaled) -> BoxNode {
        let list = vec![kern(t), self.fraction_rule(t), kern(k), Node::Box(b)];
        vpack(list, NATURAL, Tolerances::plain()).node
    }

    /// `rebox(b,w)` (§715): the same box, centred in a box `w` wide.
    fn rebox(&self, mut b: BoxNode, w: Scaled) -> BoxNode {
        if b.width == w || b.list.is_empty() {
            b.width = w;
            return b;
        }
        if b.vertical {
            b = hpack(vec![Node::Box(b)], NATURAL, Tolerances::plain(), None).node;
        }
        let mut list = std::mem::take(&mut b.list);
        // §715: a one-character box carries that character's italic
        // correction in its width, and centring it would centre the
        // correction too. A compensating kern puts it back where it belongs.
        if list.len() == 1 {
            if let Node::Char(c) = list[0] {
                if c.width != b.width {
                    list.push(kern(b.width - c.width));
                }
            }
        }
        let mut whole = vec![Node::Glue(GlueNode::new(ss_glue()))];
        whole.append(&mut list);
        whole.push(Node::Glue(GlueNode::new(ss_glue())));
        hpack(whole, Spec::Exactly(w), Tolerances::plain(), None).node
    }

    /// `clean_box(p,s)` (§720): a noad field, boxed in a given style, with a
    /// `shift_amount` of zero.
    fn clean_box(&mut self, field: &Field, style: Style) -> BoxNode {
        let q: Vec<Node> = match field {
            // §720's `sub_box` case: the box goes to `found` as it stands,
            // shift and all. It is NOT simply handed back -- a box with a
            // shift is repackaged below, which is what turns `\sum`'s
            // centring on the axis into height and depth the limits above it
            // can be measured against.
            Field::Box(b) => vec![Node::Box(b.clone())],
            Field::Char(c) => {
                let save = self.style;
                let list =
                    self.convert(&[Noad::Atom(Atom::new(Class::Ord, Field::Char(*c)))], style);
                self.set_style(save);
                list
            }
            Field::List(l) => {
                let save = self.style;
                let list = self.convert(l, style);
                self.set_style(save);
                list
            }
            Field::Literal(c) => vec![self.literal_node(*c, size_of(style))],
            Field::Empty => Vec::new(),
        };
        // §720's `found`: a list that is already ONE unshifted box is clean as
        // it stands, and anything else is packaged.
        let mut x = match q.as_slice() {
            [Node::Box(b)] if b.shift_amount == 0 => b.clone(),
            _ => hpack(q, NATURAL, Tolerances::plain(), None).node,
        };
        // §721: a character followed by nothing but its italic correction does
        // not need the correction, because the box is about to be measured
        // rather than set beside anything.
        if x.list.len() == 2 && x.list[0].is_char() && matches!(x.list[1], Node::Kern { .. }) {
            x.list.truncate(1);
        }
        x
    }

    /// A character with no Computer Modern slot, measured on the text
    /// family's quad. See `Field::Literal`.
    fn literal_node(&self, c: char, size: usize) -> Node {
        let quad = self.fonts.math_quad(size);
        Node::Char(CharNode {
            font: font_index(0, size),
            character: c,
            width: half(quad),
            height: self.fonts.math_x_height(size),
            depth: 0,
        })
    }

    // ── var_delimiter, §706-§714 ─────────────────────────────────────────

    /// `var_delimiter(d,s,v)` (§706): the smallest variant of `d` whose height
    /// plus depth is at least `v`, centred on the axis of size `s`.
    fn var_delimiter(&self, d: Delimiter, s: usize, v: Scaled) -> BoxNode {
        let mut found: Option<(usize, usize, u8)> = None;
        let mut w: Scaled = 0;
        let mut done = false;
        // §706: the small variant first, then the large one.
        for (fam, chr) in [(d.small_fam, d.small_char), (d.large_fam, d.large_char)] {
            if done {
                break;
            }
            if fam == 0 && chr == 0 {
                continue;
            }
            // §707: the sizes are tried from `s` downward -- the current size
            // first, then the larger fonts of the same family.
            for size in (0..=s.min(2)).rev() {
                if done {
                    break;
                }
                let Some(g) = self.fonts.font(fam, size) else {
                    continue;
                };
                // §708: walk the chain of ever-larger variants.
                let mut y = chr;
                for _ in 0..=256 {
                    let Some(m) = g.metrics(y) else { break };
                    if g.recipe(y).is_some() {
                        found = Some((fam, size, y));
                        done = true;
                        break;
                    }
                    let u = m.height + m.depth;
                    if u > w {
                        found = Some((fam, size, y));
                        w = u;
                        if u >= v {
                            done = true;
                            break;
                        }
                    }
                    match g.next_larger(y) {
                        Some(next) => y = next,
                        None => break,
                    }
                }
            }
        }
        let mut b = match found {
            Some((fam, size, code)) => {
                match self.fonts.font(fam, size).and_then(|f| f.recipe(code)) {
                    Some(r) => self.extensible(fam, size, r, v),
                    None => self.char_box(fam, size, code),
                }
            }
            // §706: no delimiter at all is a box of `\nulldelimiterspace`.
            None => BoxNode {
                width: NULL_DELIMITER_SPACE,
                ..BoxNode::null()
            },
        };
        b.shift_amount = half(b.height - b.depth) - self.fonts.axis_height(s);
        b
    }

    /// "Construct an extensible character in a new box `b`" (§713-§714).
    ///
    /// The recipe is four character codes -- top, mid, bot, rep -- and the
    /// delimiter is built by stacking as few copies of the repeatable piece as
    /// will reach `v`, in pairs when there is a middle piece.
    fn extensible(&self, fam: usize, size: usize, r: [u8; 4], v: Scaled) -> BoxNode {
        let (top, mid, bot, rep) = (r[0], r[1], r[2], r[3]);
        let Some(f) = self.fonts.font(fam, size) else {
            return BoxNode::null();
        };
        // §714: the width is the repeatable module's, italic correction
        // included, and the height starts as the fixed pieces' total.
        let u = f.height_plus_depth(rep);
        let width = f.metrics(rep).map(|m| m.width + m.italic).unwrap_or(0);
        let mut w: Scaled = 0;
        for c in [bot, mid, top] {
            if c != 0 {
                w += f.height_plus_depth(c);
            }
        }
        let mut n = 0;
        if u > 0 {
            while w < v {
                w += u;
                n += 1;
                if mid != 0 {
                    w += u;
                }
            }
        }
        // §713: stacked from the bottom up, so the list is built bottom-first
        // and reversed at the end -- `stack_into_box` puts each new piece on
        // TOP of what is already there.
        let mut stacked: Vec<BoxNode> = Vec::new();
        if bot != 0 {
            stacked.push(self.char_box(fam, size, bot));
        }
        for _ in 0..n {
            stacked.push(self.char_box(fam, size, rep));
        }
        if mid != 0 {
            stacked.push(self.char_box(fam, size, mid));
            for _ in 0..n {
                stacked.push(self.char_box(fam, size, rep));
            }
        }
        if top != 0 {
            stacked.push(self.char_box(fam, size, top));
        }
        // The topmost piece is the last one stacked, and §711 makes the box's
        // height that piece's height alone.
        let height = stacked.last().map(|b| b.height).unwrap_or(0);
        stacked.reverse();
        let list: Vec<Node> = stacked.into_iter().map(Node::Box).collect();
        BoxNode {
            vertical: true,
            width,
            height,
            depth: w - height,
            list,
            ..BoxNode::null()
        }
    }

    // ── the make_* procedures, §734-§759 ─────────────────────────────────

    /// `make_over(q)` (§734).
    fn make_over(&mut self, a: &mut Atom) {
        let t = self.default_rule_thickness();
        let b = self.clean_box(&a.nucleus, cramped_style(self.style));
        a.nucleus = Field::Box(self.overbar(b, 3 * t, t));
    }

    /// `make_under(q)` (§735).
    fn make_under(&mut self, a: &mut Atom) {
        let t = self.default_rule_thickness();
        let x = self.clean_box(&a.nucleus, self.style);
        let x_height = x.height;
        let list = vec![Node::Box(x), kern(3 * t), self.fraction_rule(t)];
        let mut y = vpack(list, NATURAL, Tolerances::plain()).node;
        let delta = y.height + y.depth + t;
        y.height = x_height;
        y.depth = delta - y.height;
        a.nucleus = Field::Box(y);
    }

    /// `make_radical(q)` (§737).
    fn make_radical(&mut self, r: &mut Radical) {
        let t = self.default_rule_thickness();
        let x = self.clean_box(&r.nucleus.nucleus, cramped_style(self.style));
        // §737: the clearance is larger in display style.
        let mut clr = match self.style < TEXT_STYLE {
            true => t + self.math_x_height().abs() / 4,
            false => t + t.abs() / 4,
        };
        let mut y = self.var_delimiter(r.left_delimiter, self.size, x.height + x.depth + clr + t);
        let delta = y.depth - (x.height + x.depth + clr);
        if delta > 0 {
            clr += half(delta);
        }
        y.shift_amount = -(x.height + clr);
        let y_height = y.height;
        let bar = self.overbar(x, clr, y_height);
        let list = vec![Node::Box(y), Node::Box(bar)];
        r.nucleus.nucleus = Field::Box(hpack(list, NATURAL, Tolerances::plain(), None).node);
    }

    /// `make_fraction(q)` (§743-§748). Unlike the others it produces the
    /// noad's whole hlist rather than a nucleus.
    fn make_fraction(&mut self, f: &Fraction) -> Vec<Node> {
        let thickness = f.thickness.unwrap_or_else(|| self.default_rule_thickness());
        // §744: equal-width numerator and denominator, and the default shifts.
        let mut x = self.clean_box(&Field::List(f.numerator.clone()), num_style(self.style));
        let mut z = self.clean_box(&Field::List(f.denominator.clone()), denom_style(self.style));
        match x.width < z.width {
            true => x = self.rebox(x, z.width),
            false => z = self.rebox(z, x.width),
        }
        let (mut shift_up, mut shift_down) = match self.style < TEXT_STYLE {
            true => (self.fonts.num1(self.size), self.fonts.denom1(self.size)),
            false => (
                match thickness != 0 {
                    true => self.fonts.num2(self.size),
                    false => self.fonts.num3(self.size),
                },
                self.fonts.denom2(self.size),
            ),
        };
        let mut delta = 0;
        if thickness == 0 {
            // §745: no rule, so the clearance is stated outright.
            let clr = match self.style < TEXT_STYLE {
                true => 7 * self.default_rule_thickness(),
                false => 3 * self.default_rule_thickness(),
            };
            let d = half(clr - ((shift_up - x.depth) - (z.height - shift_down)));
            if d > 0 {
                shift_up += d;
                shift_down += d;
            }
        } else {
            // §746: with a rule, the clearance depends on its thickness and
            // both halves are measured from the axis.
            let clr = match self.style < TEXT_STYLE {
                true => 3 * thickness,
                false => thickness,
            };
            delta = half(thickness);
            let delta1 = clr - ((shift_up - x.depth) - (self.axis_height() + delta));
            let delta2 = clr - ((self.axis_height() - delta) - (z.height - shift_down));
            if delta1 > 0 {
                shift_up += delta1;
            }
            if delta2 > 0 {
                shift_down += delta2;
            }
        }
        // §747: the vlist, built top down.
        let height = shift_up + x.height;
        let depth = z.depth + shift_down;
        let width = x.width;
        let mut list: Vec<Node> = vec![Node::Box(x.clone())];
        match thickness == 0 {
            true => list.push(kern((shift_up - x.depth) - (z.height - shift_down))),
            false => {
                list.push(kern((shift_up - x.depth) - (self.axis_height() + delta)));
                list.push(self.fraction_rule(thickness));
                list.push(kern((self.axis_height() - delta) - (z.height - shift_down)));
            }
        }
        list.push(Node::Box(z));
        let v = BoxNode {
            vertical: true,
            width,
            height,
            depth,
            list,
            ..BoxNode::null()
        };
        // §748: the delimiters either side, sized by `delim1` or `delim2`.
        let size = match self.style < TEXT_STYLE {
            true => self.fonts.delim1(self.size),
            false => self.fonts.delim2(self.size),
        };
        let left = self.var_delimiter(f.left_delimiter, self.size, size);
        let right = self.var_delimiter(f.right_delimiter, self.size, size);
        let whole = vec![Node::Box(left), Node::Box(v), Node::Box(right)];
        vec![Node::Box(
            hpack(whole, NATURAL, Tolerances::plain(), None).node,
        )]
    }

    /// `make_op(q)` (§749-§751). Returns the italic correction, which §754
    /// uses as the offset between a subscript and a superscript, and fills in
    /// the noad's hlist itself when the limits go above and below.
    fn make_op(&mut self, a: &mut Atom) -> (Scaled, Option<Vec<Node>>) {
        if a.limits == Limits::Normal && self.style < TEXT_STYLE {
            a.limits = Limits::Above;
        }
        let mut delta = 0;
        if let Field::Char(c) = a.nucleus {
            let mut c = c;
            // §749: in display style an operator is replaced by the next
            // larger variant, which is how `\sum` grows.
            if self.style < TEXT_STYLE {
                if let Some(f) = self.fonts.font(c.fam, self.size) {
                    if let Some(larger) = f.next_larger(c.character) {
                        if f.metrics(larger).is_some() {
                            c.character = larger;
                        }
                    }
                }
            }
            if let Some((_, m)) = self.fetch(c, self.size) {
                delta = m.italic;
            }
            a.nucleus = Field::Char(c);
            let mut x = self.clean_box(&Field::Char(c), self.style);
            if !a.subscr.is_empty() && a.limits != Limits::Above {
                x.width -= delta;
            }
            x.shift_amount = half(x.height - x.depth) - self.axis_height();
            a.nucleus = Field::Box(x);
        }
        if a.limits != Limits::Above {
            return (delta, None);
        }
        // §750: a vlist of the superscript, the operator and the subscript,
        // each reboxed to the widest of the three.
        let x = self.clean_box(&a.supscr, sup_style(self.style));
        let y = self.clean_box(&a.nucleus, self.style);
        let z = self.clean_box(&a.subscr, sub_style(self.style));
        let mut width = y.width;
        width = width.max(x.width).max(z.width);
        let mut x = self.rebox(x, width);
        let y = self.rebox(y, width);
        let mut z = self.rebox(z, width);
        x.shift_amount = half(delta);
        z.shift_amount = -x.shift_amount;
        let mut height = y.height;
        let mut depth = y.depth;
        let mut list: Vec<Node> = Vec::new();
        // §751: the kerns above and below, and what they add to the box.
        if a.supscr.is_empty() {
            list.push(Node::Box(y.clone()));
        } else {
            let mut shift_up = self.fonts.big_op_spacing3(self.size) - x.depth;
            let floor = self.fonts.big_op_spacing1(self.size);
            if shift_up < floor {
                shift_up = floor;
            }
            height += self.fonts.big_op_spacing5(self.size) + x.height + x.depth + shift_up;
            list.push(kern(self.fonts.big_op_spacing5(self.size)));
            list.push(Node::Box(x.clone()));
            list.push(kern(shift_up));
            list.push(Node::Box(y.clone()));
        }
        if !a.subscr.is_empty() {
            let mut shift_down = self.fonts.big_op_spacing4(self.size) - z.height;
            let floor = self.fonts.big_op_spacing2(self.size);
            if shift_down < floor {
                shift_down = floor;
            }
            depth += self.fonts.big_op_spacing5(self.size) + z.height + z.depth + shift_down;
            list.push(kern(shift_down));
            list.push(Node::Box(z.clone()));
            list.push(kern(self.fonts.big_op_spacing5(self.size)));
        }
        let v = BoxNode {
            vertical: true,
            width,
            height,
            depth,
            list,
            ..BoxNode::null()
        };
        (delta, Some(vec![Node::Box(v)]))
    }

    /// `math_type(nucleus(q)) := math_text_char` (§752): whether this noad's
    /// character is followed immediately by another from the same family, so
    /// that the two are a word rather than two symbols.
    fn text_char(&self, a: &Atom, next: Option<&Noad>) -> bool {
        if !a.subscr.is_empty() || !a.supscr.is_empty() || a.class() != Class::Ord {
            return false;
        }
        let (Field::Char(q), Some(Noad::Atom(p))) = (&a.nucleus, next) else {
            return false;
        };
        if !matches!(
            p.class(),
            Class::Ord
                | Class::Op
                | Class::Bin
                | Class::Rel
                | Class::Open
                | Class::Close
                | Class::Punct
        ) {
            return false;
        }
        matches!(p.nucleus, Field::Char(r) if r.fam == q.fam)
    }

    /// `make_ord(q)` (§752-§753), kerns only.
    ///
    /// The ligature half of §752 has nothing to do in a math font -- Computer
    /// Modern's math families define no ligature programs -- but the KERNS
    /// do: `cmmi10` kerns its italic letters against the punctuation and the
    /// scripts beside them, and dropping those would set `$f(x)$` with the
    /// `f` running into the parenthesis.
    fn make_ord_kern(&self, a: &Atom, next: Option<&Noad>) -> Option<Scaled> {
        if !a.subscr.is_empty() || !a.supscr.is_empty() {
            return None;
        }
        let Field::Char(q) = a.nucleus else {
            return None;
        };
        let Some(Noad::Atom(p)) = next else {
            return None;
        };
        if !matches!(
            p.class(),
            Class::Ord
                | Class::Op
                | Class::Bin
                | Class::Rel
                | Class::Open
                | Class::Close
                | Class::Punct
        ) {
            return None;
        }
        let Field::Char(r) = p.nucleus else {
            return None;
        };
        if r.fam != q.fam {
            return None;
        }
        let f = self.fonts.font(q.fam, self.size)?;
        match f.tfm.step(q.character, r.character) {
            Some(crate::tfm::Step::Kern { by, .. }) => Some((by * f.at * 65536.0).round() as i64),
            _ => None,
        }
    }

    /// `make_scripts(q,delta)` (§756-§759): the subscript and superscript,
    /// shifted onto the list the nucleus produced.
    fn make_scripts(&mut self, a: &Atom, hlist: &mut Vec<Node>, delta: Scaled) {
        let mut shift_up: Scaled = 0;
        let mut shift_down: Scaled = 0;
        // §756: a bare character sits on the baseline, so the scripts are
        // shifted from there; anything else is measured. The test is
        // `is_char_node(p)` on the FIRST node -- a character followed by its
        // italic correction is still a character node -- and an empty hlist is
        // measured, which is what gives `${}^2$` its shift.
        if !hlist.first().map(|n| n.is_char()).unwrap_or(false) {
            let z = hpack(hlist.clone(), NATURAL, Tolerances::plain(), None).node;
            let t = match self.style < SCRIPT_STYLE {
                true => 1,
                false => 2,
            };
            shift_up = z.height - self.fonts.sup_drop(t);
            shift_down = z.depth + self.fonts.sub_drop(t);
        }
        let x = if a.supscr.is_empty() {
            // §757: a subscript on its own must not rise above four fifths of
            // the x-height.
            let mut x = self.clean_box(&a.subscr, sub_style(self.style));
            x.width += SCRIPT_SPACE;
            let floor = self.fonts.sub1(self.size);
            if shift_down < floor {
                shift_down = floor;
            }
            let clr = x.height - (self.math_x_height().abs() * 4) / 5;
            if shift_down < clr {
                shift_down = clr;
            }
            x.shift_amount = shift_down;
            x
        } else {
            // §758: a superscript must not descend below a quarter of the
            // x-height.
            let mut x = self.clean_box(&a.supscr, sup_style(self.style));
            x.width += SCRIPT_SPACE;
            let mut clr = match self.style % 2 == 1 {
                true => self.fonts.sup3(self.size),
                false => match self.style < TEXT_STYLE {
                    true => self.fonts.sup1(self.size),
                    false => self.fonts.sup2(self.size),
                },
            };
            if shift_up < clr {
                shift_up = clr;
            }
            clr = x.depth + self.math_x_height().abs() / 4;
            if shift_up < clr {
                shift_up = clr;
            }
            if a.subscr.is_empty() {
                x.shift_amount = -shift_up;
                x
            } else {
                // §759: both present. They must be four rule thicknesses
                // apart, and the superscript sits `delta` to the right.
                let mut y = self.clean_box(&a.subscr, sub_style(self.style));
                y.width += SCRIPT_SPACE;
                let floor = self.fonts.sub2(self.size);
                if shift_down < floor {
                    shift_down = floor;
                }
                let mut clr = 4 * self.default_rule_thickness()
                    - ((shift_up - x.depth) - (y.height - shift_down));
                if clr > 0 {
                    shift_down += clr;
                    clr = (self.math_x_height().abs() * 4) / 5 - (shift_up - x.depth);
                    if clr > 0 {
                        shift_up += clr;
                        shift_down -= clr;
                    }
                }
                x.shift_amount = delta;
                let gap = (shift_up - x.depth) - (y.height - shift_down);
                let list = vec![Node::Box(x), kern(gap), Node::Box(y)];
                let mut v = vpack(list, NATURAL, Tolerances::plain()).node;
                v.shift_amount = shift_down;
                v
            }
        };
        hlist.push(Node::Box(x));
    }

    /// `make_left_right(q,style,max_d,max_h)` (§762): how tall a `\left` or
    /// `\right` delimiter has to be, given the formula it encloses.
    fn make_left_right(
        &mut self,
        d: Delimiter,
        style: Style,
        max_d: Scaled,
        max_h: Scaled,
    ) -> Vec<Node> {
        let size = size_of(style);
        let delta2 = max_d + self.fonts.axis_height(size);
        let mut delta1 = max_h + max_d - delta2;
        if delta2 > delta1 {
            delta1 = delta2;
        }
        let mut delta = (delta1 / 500) * DELIMITER_FACTOR;
        let delta2 = delta1 + delta1 - DELIMITER_SHORTFALL;
        if delta < delta2 {
            delta = delta2;
        }
        vec![Node::Box(self.var_delimiter(d, size, delta))]
    }

    // ── mlist_to_hlist itself, §726-§767 ─────────────────────────────────

    /// `mlist_to_hlist` (§726): the whole conversion, in the given style.
    pub fn convert(&mut self, mlist: &[Noad], style: Style) -> Vec<Node> {
        self.set_style(style);
        let mut items: Vec<Translated> = Vec::with_capacity(mlist.len());
        let mut r_index: Option<usize> = None;
        let mut r_type = RType::Class(Class::Op);
        let mut max_h: Scaled = 0;
        let mut max_d: Scaled = 0;

        for (at, item) in mlist.iter().enumerate() {
            // §730: the nodes that can appear among noads.
            match item {
                Noad::Style(s) => {
                    self.set_style(*s);
                    items.push(Translated {
                        class: None,
                        hlist: Vec::new(),
                        style: Some(*s),
                        delimiter: None,
                    });
                    continue;
                }
                Noad::Glue(w) => {
                    items.push(Translated::plain(Node::Glue(GlueNode::new(Glue::fixed(
                        *w,
                    )))));
                    continue;
                }
                Noad::Kern(w) => {
                    items.push(Translated::plain(kern(*w)));
                    continue;
                }
                Noad::Node(n) => {
                    items.push(Translated::plain(n.clone()));
                    continue;
                }
                _ => {}
            }

            // §728's Bin-to-Ord rewrites, which depend on what came before.
            let mut class = class_of(item);
            if class == RType::Class(Class::Bin)
                && matches!(
                    r_type,
                    RType::Class(Class::Bin)
                        | RType::Class(Class::Op)
                        | RType::Class(Class::Rel)
                        | RType::Class(Class::Open)
                        | RType::Class(Class::Punct)
                        | RType::Left
                )
            {
                class = RType::Class(Class::Ord);
            }
            if matches!(
                class,
                RType::Class(Class::Rel)
                    | RType::Class(Class::Close)
                    | RType::Class(Class::Punct)
                    | RType::Right
            ) {
                // §729: a Bin with nothing binary after it is an Ord.
                if r_type == RType::Class(Class::Bin) {
                    if let Some(i) = r_index {
                        items[i].class = Some(RType::Class(Class::Ord));
                    }
                }
            }

            let (hlist, delimiter) = match item {
                Noad::Left(d) | Noad::Right(d) => (Vec::new(), Some(*d)),
                Noad::Fraction(f) => (self.make_fraction(f), None),
                Noad::Radical(r) => {
                    let mut r = r.clone();
                    self.make_radical(&mut r);
                    (
                        self.nucleus_and_scripts(&r.nucleus, 0, mlist.get(at + 1)),
                        None,
                    )
                }
                Noad::Over(a) => {
                    let mut a = a.clone();
                    self.make_over(&mut a);
                    (self.nucleus_and_scripts(&a, 0, mlist.get(at + 1)), None)
                }
                Noad::Under(a) => {
                    let mut a = a.clone();
                    self.make_under(&mut a);
                    (self.nucleus_and_scripts(&a, 0, mlist.get(at + 1)), None)
                }
                Noad::Atom(a) => {
                    let mut a = a.clone();
                    let mut delta = 0;
                    let mut built = None;
                    // §733: an Op is the one class with a procedure of its
                    // own; `make_ord`'s kerns run inside `nucleus_and_scripts`
                    // below, and Open and Inner have nothing done to them.
                    if a.class() == Class::Op {
                        let (d, limits_box) = self.make_op(&mut a);
                        delta = d;
                        built = limits_box;
                    }
                    match built {
                        Some(b) => (b, None),
                        None => (self.nucleus_and_scripts(&a, delta, mlist.get(at + 1)), None),
                    }
                }
                _ => (Vec::new(), None),
            };

            // §727: `check_dimensions` -- the tallest and deepest so far, for
            // the `\left` and `\right` the second pass will size.
            if delimiter.is_none() {
                let z = hpack(hlist.clone(), NATURAL, Tolerances::plain(), None).node;
                max_h = max_h.max(z.height);
                max_d = max_d.max(z.depth);
            }
            r_index = Some(items.len());
            r_type = class;
            items.push(Translated {
                class: Some(class),
                hlist,
                style: None,
                delimiter,
            });
        }
        // §726's closing `Convert a final bin_noad to an ord_noad`.
        if r_type == RType::Class(Class::Bin) {
            if let Some(i) = r_index {
                items[i].class = Some(RType::Class(Class::Ord));
            }
        }
        self.second_pass(items, style, max_h, max_d)
    }

    /// "Convert `nucleus(q)` to an hlist and attach the sub/superscripts"
    /// (§754-§755).
    fn nucleus_and_scripts(
        &mut self,
        a: &Atom,
        mut delta: Scaled,
        next: Option<&Noad>,
    ) -> Vec<Node> {
        let mut p: Vec<Node> = match &a.nucleus {
            Field::Char(c) => {
                // §755: the character, followed by its italic correction when
                // there is no subscript to hold the correction's place.
                match self.fetch(*c, self.size) {
                    Some((f, m)) => {
                        let mut list = Vec::new();
                        if let Some(node) = self.char_node(*c, self.size) {
                            list.push(node);
                        }
                        delta = m.italic;
                        // §752: an Ord whose nucleus is followed immediately
                        // by another character of the SAME family is a
                        // `math_text_char`, and §755 gives it no italic
                        // correction when the font is a TEXT font -- one whose
                        // `space` parameter is not zero. That is what keeps
                        // `\log` from setting as `lo` and a kerned `g`: the
                        // three letters are cmr10's, and cmr10 has a space.
                        if self.text_char(a, next) && f.param(2) != 0 {
                            delta = 0;
                        }
                        if a.subscr.is_empty() && delta != 0 {
                            list.push(kern(delta));
                            delta = 0;
                        }
                        list
                    }
                    None => Vec::new(),
                }
            }
            Field::Empty => Vec::new(),
            Field::Box(b) => vec![Node::Box(b.clone())],
            Field::List(l) => {
                let save = self.style;
                let inner = self.convert(l, save);
                self.set_style(save);
                vec![Node::Box(
                    hpack(inner, NATURAL, Tolerances::plain(), None).node,
                )]
            }
            Field::Literal(c) => vec![self.literal_node(*c, self.size)],
        };
        // §752: the kern `make_ord` finds between this character and the next.
        if let Some(by) = self.make_ord_kern(a, next) {
            if by != 0 {
                p.push(kern(by));
            }
        }
        if a.subscr.is_empty() && a.supscr.is_empty() {
            return p;
        }
        self.make_scripts(a, &mut p, delta);
        p
    }

    /// "Make a second pass over the mlist" (§760-§767): the noads go, the
    /// spacing of §764 arrives.
    fn second_pass(
        &mut self,
        items: Vec<Translated>,
        style: Style,
        max_h: Scaled,
        max_d: Scaled,
    ) -> Vec<Node> {
        let mut out: Vec<Node> = Vec::new();
        let mut r_type: Option<Class> = None;
        self.set_style(style);
        for item in items {
            if let Some(s) = item.style {
                self.set_style(s);
                continue;
            }
            let Some(class) = item.class else {
                out.extend(item.hlist);
                continue;
            };
            // §761: a `left_noad` or a `right_noad` becomes an Open or a
            // Close, and its delimiter is grown to fit the formula.
            let (t, hlist) = match (class, item.delimiter) {
                (RType::Left, Some(d)) => {
                    (Class::Open, self.make_left_right(d, style, max_d, max_h))
                }
                (RType::Right, Some(d)) => {
                    (Class::Close, self.make_left_right(d, style, max_d, max_h))
                }
                (RType::Class(c), _) => (c, item.hlist),
                _ => (Class::Ord, item.hlist),
            };
            if let Some(r) = r_type {
                if let Some(g) = self.inter_element(r, t) {
                    out.push(Node::Glue(GlueNode::new(g)));
                }
            }
            out.extend(hlist);
            r_type = Some(t);
        }
        out
    }

    /// "Append inter-element spacing based on `r_type` and `t`" (§766).
    ///
    /// The glue is `math_glue` (§716) of the parameter §764's digit names,
    /// converted from `mu` to points at the current size's `cur_mu`.
    fn inter_element(&self, r: Class, t: Class) -> Option<Glue> {
        let script = self.style >= SCRIPT_STYLE;
        let (w, plus, minus) = match spacing(r, t) {
            Space::None | Space::Impossible => return None,
            Space::ConditionalThin if script => return None,
            Space::ConditionalThin | Space::Thin => THIN_MU_SKIP,
            Space::ConditionalMedium if script => return None,
            Space::ConditionalMedium => MED_MU_SKIP,
            Space::ConditionalThick if script => return None,
            Space::ConditionalThick => THICK_MU_SKIP,
        };
        Some(self.math_glue(w, plus, minus))
    }

    /// `math_glue(g,m)` (§716): a glue written in `mu`, in points.
    ///
    /// §716 splits `cur_mu` into an integer part `n` and a fraction `f` and
    /// computes `n*w + xn_over_d(w,f,"200000)` for each component, which is
    /// `w * cur_mu / 65536` done without a 64-bit multiply.
    fn math_glue(&self, w: Scaled, plus: Scaled, minus: Scaled) -> Glue {
        let mut n = x_over_n(self.mu, 65536);
        let mut f = self.mu - n * 65536;
        if f < 0 {
            n -= 1;
            f += 65536;
        }
        let mult = |x: Scaled| n * x + xn_over_d(x, f, 65536);
        Glue {
            natural: mult(w),
            stretch: mult(plus),
            stretch_order: 0,
            shrink: mult(minus),
            shrink_order: 0,
        }
    }
}

/// The `type(q)` of a noad, as §761 reads it.
fn class_of(item: &Noad) -> RType {
    match item {
        Noad::Atom(a) => RType::Class(a.class()),
        // §761: a fraction, a radical, an overline and an underline are all
        // Ord for spacing purposes.
        Noad::Fraction(_) | Noad::Radical(_) | Noad::Over(_) | Noad::Under(_) => {
            RType::Class(Class::Ord)
        }
        Noad::Left(_) => RType::Left,
        Noad::Right(_) => RType::Right,
        _ => RType::Class(Class::Ord),
    }
}

/// The formula, boxed: what `\hbox`ing the result of `mlist_to_hlist` gives.
pub fn set(fonts: &MathFonts, mlist: &[Noad], style: Style) -> BoxNode {
    let list = Converter::new(fonts).convert(mlist, style);
    hpack(list, NATURAL, Tolerances::plain(), None).node
}
