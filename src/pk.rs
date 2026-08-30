//! Reading a `.pk`, the packed bitmap font, ported from `pkfont.c` in
//! tectonic's `xdvipdfmx`.
//!
//! A `.pk` holds what a `.tfm` does not: the pixels. It is what a driver drew
//! Computer Modern from for twenty years, and it is still what
//! `xdvipdfmx` falls back to when a document names a font that has no outline.
//! The format is Rokicki's, and it is a compression scheme rather than a
//! layout: a glyph is stored as the lengths of its runs of black and white,
//! and those lengths are stored as a stream of *nybbles* whose meaning depends
//! on a per-character parameter, `dyn_f`, chosen when the font was packed.
//! Small numbers are one nybble, middling ones two, large ones a run of zeros
//! saying how many nybbles follow. A row that repeats says so once instead of
//! being stored again.
//!
//! That is three encodings in one stream, and getting one of them slightly
//! wrong gives a glyph that is plausible and not the one in the file -- so the
//! decoder is held against `gftype`'s own picture of every character, pixel for
//! pixel, in `tests/pk.rs`.

use std::collections::BTreeMap;
use std::path::Path;

/// One character: where it sits, how wide TeX thinks it is, and its pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    pub code: u32,
    /// The width TeX sets it with, as a `fix_word` of the design size.
    pub tfm_width: f64,
    /// How far the reference point moves, in pixels.
    pub dx: f64,
    pub dy: f64,
    pub width: usize,
    pub height: usize,
    /// Where the reference point is: `x_offset` columns right of the left edge
    /// and `y_offset` rows below the top one, both counted from the glyph's own
    /// corner, and both usually negative on the left.
    pub x_offset: i32,
    pub y_offset: i32,
    /// One byte per pixel, row by row from the top: 1 is black.
    pub pixels: Vec<u8>,
}

impl Glyph {
    /// Whether the pixel at `(column, row)` is black, counting from the top
    /// left.
    pub fn black(&self, column: usize, row: usize) -> bool {
        if column >= self.width || row >= self.height {
            return false;
        }
        self.pixels[row * self.width + column] == 1
    }

    /// The glyph as rows of text, which is how a person reads a bitmap.
    pub fn rows(&self) -> Vec<String> {
        (0..self.height)
            .map(|row| {
                (0..self.width)
                    .map(|column| match self.black(column, row) {
                        true => '*',
                        false => ' ',
                    })
                    .collect()
            })
            .collect()
    }
}

/// A packed font.
#[derive(Debug, Clone, Default)]
pub struct Pk {
    pub comment: String,
    /// The design size, as a `fix_word` in points.
    pub design_size: f64,
    pub checksum: u32,
    /// Pixels per point, horizontally and vertically -- which is where the
    /// resolution comes from: 600 dpi is 600/72.27 pixels per point.
    pub h_pixels_per_point: f64,
    pub v_pixels_per_point: f64,
    /// Everything a `\special` in the font said, which is nearly always
    /// nothing.
    pub specials: Vec<String>,
    glyphs: BTreeMap<u32, Glyph>,
}

const PRE: u8 = 247;
/// The id byte that says this is a packed font and not some other DVI-like
/// file.
const ID: u8 = 89;
const POST: u8 = 245;
const NO_OP: u8 = 246;
const YYY: u8 = 244;

fn fix_word(raw: i64) -> f64 {
    raw as f64 / (1 << 20) as f64
}

impl Pk {
    pub fn open(path: impl AsRef<Path>) -> Result<Pk, String> {
        let path = path.as_ref();
        let bytes =
            std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Pk::parse(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Read `bytes` as a packed font.
    pub fn parse(bytes: &[u8]) -> Result<Pk, String> {
        // The two bytes of `pre` and the id are checked before anything is
        // read from them.
        let mut at = 2usize;
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

        if bytes.first().copied() != Some(PRE) {
            return Err("does not begin with pre".into());
        }
        if bytes.get(1).copied() != Some(ID) {
            return Err(format!(
                "{} is not the packed font id ({ID})",
                bytes.get(1).copied().unwrap_or(0)
            ));
        }
        let comment_length = number(&mut at, 1)? as usize;
        let comment = bytes
            .get(at..at + comment_length)
            .ok_or("the comment runs past the end of the file")?
            .iter()
            .map(|&b| b as char)
            .collect();
        at += comment_length;

        let mut pk = Pk {
            comment,
            design_size: fix_word(number(&mut at, 4)?),
            checksum: number(&mut at, 4)? as u32,
            // §  : the resolution, as pixels per point in 2^-16ths.
            h_pixels_per_point: number(&mut at, 4)? as f64 / 65536.0,
            v_pixels_per_point: number(&mut at, 4)? as f64 / 65536.0,
            ..Pk::default()
        };

        while let Some(&flag) = bytes.get(at) {
            at += 1;
            match flag {
                POST => break,
                NO_OP => {}
                YYY => {
                    number(&mut at, 4)?;
                }
                // pk_xxx1..4: a special, whose length is 1 to 4 bytes.
                240..=243 => {
                    let length = number(&mut at, (flag - 239) as usize)? as usize;
                    let text = bytes
                        .get(at..at + length)
                        .ok_or("a special runs past the end of the file")?
                        .iter()
                        .map(|&b| b as char)
                        .collect();
                    at += length;
                    pk.specials.push(text);
                }
                PRE => return Err("a second preamble".into()),
                // A character. The flag byte carries `dyn_f`, the colour of the
                // first run, and which of the three sizes the header is in.
                _ => {
                    let dyn_f = (flag >> 4) as i32;
                    let first_black = (flag >> 3) & 1 == 1;
                    let (length, code, tfm, dx, dy, width, height, x_offset, y_offset) =
                        match flag & 7 {
                            // The long form: everything four bytes, for a glyph
                            // too big for the short ones.
                            7 => {
                                let length = number(&mut at, 4)? as usize;
                                let header = 28;
                                (
                                    length.saturating_sub(header),
                                    number(&mut at, 4)? as u32,
                                    fix_word(number(&mut at, 4)?),
                                    signed(&mut at, 4)? as f64 / 65536.0,
                                    signed(&mut at, 4)? as f64 / 65536.0,
                                    number(&mut at, 4)? as usize,
                                    number(&mut at, 4)? as usize,
                                    signed(&mut at, 4)? as i32,
                                    signed(&mut at, 4)? as i32,
                                )
                            }
                            // The middle form: two bytes for each measurement.
                            4..=6 => {
                                let length =
                                    (((flag & 3) as usize) << 16) + number(&mut at, 2)? as usize;
                                (
                                    length.saturating_sub(13),
                                    number(&mut at, 1)? as u32,
                                    fix_word(number(&mut at, 3)?),
                                    number(&mut at, 2)? as f64,
                                    0.0,
                                    number(&mut at, 2)? as usize,
                                    number(&mut at, 2)? as usize,
                                    signed(&mut at, 2)? as i32,
                                    signed(&mut at, 2)? as i32,
                                )
                            }
                            // The short form, which is nearly every character
                            // of a text font.
                            _ => {
                                let length =
                                    (((flag & 3) as usize) << 8) + number(&mut at, 1)? as usize;
                                (
                                    length.saturating_sub(8),
                                    number(&mut at, 1)? as u32,
                                    fix_word(number(&mut at, 3)?),
                                    number(&mut at, 1)? as f64,
                                    0.0,
                                    number(&mut at, 1)? as usize,
                                    number(&mut at, 1)? as usize,
                                    signed(&mut at, 1)? as i32,
                                    signed(&mut at, 1)? as i32,
                                )
                            }
                        };

                    let raster = bytes
                        .get(at..at + length)
                        .ok_or_else(|| format!("character {code}'s raster runs past the end"))?;
                    at += length;
                    let pixels = match dyn_f == 14 {
                        // §  : `dyn_f` of 14 means the pixels are not packed at
                        // all, just bits, row after row with no padding.
                        true => raw_bits(raster, width, height),
                        false => runs(raster, dyn_f, first_black, width, height)?,
                    };
                    pk.glyphs.insert(
                        code,
                        Glyph {
                            code,
                            tfm_width: tfm,
                            dx,
                            dy,
                            width,
                            height,
                            x_offset,
                            y_offset,
                            pixels,
                        },
                    );
                }
            }
        }
        Ok(pk)
    }

    pub fn glyph(&self, code: u32) -> Option<&Glyph> {
        self.glyphs.get(&code)
    }

    /// Every code the font holds, in order.
    pub fn codes(&self) -> Vec<u32> {
        self.glyphs.keys().copied().collect()
    }

    /// The resolution the font was made at, in dots per inch. A `.pk` is made
    /// for one printer at one size, which is why they are kept in directories
    /// named after the number.
    pub fn dpi(&self) -> f64 {
        // 72.27 points to the inch, which is TeX's inch and not PostScript's.
        self.h_pixels_per_point * 72.27
    }

    /// A summary a person reads.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        if !self.comment.is_empty() {
            out.push_str(&format!("comment       {}\n", self.comment));
        }
        out.push_str(&format!("designsize    {:.6}pt\n", self.design_size));
        out.push_str(&format!("checksum      0o{:o}\n", self.checksum));
        out.push_str(&format!("resolution    {:.0} dpi\n", self.dpi().round()));
        out.push_str(&format!("characters    {}\n", self.glyphs.len()));
        let black: usize = self
            .glyphs
            .values()
            .map(|g| g.pixels.iter().filter(|&&p| p == 1).count())
            .sum();
        out.push_str(&format!("black pixels  {black}\n"));
        for text in &self.specials {
            out.push_str(&format!("special       {text}\n"));
        }
        out
    }

    /// One character, drawn.
    pub fn describe(&self, code: u32) -> String {
        let Some(glyph) = self.glyph(code) else {
            return format!("{code}: the font does not hold it\n");
        };
        let shown = match char::from_u32(code).is_some_and(|c| c.is_ascii_graphic()) {
            true => format!("'{}'", code as u8 as char),
            false => format!("0o{code:o}"),
        };
        let mut out = format!(
            "{shown}  {}x{} pixels  offset {},{}  width {:.6}\n",
            glyph.width, glyph.height, glyph.x_offset, glyph.y_offset, glyph.tfm_width
        );
        for row in glyph.rows() {
            out.push_str(&format!("  {}\n", row.trim_end()));
        }
        out
    }
}

/// `dyn_f` of 14: the bitmap is bits, row after row, with no padding between
/// rows and none at the end of one.
fn raw_bits(raster: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut pixels = vec![0u8; width * height];
    for (i, pixel) in pixels.iter_mut().enumerate() {
        let byte = raster.get(i / 8).copied().unwrap_or(0);
        *pixel = (byte >> (7 - i % 8)) & 1;
    }
    pixels
}

/// A stream of nybbles, which is what a packed raster is.
struct Nybbles<'a> {
    bytes: &'a [u8],
    at: usize,
    high: bool,
}

impl Nybbles<'_> {
    fn next(&mut self) -> Result<i32, String> {
        let byte = self
            .bytes
            .get(self.at)
            .copied()
            .ok_or("the raster ended in the middle of a number")?;
        let value = match self.high {
            true => byte >> 4,
            false => {
                self.at += 1;
                byte & 0xf
            }
        };
        self.high = !self.high;
        Ok(value as i32)
    }

    /// One packed number, which is where the format earns its name: a value up
    /// to `dyn_f` is a single nybble, one up to `(13 - dyn_f) * 16 + dyn_f` is
    /// two, and anything larger begins with a run of zeros saying how many
    /// nybbles it takes.
    fn packed(&mut self, dyn_f: i32) -> Result<i64, String> {
        let first = self.next()?;
        self.packed_from(first, dyn_f)
    }

    fn packed_from(&mut self, first: i32, dyn_f: i32) -> Result<i64, String> {
        if first == 0 {
            let mut zeros = 0i32;
            let first_nonzero = loop {
                let nybble = self.next()?;
                zeros += 1;
                if nybble != 0 {
                    break nybble;
                }
                if zeros > 16 {
                    return Err("a packed number with no end".into());
                }
            };
            let mut value = first_nonzero as i64;
            for _ in 0..zeros {
                value = value * 16 + self.next()? as i64;
            }
            return Ok(value - 15 + (13 - dyn_f as i64) * 16 + dyn_f as i64);
        }
        if first <= dyn_f {
            return Ok(first as i64);
        }
        if first < 14 {
            let low = self.next()? as i64;
            return Ok((first as i64 - dyn_f as i64 - 1) * 16 + low + dyn_f as i64 + 1);
        }
        Err(format!("{first} is a repeat count, not a run length"))
    }
}

/// The packed raster: runs of black and white, with a row that repeats saying
/// so once.
///
/// The repeat count is the part that is easy to get wrong. It arrives in the
/// middle of a row -- a nybble of 15 means "repeat this row once", 14 means "a
/// count follows" -- and it applies to the row being built, when that row is
/// finished, not where it was read.
fn runs(
    raster: &[u8],
    dyn_f: i32,
    first_black: bool,
    width: usize,
    height: usize,
) -> Result<Vec<u8>, String> {
    let mut nybbles = Nybbles {
        bytes: raster,
        at: 0,
        high: true,
    };
    let mut pixels = vec![0u8; width * height];
    if width == 0 || height == 0 {
        return Ok(pixels);
    }

    let mut black = first_black;
    let mut repeat = 0usize;
    let mut row = vec![0u8; width];
    let (mut column, mut rows_done) = (0usize, 0usize);

    while rows_done < height {
        let first = nybbles.next()?;
        let mut run = match first {
            15 => {
                repeat = 1;
                continue;
            }
            14 => {
                repeat = nybbles.packed(dyn_f)? as usize;
                continue;
            }
            other => nybbles.packed_from(other, dyn_f)?,
        };

        while run > 0 && rows_done < height {
            let take = (run as usize).min(width - column);
            if black {
                for pixel in row[column..column + take].iter_mut() {
                    *pixel = 1;
                }
            }
            column += take;
            run -= take as i64;
            if column == width {
                // The row is finished, so it and its repeats go out together.
                for _ in 0..=repeat {
                    if rows_done >= height {
                        break;
                    }
                    pixels[rows_done * width..(rows_done + 1) * width].copy_from_slice(&row);
                    rows_done += 1;
                }
                repeat = 0;
                row = vec![0u8; width];
                column = 0;
            }
        }
        black = !black;
    }
    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed() -> Option<Vec<u8>> {
        let found = std::process::Command::new("kpsewhich")
            .arg("-format=pk")
            .arg("cmr10.600pk")
            .output()
            .ok()?;
        let path = String::from_utf8_lossy(&found.stdout).trim().to_string();
        std::fs::read(path).ok()
    }

    /// The font every driver drew for twenty years, at the resolution TeX Live
    /// still ships it at.
    #[test]
    fn a_packed_font_reads_back_as_pixels() {
        let Some(bytes) = installed() else { return };
        let pk = Pk::parse(&bytes).expect("cmr10 reads");

        assert!((pk.design_size - 10.0).abs() < 1e-9, "{}", pk.design_size);
        assert_eq!(pk.dpi().round(), 600.0, "{}", pk.dpi());
        assert_eq!(pk.codes().len(), 128, "a text font's 128 characters");

        // An A is taller than it is wide, sits on the baseline, and is about a
        // third black.
        let a = pk.glyph(b'A' as u32).expect("an A");
        assert!(a.height > a.width, "{}x{}", a.width, a.height);
        let black = a.pixels.iter().filter(|&&p| p == 1).count();
        assert!(
            black > a.pixels.len() / 8 && black < a.pixels.len() / 2,
            "{black} of {}",
            a.pixels.len()
        );
        // Its top row has ink and its bottom row has ink at both ends: an A is
        // a peak with two feet.
        assert!(a.rows()[0].contains('*'), "{:?}", a.rows()[0]);
        let feet = a.rows()[a.height - 1].clone();
        assert!(
            feet.starts_with('*') && feet.trim_end().ends_with('*'),
            "{feet:?}"
        );
        // The middle of the bottom row is white: that is the gap between them.
        assert!(!a.black(a.width / 2, a.height - 1), "{feet:?}");
    }

    /// What is not a packed font is refused rather than drawn.
    #[test]
    fn what_is_not_a_packed_font_is_refused() {
        assert!(Pk::parse(b"").is_err());
        assert!(Pk::parse(b"not a font").is_err());
        // The right first byte, the wrong id: a DVI file begins 247 and 2, a
        // virtual font 247 and 202.
        assert!(Pk::parse(&[PRE, 2, 0]).unwrap_err().contains("89"));
        assert!(Pk::parse(&[PRE, 202, 0]).unwrap_err().contains("89"));

        // A character whose raster runs past the end of the file.
        let mut truncated = vec![PRE, ID, 0];
        truncated.extend([0, 0x10, 0, 0]); // design size
        truncated.extend([0, 0, 0, 0]); // checksum
        truncated.extend([0, 0, 0, 0]); // hppp
        truncated.extend([0, 0, 0, 0]); // vppp
        truncated.extend([0x88, 40, b'A', 0, 0, 0, 5, 5, 5, 0, 0]);
        assert!(
            Pk::parse(&truncated).unwrap_err().contains("past the end"),
            "{:?}",
            Pk::parse(&truncated)
        );
    }
}
