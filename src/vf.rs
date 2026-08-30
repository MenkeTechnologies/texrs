//! Reading a `.vf`, the virtual font a driver has to unpack before it can set
//! a character.
//!
//! Ported from the VF half of tectonic's `xdvipdfmx` (`vf.c`), which is
//! Knuth's `vftovp.web` in C. A virtual font has no glyphs of its own. Each of
//! its characters is a little DVI program saying what to set in some *other*
//! font: `ptmr7t`'s `fi` is one glyph of `ptmr8r`, its `\v{c}` is a `c` with a
//! caron moved into place, and its `dotlessj` is a rule and a `\special`
//! complaining that the glyph does not exist. This is how TeX's 7-bit font
//! encodings were carried onto PostScript fonts that were laid out differently,
//! and every `.dvi` that names a `.vf` font is unreadable without it.
//!
//! The packets are DVI, so they are read with the DVI reader rather than a
//! second copy of it: the same opcodes, the same operands, and
//! [`crate::dvi::Op`] for what comes back. Lengths inside a packet are
//! `fix_word`s of the design size, as they are in a `.tfm`.

use std::collections::BTreeMap;
use std::path::Path;

use crate::dvi::{Dvi, Op};

/// A font a virtual font sets characters in.
#[derive(Debug, Clone, PartialEq)]
pub struct MapFont {
    pub number: u32,
    pub name: String,
    pub checksum: u32,
    /// The size it is used at, and its design size, both in points of the
    /// virtual font's own design size.
    pub at: f64,
    pub design_size: f64,
}

/// One character of a virtual font: its width, and the program that sets it.
#[derive(Debug, Clone, PartialEq)]
pub struct VfChar {
    pub code: u32,
    /// The width TeX uses for this character, in design-size units. It is the
    /// virtual font's own width, and need not be what the program it runs
    /// actually draws.
    pub width: f64,
    pub ops: Vec<Op>,
}

impl VfChar {
    /// Which characters of which fonts this one really sets: what a driver
    /// puts on the page. Font 0 until a `Font` op says otherwise, as in DVI.
    pub fn glyphs(&self) -> Vec<(u32, u32)> {
        let mut font = 0u32;
        let mut out = Vec::new();
        for op in &self.ops {
            match op {
                Op::Font(number) => font = *number,
                Op::SetChar(code) | Op::PutChar(code) => out.push((font, *code)),
                _ => {}
            }
        }
        out
    }
}

/// A virtual font.
#[derive(Debug, Clone, Default)]
pub struct Vf {
    pub comment: String,
    pub checksum: u32,
    pub design_size: f64,
    pub fonts: Vec<MapFont>,
    chars: BTreeMap<u32, VfChar>,
}

/// `vftovp.web` §  : a `.vf` begins with `pre` and the id byte 202.
const PRE: u8 = 247;
const ID: u8 = 202;
/// The long form of a character packet, for one that does not fit in a byte.
const LONG_CHAR: u8 = 242;
const POST: u8 = 248;

/// A `fix_word`, as in a `.tfm`: 20 bits of fraction, so 1.0 is the design
/// size.
fn fix_word(raw: i64) -> f64 {
    raw as f64 / (1 << 20) as f64
}

impl Vf {
    pub fn open(path: impl AsRef<Path>) -> Result<Vf, String> {
        let path = path.as_ref();
        let bytes =
            std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Vf::parse(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Read `bytes` as a virtual font.
    pub fn parse(bytes: &[u8]) -> Result<Vf, String> {
        let mut at = 0usize;
        let byte = |at: &mut usize| -> Result<u8, String> {
            let b = bytes
                .get(*at)
                .copied()
                .ok_or_else(|| format!("byte {at} is past the end of the file"))?;
            *at += 1;
            Ok(b)
        };
        let number = |at: &mut usize, width: usize| -> Result<i64, String> {
            let region = bytes
                .get(*at..*at + width)
                .ok_or_else(|| format!("byte {at} is past the end of the file"))?;
            *at += width;
            Ok(region
                .iter()
                .fold(0i64, |value, &b| (value << 8) | b as i64))
        };
        let signed = |at: &mut usize, width: usize| -> Result<i64, String> {
            let value = number(at, width)?;
            let top = 1i64 << (8 * width - 1);
            Ok(match value >= top {
                true => value - 2 * top,
                false => value,
            })
        };

        if byte(&mut at)? != PRE {
            return Err("does not begin with pre".into());
        }
        let id = byte(&mut at)?;
        if id != ID {
            return Err(format!("{id} is not the virtual font id ({ID})"));
        }
        let comment_length = byte(&mut at)? as usize;
        let comment = bytes
            .get(at..at + comment_length)
            .ok_or("the comment runs past the end of the file")?
            .iter()
            .map(|&b| b as char)
            .collect();
        at += comment_length;

        let mut vf = Vf {
            comment,
            checksum: number(&mut at, 4)? as u32,
            design_size: fix_word(number(&mut at, 4)?),
            ..Vf::default()
        };

        // A file that stops without a `post` is damaged, but what was read is
        // still what it said.
        while let Some(&opcode) = bytes.get(at) {
            at += 1;
            match opcode {
                POST => break,
                // §  : fnt_def1..4, laid out as they are in a DVI file.
                243..=246 => {
                    let width = (opcode - 242) as usize;
                    let number_of = number(&mut at, width)? as u32;
                    let checksum = number(&mut at, 4)? as u32;
                    let size = fix_word(number(&mut at, 4)?);
                    let design = fix_word(number(&mut at, 4)?);
                    let area = byte(&mut at)? as usize;
                    let file = byte(&mut at)? as usize;
                    let name: String = bytes
                        .get(at..at + area + file)
                        .ok_or("a font name runs past the end of the file")?
                        .iter()
                        .map(|&b| b as char)
                        .collect();
                    at += area + file;
                    vf.fonts.push(MapFont {
                        number: number_of,
                        name,
                        checksum,
                        at: size,
                        design_size: design,
                    });
                }
                // The long form of a character packet, for a packet longer
                // than 241 bytes or a character above 255.
                LONG_CHAR => {
                    let length = number(&mut at, 4)? as usize;
                    let code = number(&mut at, 4)? as u32;
                    let width = fix_word(signed(&mut at, 4)?);
                    vf.read_packet(bytes, &mut at, code, width, length)?;
                }
                // §  : the opcode IS the packet's length, which is what makes
                // a virtual font small -- most characters are one `set_char`.
                0..=241 => {
                    let length = opcode as usize;
                    let code = byte(&mut at)? as u32;
                    let width = fix_word(number(&mut at, 3)?);
                    vf.read_packet(bytes, &mut at, code, width, length)?;
                }
                other => return Err(format!("byte {}: {other} is not a VF opcode", at - 1)),
            }
        }
        Ok(vf)
    }

    fn read_packet(
        &mut self,
        bytes: &[u8],
        at: &mut usize,
        code: u32,
        width: f64,
        length: usize,
    ) -> Result<(), String> {
        let packet = bytes
            .get(*at..*at + length)
            .ok_or_else(|| format!("the packet for character {code} runs past the end"))?;
        *at += length;
        // The packet is DVI, so it is read by the DVI reader.
        let ops = Dvi::parse(packet)
            .map_err(|e| format!("character {code}: {e}"))?
            .ops;
        self.chars.insert(code, VfChar { code, width, ops });
        Ok(())
    }

    /// The character with this code.
    pub fn char(&self, code: u32) -> Option<&VfChar> {
        self.chars.get(&code)
    }

    /// Every code the font defines, in order.
    pub fn codes(&self) -> Vec<u32> {
        self.chars.keys().copied().collect()
    }

    /// The font a `Font` number names.
    pub fn font(&self, number: u32) -> Option<&MapFont> {
        self.fonts.iter().find(|f| f.number == number)
    }

    /// A summary a person reads.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        if !self.comment.is_empty() {
            out.push_str(&format!("comment       {}\n", self.comment));
        }
        out.push_str(&format!("designsize    {:.6}pt\n", self.design_size));
        out.push_str(&format!("checksum      0o{:o}\n", self.checksum));
        out.push_str(&format!("characters    {}\n", self.chars.len()));
        for font in &self.fonts {
            out.push_str(&format!(
                "font {:<3}      {} at {:.6}\n",
                font.number, font.name, font.at
            ));
        }
        out
    }

    /// One character's line: its width, and what it really sets.
    pub fn describe(&self, code: u32) -> String {
        let Some(c) = self.char(code) else {
            return format!("{code}: the font does not define it\n");
        };
        let shown = match char::from_u32(code).is_some_and(|c| c.is_ascii_graphic()) {
            true => format!("'{}'", code as u8 as char),
            false => format!("0o{code:o}"),
        };
        let mut out = format!("{shown}  width {:.6}\n", c.width);
        for op in &c.ops {
            let line = match op {
                Op::SetChar(code) => format!("set 0o{code:o}"),
                Op::PutChar(code) => format!("put 0o{code:o}"),
                Op::Right(amount) => format!("right {:.6}", fix_word(*amount as i64)),
                Op::Down(amount) => format!("down {:.6}", fix_word(*amount as i64)),
                Op::Rule { height, width, .. } => format!(
                    "rule {:.6} by {:.6}",
                    fix_word(*height as i64),
                    fix_word(*width as i64)
                ),
                Op::Font(number) => match self.font(*number) {
                    Some(font) => format!("font {number} ({})", font.name),
                    None => format!("font {number}"),
                },
                Op::Special(text) => format!("special {text}"),
                Op::Push => "push".into(),
                Op::Pop => "pop".into(),
                other => format!("{other:?}"),
            };
            out.push_str(&format!("  {line}\n"));
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

    /// A virtual font that ships with TeX Live: Times in TeX's own text
    /// encoding, which is a program per character over the real Times.
    #[test]
    fn a_virtual_font_says_what_it_really_sets() {
        let Some(bytes) = installed("ptmr7t.vf") else {
            return;
        };
        let vf = Vf::parse(&bytes).expect("ptmr7t reads");

        assert!((vf.design_size - 10.0).abs() < 1e-9, "{}", vf.design_size);
        // It sets everything in one font, the real Times in its own encoding.
        assert_eq!(vf.fonts.len(), 1, "{:?}", vf.fonts);
        assert_eq!(vf.fonts[0].name, "ptmr8r");
        assert!(
            (vf.fonts[0].at - 1.0).abs() < 1e-9,
            "used at its design size"
        );

        // The common case, and the reason the format is small: one character
        // of the virtual font is one character of the real one.
        let a = vf.char(b'A' as u32).expect("an A");
        assert_eq!(a.glyphs(), vec![(0, b'A' as u32)]);
        assert!((a.width - 0.721997).abs() < 5e-6, "{}", a.width);

        // The interesting case: a character TeX has and the real font does
        // not, built out of pieces.
        let ff = vf.char(0o13).expect("the ff ligature");
        assert!(ff.ops.len() > 1 || !ff.glyphs().is_empty(), "{:?}", ff.ops);
    }

    /// What is not a virtual font is refused, rather than read as one.
    #[test]
    fn what_is_not_a_virtual_font_is_refused() {
        assert!(Vf::parse(b"").is_err());
        assert!(Vf::parse(b"not a font").is_err());
        // A .tfm begins with lengths, not with `pre`.
        if let Some(tfm) = installed("cmr10.tfm") {
            let e = Vf::parse(&tfm).unwrap_err();
            assert!(e.contains("pre"), "{e}");
        }
        // The right first byte and the wrong id: a DVI file, which begins
        // `pre` and 2.
        assert!(Vf::parse(&[247, 2, 0]).unwrap_err().contains("202"));
        // A packet whose length runs past the end of the file.
        let mut truncated = vec![247, 202, 0];
        truncated.extend([0, 0, 0, 0]); // checksum
        truncated.extend([0, 0x10, 0, 0]); // design size
        truncated.extend([40, b'A', 0, 0, 0]); // a 40-byte packet, and no bytes
        assert!(Vf::parse(&truncated).unwrap_err().contains("past the end"));
    }
}
