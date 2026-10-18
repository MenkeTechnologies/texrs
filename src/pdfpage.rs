//! A page's content stream, written as LuaTeX's PDF backend writes it: a port
//! of `pdfpage.c`, `pdfglyph.c`, `pdfrule.c`, `pdfliteral.c`,
//! `pdfsaverestore.c` and `pdfsetmatrix.c` from `luatexdir/pdf`.
//!
//! The shipper walks a box and says, for each glyph and rule, WHERE it is: a
//! position in scaled points from the bottom left corner of the page
//! (`posstruct->pos`). Everything about how that becomes PDF operators is
//! here, and it is the part that decides the bytes:
//!
//! - positions are rounded to `\pdfvariable decimaldigits` places of a big
//!   point ONCE, as integers, and every later movement is a difference of two
//!   rounded integers, so no error accumulates along a line (`calc_pdfpos`);
//! - text is written in four nested modes -- page, `BT ... ET`, `[ ... ]TJ`,
//!   `( ... )` -- and a mode is only left when what comes next needs an
//!   outer one;
//! - inside a `TJ` the writer tracks where the reader THINKS the next glyph
//!   goes (`cw`, the advance of the glyphs written so far) and writes the
//!   difference from where the shipper puts it as a kern in thousandths of the
//!   font size. That is why `Hello world.` comes out as
//!   `[(Hello)-333(w)27(orld.)]TJ`: an interword glue is a kern, and so is a
//!   font kern, and no space character is ever drawn;
//! - a font size is the TFM size in big points, `10pt` being `9.96264`:
//!   `setup_fontparameters` divides the size in scaled points by
//!   `by_one_bp`, 65536 x 72.27 / 72 of them to the big point, which is the
//!   unit a PDF is measured in.
//!
//! Numbers are LuaTeX's `pdffloat`: an integer mantissa and a decimal
//! exponent, printed by `print_pdffloat`, never through a float format.

use std::collections::{BTreeMap, BTreeSet};

/// A scaled point (`tex.web` §101): 1/65536 of a TeX point.
pub type Scaled = i64;

/// `by_one_bp` (`pdftables.h`): the scaled points in one big point.
pub const BY_ONE_BP: f64 = 65536.0 * 72.27 / 72.0;

/// `e_tj` (`pdfglyph.c`): a movement inside `[]TJ` is in 1/10^3 of the font
/// size.
const E_TJ: u32 = 3;

/// `ten_pow`.
fn ten_pow(e: u32) -> i64 {
    10i64.pow(e)
}

/// `i64round`, which LuaTeX defines as `llround`: half away from zero.
fn i64round(x: f64) -> i64 {
    x.round() as i64
}

/// LuaTeX's `pdffloat`: the number `m / 10^e`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PdfFloat {
    pub m: i64,
    pub e: u32,
}

impl PdfFloat {
    pub fn new(m: i64, e: u32) -> PdfFloat {
        PdfFloat { m, e }
    }

    fn to_f64(self) -> f64 {
        self.m as f64 / ten_pow(self.e) as f64
    }
}

/// `print_pdffloat` (`pdfgen.c`): the integer part, and the fraction with
/// its trailing zeros dropped; `1` exactly when the value is one.
pub fn print_pdffloat(out: &mut Vec<u8>, f: PdfFloat) {
    let mut m = f.m;
    if m == 0 {
        out.push(b'0');
        return;
    }
    if f.e == 0 {
        out.extend(m.to_string().as_bytes());
        return;
    }
    let t = ten_pow(f.e);
    if t == m {
        out.push(b'1');
        return;
    }
    if m < 0 {
        out.push(b'-');
        m = -m;
    }
    out.extend((m / t).to_string().as_bytes());
    let l = m % t;
    if l != 0 {
        out.push(b'.');
        // `snprintf("%d", l + t)` and the digits after its leading `1`, with
        // the trailing zeros cut.
        let digits = (l + t).to_string();
        out.extend(digits[1..].trim_end_matches('0').as_bytes());
    }
}

/// `pdf_add_bp`'s number: a dimension in big points, `decimal_digits` places.
pub fn bp(s: Scaled, decimal_digits: u32) -> PdfFloat {
    PdfFloat::new(
        i64round(s as f64 * (ten_pow(decimal_digits) as f64 / BY_ONE_BP)),
        decimal_digits,
    )
}

/// What a page or form needs to know of a font to place its glyphs.
///
/// `id` is the frontend's font (`f`), `resource` the font object it shares
/// (`pdf_font_num`, written `/F<resource>`), and the widths are the RESOURCE
/// font's own -- LuaTeX advances `cw` by `pdf_char_width(p, p->f_pdf, c)`,
/// the width in the font the resource was made for, whatever size the glyph
/// is set at.
#[derive(Clone)]
pub struct GlyphFont<'a> {
    pub id: u32,
    pub resource: u32,
    /// `font_size(f)`, in scaled points.
    pub size: Scaled,
    /// `font_size(f_pdf)`.
    pub resource_size: Scaled,
    /// `char_width(f_pdf, c)` for each code, in scaled points.
    pub resource_widths: &'a dyn Fn(u8) -> Scaled,
}

/// `pdfstructure`'s `mode`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Page,
    Text,
    CharArray,
    Char,
}

/// The literal modes of `\pdfextension literal` (`pdfliteral.c`): where the
/// text is put relative to the text object being written.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LiteralMode {
    /// The default: the page's coordinate system moved to the current point.
    SetOrigin,
    /// `page`: outside any text object, at the page origin.
    DirectPage,
    /// `text`: inside a text object (written through `pdf_goto_fontmode`).
    DirectText,
    /// `direct`: wherever the writer is, leaving any string first.
    DirectAlways,
    /// `raw`: the same, without asking for a new text matrix after it.
    DirectRaw,
}

/// One entry of the `\pdfextension setmatrix` stack (`matrix_entry`).
#[derive(Clone, Copy, Debug)]
struct Matrix {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

/// LuaTeX's `pdfstructure` and the stream it writes: one page or form.
pub struct PageWriter {
    out: Vec<u8>,
    decimal_digits: u32,
    /// Where the reader is, rounded: `p->pdf`.
    pdf: (PdfFloat, PdfFloat),
    /// Where the current `BT` started: `p->pdf_bt_pos`.
    pdf_bt_pos: (PdfFloat, PdfFloat),
    /// Where the current `[]TJ` started: `p->pdf_tj_pos`.
    pdf_tj_pos: (PdfFloat, PdfFloat),
    /// The advance of the glyphs written into the current `TJ`.
    cw: PdfFloat,
    fs: PdfFloat,
    fs_cur: PdfFloat,
    tm: [PdfFloat; 6],
    tm0_cur: PdfFloat,
    cm: [PdfFloat; 6],
    tj_delta: PdfFloat,
    /// The resource font of the current `Tf`, and the one asked for.
    f_pdf: Option<u32>,
    f_pdf_cur: Option<u32>,
    /// `pdf->f_cur`: the frontend font last set up.
    f_cur: Option<u32>,
    mode: Mode,
    need_tf: bool,
    need_tm: bool,
    k1: f64,
    k2: f64,
    /// `pos_stack` for `save` and `restore`.
    saves: Vec<((Scaled, Scaled), usize)>,
    /// `matrix_stack` for `setmatrix`.
    matrices: Vec<Matrix>,
    /// Every code drawn in each resource font: `pdf_mark_char`.
    pub used: BTreeMap<u32, BTreeSet<u8>>,
    /// The resource fonts in the order the page first drew in them, which
    /// is the order `pdf_init_font` numbers fonts in.
    pub font_order: Vec<u32>,
    /// What LuaTeX would warn about, in order: a `restore` with no `save`, a
    /// misplaced one, a non-PDF `\special`.
    pub warnings: Vec<String>,
}

impl PageWriter {
    /// `init_pdf_pagecalculations` for a page written at `decimal_digits`.
    pub fn new(decimal_digits: u32) -> PageWriter {
        let dd = decimal_digits;
        let fs_e = if dd < 4 { 5 } else { 6 };
        PageWriter {
            out: Vec::new(),
            decimal_digits: dd,
            pdf: (PdfFloat::new(0, dd), PdfFloat::new(0, dd)),
            pdf_bt_pos: (PdfFloat::new(0, dd), PdfFloat::new(0, dd)),
            pdf_tj_pos: (PdfFloat::new(0, dd), PdfFloat::new(0, dd)),
            cw: PdfFloat::new(0, 1),
            fs: PdfFloat::new(0, fs_e),
            fs_cur: PdfFloat::new(0, fs_e),
            tm: [
                PdfFloat::new(ten_pow(6), 6),
                PdfFloat::new(0, 0),
                PdfFloat::new(0, 3),
                PdfFloat::new(ten_pow(6), 6),
                PdfFloat::new(0, dd),
                PdfFloat::new(0, dd),
            ],
            tm0_cur: PdfFloat::new(0, 6),
            cm: [
                PdfFloat::new(1, 0),
                PdfFloat::new(0, 0),
                PdfFloat::new(0, 0),
                PdfFloat::new(1, 0),
                PdfFloat::new(0, dd),
                PdfFloat::new(0, dd),
            ],
            tj_delta: PdfFloat::new(0, 0),
            f_pdf: None,
            f_pdf_cur: None,
            f_cur: None,
            mode: Mode::Page,
            need_tf: false,
            need_tm: false,
            k1: ten_pow(dd) as f64 / BY_ONE_BP,
            k2: 0.0,
            saves: Vec::new(),
            matrices: Vec::new(),
            used: BTreeMap::new(),
            font_order: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn puts(&mut self, s: &str) {
        self.out.extend(s.as_bytes());
    }

    fn float(&mut self, f: PdfFloat) {
        print_pdffloat(&mut self.out, f);
    }

    /// `calc_pdfpos`: where `pos` is in the current mode's terms, and whether
    /// that is somewhere the reader is not.
    fn calc_pdfpos(&mut self, pos: (Scaled, Scaled)) -> bool {
        let (h, v) = (pos.0 as f64, pos.1 as f64);
        match self.mode {
            Mode::Page => {
                let new = (i64round(h * self.k1), i64round(v * self.k1));
                self.cm[4].m = new.0 - self.pdf.0.m;
                self.cm[5].m = new.1 - self.pdf.1.m;
                new.0 != self.pdf.0.m || new.1 != self.pdf.1.m
            }
            Mode::Text => {
                let new = (i64round(h * self.k1), i64round(v * self.k1));
                self.tm[4].m = new.0 - self.pdf_bt_pos.0.m;
                self.tm[5].m = new.1 - self.pdf_bt_pos.1.m;
                new.0 != self.pdf.0.m || new.1 != self.pdf.1.m
            }
            Mode::Char | Mode::CharArray => {
                let new_h = i64round((h * self.k1 - self.pdf_tj_pos.0.m as f64) * self.k2);
                let new_v = i64round(v * self.k1);
                // `(new.h - p->cw.m) / ten_pow[...]` is integer division in C,
                // the quotient then rounded: a truncation toward zero.
                let scale = ten_pow(self.cw.e - self.tj_delta.e);
                self.tj_delta.m = -((new_h - self.cw.m) / scale);
                self.tm[5].m = new_v - self.pdf_bt_pos.1.m;
                self.tj_delta.m != 0 || new_v != self.pdf.1.m
            }
        }
    }

    /// `print_pdf_matrix`.
    fn matrix(&mut self, m: [PdfFloat; 6]) {
        for (i, f) in m.iter().enumerate() {
            if i > 0 {
                self.out.push(b' ');
            }
            self.float(*f);
        }
    }

    /// `pdf_set_pos`: move the page's origin to `pos` with a `cm`.
    pub fn set_pos(&mut self, pos: (Scaled, Scaled)) {
        if self.calc_pdfpos(pos) {
            let cm = self.cm;
            self.matrix(cm);
            self.puts(" cm\n");
            self.pdf.0.m += self.cm[4].m;
            self.pdf.1.m += self.cm[5].m;
        }
    }

    /// `pdf_set_pos_temp`: the same `cm`, inside a `q ... Q` the caller
    /// writes, so the writer's idea of the origin does not move.
    pub fn set_pos_temp(&mut self, pos: (Scaled, Scaled)) {
        if self.calc_pdfpos(pos) {
            let cm = self.cm;
            self.matrix(cm);
            self.puts(" cm\n");
        }
    }

    fn begin_text(&mut self) {
        self.pdf_bt_pos = self.pdf;
        self.puts("BT\n");
        self.mode = Mode::Text;
        self.need_tf = true;
    }

    fn end_text(&mut self) {
        self.puts("ET\n");
        self.pdf = self.pdf_bt_pos;
        self.mode = Mode::Page;
    }

    fn end_charmode(&mut self) {
        self.out.push(b')');
        self.mode = Mode::CharArray;
    }

    fn begin_chararray(&mut self) {
        self.pdf_tj_pos = self.pdf;
        self.cw.m = 0;
        self.out.push(b'[');
        self.mode = Mode::CharArray;
    }

    fn end_chararray(&mut self) {
        self.puts("]TJ\n");
        self.pdf = self.pdf_tj_pos;
        self.mode = Mode::Text;
    }

    /// `pdf_end_string_nl`: close any string and `TJ` array.
    pub fn end_string(&mut self) {
        if self.mode == Mode::Char {
            self.end_charmode();
        }
        if self.mode == Mode::CharArray {
            self.end_chararray();
        }
    }

    /// `pdf_goto_pagemode`.
    pub fn goto_pagemode(&mut self) {
        self.end_string();
        if self.mode == Mode::Text {
            self.end_text();
        }
    }

    /// `pdf_goto_textmode`.
    pub fn goto_textmode(&mut self) {
        if self.mode == Mode::Page {
            self.set_pos((0, 0));
            self.begin_text();
        } else {
            self.end_string();
        }
    }

    /// `pdf_goto_fontmode`: out of everything, back at the page origin.
    pub fn goto_fontmode(&mut self) {
        self.goto_pagemode();
        self.set_pos((0, 0));
        self.mode = Mode::Page;
    }

    /// `setup_fontparameters` for a simple, upright, unexpanded font.
    fn setup_fontparameters(&mut self, font: &GlyphFont) {
        self.f_cur = Some(font.id);
        self.f_pdf = Some(font.resource);
        self.fs.m = i64round(font.size as f64 / BY_ONE_BP * ten_pow(self.fs.e) as f64);
        // "The -1 makes less corrections inside []TJ", and there is no need
        // to be finer than a scaled point for a tiny font.
        self.tj_delta.e = self.cw.e - 1;
        while self.tj_delta.e > 0
            && (font.size as f64) / (ten_pow(self.tj_delta.e + E_TJ) as f64) < 0.5
        {
            self.tj_delta.e -= 1;
        }
        self.tm[0].m = ten_pow(self.tm[0].e);
        self.tm[2].m = 0;
        self.tm[3].m = ten_pow(self.tm[3].e);
        self.k2 = ten_pow(E_TJ + self.cw.e) as f64
            / (ten_pow(self.pdf.0.e) as f64 * self.fs.to_f64() * self.tm[0].to_f64());
    }

    /// `set_font`: the `Tf`, and a `Tm` to follow it.
    fn set_font(&mut self) {
        let f = self.f_pdf.unwrap_or(0);
        self.puts(&format!("/F{f} "));
        let fs = self.fs;
        self.float(fs);
        self.puts(" Tf\n");
        self.f_pdf_cur = self.f_pdf;
        self.fs_cur.m = self.fs.m;
        self.need_tf = false;
        self.need_tm = true;
    }

    /// `set_textmatrix`.
    fn set_textmatrix(&mut self, pos: (Scaled, Scaled)) {
        let moved = self.calc_pdfpos(pos);
        if self.need_tm || moved {
            let tm = self.tm;
            self.matrix(tm);
            self.puts(" Tm ");
            self.pdf.0.m = self.pdf_bt_pos.0.m + self.tm[4].m;
            self.pdf.1.m = self.pdf_bt_pos.1.m + self.tm[5].m;
            self.need_tm = false;
        }
        self.tm0_cur.m = self.tm[0].m;
    }

    /// `pdf_print_char`: a code below 33, above 126, or one of `\()`, in
    /// octal.
    fn print_char(&mut self, c: u8) {
        if c <= 32 || c == b'\\' || c == b'(' || c == b')' || c > 127 {
            self.out.push(b'\\');
            self.out.push(b'0' + ((c >> 6) & 3));
            self.out.push(b'0' + ((c >> 3) & 7));
            self.out.push(b'0' + (c & 7));
        } else {
            self.out.push(c);
        }
    }

    /// `pdf_char_width`: a code's advance in thousandths of the size, to
    /// `cw.e` more places.
    fn char_width(&self, font: &GlyphFont, c: u8) -> i64 {
        i64round(
            (font.resource_widths)(c) as f64 / font.resource_size as f64
                * ten_pow(E_TJ + self.cw.e) as f64,
        )
    }

    /// `pdf_place_glyph`: code `c` of `font` with its reference point at
    /// `pos`.
    pub fn place_glyph(&mut self, font: &GlyphFont, c: u8, pos: (Scaled, Scaled)) {
        let font_changed = self.f_cur != Some(font.id)
            || self.f_pdf != self.f_pdf_cur
            || self.fs.m != self.fs_cur.m;
        if self.need_tf || font_changed || self.mode == Mode::Page {
            self.goto_textmode();
            self.setup_fontparameters(font);
            self.set_font();
        } else if self.tm0_cur.m != self.tm[0].m {
            self.setup_fontparameters(font);
            self.need_tm = true;
        }
        let mut moved = self.calc_pdfpos(pos);
        if moved || self.need_tm {
            if self.need_tm
                || self.pdf_bt_pos.1.m + self.tm[5].m != self.pdf.1.m
                || self.tj_delta.m.abs() >= 1_000_000
            {
                self.goto_textmode();
                self.set_textmatrix(pos);
                self.begin_chararray();
                moved = self.calc_pdfpos(pos);
            }
            if moved {
                if self.mode == Mode::Char {
                    self.end_charmode();
                }
                let delta = self.tj_delta;
                self.float(delta);
                self.cw.m -= self.tj_delta.m * ten_pow(self.cw.e - self.tj_delta.e);
            }
        }
        if self.mode == Mode::CharArray {
            self.out.push(b'(');
            self.mode = Mode::Char;
        }
        if !self.used.contains_key(&font.resource) {
            self.font_order.push(font.resource);
        }
        self.used.entry(font.resource).or_default().insert(c);
        self.print_char(c);
        self.cw.m += self.char_width(font, c);
    }

    /// `pdf_place_rule` for a normal rule of `size` (width, height plus
    /// depth) whose bottom left corner is at `pos`: a stroked line when it is
    /// at most a big point thick, a filled rectangle otherwise.
    pub fn place_rule(&mut self, pos: (Scaled, Scaled), size: (Scaled, Scaled)) {
        let one_bp: Scaled = 65781; // `one_bp` in `pdfgen.c`.
        self.goto_pagemode();
        let dd = self.pdf.0.e;
        let dim_h = PdfFloat::new(i64round(size.0 as f64 * self.k1), dd);
        let dim_v = PdfFloat::new(i64round(size.1 as f64 * self.k1), dd);
        let mut pos = pos;
        self.puts("q\n");
        if size.1 <= one_bp {
            pos.1 += i64round(0.5 * size.1 as f64);
            self.set_pos_temp(pos);
            self.puts("[] 0 d 0 J ");
            self.float(dim_v);
            self.puts(" w 0 0 m ");
            self.float(dim_h);
            self.puts(" 0 l S\n");
        } else if size.0 <= one_bp {
            pos.0 += i64round(0.5 * size.0 as f64);
            self.set_pos_temp(pos);
            self.puts("[] 0 d 0 J ");
            self.float(dim_h);
            self.puts(" w 0 0 m 0 ");
            self.float(dim_v);
            self.puts(" l S\n");
        } else {
            self.set_pos_temp(pos);
            self.puts("0 0 ");
            self.float(dim_h);
            self.out.push(b' ');
            self.float(dim_v);
            self.puts(" re f\n");
        }
        self.puts("Q\n");
    }

    /// `pdf_literal_set_mode`'s move, then nothing written.
    fn literal_mode(&mut self, mode: LiteralMode, pos: (Scaled, Scaled)) {
        match mode {
            LiteralMode::SetOrigin => {
                self.goto_pagemode();
                self.set_pos(pos);
            }
            LiteralMode::DirectPage => self.goto_pagemode(),
            LiteralMode::DirectText => self.goto_fontmode(),
            LiteralMode::DirectAlways => {
                self.end_string();
                self.need_tm = true;
            }
            LiteralMode::DirectRaw => self.end_string(),
        }
    }

    /// `pdf_literal`: `text` in `mode` at `pos`, and a line end.
    pub fn literal(&mut self, text: &[u8], mode: LiteralMode, pos: (Scaled, Scaled)) {
        self.literal_mode(mode, pos);
        self.out.extend(text);
        self.out.push(b'\n');
    }

    /// `pdf_special`: a `\special` whose text begins `pdf:` (or `PDF:`) is a
    /// literal, its mode named by what follows -- `direct:`, `page:`,
    /// `text:`, `raw:`, `origin:` or nothing, which is the origin. Anything
    /// else is ignored with LuaTeX's warning, and a `src:` special silently.
    pub fn special(&mut self, text: &[u8], pos: (Scaled, Scaled)) {
        let rest = match text
            .strip_prefix(b"pdf:")
            .or_else(|| text.strip_prefix(b"PDF:"))
        {
            Some(rest) => rest,
            None => {
                let src = text.starts_with(b"src:") || text.starts_with(b"SRC:");
                if !src || text.is_empty() {
                    self.warnings.push("Non-PDF special ignored!".to_string());
                }
                return;
            }
        };
        let prefixes: [(&[u8], LiteralMode); 5] = [
            (b"direct:", LiteralMode::DirectAlways),
            (b"page:", LiteralMode::DirectPage),
            (b"text:", LiteralMode::DirectText),
            (b"raw:", LiteralMode::DirectRaw),
            (b"origin:", LiteralMode::SetOrigin),
        ];
        let (mode, body) = prefixes
            .iter()
            .find_map(|(p, m)| rest.strip_prefix(*p).map(|b| (*m, b)))
            .unwrap_or((LiteralMode::SetOrigin, rest));
        self.literal(body, mode, pos);
    }

    /// `pdf_out_save`: `q`, remembering where it was said.
    pub fn save(&mut self, pos: (Scaled, Scaled)) {
        self.saves.push((pos, self.matrices.len()));
        self.literal(b"q", LiteralMode::SetOrigin, pos);
    }

    /// `pdf_out_restore`: `Q`, warning when it does not match a `save` at the
    /// same place.
    pub fn restore(&mut self, pos: (Scaled, Scaled)) {
        match self.saves.pop() {
            None => self
                .warnings
                .push("'restore' is missing a 'save'".to_string()),
            Some((at, matrices)) => {
                let diff = (pos.0 - at.0, pos.1 - at.1);
                if diff != (0, 0) {
                    self.warnings.push(format!(
                        "misplaced 'restore' by ({}sp, {}sp)",
                        diff.0, diff.1
                    ));
                }
                self.matrices.truncate(matrices);
            }
        }
        self.literal(b"Q", LiteralMode::SetOrigin, pos);
    }

    /// `pdf_out_setmatrix`: `a b c d 0 0 cm` at `pos`, with the matrix
    /// pushed for the annotations that read it.
    pub fn setmatrix(&mut self, spec: &str, pos: (Scaled, Scaled)) {
        let nums: Vec<f64> = spec
            .split_whitespace()
            .filter_map(|w| w.parse().ok())
            .collect();
        match nums[..] {
            [a, b, c, d] => {
                let (h, v) = (pos.0 as f64, pos.1 as f64);
                let x = Matrix {
                    a,
                    b,
                    c,
                    d,
                    e: h * (1.0 - a) - v * c,
                    f: v * (1.0 - d) - h * b,
                };
                let z = match self.matrices.last() {
                    Some(y) => Matrix {
                        a: x.a * y.a + x.b * y.c,
                        b: x.a * y.b + x.b * y.d,
                        c: x.c * y.a + x.d * y.c,
                        d: x.c * y.b + x.d * y.d,
                        e: x.e * y.a + x.f * y.c + y.e,
                        f: x.e * y.b + x.f * y.d + y.f,
                    },
                    None => x,
                };
                self.matrices.push(z);
            }
            _ => self
                .warnings
                .push(format!("unrecognized format of setmatrix: {spec}")),
        }
        let text = format!("{spec} 0 0 cm");
        self.literal(text.as_bytes(), LiteralMode::SetOrigin, pos);
    }

    /// `matrixtransformrect`: a rectangle as the current `setmatrix` stack
    /// moves it, for an annotation's `/Rect`.
    pub fn transform_rect(&self, ll: (Scaled, Scaled), ur: (Scaled, Scaled)) -> [Scaled; 4] {
        let Some(m) = self.matrices.last() else {
            return [ll.0, ll.1, ur.0, ur.1];
        };
        let round = |x: f64| {
            if x > 0.0 {
                (x + 0.5) as Scaled
            } else {
                (x - 0.5) as Scaled
            }
        };
        let t = |x: Scaled, y: Scaled| {
            let (x, y) = (x as f64, y as f64);
            (
                round(x * m.a + y * m.c + m.e),
                round(x * m.b + y * m.d + m.f),
            )
        };
        let pts = [t(ll.0, ll.1), t(ll.0, ur.1), t(ur.0, ll.1), t(ur.0, ur.1)];
        [
            pts.iter().map(|p| p.0).min().unwrap_or(0),
            pts.iter().map(|p| p.1).min().unwrap_or(0),
            pts.iter().map(|p| p.0).max().unwrap_or(0),
            pts.iter().map(|p| p.1).max().unwrap_or(0),
        ]
    }

    /// How deep the `save` stack is: `pos_stack_used`, which a page must end
    /// at zero.
    pub fn open_saves(&self) -> usize {
        self.saves.len()
    }

    /// The finished stream: `pdf_end_page`'s `pdf_goto_pagemode`, and the
    /// bytes.
    pub fn finish(mut self) -> (Vec<u8>, BTreeMap<u32, BTreeSet<u8>>) {
        self.goto_pagemode();
        (self.out, self.used)
    }

    /// The decimal digits the page is written at.
    pub fn decimal_digits(&self) -> u32 {
        self.decimal_digits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn printed(f: PdfFloat) -> String {
        let mut out = Vec::new();
        print_pdffloat(&mut out, f);
        String::from_utf8(out).unwrap()
    }

    /// `print_pdffloat`'s spellings, each a case of its own branch.
    #[test]
    fn a_pdffloat_prints_as_luatex_prints_it() {
        assert_eq!(printed(PdfFloat::new(0, 3)), "0");
        assert_eq!(printed(PdfFloat::new(1000, 3)), "1");
        assert_eq!(printed(PdfFloat::new(710037, 3)), "710.037");
        assert_eq!(printed(PdfFloat::new(55290, 3)), "55.29");
        assert_eq!(printed(PdfFloat::new(996264, 5)), "9.96264");
        assert_eq!(printed(PdfFloat::new(-333, 0)), "-333");
        assert_eq!(printed(PdfFloat::new(-1500, 3)), "-1.5");
        assert_eq!(printed(PdfFloat::new(-1000, 3)), "-1");
        assert_eq!(printed(PdfFloat::new(42, 0)), "42");
        assert_eq!(printed(PdfFloat::new(5, 3)), "0.005");
    }

    /// cmr10's widths at 10pt, in scaled points, measured with
    /// `\fontcharwd` under luatex 1.24.0.
    fn cmr10(c: u8) -> Scaled {
        match c {
            b'H' => 491521,
            b'e' => 291271,
            b'l' => 182045,
            b'o' | b'1' => 327681,
            b'w' => 473316,
            b'r' => 256683,
            b'd' => 364090,
            b'.' => 182045,
            _ => 0,
        }
    }

    /// `Hello world.` and its folio set by plain luatex: the content stream
    /// luatex 1.24.0 writes for it, byte for byte, from the positions its
    /// `hlist_out` computes -- `\pdfvariable horigin` and the page height
    /// for the reference point, `\parindent` in front, cmr10's widths, its
    /// `w`-`o` kern of -18205sp, the 218453sp interword glue, and the folio
    /// centred in `\hsize` on the footline's baseline.
    #[test]
    fn hello_world_is_drawn_as_luatex_draws_it() {
        let widths = cmr10;
        let font = GlyphFont {
            id: 1,
            resource: 1,
            size: 655360,
            resource_size: 655360,
            resource_widths: &widths,
        };
        let mut page = PageWriter::new(3);
        let (origin_h, page_v) = (4736286, 52099153 - 4736286);
        let baseline = page_v - 655360;
        let mut h = origin_h + 1310720;
        for c in b"Hello world." {
            match c {
                b' ' => h += 218453,
                c => {
                    page.place_glyph(&font, *c, (h, baseline));
                    h += cmr10(*c);
                    if *c == b'w' {
                        h -= 18205;
                    }
                }
            }
        }
        let folio_v = page_v - 42152922 - 1572864;
        page.place_glyph(&font, b'1', (origin_h + 15229091, folio_v));
        let (stream, used) = page.finish();
        assert_eq!(
            String::from_utf8(stream).unwrap(),
            "BT\n/F1 9.96264 Tf\n1 0 0 1 91.925 710.037 Tm [(Hello)-333(w)27(orld.)]TJ\n\
             1 0 0 1 303.509 55.29 Tm [(1)]TJ\nET\n"
        );
        assert_eq!(
            used[&1].iter().map(|&c| c as char).collect::<String>(),
            ".1Hdelorw"
        );
    }

    /// Two rules, three specials and a save, a matrix and a restore in one
    /// `\hbox` shipped at the top left corner of a 200bp page: the stream
    /// luatex 1.24.0 writes for it, measured with compression off.
    ///
    /// A rule at most a big point thick is a stroked line through its middle,
    /// a thicker one a filled rectangle, both inside `q ... Q` with the
    /// origin moved there and not left there. A `pdf:direct:` special is
    /// written where the writer stands, a plain `pdf:` one after moving the
    /// origin to the current point, and `color push` -- no `pdf:` -- is
    /// ignored with luatex's warning. The save, the matrix and the restore
    /// stand at the point the origin is already at, so no `cm` precedes them.
    #[test]
    fn rules_specials_and_matrices_are_written_as_luatex_writes_them() {
        let mut page = PageWriter::new(3);
        let baseline = 13156352 - 655360;
        page.place_rule((0, baseline), (655360, 26214));
        page.place_rule((655360, baseline), (6553600, 655360));
        let here = (7208960, baseline);
        page.special(b"pdf:direct:1 0 0 rg", here);
        page.special(b"color push", here);
        page.special(b"pdf:0 g", here);
        page.save(here);
        page.setmatrix("2 0 0 2", here);
        page.restore(here);
        assert_eq!(page.warnings, vec!["Non-PDF special ignored!".to_string()]);
        let (stream, _) = page.finish();
        assert_eq!(
            String::from_utf8(stream).unwrap(),
            "q\n1 0 0 1 0 190.237 cm\n[] 0 d 0 J 0.398 w 0 0 m 9.963 0 l S\nQ\n\
             q\n1 0 0 1 9.963 190.037 cm\n0 0 99.626 9.963 re f\nQ\n\
             1 0 0 rg\n1 0 0 1 109.589 190.037 cm\n0 g\nq\n2 0 0 2 0 0 cm\nQ\n"
        );
    }

    /// `save` and `restore` are `q` and `Q` at the current point, and a
    /// restore somewhere else is warned about, as luatex does.
    #[test]
    fn save_and_restore_warn_when_they_do_not_match() {
        let mut page = PageWriter::new(3);
        page.save((0, 0));
        page.restore((5, 0));
        page.restore((5, 0));
        assert_eq!(
            page.warnings,
            vec![
                "misplaced 'restore' by (5sp, 0sp)".to_string(),
                "'restore' is missing a 'save'".to_string()
            ]
        );
    }
}
