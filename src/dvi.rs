//! Reading DVI, ported from tectonic's `xdv`.
//!
//! texrs has no stomach yet: it stops where the boxes would begin, so nothing
//! in it produces a page today. Both halves of the format are here anyway, and
//! for the same reason. Reading, because the parity contract stops in the same
//! place -- today the harness compares `\message` streams, which is everything
//! the mouth and the expander produce and nothing the rest of TeX does, and the
//! moment texrs sets a character the reference to compare against is what real
//! tex shipped, which is a DVI file. Writing, because that is what the stomach
//! will call, and a DVI file is more than its opcodes: it is a linked list read
//! backwards, and the pointers can only be filled in while writing. Both are
//! held against the tools that read the format -- `dvitype` accepts what
//! [`Writer`] writes, and a file of real tex's, read and written back, is the
//! same document.
//!
//! It is also the honest shape of the port. tectonic's `xdv` is a parser and an
//! event stream, not a typesetter; the typesetter is `engine_xetex`, which is a
//! transpile of Knuth's C and not something to port a line at a time. What can
//! be carried across is the byte format, and the byte format is `dvitype`'s
//! table in `tex.web` §583–590.
//!
//! What is NOT here: the XeTeX extensions (opcodes 252–254, native fonts and
//! glyph runs). They are recognised and named, because pointing this at an
//! `.xdv` should say what it is rather than fail as if the file were damaged,
//! but their operands are XeTeX's business and texrs will never emit them.

use std::path::Path;

/// One thing a DVI file says.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// The preamble: DVI version, and the comment tex stamps in.
    Preamble {
        version: u8,
        comment: String,
    },
    /// A page begins, carrying `\count0..9` — which is how a page knows its
    /// own number.
    BeginPage {
        counts: [i32; 10],
    },
    EndPage,
    /// A character set in the current font, at the current position.
    SetChar(u32),
    /// The same, without moving afterwards.
    PutChar(u32),
    /// A rule: height and width in scaled points.
    Rule {
        height: i32,
        width: i32,
        set: bool,
    },
    Push,
    Pop,
    /// Movement, in scaled points. Positive right is right, positive down is
    /// DOWN the page — the y axis points the other way from the one most
    /// graphics formats use.
    Right(i32),
    Down(i32),
    /// A font is selected by number.
    Font(u32),
    /// A font is defined: its number, its name, and the size it is used at.
    DefineFont {
        number: u32,
        name: String,
        at: i32,
    },
    /// `\special{…}`, which is how a document says something DVI has no word
    /// for — a colour, an included graphic.
    Special(String),
    /// The postamble, with what tex counted while writing.
    Postamble {
        pages: u16,
        max_stack: u16,
    },
    /// An XDV extension: XeTeX's, and named rather than decoded.
    Extension(u8),
    Noop,
}

/// What a DVI file holds, in order.
#[derive(Debug, Clone, Default)]
pub struct Dvi {
    pub ops: Vec<Op>,
}

impl Dvi {
    /// Parse `bytes` as DVI.
    pub fn parse(bytes: &[u8]) -> Result<Dvi, String> {
        let mut at = 0usize;
        let mut ops = Vec::new();
        // The four movement registers. A DVI file spends most of its bytes on
        // movement, so it keeps the last horizontal distance in `w` and `x` and
        // the last vertical one in `y` and `z`, and repeats a movement with a
        // single opcode. A reader that took `w0` for a movement of zero would
        // stack a line of text on one spot.
        let (mut w, mut x, mut y, mut z) = (0i32, 0i32, 0i32, 0i32);
        let mut saved: Vec<(i32, i32, i32, i32)> = Vec::new();
        while at < bytes.len() {
            let opcode = bytes[at];
            at += 1;
            let op = match opcode {
                // §585: an opcode below 128 IS the character it sets.
                0..=127 => Op::SetChar(opcode as u32),
                128..=131 => {
                    let n = (opcode - 127) as usize;
                    Op::SetChar(read_unsigned(bytes, &mut at, n)?)
                }
                132 => Op::Rule {
                    height: read_signed(bytes, &mut at, 4)?,
                    width: read_signed(bytes, &mut at, 4)?,
                    set: true,
                },
                133..=136 => {
                    let n = (opcode - 132) as usize;
                    Op::PutChar(read_unsigned(bytes, &mut at, n)?)
                }
                137 => Op::Rule {
                    height: read_signed(bytes, &mut at, 4)?,
                    width: read_signed(bytes, &mut at, 4)?,
                    set: false,
                },
                138 => Op::Noop,
                139 => {
                    let mut counts = [0i32; 10];
                    for count in counts.iter_mut() {
                        *count = read_signed(bytes, &mut at, 4)?;
                    }
                    // The pointer to the previous page, which a sequential read
                    // does not need.
                    let _previous = read_signed(bytes, &mut at, 4)?;
                    // A page begins with everything at zero.
                    w = 0;
                    x = 0;
                    y = 0;
                    z = 0;
                    saved.clear();
                    Op::BeginPage { counts }
                }
                140 => Op::EndPage,
                141 => {
                    saved.push((w, x, y, z));
                    Op::Push
                }
                142 => {
                    // §  : push and pop save and restore the movement
                    // registers along with the position.
                    if let Some((sw, sx, sy, sz)) = saved.pop() {
                        w = sw;
                        x = sx;
                        y = sy;
                        z = sz;
                    }
                    Op::Pop
                }
                // right1..4 move without remembering how far.
                143..=146 => Op::Right(read_signed(bytes, &mut at, (opcode - 142) as usize)?),
                // §  : w0 moves by w, the last distance a w-instruction set --
                // which is how a file sets a line of text in one font without
                // repeating the same operand between every pair of letters.
                // The same for x, and for y and z going down.
                147 => Op::Right(w),
                148..=151 => {
                    w = read_signed(bytes, &mut at, (opcode - 147) as usize)?;
                    Op::Right(w)
                }
                152 => Op::Right(x),
                153..=156 => {
                    x = read_signed(bytes, &mut at, (opcode - 152) as usize)?;
                    Op::Right(x)
                }
                157..=160 => Op::Down(read_signed(bytes, &mut at, (opcode - 156) as usize)?),
                161 => Op::Down(y),
                162..=165 => {
                    y = read_signed(bytes, &mut at, (opcode - 161) as usize)?;
                    Op::Down(y)
                }
                166 => Op::Down(z),
                167..=170 => {
                    z = read_signed(bytes, &mut at, (opcode - 166) as usize)?;
                    Op::Down(z)
                }
                // §586: the font number is in the opcode, as the character is.
                171..=234 => Op::Font((opcode - 171) as u32),
                235..=238 => {
                    let n = (opcode - 234) as usize;
                    Op::Font(read_unsigned(bytes, &mut at, n)?)
                }
                239..=242 => {
                    let n = (opcode - 238) as usize;
                    let length = read_unsigned(bytes, &mut at, n)? as usize;
                    Op::Special(read_string(bytes, &mut at, length)?)
                }
                243..=246 => {
                    let n = (opcode - 242) as usize;
                    let number = read_unsigned(bytes, &mut at, n)?;
                    let _checksum = read_unsigned(bytes, &mut at, 4)?;
                    let at_size = read_signed(bytes, &mut at, 4)?;
                    let _design = read_signed(bytes, &mut at, 4)?;
                    // The name is in two parts, an area and a file, whose
                    // lengths come first.
                    let area = read_unsigned(bytes, &mut at, 1)? as usize;
                    let file = read_unsigned(bytes, &mut at, 1)? as usize;
                    let name = read_string(bytes, &mut at, area + file)?;
                    Op::DefineFont {
                        number,
                        name,
                        at: at_size,
                    }
                }
                247 => {
                    let version = read_unsigned(bytes, &mut at, 1)? as u8;
                    // num, den, mag — the units the file is written in.
                    for _ in 0..3 {
                        read_unsigned(bytes, &mut at, 4)?;
                    }
                    let length = read_unsigned(bytes, &mut at, 1)? as usize;
                    Op::Preamble {
                        version,
                        comment: read_string(bytes, &mut at, length)?,
                    }
                }
                248 => {
                    // The final page pointer and the units again, then what tex
                    // counted: the page height and width, the stack depth, and
                    // the number of pages.
                    for _ in 0..6 {
                        read_unsigned(bytes, &mut at, 4)?;
                    }
                    let max_stack = read_unsigned(bytes, &mut at, 2)? as u16;
                    let pages = read_unsigned(bytes, &mut at, 2)? as u16;
                    Op::Postamble { pages, max_stack }
                }
                249 => {
                    let _postamble = read_unsigned(bytes, &mut at, 4)?;
                    let _version = read_unsigned(bytes, &mut at, 1)?;
                    // §590: the file is padded with 223s to a multiple of four,
                    // and the padding is the end of it.
                    while at < bytes.len() && bytes[at] == 223 {
                        at += 1;
                    }
                    ops.push(Op::Noop);
                    break;
                }
                250..=254 => Op::Extension(opcode),
                255 => return Err(format!("byte {}: 255 is not a DVI opcode", at - 1)),
            };
            ops.push(op);
        }
        Ok(Dvi { ops })
    }

    /// Read the file at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Dvi, String> {
        let path = path.as_ref();
        let bytes =
            std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Dvi::parse(&bytes)
    }

    /// How many pages the file ships.
    pub fn pages(&self) -> usize {
        self.ops
            .iter()
            .filter(|op| matches!(op, Op::BeginPage { .. }))
            .count()
    }

    /// The characters set, in order, as text.
    ///
    /// This is the comparison a parity harness will want when texrs can set a
    /// character: not the bytes of the file, which carry positions and font
    /// numbers that two correct engines may legitimately differ on, but what
    /// ended up on the page.
    ///
    /// The codes are the FONT's, not Unicode. DVI says "set character 11 of the
    /// current font", and in cmr10 that is the `ff` ligature — so `different`
    /// comes back as `di\u{b}erent`. Mapping it would need the font's encoding,
    /// which is a `.tfm` file this does not read; for comparing two engines it
    /// does not matter, because both write the same code.
    pub fn text(&self) -> String {
        self.ops
            .iter()
            .filter_map(|op| match op {
                Op::SetChar(c) | Op::PutChar(c) => char::from_u32(*c),
                _ => None,
            })
            .collect()
    }

    /// A summary a person reads, which is what `-X dvi` prints.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        for op in &self.ops {
            match op {
                Op::Preamble { version, comment } => {
                    out.push_str(&format!("preamble   version {version}, {comment:?}\n"));
                }
                Op::BeginPage { counts } => {
                    out.push_str(&format!("page       \\count0={}\n", counts[0]));
                }
                Op::DefineFont { number, name, at } => {
                    out.push_str(&format!("font {number:<5} {name} at {at}sp\n"));
                }
                Op::Special(text) => out.push_str(&format!("special    {text:?}\n")),
                Op::Postamble { pages, max_stack } => {
                    out.push_str(&format!("postamble  {pages} page(s), stack {max_stack}\n"));
                }
                Op::Extension(opcode) => {
                    out.push_str(&format!(
                        "extension  opcode {opcode} (XeTeX; not decoded)\n"
                    ));
                }
                _ => {}
            }
        }
        let text = self.text();
        if !text.is_empty() {
            out.push_str(&format!("text       {text:?}\n"));
        }
        out
    }
}

/// How two DVI files differ, in the terms a parity harness cares about.
#[derive(Debug, Clone, PartialEq)]
pub enum Difference {
    Pages {
        left: usize,
        right: usize,
    },
    /// The characters set differ, with the first place they part company.
    Text {
        at: usize,
        left: String,
        right: String,
    },
    /// One file draws a rule the other does not.
    Rules {
        left: usize,
        right: usize,
    },
    /// A `\special` one carries and the other does not.
    Special {
        only_in_left: bool,
        text: String,
    },
    /// The fonts the two files ask for.
    Fonts {
        left: Vec<String>,
        right: Vec<String>,
    },
}

impl Dvi {
    /// The rules drawn, in order.
    fn rules(&self) -> Vec<(i32, i32)> {
        self.ops
            .iter()
            .filter_map(|op| match op {
                Op::Rule { height, width, .. } => Some((*height, *width)),
                _ => None,
            })
            .collect()
    }

    /// The specials carried, in order.
    fn specials(&self) -> Vec<String> {
        self.ops
            .iter()
            .filter_map(|op| match op {
                Op::Special(text) => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// The fonts asked for, sorted and deduplicated.
    fn fonts(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::DefineFont { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// What differs between two DVI files, in what a reader would notice.
    ///
    /// Deliberately NOT a byte comparison. Two engines that typeset the same
    /// document identically still write different files: the comment carries a
    /// timestamp, the movement opcodes chosen for one displacement are a
    /// choice, and the postamble's pointers depend on where things landed. A
    /// parity harness that diffed bytes would report all of that as a
    /// divergence and none of it would be one.
    ///
    /// What is compared is what a reader would see: how many pages, the
    /// characters set, the rules drawn, the specials carried, the fonts asked
    /// for. Positions are left out for the same reason — two correct engines
    /// may legitimately round a displacement differently — which is why this
    /// says "the same document" rather than "the same file".
    pub fn compare(&self, other: &Dvi) -> Vec<Difference> {
        let mut out = Vec::new();
        if self.pages() != other.pages() {
            out.push(Difference::Pages {
                left: self.pages(),
                right: other.pages(),
            });
        }

        let (left_text, right_text) = (self.text(), other.text());
        if left_text != right_text {
            let at = left_text
                .chars()
                .zip(right_text.chars())
                .position(|(a, b)| a != b)
                .unwrap_or(left_text.chars().count().min(right_text.chars().count()));
            out.push(Difference::Text {
                at,
                left: left_text,
                right: right_text,
            });
        }

        let (left_rules, right_rules) = (self.rules(), other.rules());
        if left_rules.len() != right_rules.len() {
            out.push(Difference::Rules {
                left: left_rules.len(),
                right: right_rules.len(),
            });
        }

        for text in self.specials() {
            if !other.specials().contains(&text) {
                out.push(Difference::Special {
                    only_in_left: true,
                    text,
                });
            }
        }
        for text in other.specials() {
            if !self.specials().contains(&text) {
                out.push(Difference::Special {
                    only_in_left: false,
                    text,
                });
            }
        }

        let (left_fonts, right_fonts) = (self.fonts(), other.fonts());
        if left_fonts != right_fonts {
            out.push(Difference::Fonts {
                left: left_fonts,
                right: right_fonts,
            });
        }
        out
    }
}

/// `n` bytes, big-endian, unsigned. DVI is big-endian throughout.
fn read_unsigned(bytes: &[u8], at: &mut usize, n: usize) -> Result<u32, String> {
    if *at + n > bytes.len() {
        return Err(format!("byte {at}: the file ends inside an operand"));
    }
    let mut value: u32 = 0;
    for byte in &bytes[*at..*at + n] {
        value = (value << 8) | *byte as u32;
    }
    *at += n;
    Ok(value)
}

/// The same, signed: the top bit of the FIRST byte is the sign, so a
/// three-byte -1 is `ff ff ff` and not a very large positive number.
fn read_signed(bytes: &[u8], at: &mut usize, n: usize) -> Result<i32, String> {
    if *at + n > bytes.len() {
        return Err(format!("byte {at}: the file ends inside an operand"));
    }
    let mut value: i32 = if bytes[*at] & 0x80 != 0 { -1 } else { 0 };
    for byte in &bytes[*at..*at + n] {
        value = (value << 8) | *byte as i32;
    }
    *at += n;
    Ok(value)
}

fn read_string(bytes: &[u8], at: &mut usize, n: usize) -> Result<String, String> {
    if *at + n > bytes.len() {
        return Err(format!("byte {at}: the file ends inside a string"));
    }
    let text = String::from_utf8_lossy(&bytes[*at..*at + n]).into_owned();
    *at += n;
    Ok(text)
}

impl Dvi {
    /// Write these ops back out as a DVI file.
    ///
    /// Not the same bytes that were read: this is a re-encoding, not a copy.
    /// A movement written as `w0` (repeat the last horizontal move) comes back
    /// as an explicit `right`, a number written wider than it needed to be
    /// comes back in the narrowest form that holds it, and the postamble's
    /// counted maxima are counted again from what the pages do. What survives
    /// is everything a driver acts on, which is what
    /// [`Dvi::compare`] compares -- so a file read and written back sets the
    /// same characters, in the same fonts, at the same places.
    ///
    /// Characters do not carry their widths, so the horizontal extent this
    /// counts is a lower bound; a driver reads the `.tfm` for the real one.
    pub fn rewrite(&self) -> Vec<u8> {
        let comment = self
            .ops
            .iter()
            .find_map(|op| match op {
                Op::Preamble { comment, .. } => Some(comment.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let mut out = Writer::new(&comment);
        for op in &self.ops {
            match op {
                // The preamble is already written, and everything from the
                // postamble on is what `finish` builds: the fonts are defined
                // there a second time, and re-playing those definitions would
                // define each font twice.
                Op::Preamble { .. } => {}
                Op::Postamble { .. } => break,
                Op::BeginPage { counts } => out.begin_page(*counts),
                Op::EndPage => out.end_page(),
                Op::SetChar(code) => out.set_char(*code, 0),
                Op::PutChar(code) => out.put_char(*code),
                Op::Rule { height, width, set } => out.rule(*height, *width, *set),
                Op::Push => out.push(),
                Op::Pop => out.pop(),
                Op::Right(amount) => out.right(*amount),
                Op::Down(amount) => out.down(*amount),
                Op::Font(number) => out.font(*number),
                Op::DefineFont { number, name, at } => {
                    // A checksum of zero tells a driver not to check, which is
                    // the honest thing to write for a font this never read.
                    out.define_font(*number, name, *at, 0, *at)
                }
                Op::Special(text) => out.special(text),
                Op::Noop => out.noop(),
                Op::Extension(_) => {}
            }
        }
        out.finish()
    }
}

// ---------------------------------------------------------------------------
// Writing DVI.
// ---------------------------------------------------------------------------

/// A DVI file being written.
///
/// The reading half above exists so texrs can be compared with what real tex
/// shipped. This is the half the stomach will call when there is one: it takes
/// the same events the reader hands back and lays them out as `tex.web`
/// §583-§590 says a DVI file is laid out, which is more than writing opcodes.
/// A DVI file is a linked list read backwards -- every page points at the one
/// before it, the postamble points at the last page, and the last four bytes
/// point at the postamble -- so the pointers can only be filled in while
/// writing, and a writer that gets them wrong produces a file every viewer
/// refuses even though every opcode in it is correct.
///
/// The numbers in the postamble are the other half of that: the tallest and
/// widest a page reached, and the deepest the stack went. They are what a
/// driver allocates from before it reads a single page, so they are counted
/// here rather than asked for.
pub struct Writer {
    bytes: Vec<u8>,
    /// Where the last `bop` began, which the next one points at.
    previous_page: i32,
    pages: u16,
    fonts: Vec<(u32, String, i32, u32, i32)>,
    /// Position, and the extents a page reached, in scaled points.
    h: i32,
    v: i32,
    max_h: i32,
    max_v: i32,
    stack: Vec<(i32, i32)>,
    max_stack: u16,
}

/// The units tex writes: `num`/`den` say that one DVI unit is
/// 25400000/473628672 metres, which is 1/100000 of an inch.
const NUM: u32 = 25_400_000;
const DEN: u32 = 473_628_672;
/// DVI format 2, which is what every driver reads.
pub const VERSION: u8 = 2;

impl Default for Writer {
    fn default() -> Self {
        Writer::new("texrs")
    }
}

impl Writer {
    /// A writer whose preamble carries `comment` -- where tex stamps the format
    /// it used and the date it ran.
    pub fn new(comment: &str) -> Writer {
        let mut w = Writer {
            bytes: Vec::new(),
            previous_page: -1,
            pages: 0,
            fonts: Vec::new(),
            h: 0,
            v: 0,
            max_h: 0,
            max_v: 0,
            stack: Vec::new(),
            max_stack: 0,
        };
        w.byte(247);
        w.byte(VERSION);
        w.unsigned(NUM, 4);
        w.unsigned(DEN, 4);
        w.unsigned(1000, 4); // mag
        let comment: Vec<u8> = comment.bytes().take(255).collect();
        w.byte(comment.len() as u8);
        w.bytes.extend(&comment);
        w
    }

    fn byte(&mut self, b: u8) {
        self.bytes.push(b);
    }

    fn unsigned(&mut self, value: u32, width: usize) {
        for i in (0..width).rev() {
            self.bytes.push((value >> (8 * i)) as u8);
        }
    }

    fn signed(&mut self, value: i32, width: usize) {
        self.unsigned(value as u32, width);
    }

    /// The narrowest of the four widths that can hold `value`, which is what
    /// keeps a DVI file small: most movements fit in one byte.
    fn width_for(value: i32) -> usize {
        match value {
            -0x80..=0x7f => 1,
            -0x8000..=0x7fff => 2,
            -0x80_0000..=0x7f_ffff => 3,
            _ => 4,
        }
    }

    /// Define a font: its number, its name, the size it is used at, and the
    /// checksum and design size from its `.tfm`, which a driver compares
    /// against the font it finds so a document set with one cmr10 is not
    /// silently drawn with another.
    pub fn define_font(&mut self, number: u32, name: &str, at: i32, checksum: u32, design: i32) {
        self.fonts
            .push((number, name.to_string(), at, checksum, design));
        self.write_font_definition(number, name, at, checksum, design);
    }

    fn write_font_definition(
        &mut self,
        number: u32,
        name: &str,
        at: i32,
        checksum: u32,
        design: i32,
    ) {
        let width = Writer::width_for(number as i32).max(1);
        self.byte(242 + width as u8);
        self.unsigned(number, width);
        self.unsigned(checksum, 4);
        self.signed(at, 4);
        self.signed(design, 4);
        // The name is an area and a file name; texrs writes no area, and lets
        // the driver's own search find the font.
        let name: Vec<u8> = name.bytes().take(255).collect();
        self.byte(0);
        self.byte(name.len() as u8);
        self.bytes.extend(&name);
    }

    /// Select a font by number.
    pub fn font(&mut self, number: u32) {
        // §586: the first 64 fonts are an opcode each.
        if number < 64 {
            self.byte(171 + number as u8);
            return;
        }
        let width = Writer::width_for(number as i32).max(1);
        self.byte(234 + width as u8);
        self.unsigned(number, width);
    }

    /// Begin a page carrying `\count0..9`.
    pub fn begin_page(&mut self, counts: [i32; 10]) {
        let here = self.bytes.len() as i32;
        self.byte(139);
        for count in counts {
            self.signed(count, 4);
        }
        self.signed(self.previous_page, 4);
        self.previous_page = here;
        self.pages += 1;
        self.h = 0;
        self.v = 0;
        self.stack.clear();
    }

    pub fn end_page(&mut self) {
        self.byte(140);
    }

    /// Set a character, moving right by its width -- which the caller knows and
    /// this does not, so it is given.
    pub fn set_char(&mut self, code: u32, width: i32) {
        match code < 128 {
            true => self.byte(code as u8),
            false => {
                let bytes = Writer::width_for(code as i32).max(1);
                self.byte(127 + bytes as u8);
                self.unsigned(code, bytes);
            }
        }
        self.advance(width);
    }

    /// Set a character without moving.
    pub fn put_char(&mut self, code: u32) {
        let bytes = Writer::width_for(code as i32).max(1);
        self.byte(132 + bytes as u8);
        self.unsigned(code, bytes);
    }

    /// A rule, which moves right by its width when `set`.
    pub fn rule(&mut self, height: i32, width: i32, set: bool) {
        self.byte(match set {
            true => 132,
            false => 137,
        });
        self.signed(height, 4);
        self.signed(width, 4);
        if set {
            self.advance(width);
        }
        self.max_v = self.max_v.max(self.v + height.max(0));
    }

    /// Move right (or left, for a negative amount).
    pub fn right(&mut self, amount: i32) {
        let width = Writer::width_for(amount);
        self.byte(142 + width as u8);
        self.signed(amount, width);
        self.advance(amount);
    }

    /// Move down the page. Positive is down: DVI's y axis points the other way
    /// from the one most graphics formats use.
    pub fn down(&mut self, amount: i32) {
        let width = Writer::width_for(amount);
        self.byte(156 + width as u8);
        self.signed(amount, width);
        self.v += amount;
        self.max_v = self.max_v.max(self.v);
    }

    fn advance(&mut self, amount: i32) {
        self.h += amount;
        self.max_h = self.max_h.max(self.h);
    }

    pub fn push(&mut self) {
        self.byte(141);
        self.stack.push((self.h, self.v));
        self.max_stack = self.max_stack.max(self.stack.len() as u16);
    }

    pub fn pop(&mut self) {
        self.byte(142);
        if let Some((h, v)) = self.stack.pop() {
            self.h = h;
            self.v = v;
        }
    }

    /// `\special{…}`: what a document says that DVI has no word for.
    pub fn special(&mut self, text: &str) {
        let text = text.as_bytes();
        let width = Writer::width_for(text.len() as i32).max(1);
        self.byte(238 + width as u8);
        self.unsigned(text.len() as u32, width);
        self.bytes.extend(text);
    }

    pub fn noop(&mut self) {
        self.byte(138);
    }

    /// Close the file: the postamble, the fonts again, the pointer back to the
    /// postamble, and the padding that ends every DVI file.
    pub fn finish(mut self) -> Vec<u8> {
        let postamble = self.bytes.len() as u32;
        self.byte(248);
        self.signed(self.previous_page, 4);
        self.unsigned(NUM, 4);
        self.unsigned(DEN, 4);
        self.unsigned(1000, 4);
        self.signed(self.max_v, 4);
        self.signed(self.max_h, 4);
        self.unsigned(self.max_stack as u32, 2);
        self.unsigned(self.pages as u32, 2);
        // Every font is defined again in the postamble, so a driver can load
        // them all before reading a page.
        for (number, name, at, checksum, design) in self.fonts.clone() {
            self.write_font_definition(number, &name, at, checksum, design);
        }
        self.byte(249);
        self.unsigned(postamble, 4);
        self.byte(VERSION);
        // §590: at least four 223s, and enough of them to make the file a
        // multiple of four bytes long.
        for _ in 0..4 {
            self.byte(223);
        }
        while !self.bytes.len().is_multiple_of(4) {
            self.byte(223);
        }
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What real tex ships for `src`, or `None` when there is no tex here.
    fn tex_dvi(name: &str, src: &str) -> Option<Vec<u8>> {
        engine_dvi("tex", name, src)
    }

    /// The same, from whichever engine is named. pdftex needs telling to write
    /// DVI rather than PDF; tex has no such flag and ignores it.
    fn engine_dvi(engine: &str, name: &str, src: &str) -> Option<Vec<u8>> {
        let dir = std::env::temp_dir().join(format!("texrs_dvi_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).ok()?;
        std::fs::write(dir.join("t.tex"), src).ok()?;
        let mut command = std::process::Command::new(engine);
        if engine != "tex" {
            command.arg("-output-format=dvi");
        }
        let ran = command
            .arg("-interaction=batchmode")
            .arg("t.tex")
            .current_dir(&dir)
            .output()
            .ok()?;
        let _ = ran;
        let bytes = std::fs::read(dir.join("t.dvi")).ok();
        let _ = std::fs::remove_dir_all(&dir);
        bytes
    }

    /// What `dvitype` says about `bytes`, or `None` when there is no TeX here.
    /// It is Knuth's own reader and it validates as it goes, so it is the
    /// oracle for a file this wrote.
    fn dvitype(bytes: &[u8]) -> Option<String> {
        let dir = std::env::temp_dir().join(format!("texrs_dvitype_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).ok()?;
        let file = dir.join("w.dvi");
        std::fs::write(&file, bytes).ok()?;
        let ran = std::process::Command::new("dvitype")
            .arg("w.dvi")
            .current_dir(&dir)
            .output()
            .ok()?;
        let said = format!(
            "{}{}",
            String::from_utf8_lossy(&ran.stdout),
            String::from_utf8_lossy(&ran.stderr)
        );
        let _ = std::fs::remove_dir_all(&dir);
        Some(said)
    }

    /// A file this writes is a file Knuth's own reader accepts, and one this
    /// reads back is the same document.
    ///
    /// The pointers are what makes this worth testing: a DVI file is a linked
    /// list read backwards, and a writer that gets the back-pointers or the
    /// postamble's position wrong produces a file every driver refuses even
    /// though every opcode in it is right. `dvitype` follows those pointers.
    #[test]
    fn what_this_writes_is_a_file_dvitype_reads() {
        let Some(bytes) = tex_dvi("rewrite", "Hello DVI world.\n\\bye\n") else {
            return;
        };
        let original = Dvi::parse(&bytes).expect("real tex parses");
        let written = original.rewrite();
        let again = Dvi::parse(&written).expect("what this wrote parses");

        // The same document: the same pages, the same text, the same fonts, in
        // the same places.
        assert_eq!(
            original.compare(&again),
            Vec::new(),
            "a file read and written back is a different document"
        );
        assert_eq!(again.pages(), original.pages());
        assert_eq!(again.text(), original.text());

        // And it is a file rather than a pile of opcodes.
        let Some(said) = dvitype(&written) else {
            return;
        };
        assert!(
            !said.contains("Bad DVI file"),
            "dvitype refused what this wrote: {said}"
        );
        assert!(said.contains("totalpages=1"), "{said}");
        assert!(said.contains("cmr10"), "the font came through: {said}");
        // dvitype checks the postamble's counted maxima against the pages it
        // reads and complains when they disagree.
        assert!(!said.contains("should be"), "{said}");
    }

    /// A page built from nothing, which is what the stomach will do: define a
    /// font, put the origin where tex puts it, and set some characters.
    #[test]
    fn a_page_written_from_nothing_says_what_it_holds() {
        // The checksum a driver compares against the font it finds, from the
        // font itself.
        let found = std::process::Command::new("kpsewhich")
            .arg("cmr10.tfm")
            .output();
        let Ok(found) = found else { return };
        let path = String::from_utf8_lossy(&found.stdout).trim().to_string();
        let Ok(tfm) = crate::tfm::Tfm::open(&path) else {
            return;
        };
        // 10pt, in the scaled points a DVI file counts in.
        let ten_pt = 655_360;
        let checksum = tfm.checksum;

        let mut w = Writer::new("texrs test");
        w.define_font(0, "cmr10", ten_pt, checksum, ten_pt);
        w.begin_page([1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        w.push();
        // One inch down and one inch across from the corner tex measures from.
        w.down(ten_pt * 72 / 10);
        w.right(ten_pt * 72 / 10);
        w.font(0);
        for c in "Hi".chars() {
            let width = tfm.char(c as u8).map(|m| m.width).unwrap_or(0.0);
            w.set_char(c as u32, (width * ten_pt as f64) as i32);
        }
        // A rule under it, which is the other thing a page can hold.
        w.rule(ten_pt / 10, ten_pt, true);
        w.pop();
        w.end_page();
        let bytes = w.finish();

        // It reads back as what was written.
        let dvi = Dvi::parse(&bytes).expect("parses");
        assert_eq!(dvi.pages(), 1);
        assert_eq!(dvi.text(), "Hi", "{:?}", dvi.text());
        assert!(
            bytes.len().is_multiple_of(4),
            "a DVI file is a multiple of four bytes"
        );
        assert!(
            bytes.ends_with(&[223, 223, 223, 223]),
            "and ends in padding"
        );

        // And dvitype reads it, checksum and all: a mismatch there is what it
        // reports when a document was set with a different font of the same
        // name.
        let Some(said) = dvitype(&bytes) else { return };
        assert!(!said.contains("Bad DVI file"), "{said}");
        assert!(!said.contains("checksum doesn't match"), "{said}");
        assert!(said.contains("totalpages=1"), "{said}");
        assert!(said.contains("[Hi]"), "the characters it set: {said}");
    }

    /// The postamble's numbers are counted while writing, and they are what a
    /// driver allocates from before it has read a page: the tallest and widest
    /// the pages reached, the deepest the stack went, and how many pages there
    /// are. This checks the bookkeeping directly, because `dvitype` reads them
    /// without checking them.
    #[test]
    fn the_postamble_counts_what_the_pages_did() {
        let mut w = Writer::new("counting");
        w.begin_page([1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        w.push();
        w.down(100);
        w.push();
        w.down(400); // 500 down, the deepest this page goes
        w.right(700);
        w.pop();
        w.right(50); // back at 100 down, so this does not raise the maximum
        w.pop();
        w.end_page();
        w.begin_page([2, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        w.right(900); // wider than page one
        w.end_page();
        let bytes = w.finish();

        // Follow the file the way a driver does: the last bytes point at the
        // postamble.
        let end = bytes.len() - bytes.iter().rev().take_while(|&&b| b == 223).count();
        let at = u32::from_be_bytes([
            bytes[end - 5],
            bytes[end - 4],
            bytes[end - 3],
            bytes[end - 2],
        ]) as usize;
        assert_eq!(bytes[at], 248, "the pointer lands on the postamble");
        let number = |from: usize, width: usize| -> i64 {
            bytes[from..from + width]
                .iter()
                .fold(0i64, |value, &b| (value << 8) | b as i64)
        };
        // post: p(4) num(4) den(4) mag(4) max_v(4) max_h(4) max_stack(2) pages(2)
        assert_eq!(number(at + 17, 4), 500, "the tallest a page reached");
        assert_eq!(number(at + 21, 4), 900, "the widest");
        assert_eq!(number(at + 25, 2), 2, "the deepest the stack went");
        assert_eq!(number(at + 27, 2), 2, "two pages");

        // The last page points back at the one before it, which is how a
        // driver reads a document backwards.
        let last = number(at + 1, 4) as usize;
        assert_eq!(bytes[last], 139, "the pointer lands on a page");
        let previous = number(last + 41, 4);
        assert!(previous > 0, "the second page points at the first");
        assert_eq!(bytes[previous as usize], 139);
        // `number` reads unsigned, and a pointer to nothing is -1 written in
        // four bytes.
        assert_eq!(
            number(previous as usize + 41, 4),
            0xffff_ffff,
            "the first points at nothing"
        );
    }

    /// `w0` moves by the last distance a `w` instruction set, not by nothing.
    ///
    /// This is how a DVI file sets a line of text without repeating the same
    /// operand between every pair of letters, and reading it as a movement of
    /// zero stacks the line on one spot. The virtual fonts caught this: their
    /// packets use `w0` where a character is set twice the same distance
    /// apart, and `vftovp` prints the distance both times.
    #[test]
    fn the_movement_registers_repeat_the_last_distance() {
        // w3 100, w0, x3 7, x0, then y3 40, y0 -- each register its own.
        let page = [
            150, 0, 0, 100, // w3 100
            147, // w0
            155, 0, 0, 7,   // x3 7
            152, // x0
            164, 0, 0, 40,  // y3 40
            161, // y0
        ];
        let ops = Dvi::parse(&page).expect("parses").ops;
        assert_eq!(
            ops,
            vec![
                Op::Right(100),
                Op::Right(100),
                Op::Right(7),
                Op::Right(7),
                Op::Down(40),
                Op::Down(40),
            ]
        );

        // §  : push and pop save and restore the registers with the position,
        // so a movement set inside a group is forgotten when it ends.
        let page = [
            150, 0, 0, 100, // w3 100
            141, // push
            150, 0, 0, 5,   // w3 5
            147, // w0 -- 5
            142, // pop
            147, // w0 -- 100 again
        ];
        let ops = Dvi::parse(&page).expect("parses").ops;
        assert_eq!(
            ops,
            vec![
                Op::Right(100),
                Op::Push,
                Op::Right(5),
                Op::Right(5),
                Op::Pop,
                Op::Right(100),
            ]
        );
    }

    #[test]
    fn a_page_of_real_tex_reads_back_as_its_text() {
        let Some(bytes) = tex_dvi("hello", "Hello DVI world.\n\\bye\n") else {
            // No tex here: the parity suite needs one too, so this is the same
            // skip those tests take rather than a failure of this one.
            return;
        };
        let dvi = Dvi::parse(&bytes).expect("parses");

        assert_eq!(dvi.pages(), 1, "one page");
        let text = dvi.text();
        assert!(text.contains("Hello"), "{text:?}");
        assert!(text.contains("world"), "{text:?}");

        // The preamble says what wrote it, and the postamble counts what it
        // wrote — the two ends of the file agreeing is the check that the walk
        // did not lose its place.
        let preamble = dvi.ops.first().expect("an op");
        assert!(
            matches!(preamble, Op::Preamble { version: 2, .. }),
            "{preamble:?}"
        );
        let counted = dvi.ops.iter().find_map(|op| match op {
            Op::Postamble { pages, .. } => Some(*pages),
            _ => None,
        });
        assert_eq!(counted, Some(1), "the postamble counts the page");
        assert!(
            dvi.ops.iter().any(|op| matches!(op, Op::DefineFont { .. })),
            "a page that sets characters defines a font"
        );
    }

    #[test]
    fn two_pages_are_two_pages() {
        let Some(bytes) = tex_dvi("pages", "One.\n\\eject\nTwo.\n\\bye\n") else {
            return;
        };
        let dvi = Dvi::parse(&bytes).expect("parses");
        assert_eq!(dvi.pages(), 2);
        // `\count0` is the page number, which is what a page carries in its
        // first counter.
        let numbers: Vec<i32> = dvi
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::BeginPage { counts } => Some(counts[0]),
                _ => None,
            })
            .collect();
        assert_eq!(numbers, vec![1, 2]);
    }

    #[test]
    fn a_special_survives_as_the_text_it_carried() {
        let Some(bytes) = tex_dvi("special", "\\special{color push rgb 1 0 0}A\n\\bye\n") else {
            return;
        };
        let dvi = Dvi::parse(&bytes).expect("parses");
        let specials: Vec<&String> = dvi
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Special(text) => Some(text),
                _ => None,
            })
            .collect();
        assert_eq!(specials.len(), 1, "{:?}", dvi.summary());
        assert!(specials[0].contains("color push"), "{:?}", specials[0]);
    }

    #[test]
    fn two_engines_typesetting_one_document_agree() {
        // The comparison the parity contract will need when texrs ships DVI,
        // usable today to check one engine against another: tex and pdftex
        // write different bytes for the same document — the comment carries a
        // timestamp, and the movement opcodes are a choice — and this must
        // report no difference all the same.
        let Some(from_tex) = tex_dvi("cmp_tex", "Hello DVI world.\n\\bye\n") else {
            return;
        };
        let Some(from_pdftex) = engine_dvi("pdftex", "cmp_pdftex", "Hello DVI world.\n\\bye\n")
        else {
            return;
        };
        let left = Dvi::parse(&from_tex).expect("parses");
        let right = Dvi::parse(&from_pdftex).expect("parses");
        assert_eq!(
            left.compare(&right),
            vec![],
            "the same document typeset by two engines is the same document"
        );

        // And what the comparison deliberately ignores: the preamble's comment
        // carries the time of the run, so two runs a minute apart write
        // different bytes for one document. A harness that compared those
        // would report a divergence every time the clock ticked.
        let mut later = left.clone();
        for op in &mut later.ops {
            if let Op::Preamble { comment, .. } = op {
                *comment = " TeX output 1999.12.31:2359".to_string();
            }
        }
        assert_ne!(later.ops, left.ops, "the files differ");
        assert_eq!(later.compare(&left), vec![], "the documents do not");
    }

    #[test]
    fn a_document_that_differs_says_where() {
        let Some(one) = tex_dvi("cmp_one", "Hello.\n\\bye\n") else {
            return;
        };
        let Some(two) = tex_dvi("cmp_two", "Hellp.\n\\bye\n") else {
            return;
        };
        let differences = Dvi::parse(&one)
            .unwrap()
            .compare(&Dvi::parse(&two).unwrap());
        match differences.first() {
            Some(Difference::Text { at, left, right }) => {
                assert_eq!(
                    *at, 4,
                    "the fifth character is where they part: {left} {right}"
                );
            }
            other => panic!("expected a text difference, got {other:?}"),
        }

        // A page more is a difference of its own.
        let Some(long) = tex_dvi("cmp_pages", "One.\n\\eject\nTwo.\n\\bye\n") else {
            return;
        };
        let differences = Dvi::parse(&one)
            .unwrap()
            .compare(&Dvi::parse(&long).unwrap());
        assert!(
            differences
                .iter()
                .any(|d| matches!(d, Difference::Pages { left: 1, right: 2 })),
            "{differences:?}"
        );
    }

    #[test]
    fn the_operands_are_big_endian_and_signed_where_the_format_says() {
        // §583: a number is big-endian, and a signed one takes its sign from
        // the top bit of the FIRST byte — so three bytes of 0xff are -1 and
        // not 16777215.
        let mut at = 0;
        assert_eq!(read_unsigned(&[0x01, 0x02], &mut at, 2).unwrap(), 0x0102);
        let mut at = 0;
        assert_eq!(read_signed(&[0xff, 0xff, 0xff], &mut at, 3).unwrap(), -1);
        let mut at = 0;
        assert_eq!(read_signed(&[0x7f, 0xff], &mut at, 2).unwrap(), 32767);
        let mut at = 0;
        assert_eq!(read_signed(&[0x80, 0x00], &mut at, 2).unwrap(), -32768);

        // A file that ends inside an operand says so rather than reading past.
        let mut at = 0;
        assert!(read_unsigned(&[0x01], &mut at, 4).is_err());
        assert!(Dvi::parse(&[139, 0, 0]).is_err(), "a truncated page");
    }

    #[test]
    fn what_is_not_dvi_is_refused_and_xetex_is_named() {
        // 255 is not an opcode in any version of the format.
        assert!(Dvi::parse(&[255]).unwrap_err().contains("not a DVI opcode"));
        // An empty file parses to nothing rather than failing: it is not
        // damaged, it is empty.
        assert!(Dvi::parse(&[]).unwrap().ops.is_empty());
        // XeTeX's extensions are named rather than decoded, so pointing this
        // at an .xdv says what it is.
        let dvi = Dvi::parse(&[253]).unwrap();
        assert_eq!(dvi.ops, vec![Op::Extension(253)]);
        assert!(dvi.summary().contains("XeTeX"), "{}", dvi.summary());
    }
}
