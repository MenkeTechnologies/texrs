//! Reading DVI, ported from tectonic's `xdv`.
//!
//! texrs has no stomach and writes no DVI: it stops where the boxes would
//! begin. So why read it? Because the parity contract stops there too. Today
//! the harness compares `\message` streams, which is everything the mouth and
//! the expander produce and nothing the rest of TeX does; the moment texrs sets
//! a character, the reference to compare against is what real tex shipped, and
//! that is a DVI file. This is the reading half of that comparison, written
//! now, while the format can be read against a document whose output is known.
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
                    Op::BeginPage { counts }
                }
                140 => Op::EndPage,
                141 => Op::Push,
                142 => Op::Pop,
                // right1..4, w0, w1..4, x0, x1..4 — all horizontal.
                143..=146 => Op::Right(read_signed(bytes, &mut at, (opcode - 142) as usize)?),
                147 => Op::Right(0),
                148..=151 => Op::Right(read_signed(bytes, &mut at, (opcode - 147) as usize)?),
                152 => Op::Right(0),
                153..=156 => Op::Right(read_signed(bytes, &mut at, (opcode - 152) as usize)?),
                // down1..4, y0, y1..4, z0, z1..4 — all vertical.
                157..=160 => Op::Down(read_signed(bytes, &mut at, (opcode - 156) as usize)?),
                161 => Op::Down(0),
                162..=165 => Op::Down(read_signed(bytes, &mut at, (opcode - 161) as usize)?),
                166 => Op::Down(0),
                167..=170 => Op::Down(read_signed(bytes, &mut at, (opcode - 166) as usize)?),
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
