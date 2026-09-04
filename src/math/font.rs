//! The three families a math formula is set from, and their `fontdimen`
//! parameters.
//!
//! `tex.web` §699-§701. A formula reaches the page through four font families
//! and three sizes of each:
//!
//! | family | plain.tex (§477-§483) | what it carries |
//! |---|---|---|
//! | 0 | `cmr10` / `cmr7` / `cmr5` | roman: digits, upright letters, `+`, `=` |
//! | 1 | `cmmi10` / `cmmi7` / `cmmi5` | math italic: the variables, lowercase greek |
//! | 2 | `cmsy10` / `cmsy7` / `cmsy5` | the symbols, and §700's twenty-two parameters |
//! | 3 | `cmex10` at all three sizes | the big operators, and §701's thirteen |
//!
//! §700 and §701 are the reason this module exists rather than the character
//! metrics: EVERY shift, gap and clearance in `mlist_to_hlist` is one of those
//! parameters. `\sqrt`'s clearance is `default_rule_thickness`, a superscript's
//! height is `sup1`, a fraction bar sits at `axis_height`. Without them there
//! is no math layout, only characters in a row.
//!
//! The size codes here are 0, 1, 2 where `tex.web` uses 0, 16, 32 (§699): the
//! multiple of sixteen is how Knuth packs a family and a size into one index
//! into `fam_fnt`, and this carries the two apart.

use crate::tfm::{Tag, Tfm};

/// A dimension in scaled points, as everywhere else in the stomach.
pub type Scaled = i64;

/// `text_size` (§699), renumbered: the largest size in a family.
pub const TEXT_SIZE: usize = 0;
/// `script_size` (§699), renumbered.
pub const SCRIPT_SIZE: usize = 1;
/// `script_script_size` (§699), renumbered.
pub const SCRIPT_SCRIPT_SIZE: usize = 2;

/// The four families, at the three sizes, as plain.tex loads them (§477-§483).
///
/// `\textfont3`, `\scriptfont3` and `\scriptscriptfont3` are all `cmex10`:
/// the extension font has one size and the smaller styles use it unchanged.
const FAMILY_FILES: [[&str; 3]; 4] = [
    ["cmr10", "cmr7", "cmr5"],
    ["cmmi10", "cmmi7", "cmmi5"],
    ["cmsy10", "cmsy7", "cmsy5"],
    ["cmex10", "cmex10", "cmex10"],
];

/// The size each of those is loaded AT, in points, for a 10pt document.
///
/// plain.tex loads `cmr10` at its design size, `cmr7` at seven points and
/// `cmr5` at five. A document set at another type size scales all three by the
/// same ratio, which is what `MathFonts::load` does with `size`.
const FAMILY_AT: [[f64; 3]; 4] = [
    [10.0, 7.0, 5.0],
    [10.0, 7.0, 5.0],
    [10.0, 7.0, 5.0],
    [10.0, 10.0, 10.0],
];

/// One loaded font: its metrics, the size it is set at, and its extensible
/// recipes.
pub struct MathFont {
    pub tfm: Tfm,
    /// The size the font is loaded at, in points.
    pub at: f64,
    /// The `exten` table (§544): four character codes -- top, mid, bot, rep --
    /// per recipe, indexed by a character's `remainder` when its tag is
    /// `ext_tag`. `src/tfm.rs` reads every other table but not this one, so it
    /// is read here from the same file rather than being reached for through
    /// an API that has not got it.
    pub exten: Vec<[u8; 4]>,
}

impl MathFont {
    /// A value in design-size units, in scaled points at this font's size.
    ///
    /// `tex.web` §572 does the same multiplication in fixed point while it
    /// reads the file; `src/tfm.rs` has already converted to `f64`, so this
    /// rounds rather than repeating Knuth's arithmetic, and the two can differ
    /// by a single scaled point on a value that lands exactly between.
    fn sp(&self, v: f64) -> Scaled {
        (v * self.at * 65536.0).round() as i64
    }

    /// `param(n)` for a one-based `fontdimen` number, the way §700's macros
    /// index it.
    pub fn param(&self, n: usize) -> Scaled {
        match n.checked_sub(1).and_then(|i| self.tfm.raw_params.get(i)) {
            Some(v) => self.sp(*v),
            None => 0,
        }
    }

    /// `char_width`, `char_height`, `char_depth`, `char_italic` for one code
    /// (§554), in scaled points, or `None` when the font has no such character.
    pub fn metrics(&self, code: u8) -> Option<CharDim> {
        let m = self.tfm.char(code)?;
        Some(CharDim {
            width: self.sp(m.width),
            height: self.sp(m.height),
            depth: self.sp(m.depth),
            italic: self.sp(m.italic),
        })
    }

    /// `height_plus_depth(f,c)` (§712).
    pub fn height_plus_depth(&self, code: u8) -> Scaled {
        self.metrics(code).map(|m| m.height + m.depth).unwrap_or(0)
    }

    /// `char_tag(q)=list_tag` (§544): the next larger character in a chain of
    /// sizes, which is how `\sum` grows in display style (§749) and how
    /// `var_delimiter` walks the variants (§708).
    pub fn next_larger(&self, code: u8) -> Option<u8> {
        match self.tfm.tag(code) {
            Tag::List => self.tfm.char(code).map(|c| c.remainder),
            _ => None,
        }
    }

    /// The `ext_top`, `ext_mid`, `ext_bot`, `ext_rep` of a character whose tag
    /// is `ext_tag` (§544, §713), or `None` when it has no recipe.
    pub fn recipe(&self, code: u8) -> Option<[u8; 4]> {
        match self.tfm.tag(code) {
            Tag::Extensible => {
                let at = self.tfm.char(code)?.remainder as usize;
                self.exten.get(at).copied()
            }
            _ => None,
        }
    }
}

/// One character's four dimensions, in scaled points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CharDim {
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    pub italic: Scaled,
}

/// The families and sizes a formula is set from.
pub struct MathFonts {
    fonts: [[Option<MathFont>; 3]; 4],
    /// The document's type size, in points: what a 10pt design is scaled to.
    pub size: f64,
}

impl MathFonts {
    /// Load every family at every size, skipping any the machine has not got.
    ///
    /// A missing font is not an error here for the same reason it is not one
    /// in `FontChain::load`: the metrics belong to a TeX INSTALLATION, and an
    /// engine that refused to run without one would refuse on a machine that
    /// can still set the text.
    pub fn load(size: f64) -> MathFonts {
        let ratio = size / 10.0;
        let mut fonts: [[Option<MathFont>; 3]; 4] = Default::default();
        for fam in 0..4 {
            for s in 0..3 {
                let Some(path) = crate::typeset::find_font(FAMILY_FILES[fam][s]) else {
                    continue;
                };
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                let Ok(tfm) = Tfm::parse(&bytes) else {
                    continue;
                };
                fonts[fam][s] = Some(MathFont {
                    tfm,
                    at: FAMILY_AT[fam][s] * ratio,
                    exten: exten_table(&bytes),
                });
            }
        }
        MathFonts { fonts, size }
    }

    /// Whether the installation carried enough to set a formula at all.
    ///
    /// Family 2 holds §700's parameters and family 3 holds §701's; without
    /// either there are no shifts, no gaps and no rule thickness, so there is
    /// nothing to lay out with.
    pub fn usable(&self) -> bool {
        self.fonts[2][0].is_some() && self.fonts[3][0].is_some()
    }

    /// `fam_fnt(fam+size)` (§699).
    pub fn font(&self, fam: usize, size: usize) -> Option<&MathFont> {
        self.fonts.get(fam)?.get(size)?.as_ref()
    }

    /// The point size a family is set at, which is what a page has to write
    /// into its `Tf` operator.
    pub fn at(&self, fam: usize, size: usize) -> f64 {
        self.font(fam, size)
            .map(|f| f.at)
            .unwrap_or(FAMILY_AT[fam.min(3)][size.min(2)] * self.size / 10.0)
    }

    /// `mathsy(n)(size)` (§700): a parameter of the math SYMBOL font, family 2.
    pub fn sigma(&self, size: usize, n: usize) -> Scaled {
        self.font(2, size).map(|f| f.param(n)).unwrap_or(0)
    }

    /// `mathex(n)` (§701): a parameter of the math EXTENSION font, family 3.
    pub fn xi(&self, size: usize, n: usize) -> Scaled {
        self.font(3, size).map(|f| f.param(n)).unwrap_or(0)
    }

    /// `math_x_height` (§700) — the height of `x`.
    pub fn math_x_height(&self, size: usize) -> Scaled {
        self.sigma(size, 5)
    }
    /// `math_quad` (§700) — `18mu`.
    pub fn math_quad(&self, size: usize) -> Scaled {
        self.sigma(size, 6)
    }
    /// `num1` (§700) — numerator shift-up in display styles.
    pub fn num1(&self, size: usize) -> Scaled {
        self.sigma(size, 8)
    }
    /// `num2` (§700) — numerator shift-up, non-display, with a rule.
    pub fn num2(&self, size: usize) -> Scaled {
        self.sigma(size, 9)
    }
    /// `num3` (§700) — numerator shift-up, non-display `\atop`.
    pub fn num3(&self, size: usize) -> Scaled {
        self.sigma(size, 10)
    }
    /// `denom1` (§700).
    pub fn denom1(&self, size: usize) -> Scaled {
        self.sigma(size, 11)
    }
    /// `denom2` (§700).
    pub fn denom2(&self, size: usize) -> Scaled {
        self.sigma(size, 12)
    }
    /// `sup1` (§700) — superscript shift-up, uncramped display.
    pub fn sup1(&self, size: usize) -> Scaled {
        self.sigma(size, 13)
    }
    /// `sup2` (§700) — superscript shift-up, uncramped non-display.
    pub fn sup2(&self, size: usize) -> Scaled {
        self.sigma(size, 14)
    }
    /// `sup3` (§700) — superscript shift-up, cramped.
    pub fn sup3(&self, size: usize) -> Scaled {
        self.sigma(size, 15)
    }
    /// `sub1` (§700) — subscript shift-down with no superscript.
    pub fn sub1(&self, size: usize) -> Scaled {
        self.sigma(size, 16)
    }
    /// `sub2` (§700) — subscript shift-down with a superscript.
    pub fn sub2(&self, size: usize) -> Scaled {
        self.sigma(size, 17)
    }
    /// `sup_drop` (§700).
    pub fn sup_drop(&self, size: usize) -> Scaled {
        self.sigma(size, 18)
    }
    /// `sub_drop` (§700).
    pub fn sub_drop(&self, size: usize) -> Scaled {
        self.sigma(size, 19)
    }
    /// `delim1` (§700) — delimiter size in display styles.
    pub fn delim1(&self, size: usize) -> Scaled {
        self.sigma(size, 20)
    }
    /// `delim2` (§700) — delimiter size in the other styles.
    pub fn delim2(&self, size: usize) -> Scaled {
        self.sigma(size, 21)
    }
    /// `axis_height` (§700) — where a fraction bar and a `\left(` are centred.
    pub fn axis_height(&self, size: usize) -> Scaled {
        self.sigma(size, 22)
    }

    /// `default_rule_thickness` (§701) — the thickness of an `\over` bar, and
    /// the unit almost every clearance in `mlist_to_hlist` is stated in.
    pub fn default_rule_thickness(&self, size: usize) -> Scaled {
        self.xi(size, 8)
    }
    /// `big_op_spacing1` (§701).
    pub fn big_op_spacing1(&self, size: usize) -> Scaled {
        self.xi(size, 9)
    }
    /// `big_op_spacing2` (§701).
    pub fn big_op_spacing2(&self, size: usize) -> Scaled {
        self.xi(size, 10)
    }
    /// `big_op_spacing3` (§701).
    pub fn big_op_spacing3(&self, size: usize) -> Scaled {
        self.xi(size, 11)
    }
    /// `big_op_spacing4` (§701).
    pub fn big_op_spacing4(&self, size: usize) -> Scaled {
        self.xi(size, 12)
    }
    /// `big_op_spacing5` (§701).
    pub fn big_op_spacing5(&self, size: usize) -> Scaled {
        self.xi(size, 13)
    }
}

/// The `exten` table out of a `.tfm` file's bytes (§540, §544).
///
/// `src/tfm.rs` reads every other table in the file and skips this one, and it
/// is the table `var_delimiter` builds an arbitrarily tall `\left(` out of
/// (§713). The twelve halfword lengths at the head of the file say where it
/// starts; the arithmetic here is §540's, and anything that does not add up
/// gives an empty table rather than a wrong one.
fn exten_table(bytes: &[u8]) -> Vec<[u8; 4]> {
    let half = |at: usize| -> Option<usize> {
        bytes
            .get(at * 2..at * 2 + 2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]) as usize)
    };
    let (Some(lf), Some(lh), Some(bc), Some(ec)) = (half(0), half(1), half(2), half(3)) else {
        return Vec::new();
    };
    let (Some(nw), Some(nh), Some(nd), Some(ni)) = (half(4), half(5), half(6), half(7)) else {
        return Vec::new();
    };
    let (Some(nl), Some(nk), Some(ne), Some(np)) = (half(8), half(9), half(10), half(11)) else {
        return Vec::new();
    };
    if bc > ec + 1 || ec > 255 {
        return Vec::new();
    }
    if lf != 6 + lh + (ec + 1 - bc) + nw + nh + nd + ni + nl + nk + ne + np {
        return Vec::new();
    }
    let exten = 6 + lh + (ec + 1 - bc) + nw + nh + nd + ni + nl + nk;
    let mut out = Vec::with_capacity(ne);
    for i in 0..ne {
        let at = (exten + i) * 4;
        match bytes.get(at..at + 4) {
            Some(w) => out.push([w[0], w[1], w[2], w[3]]),
            None => break,
        }
    }
    out
}

/// The Unicode character a family-and-code names, for a page that sets the
/// formula in a font that is not Computer Modern.
///
/// The geometry `mlist_to_hlist` computes is Computer Modern's, out of the
/// `.tfm` files above; the glyph a PDF draws comes from whichever face the
/// document is set in, and those are addressed by character rather than by a
/// Computer Modern slot. So every slot a formula can reach says which
/// character it IS -- `cmsy10`'s slot 0x00 is a minus sign, not a hyphen, and
/// its 0x32 is `∈`. A slot with no Unicode spelling draws nothing rather than
/// drawing whatever the face has in that position.
pub fn unicode(fam: usize, code: u8) -> Option<char> {
    match fam {
        0 => roman(code),
        1 => italic(code),
        2 => symbol(code),
        3 => extension(code),
        _ => None,
    }
}

/// `cmr10`'s layout: OT1 text encoding, whose printable ASCII range is itself.
fn roman(code: u8) -> Option<char> {
    // 0x00-0x0A: the upright uppercase Greek `\Gamma` … `\Omega` (plain.tex
    // `\mathchardef\Gamma="7000`).
    const GREEK: [char; 11] = ['Γ', 'Δ', 'Θ', 'Λ', 'Ξ', 'Π', 'Σ', 'Υ', 'Φ', 'Ψ', 'Ω'];
    match code {
        0x00..=0x0A => Some(GREEK[code as usize]),
        // The accents OT1 puts below 0x20, which `\mathaccent` reaches and
        // nothing else does (plain.tex:939-948). Unicode's SPACING accents,
        // because a formula draws the accent as a glyph of its own at a
        // position `make_math_accent` computed -- a combining character would
        // be composed onto whatever the face drew last instead.
        0x12 => Some('\u{2CB}'),
        0x13 => Some('\u{2CA}'),
        0x14 => Some('\u{2C7}'),
        0x15 => Some('\u{2D8}'),
        0x16 => Some('\u{AF}'),
        0x20..=0x5E => Some(code as char),
        // OT1's top five slots are not ASCII: `"5F` is the dot accent, `"7B`
        // and `"7C` the two dashes, `"7D` the hungarumlaut, `"7E` the tilde
        // accent and `"7F` the dieresis. A formula reaches the four accents
        // through `\dot`, `\tilde` and `\ddot`.
        0x5F => Some('\u{2D9}'),
        0x60..=0x7A => Some(code as char),
        0x7B => Some('\u{2013}'),
        0x7C => Some('\u{2014}'),
        0x7D => Some('\u{2DD}'),
        0x7E => Some('\u{2DC}'),
        0x7F => Some('\u{A8}'),
        _ => None,
    }
}

/// `cmmi10`'s layout: math italic (§700's family 1).
fn italic(code: u8) -> Option<char> {
    const CAPS: [char; 11] = ['Γ', 'Δ', 'Θ', 'Λ', 'Ξ', 'Π', 'Σ', 'Υ', 'Φ', 'Ψ', 'Ω'];
    const LOWER: [char; 23] = [
        'α', 'β', 'γ', 'δ', 'ε', 'ζ', 'η', 'θ', 'ι', 'κ', 'λ', 'μ', 'ν', 'ξ', 'π', 'ρ', 'σ', 'τ',
        'υ', 'φ', 'χ', 'ψ', 'ω',
    ];
    const VARIANT: [char; 6] = ['ε', 'ϑ', 'ϖ', 'ϱ', 'ς', 'φ'];
    match code {
        0x00..=0x0A => Some(CAPS[code as usize]),
        0x0B..=0x21 => Some(LOWER[(code - 0x0B) as usize]),
        0x22..=0x27 => Some(VARIANT[(code - 0x22) as usize]),
        0x28 => Some('↼'),
        0x29 => Some('↽'),
        0x2A => Some('⇀'),
        0x2B => Some('⇁'),
        // 0x2C and 0x2D are `\lhook` and `\rhook`, the tails `\hookrightarrow`
        // is welded out of. Half an arrow is not a character, so they draw
        // nothing rather than drawing something else.
        0x2E => Some('▹'),
        0x2F => Some('◃'),
        // The old-style digits, and the punctuation family 1 carries.
        0x30..=0x39 => Some((b'0' + (code - 0x30)) as char),
        0x3A => Some('.'),
        0x3B => Some(','),
        0x3C => Some('<'),
        0x3D => Some('/'),
        0x3E => Some('>'),
        0x3F => Some('⋆'),
        0x40 => Some('∂'),
        0x41..=0x5A => Some(code as char),
        0x5B => Some('♭'),
        0x5C => Some('♮'),
        0x5D => Some('♯'),
        0x5E => Some('⌣'),
        0x5F => Some('⌢'),
        0x60 => Some('ℓ'),
        0x61..=0x7A => Some(code as char),
        0x7B => Some('ı'),
        0x7C => Some('ȷ'),
        0x7D => Some('℘'),
        0x7E => Some('⃗'),
        _ => None,
    }
}

/// `cmsy10`'s layout: the symbols (§700's family 2).
fn symbol(code: u8) -> Option<char> {
    const LOW: [char; 32] = [
        '−', '·', '×', '∗', '÷', '⋄', '±', '∓', '⊕', '⊖', '⊗', '⊘', '⊙', '○', '∘', '•', '≍', '≡',
        '⊆', '⊇', '≤', '≥', '⪯', '⪰', '∼', '≈', '⊂', '⊃', '≪', '≫', '≺', '≻',
    ];
    const ARROWS: [char; 32] = [
        '←', '→', '↑', '↓', '↔', '↗', '↘', '≃', '⇐', '⇒', '⇑', '⇓', '⇔', '↖', '↙', '∝', '′', '∞',
        '∈', '∋', '△', '▽', '̸', '↦', '∀', '∃', '¬', '∅', 'ℜ', 'ℑ', '⊤', '⊥',
    ];
    // Indexed by `code - 0x5A`, so that `\aleph` at 0x40 is the entry before
    // the run that starts at 0x5B.
    const HIGH: [char; 30] = [
        'ℵ', '∪', '∩', '⊎', '∧', '∨', '⊢', '⊣', '⌊', '⌋', '⌈', '⌉', '{', '}', '⟨', '⟩', '|', '‖',
        '↕', '⇕', '∖', '≀', '√', '∐', '∇', '∫', '⊔', '⊓', '⊑', '⊒',
    ];
    match code {
        0x00..=0x1F => Some(LOW[code as usize]),
        0x20..=0x3F => Some(ARROWS[(code - 0x20) as usize]),
        0x40 => Some(HIGH[0]),
        // 0x41-0x5A are the calligraphic capitals, which no text face carries;
        // the letter itself is what a reader gets rather than nothing.
        0x41..=0x5A => Some(code as char),
        0x5B..=0x77 => HIGH.get((code - 0x5A) as usize).copied(),
        0x78 => Some('§'),
        0x79 => Some('†'),
        0x7A => Some('‡'),
        0x7B => Some('¶'),
        0x7C => Some('♣'),
        0x7D => Some('♦'),
        0x7E => Some('♥'),
        0x7F => Some('♠'),
        _ => None,
    }
}

/// `cmex10`'s layout: the big operators and the extensible delimiters
/// (§700's family 3).
///
/// Only the codes a formula addresses by name are here. The pieces an
/// extensible delimiter is built from (`ext_top` and friends) have no Unicode
/// spelling at all -- half a brace is not a character -- so a built-up
/// delimiter draws as the variant it grew from rather than as its pieces.
fn extension(code: u8) -> Option<char> {
    match code {
        0x00 | 0x10 => Some('('),
        0x01 | 0x11 => Some(')'),
        0x02 | 0x12 => Some('['),
        0x03 | 0x13 => Some(']'),
        0x04 | 0x14 => Some('⌊'),
        0x05 | 0x15 => Some('⌋'),
        0x06 | 0x16 => Some('⌈'),
        0x07 | 0x17 => Some('⌉'),
        0x08 | 0x18 => Some('{'),
        0x09 | 0x19 => Some('}'),
        0x0A | 0x1A => Some('⟨'),
        0x0B | 0x1B => Some('⟩'),
        0x0C | 0x1C => Some('|'),
        0x0D | 0x1D => Some('‖'),
        0x0E => Some('/'),
        0x0F => Some('∖'),
        0x46 | 0x47 => Some('⊔'),
        0x48 | 0x49 => Some('∮'),
        0x4A | 0x4B => Some('⊙'),
        0x4C | 0x4D => Some('⊕'),
        0x4E | 0x4F => Some('⊗'),
        0x50 | 0x58 => Some('∑'),
        0x51 | 0x59 => Some('∏'),
        0x52 | 0x5A => Some('∫'),
        0x53 | 0x5B => Some('∪'),
        0x54 | 0x5C => Some('∩'),
        0x55 | 0x5D => Some('⊎'),
        0x56 | 0x5E => Some('∧'),
        0x57 | 0x5F => Some('∨'),
        0x60 | 0x61 => Some('∐'),
        // The wide accents `\widehat` and `\widetilde` reach (plain.tex:949-
        // 950), each a charlist of three ever-wider variants that §741 walks.
        // Unicode has no wide circumflex, so all three of each draw the
        // spacing accent the narrow one does.
        0x62..=0x64 => Some('\u{2C6}'),
        0x65..=0x67 => Some('\u{2DC}'),
        0x70..=0x75 => Some('√'),
        _ => None,
    }
}
