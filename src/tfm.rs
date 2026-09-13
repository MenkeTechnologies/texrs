//! Reading a `.tfm`, the font metrics every TeX engine measures with.
//!
//! Ported from tectonic's xetex `read_font_info` (`xetex_ini.c`), which is
//! Knuth's `tex.web` §539–§576 in C. The format is the one thing TeX cannot do
//! without: a `.tfm` says how wide, how tall and how deep each character is, in
//! units of the font's design size, plus the ligature and kern program that
//! says what happens between two of them. Nothing that sets type can be right
//! without it.
//!
//! It is a file of 32-bit big-endian words, and it is entirely a table of
//! indices: a character does not carry its width, it carries an index into a
//! width table, because most characters in a text font share a handful of
//! widths. Reading one is bounds checking, all the way down — a `.tfm` with an
//! index past the end of its own table is the classic corrupt font, and tex
//! refuses it rather than reading a neighbouring number.

use std::path::Path;

/// A `fix_word`: a signed 32-bit fixed-point number with 20 bits of fraction,
/// which is how every length in a `.tfm` is written. `1.0` is the design size.
fn fix_word(raw: i32) -> f64 {
    raw as f64 / (1 << 20) as f64
}

/// What a character's `tag` says about its `remainder` field.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Tag {
    /// Nothing follows.
    #[default]
    None,
    /// The remainder is where this character's ligature/kern program starts.
    LigKern,
    /// The remainder is the next larger character in a chain of sizes.
    List,
    /// The remainder indexes the extensible recipe table.
    Extensible,
}

/// One character's metrics, in design-size units.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CharMetrics {
    pub width: f64,
    pub height: f64,
    pub depth: f64,
    pub italic: f64,
    pub tag: u8,
    pub remainder: u8,
}

/// A step of a ligature/kern program.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    /// `a` followed by `next` becomes character `with`.
    Ligature { next: u8, with: u8, op: u8 },
    /// `a` followed by `next` is set `by` further apart, in design-size units.
    Kern { next: u8, by: f64 },
}

/// What a run of characters actually puts on the page once the font's own
/// ligature and kern program has had its say.
///
/// This is the distinction that makes a `.tfm` more than a table of widths:
/// the characters a document holds are NOT the characters TeX draws. `f` then
/// `i` in cmr10 is one character, code 0o14, and `A` then `V` is two
/// characters with a negative movement between them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Set {
    /// A character actually drawn.
    Char(u8),
    /// An explicit kern between the characters either side, in design-size
    /// units. `tex.web` §625 draws it as a movement, not as a glyph.
    Kern(f64),
}

/// `tex.web` §545's `non_char`: 256, a value no character code can take, used
/// for "no character here" and for the implicit boundary either side of a run.
const NON_CHAR: u16 = 256;

/// §545's `stop_flag` and `kern_flag`, which share the value 128: a `skip_byte`
/// of 128 or more ends a program, and an `op_byte` of 128 or more is a kern.
const STOP_FLAG: u8 = 128;
const KERN_FLAG: u8 = 128;

/// §906's "pseudo-ligature": a character a ligature op has invented, waiting to
/// the right of the cursor, and the original character it stands in front of.
#[derive(Debug, Clone, Copy)]
struct LigItem {
    character: u16,
    lig_ptr: Option<u8>,
}

/// §910's `wrap_lig`: turn what has been built since `cur_q` into one ligature
/// character.
///
/// `new_ligature(hf,cur_l,link(cur_q))` hangs the characters that were consumed
/// off the new node as its `lig_ptr` list, and a driver draws only the node's
/// own character. So the consumed characters leave the page, which is why this
/// truncates rather than appends: `fi` is ONE character, not three.
///
/// `lft_hit` and `rt_hit` are not tracked. They only set the ligature node's
/// subtype, which says whether a boundary character was eaten — a fact
/// `\showlists` prints and the page does not carry.
fn wrap_lig(out: &mut Vec<Set>, cur_q: usize, cur_l: u16, ligature_present: &mut bool) {
    if !*ligature_present {
        return;
    }
    out.truncate(cur_q);
    if cur_l < NON_CHAR {
        out.push(Set::Char(cur_l as u8));
    }
    *ligature_present = false;
}

/// §910's `pop_lig_stack`: take the pseudo-ligature at the cursor, put back the
/// character it stood in front of, and re-read `cur_r`.
fn pop_lig_stack(
    lig_stack: &mut Vec<LigItem>,
    out: &mut Vec<Set>,
    hu: &[u16],
    last: usize,
    bchar: u16,
    j: &mut usize,
    cur_r: &mut u16,
) {
    let Some(top) = lig_stack.pop() else {
        return;
    };
    if let Some(c) = top.lig_ptr {
        // "this is a charnode for hu[j+1]"
        out.push(Set::Char(c));
        *j += 1;
    }
    // §908's `set_cur_r`, with `cur_rh` always `non_char` here.
    *cur_r = match lig_stack.last() {
        Some(next) => next.character,
        None if *j < last => hu[*j + 1],
        None => bchar,
    };
}

/// `tex.web` §571's `store_scaled`: a `fix_word` at a font's size, in scaled
/// points.
///
/// Ported rather than replaced by `v * at`, because the two do not agree. The
/// `.tfm` number is a 32-bit integer with 20 bits of fraction and Knuth's rule
/// is a chain of TRUNCATING integer divisions:
///
/// ```text
/// sw:=(((((d*z)div@'400)+(c*z))div@'400)+(b*z))div beta;
/// if a=0 then #:=sw else if a=255 then #:=sw-alpha
/// ```
///
/// Measured on cmr10's own quad: the file holds 1048579/2^20 and `tex` answers
/// `10.00002pt` for `1em`. §571 computes 655361 scaled points; the product
/// rounded in floating point is 655361.875, which rounds to 655362 and prints
/// `10.00003pt`. One scaled point, on the number every `em` in a document is a
/// multiple of.
///
/// `v` comes back through `fix_word`, which is exact — a 32-bit integer over
/// 2^20 has 32 significant bits and an `f64` has 53 — so the four bytes Knuth
/// reads off the file are recovered here rather than re-read.
pub fn store_scaled(v: f64, at: i64) -> i64 {
    if at <= 0 {
        return 0;
    }
    // §572: halve `z` until it is small enough that `alpha*z` stays in a word,
    // doubling `alpha` to match. `beta` is what is divided out at the end.
    let (mut z, mut alpha) = (at, 16i64);
    while z >= 0o4000_0000 {
        z /= 2;
        alpha += alpha;
    }
    let beta = 256 / alpha;
    let alpha = alpha * z;
    // The four bytes of the `fix_word`, as the file holds them: a negative one
    // is written in two's complement, so `a` is 255 and not a sign bit.
    let raw = (v * f64::from(1 << 20)).round() as i64 as u32;
    let [a, b, c, d] = raw.to_be_bytes().map(i64::from);
    let sw = ((((d * z) / 256) + (c * z)) / 256) + (b * z);
    let sw = sw / beta;
    match a {
        0 => sw,
        // §571's other legal byte. Anything else is a corrupt `fix_word`; tex
        // calls `abort` on the whole file, and a font already parsed cannot be
        // abandoned here, so the parameter reads as zero.
        255 => sw - alpha,
        _ => 0,
    }
}

/// `tex.web` §455's two font-dependent units, in scaled points.
///
/// `em` is the quad (`\fontdimen6`) and `ex` is the x-height (`\fontdimen5`) of
/// the font IN FORCE where the dimension is scanned — which is the whole
/// difference between these and every other unit: `pt` and `in` are exact
/// ratios to each other and these are numbers a font file states.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FontUnits {
    /// `\fontdimen6`, which `em` is.
    pub quad: i64,
    /// `\fontdimen5`, which `ex` is.
    pub x_height: i64,
}

impl FontUnits {
    /// `tex.web` §552's `null_font`: every parameter zero. What TeX measures
    /// with when no font could be loaded, so `1em` is `0.0pt` there.
    pub const NULL: FontUnits = FontUnits {
        quad: 0,
        x_height: 0,
    };

    /// The two parameters of the `.tfm` named `font`, loaded at `at` scaled
    /// points, or `None` when the installation has not got the file.
    pub fn of_tfm(font: &str, at: i64) -> Option<FontUnits> {
        let tfm = Tfm::open(crate::typeset::find_font(font)?).ok()?;
        Some(FontUnits {
            quad: tfm.param_at(6, at),
            x_height: tfm.param_at(5, at),
        })
    }
}

/// The `fontdimen` parameters, by the names `tex.web` §547 gives them.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Params {
    pub slant: f64,
    pub space: f64,
    pub stretch: f64,
    pub shrink: f64,
    pub x_height: f64,
    pub quad: f64,
    pub extra_space: f64,
}

/// A font's metrics.
#[derive(Debug, Clone)]
pub struct Tfm {
    /// The checksum a DVI file repeats, so a driver can tell that the font it
    /// found is the font the document was set with.
    pub checksum: u32,
    /// The design size, in points.
    pub design_size: f64,
    /// `TEX TEXT`, `TEX MATH ITALIC`, and so on.
    pub coding_scheme: String,
    pub family: String,
    /// The first and last character codes the font defines.
    pub first: u8,
    pub last: u8,
    pub params: Params,
    /// Every parameter, including the ones past the seven named ones that a
    /// math font carries.
    pub raw_params: Vec<f64>,
    chars: Vec<Option<CharMetrics>>,
    lig_kern: Vec<[u8; 4]>,
    kerns: Vec<f64>,
    /// §545: the font's right boundary character, when its lig/kern array's
    /// FIRST instruction has `skip_byte=255`. `non_char` when it has none.
    bchar: u16,
    /// §545: where the LEFT boundary character's own lig/kern program starts,
    /// when the array's LAST instruction has `skip_byte=255`.
    bchar_label: Option<usize>,
}

impl Tfm {
    pub fn open(path: impl AsRef<Path>) -> Result<Tfm, String> {
        let path = path.as_ref();
        let bytes =
            std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Tfm::parse(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Read `bytes`. Every length in the file is checked against every other
    /// one before a single index is followed, because that is the difference
    /// between refusing a corrupt font and reading whatever is next to it.
    pub fn parse(bytes: &[u8]) -> Result<Tfm, String> {
        let word = |at: usize| -> Result<u32, String> {
            bytes
                .get(at * 4..at * 4 + 4)
                .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
                .ok_or_else(|| format!("word {at} is past the end of the file"))
        };
        let half = |at: usize| -> Result<usize, String> {
            bytes
                .get(at * 2..at * 2 + 2)
                .map(|b| u16::from_be_bytes([b[0], b[1]]) as usize)
                .ok_or_else(|| "the file is shorter than its own header".to_string())
        };

        // §540: twelve halfwords of lengths, and they have to agree.
        let lf = half(0)?;
        let lh = half(1)?;
        let bc = half(2)?;
        let ec = half(3)?;
        let nw = half(4)?;
        let nh = half(5)?;
        let nd = half(6)?;
        let ni = half(7)?;
        let nl = half(8)?;
        let nk = half(9)?;
        let ne = half(10)?;
        let np = half(11)?;

        if bc > ec + 1 || ec > 255 {
            return Err(format!("characters {bc}..{ec} are not a range"));
        }
        let expected = 6 + lh + (ec + 1 - bc) + nw + nh + nd + ni + nl + nk + ne + np;
        if lf != expected {
            return Err(format!(
                "the header says {lf} words, the tables need {expected}"
            ));
        }
        if bytes.len() < lf * 4 {
            return Err(format!(
                "the header says {lf} words, the file holds {}",
                bytes.len() / 4
            ));
        }
        // §543: the first entry of each of these tables is zero, so an index of
        // zero means "no width", and a font with an empty width table is one
        // that cannot say how wide anything is.
        if nw == 0 || nh == 0 || nd == 0 || ni == 0 {
            return Err("a table that must hold at least its zero entry is empty".into());
        }

        let header = 6;
        let checksum = word(header)?;
        let design_size = fix_word(word(header + 1)? as i32);
        if design_size <= 0.0 {
            return Err(format!("the design size is {design_size}"));
        }

        // A BCPL string: a length byte, then that many characters, in a fixed
        // number of words.
        let text = |at: usize, words: usize| -> String {
            let start = at * 4;
            let Some(region) = bytes.get(start..start + words * 4) else {
                return String::new();
            };
            let len = (region[0] as usize).min(region.len() - 1);
            region[1..1 + len].iter().map(|&b| b as char).collect()
        };
        let coding_scheme = if lh > 2 {
            text(header + 2, 10)
        } else {
            String::new()
        };
        let family = if lh > 12 {
            text(header + 12, 5)
        } else {
            String::new()
        };

        let char_info = header + lh;
        let width = char_info + (ec + 1 - bc);
        let height = width + nw;
        let depth = height + nh;
        let italic = depth + nd;
        let lig_kern = italic + ni;
        let kern = lig_kern + nl;
        let exten = kern + nk;
        let param = exten + ne;

        let at = |base: usize, index: usize, len: usize, what: &str| -> Result<f64, String> {
            if index >= len {
                return Err(format!(
                    "a character's {what} index {index} is past the {len}-entry table"
                ));
            }
            Ok(fix_word(word(base + index)? as i32))
        };

        let mut chars: Vec<Option<CharMetrics>> = vec![None; 256];
        for (code, slot) in chars.iter_mut().enumerate().take(ec + 1).skip(bc) {
            let w = word(char_info + code - bc)?.to_be_bytes();
            // §554: a width index of zero means the character does not exist,
            // which is how a font leaves gaps in its range.
            if w[0] == 0 {
                continue;
            }
            *slot = Some(CharMetrics {
                width: at(width, w[0] as usize, nw, "width")?,
                height: at(height, (w[1] >> 4) as usize, nh, "height")?,
                depth: at(depth, (w[1] & 0xf) as usize, nd, "depth")?,
                italic: at(italic, (w[2] >> 2) as usize, ni, "italic")?,
                tag: w[2] & 0x3,
                remainder: w[3],
            });
        }

        let mut program = Vec::with_capacity(nl);
        for i in 0..nl {
            program.push(word(lig_kern + i)?.to_be_bytes());
        }
        let mut kerns = Vec::with_capacity(nk);
        for i in 0..nk {
            kerns.push(fix_word(word(kern + i)? as i32));
        }
        let mut raw_params = Vec::with_capacity(np);
        for i in 0..np {
            raw_params.push(fix_word(word(param + i)? as i32));
        }
        let p = |i: usize| raw_params.get(i).copied().unwrap_or(0.0);
        let params = Params {
            slant: p(0),
            space: p(1),
            stretch: p(2),
            shrink: p(3),
            x_height: p(4),
            quad: p(5),
            extra_space: p(6),
        };

        // §545: "If the very first instruction of the lig_kern array has
        // skip_byte=255, the next_char byte is the so-called boundary
        // character of this font ... If the very last instruction of the
        // lig_kern array has skip_byte=255, there is a special
        // ligature/kerning program for a boundary character at the left,
        // beginning at location 256*op_byte+remainder."
        let bchar = match program.first() {
            Some(f) if f[0] == 255 => f[1] as u16,
            _ => NON_CHAR,
        };
        let bchar_label = match program.last() {
            Some(l) if l[0] == 255 => Some(256 * l[2] as usize + l[3] as usize),
            _ => None,
        };

        Ok(Tfm {
            checksum,
            design_size,
            coding_scheme,
            family,
            first: bc as u8,
            last: ec as u8,
            params,
            raw_params,
            chars,
            lig_kern: program,
            kerns,
            bchar,
            bchar_label,
        })
    }

    /// The metrics of `code`, or `None` when the font does not define it.
    pub fn char(&self, code: u8) -> Option<CharMetrics> {
        self.chars[code as usize]
    }

    /// What `code`'s `tag` field means.
    pub fn tag(&self, code: u8) -> Tag {
        match self.char(code).map(|c| c.tag) {
            Some(1) => Tag::LigKern,
            Some(2) => Tag::List,
            Some(3) => Tag::Extensible,
            _ => Tag::None,
        }
    }

    /// The codes the font defines, in order.
    pub fn codes(&self) -> Vec<u8> {
        (0..=255u8)
            .filter(|&c| self.chars[c as usize].is_some())
            .collect()
    }

    /// What happens between `left` and `right`: a ligature, a kern, or nothing.
    ///
    /// §545: a program step whose `skip_byte` is above 128 is a jump to where
    /// the program really starts, which is how a font addresses a program past
    /// the 256th word; a step's `skip_byte` otherwise says how many steps to
    /// skip when it does not match, and 128 or more ends the program.
    pub fn step(&self, left: u8, right: u8) -> Option<Step> {
        if self.tag(left) != Tag::LigKern {
            return None;
        }
        let mut at = self.char(left)?.remainder as usize;
        let first = *self.lig_kern.get(at)?;
        if first[0] > 128 {
            at = 256 * first[2] as usize + first[3] as usize;
        }
        // A malformed program could point at itself; the table is finite, so
        // bound the walk by its length rather than trusting it to stop.
        for _ in 0..=self.lig_kern.len() {
            let step = *self.lig_kern.get(at)?;
            let (skip, next, op, remainder) = (step[0], step[1], step[2], step[3]);
            if skip <= 128 && next == right {
                return Some(match op >= 128 {
                    true => Step::Kern {
                        next: right,
                        by: *self
                            .kerns
                            .get(256 * (op as usize - 128) + remainder as usize)?,
                    },
                    false => Step::Ligature {
                        next: right,
                        with: remainder,
                        op,
                    },
                });
            }
            if skip >= 128 {
                return None;
            }
            at += skip as usize + 1;
        }
        None
    }

    /// Where `code`'s ligature/kern program starts, following §545's jump when
    /// the first instruction is one.
    ///
    /// "If the very first instruction of a character's lig_kern program has
    /// skip_byte>128, the program actually begins in location
    /// 256*op_byte+remainder. This feature allows access to large lig_kern
    /// arrays, because the first instruction must otherwise appear in a
    /// location <=255."
    fn program_start(&self, code: u16) -> Option<usize> {
        // §909: `if cur_l=non_char then k:=bchar_label[hf]`.
        if code == NON_CHAR {
            return self.bchar_label;
        }
        let code = code as u8;
        if self.tag(code) != Tag::LigKern {
            return None;
        }
        let mut k = self.char(code)?.remainder as usize;
        match self.lig_kern.get(k) {
            Some(q) if q[0] > STOP_FLAG => k = 256 * q[2] as usize + q[3] as usize,
            _ => {}
        }
        Some(k)
    }

    /// What `codes` actually sets in this font: `tex.web` §906-§911's
    /// `reconstitute`, run over the whole run.
    ///
    /// This is the routine TeX itself uses to rebuild a word once it knows the
    /// characters, and it is the same algorithm the main loop (§1034-§1040)
    /// runs while reading one. `reconstitute` translates a "cut prefix" and
    /// returns how far it got; the caller repeats from there until the run is
    /// consumed, which is what §913 does around it.
    ///
    /// The run is a single font's, with nothing between the characters: a
    /// space is glue and a font switch is a boundary, and TeX's ligature
    /// machinery does not reach across either. The implicit boundary
    /// characters of §545 are supplied here, so a font that has them (cmr10
    /// does not) behaves as it does under tex.
    pub fn set_run(&self, codes: &[u8]) -> Vec<Set> {
        if codes.is_empty() {
            return Vec::new();
        }
        // §545: "TeX puts implicit boundary characters before and after each
        // consecutive string of characters from the same font."
        let mut hu: Vec<u16> = Vec::with_capacity(codes.len() + 1);
        if self.bchar_label.is_some() {
            hu.push(NON_CHAR);
        }
        hu.extend(codes.iter().map(|&c| c as u16));

        let mut out = Vec::with_capacity(codes.len());
        let mut j = 0usize;
        while j < hu.len() {
            j = self.reconstitute(&hu, j, &mut out) + 1;
        }
        out
    }

    /// One call of §906's `reconstitute`: translate `hu[j..]` as far as it can
    /// go and return the index of the last character consumed.
    ///
    /// Hyphenation is not in play here — this is called on a word that is
    /// already broken — so §906's `hyf`, `hchar` and `cur_rh` are the constants
    /// they take when no hyphen is being tried: `hyf[j]` even everywhere and
    /// `hchar=non_char`, which makes `test_char` always `cur_r` and
    /// `hyphen_passed` always zero. §908's `init_list` case (`j=0`) is likewise
    /// empty, because nothing precedes the run.
    fn reconstitute(&self, hu: &[u16], j0: usize, out: &mut Vec<Set>) -> usize {
        let last = hu.len() - 1;
        let mut j = j0;
        let mut bchar = self.bchar;
        // §906: `w` is the amount of kerning found, appended below.
        let mut w = 0.0f64;
        let mut ligature_present = false;
        let mut lig_stack: Vec<LigItem> = Vec::new();

        // §908: set up data structures with the cursor following position j.
        let mut cur_l = hu[j];
        let mut cur_q = out.len();
        if cur_l < NON_CHAR {
            out.push(Set::Char(cur_l as u8));
        }
        let mut cur_r = if j < last { hu[j + 1] } else { bchar };

        // §911 calls `check_interrupt` here, "to allow a way out in case
        // there's an infinite ligature loop" — a font whose program turns one
        // character into another and back. A library cannot be interrupted, so
        // the way out is a budget: every turn of this loop either advances the
        // cursor or applies one more lig/kern step, so a run that has taken
        // more turns than either could account for is a font cycling.
        let mut budget = 4 * hu.len() + self.lig_kern.len() + 8;

        'restart: loop {
            budget -= 1;
            if budget == 0 {
                return j;
            }
            // §909: if there's a ligature or kern at the cursor position,
            // update the data structures, possibly advancing j.
            'program: {
                let Some(mut k) = self.program_start(cur_l) else {
                    break 'program;
                };
                // The array is finite, so bound the walk by its length rather
                // than trusting a malformed program to stop.
                for _ in 0..=self.lig_kern.len() {
                    let Some(&q) = self.lig_kern.get(k) else {
                        break 'program;
                    };
                    let (skip, next, op, rem) = (q[0], q[1], q[2], q[3]);
                    if next as u16 == cur_r && skip <= STOP_FLAG {
                        if op >= KERN_FLAG {
                            // §909: "this kern will be inserted below".
                            w = self
                                .kerns
                                .get(256 * (op as usize - KERN_FLAG as usize) + rem as usize)
                                .copied()
                                .unwrap_or(0.0);
                            break 'program;
                        }
                        // §911: carry out a ligature replacement.
                        match op {
                            // =:| and =:|> — the left character is replaced
                            // and the right one is kept.
                            1 | 5 => {
                                cur_l = rem as u16;
                                ligature_present = true;
                            }
                            // |=: and |=:> — the left character is kept and
                            // the right one is replaced.
                            2 | 6 => {
                                cur_r = rem as u16;
                                match lig_stack.last_mut() {
                                    Some(top) => top.character = cur_r,
                                    None => {
                                        let lig_ptr = match j == last {
                                            true => {
                                                bchar = NON_CHAR;
                                                None
                                            }
                                            false => Some(hu[j + 1] as u8),
                                        };
                                        lig_stack.push(LigItem {
                                            character: cur_r,
                                            lig_ptr,
                                        });
                                    }
                                }
                            }
                            // |=:| — both characters are kept and a new one
                            // is inserted between them.
                            3 => {
                                cur_r = rem as u16;
                                lig_stack.push(LigItem {
                                    character: cur_r,
                                    lig_ptr: None,
                                });
                            }
                            // |=:|> and |=:|>> — the same, but the cursor
                            // passes the inserted character, so what is built
                            // so far is wrapped first.
                            7 | 11 => {
                                wrap_lig(out, cur_q, cur_l, &mut ligature_present);
                                cur_q = out.len();
                                cur_l = rem as u16;
                                ligature_present = true;
                            }
                            // =: — both characters become one.
                            _ => {
                                cur_l = rem as u16;
                                ligature_present = true;
                                if !lig_stack.is_empty() {
                                    pop_lig_stack(
                                        &mut lig_stack,
                                        out,
                                        hu,
                                        last,
                                        bchar,
                                        &mut j,
                                        &mut cur_r,
                                    );
                                } else if j == last {
                                    break 'program;
                                } else {
                                    out.push(Set::Char(cur_r as u8));
                                    j += 1;
                                    cur_r = if j < last { hu[j + 1] } else { bchar };
                                }
                            }
                        }
                        // §911: "if op_byte(q)>4 then if op_byte(q)<>7 then
                        // goto done" — the ops that pass over a character are
                        // finished with this cursor position.
                        if op > 4 && op != 7 {
                            break 'program;
                        }
                        continue 'restart;
                    }
                    if skip >= STOP_FLAG {
                        break 'program;
                    }
                    k += skip as usize + 1;
                }
                break 'program;
            }

            // §910: append a ligature and/or kern to the translation.
            wrap_lig(out, cur_q, cur_l, &mut ligature_present);
            if w != 0.0 {
                out.push(Set::Kern(w));
                w = 0.0;
            }
            let Some(top) = lig_stack.last().copied() else {
                return j;
            };
            cur_q = out.len();
            cur_l = top.character;
            ligature_present = true;
            pop_lig_stack(&mut lig_stack, out, hu, last, bchar, &mut j, &mut cur_r);
        }
    }

    /// The width of `text` set in this font, in design-size units, with the
    /// font's own ligatures and kerns applied — which is the only way to
    /// measure it that agrees with what TeX will print. A character the font
    /// does not define is skipped, as tex skips one after complaining.
    pub fn width_of(&self, text: &str) -> f64 {
        let codes: Vec<u8> = text
            .chars()
            .filter(|c| c.is_ascii())
            .map(|c| c as u8)
            .collect();
        self.set_run(&codes)
            .into_iter()
            .map(|set| match set {
                Set::Char(c) => self.char(c).map(|m| m.width).unwrap_or(0.0),
                Set::Kern(by) => by,
            })
            .sum()
    }

    /// `\fontdimen n` of this font loaded at `at` scaled points, in scaled
    /// points, with `n` numbered from one as TeX numbers it.
    ///
    /// A parameter short of the seventh that the file does not carry is zero,
    /// which is §575's own `for k:=np+1 to 7 do font_info[...]:=0` and is why
    /// `1ex` in a font that states no x-height is `0.0pt` and not an error.
    /// Past the seventh the two differ: §578 makes `\fontdimen22` of a text
    /// font `! Font x has only 7 fontdimen parameters`, and this answers zero,
    /// because nothing here reads a parameter a document named — only the two
    /// §455 reads, which every font has.
    pub fn param_at(&self, n: usize, at: i64) -> i64 {
        match n.checked_sub(1).and_then(|i| self.raw_params.get(i)) {
            Some(v) => store_scaled(*v, at),
            None => 0,
        }
    }

    /// A summary a person reads, in the units `tftopl` prints.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("family        {}\n", self.family));
        out.push_str(&format!("codingscheme  {}\n", self.coding_scheme));
        out.push_str(&format!("designsize    {:.6}pt\n", self.design_size));
        out.push_str(&format!("checksum      0o{:o}\n", self.checksum));
        out.push_str(&format!(
            "characters    {} (codes {}..{})\n",
            self.codes().len(),
            self.first,
            self.last
        ));
        let p = &self.params;
        out.push_str(&format!(
            "slant {:.6}  space {:.6}  stretch {:.6}  shrink {:.6}\n",
            p.slant, p.space, p.stretch, p.shrink
        ));
        out.push_str(&format!(
            "xheight {:.6}  quad {:.6}  extraspace {:.6}\n",
            p.x_height, p.quad, p.extra_space
        ));
        if self.raw_params.len() > 7 {
            out.push_str(&format!("fontdimens    {}\n", self.raw_params.len()));
        }
        out
    }

    /// One character's line, for `-X tfm FILE.tfm CHAR`.
    pub fn describe(&self, code: u8) -> String {
        let Some(m) = self.char(code) else {
            return format!("{code}: the font does not define it\n");
        };
        let shown = match (code as char).is_ascii_graphic() {
            true => format!("'{}'", code as char),
            false => format!("0o{code:o}"),
        };
        let mut out = format!(
            "{shown}  width {:.6}  height {:.6}  depth {:.6}  italic {:.6}\n",
            m.width, m.height, m.depth, m.italic
        );
        for right in self.codes() {
            match self.step(code, right) {
                Some(Step::Ligature { with, op, .. }) => out.push_str(&format!(
                    "  lig  {} -> 0o{with:o}{}\n",
                    right as char,
                    match op {
                        0 => String::new(),
                        op => format!(" (op {op})"),
                    }
                )),
                Some(Step::Kern { by, .. }) => {
                    out.push_str(&format!("  kern {} {by:+.6}\n", right as char))
                }
                None => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(name: &str) -> Option<Vec<u8>> {
        let found = std::process::Command::new("kpsewhich")
            .arg(name)
            .output()
            .ok()?;
        let path = String::from_utf8_lossy(&found.stdout).trim().to_string();
        std::fs::read(path).ok()
    }

    /// The oracle: `tftopl` is the program Knuth shipped for reading a `.tfm`,
    /// and it prints every number this reads. Agreeing with it on cmr10 — the
    /// font every plain document sets its text in — is the test that says the
    /// reader is right rather than merely self-consistent.
    #[test]
    fn cmr10_reads_the_same_numbers_tftopl_prints() {
        let Some(bytes) = installed("cmr10.tfm") else {
            return;
        };
        let tfm = Tfm::parse(&bytes).expect("cmr10 reads");

        assert_eq!(tfm.family, "CMR");
        // tftopl upcases the scheme when it prints it; the file stores the
        // mixed case, and this reads what the file holds.
        assert_eq!(tfm.coding_scheme, "TeX text");
        assert!((tfm.design_size - 10.0).abs() < 1e-9, "{}", tfm.design_size);
        // (CHECKSUM O 11374260171)
        assert_eq!(format!("{:o}", tfm.checksum), "11374260171");

        // The FONTDIMEN block, to the six places tftopl prints.
        let p = tfm.params;
        for (got, want, name) in [
            (p.slant, 0.0, "slant"),
            (p.space, 0.333334, "space"),
            (p.stretch, 0.166667, "stretch"),
            (p.shrink, 0.111112, "shrink"),
            (p.x_height, 0.430555, "xheight"),
            (p.quad, 1.000003, "quad"),
            (p.extra_space, 0.111112, "extraspace"),
        ] {
            assert!((got - want).abs() < 5e-7, "{name}: {got} not {want}");
        }

        // (CHARACTER C A (CHARWD R 0.750002) (CHARHT R 0.683332))
        let a = tfm.char(b'A').expect("cmr10 has an A");
        assert!((a.width - 0.750002).abs() < 5e-7, "{}", a.width);
        assert!((a.height - 0.683332).abs() < 5e-7, "{}", a.height);
        assert!(a.depth.abs() < 5e-7, "an A has no depth: {}", a.depth);

        // The ligature program, which is what makes this more than a table of
        // widths: (LABEL C f) (LIG C i O 14) — f then i becomes the fi
        // ligature at code 0o14.
        assert_eq!(
            tfm.step(b'f', b'i'),
            Some(Step::Ligature {
                next: b'i',
                with: 0o14,
                op: 0
            }),
            "f i is a ligature"
        );
        assert_eq!(
            tfm.step(b'f', b'f'),
            Some(Step::Ligature {
                next: b'f',
                with: 0o13,
                op: 0
            })
        );
        // (LABEL O 40) (KRN C l R -0.277779) — a space kerns before an l.
        match tfm.step(0o40, b'l') {
            Some(Step::Kern { by, .. }) => assert!((by + 0.277779).abs() < 5e-7, "{by}"),
            other => panic!("a space before an l kerns, not {other:?}"),
        }
        // (LABEL C A) (KRN C V R -0.111112)
        match tfm.step(b'A', b'V') {
            Some(Step::Kern { by, .. }) => assert!((by + 0.111112).abs() < 5e-7, "{by}"),
            other => panic!("A V kerns, not {other:?}"),
        }
        // And a pair with nothing between them is nothing, not a zero kern.
        assert_eq!(tfm.step(b'A', b'A'), None);
    }

    /// Measuring a string is the reason to read the file, and the ligature and
    /// the kern have to be in the number or it is just a sum of widths.
    #[test]
    fn a_string_is_measured_with_its_ligatures_and_kerns() {
        let Some(bytes) = installed("cmr10.tfm") else {
            return;
        };
        let tfm = Tfm::parse(&bytes).expect("cmr10 reads");

        // "ff" sets as one ligature, so it is narrower than two f's.
        let two_f = 2.0 * tfm.char(b'f').unwrap().width;
        assert!(
            tfm.width_of("ff") < two_f,
            "the ff ligature is narrower than two f's: {} vs {two_f}",
            tfm.width_of("ff")
        );
        assert!(
            (tfm.width_of("ff") - tfm.char(0o13).unwrap().width).abs() < 1e-9,
            "and is exactly the ligature's own width"
        );

        // "AV" kerns tighter than the two widths alone.
        let plain = tfm.char(b'A').unwrap().width + tfm.char(b'V').unwrap().width;
        assert!(
            tfm.width_of("AV") < plain,
            "AV kerns: {} vs {plain}",
            tfm.width_of("AV")
        );

        // A character the font does not define costs nothing rather than
        // panicking, which is what tex does after complaining.
        assert_eq!(tfm.width_of(""), 0.0);
        // A character outside the font's 8-bit world contributes nothing
        // rather than panicking: cmr10 has 128 characters, and a document that
        // reaches past them is tex's problem, not the reader's.
        assert!(tfm.width_of("\u{e9}").abs() < 1e-9);
    }

    /// A corrupt font is refused rather than read past. Every one of these is a
    /// real way a `.tfm` goes wrong: a truncated download, a text file renamed,
    /// a table whose length does not match the header.
    #[test]
    fn a_file_that_is_not_a_font_is_refused() {
        assert!(Tfm::parse(b"").is_err());
        assert!(Tfm::parse(b"not a font").is_err());

        let Some(bytes) = installed("cmr10.tfm") else {
            return;
        };
        // Cut short: the header still says how many words there are.
        let short = Tfm::parse(&bytes[..bytes.len() / 2]).unwrap_err();
        assert!(short.contains("the file holds"), "{short}");

        // A header claiming a length its tables do not add up to.
        let mut lying = bytes.clone();
        lying[0] = 0xff;
        assert!(Tfm::parse(&lying).is_err());

        // A character range that is not one.
        let mut backwards = bytes.clone();
        backwards[4] = 0x00;
        backwards[5] = 0xff; // bc = 255
        backwards[6] = 0x00;
        backwards[7] = 0x01; // ec = 1
        let e = Tfm::parse(&backwards).unwrap_err();
        assert!(e.contains("not a range"), "{e}");
    }

    /// The fonts a plain document loads besides cmr10 — the math ones carry
    /// more parameters than the seven a text font has, and a reader that
    /// assumed seven would read them as missing.
    #[test]
    fn a_math_font_carries_its_extra_parameters() {
        let Some(bytes) = installed("cmsy10.tfm") else {
            return;
        };
        let tfm = Tfm::parse(&bytes).expect("cmsy10 reads");
        assert_eq!(tfm.coding_scheme, "TeX math symbols");
        // §700: a symbol font carries 22 fontdimens; the axis height is the
        // 22nd, and it is what a fraction bar is centred on.
        assert!(
            tfm.raw_params.len() >= 22,
            "a math symbol font has 22 parameters, not {}",
            tfm.raw_params.len()
        );
        assert!(tfm.params.slant > 0.0, "cmsy10 is slanted");
        assert!(
            tfm.summary().contains("fontdimens    22"),
            "{}",
            tfm.summary()
        );
    }

    /// `tex.web` §455's two units out of the font `tex` itself measures them
    /// in, against the numbers `tex` prints.
    ///
    /// Read off `tex` 3.141592653 on a file that sets no font at all, so the
    /// current font is plain's own cmr10:
    ///
    /// ```text
    /// \dimen0=1em \dimen1=1ex \message{[\the\dimen0][\the\dimen1]}
    /// → [10.00002pt][4.30554pt]
    /// ```
    ///
    /// which is 655361 and 282168 scaled points. Both are one unit away from
    /// what a floating-point product gives: cmr10's quad is 1048579/2^20, and
    /// `1048579/2^20 × 10 × 65536` is 655361.875, which rounds UP to 655362 and
    /// prints `10.00003pt`. §571 divides in integers and truncates, so the
    /// last division is what throws the .875 away.
    #[test]
    fn the_font_units_are_the_scaled_points_tex_measures_them_as() {
        let Some(bytes) = installed("cmr10.tfm") else {
            return;
        };
        let tfm = Tfm::parse(&bytes).expect("cmr10 reads");
        let at = 10 * 65536;
        assert_eq!(tfm.param_at(6, at), 655361, "1em");
        assert_eq!(tfm.param_at(5, at), 282168, "1ex");
        // A parameter past the end of the file is zero (§580), not a panic.
        assert_eq!(tfm.param_at(22, at), 0);
        assert_eq!(tfm.param_at(0, at), 0);
    }

    /// §571's arithmetic on its own, including the two branches the font files
    /// above never take.
    ///
    /// A `fix_word` of exactly 1.0 is the size it is loaded at, whatever that
    /// size is, which is the whole meaning of a design-size unit.
    ///
    /// The negative branch is §571's `a=255`: a `fix_word` below zero is held
    /// in two's complement, so its top byte is 255 rather than a sign bit and
    /// `alpha` comes back off the end. Every parameter of every `cm` font is
    /// positive, so it is exercised here on the first negative number cmr10
    /// really holds — the kern it states between a space and an `l`, which
    /// `tftopl` prints as `-0.277779` and the file holds as -291272/2^20.
    #[test]
    fn store_scaled_is_knuths_integer_arithmetic_and_not_a_product() {
        let ten = 10 * 65536;
        assert_eq!(store_scaled(1.0, ten), ten);
        assert_eq!(store_scaled(1.0, 12 * 65536), 12 * 65536);
        assert_eq!(store_scaled(0.5, ten), ten / 2);
        assert_eq!(store_scaled(0.0, ten), 0);
        // The quad of cmr10, as the file holds it, at two sizes.
        let quad = 1048579.0 / f64::from(1 << 20);
        assert_eq!(store_scaled(quad, ten), 655361);
        assert_eq!(store_scaled(quad, 12 * 65536), 786434);
        // A size of nothing cannot divide by zero.
        assert_eq!(store_scaled(quad, 0), 0);
        // §571's `a=255` branch, and its symmetry with the positive value: the
        // same bytes meet the same truncation either side of zero.
        let kern = -291272.0 / f64::from(1 << 20);
        assert_eq!(store_scaled(kern, ten), -182045);
        assert_eq!(store_scaled(-kern, ten), 182045);
    }
}
