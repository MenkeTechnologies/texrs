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
        let mut total = 0.0;
        let mut i = 0;
        while i < codes.len() {
            let code = codes[i];
            // A ligature replaces both characters with one, so its width is
            // the replacement's, not the pair's.
            if let Some(&next) = codes.get(i + 1) {
                match self.step(code, next) {
                    // Only the plain ligature (op 0) replaces both; the others
                    // keep one or both characters, so they are left to the
                    // stomach rather than guessed at here.
                    Some(Step::Ligature { with, op: 0, .. }) => {
                        total += self.char(with).map(|c| c.width).unwrap_or(0.0);
                        i += 2;
                        continue;
                    }
                    Some(Step::Kern { by, .. }) => total += by,
                    _ => {}
                }
            }
            total += self.char(code).map(|c| c.width).unwrap_or(0.0);
            i += 1;
        }
        total
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
}
