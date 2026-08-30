//! Reading an OpenType or TrueType font, ported from `sfnt.c`, `tt_table.c`,
//! `tt_cmap.c` and `tt_post.c` in tectonic's `xdvipdfmx`.
//!
//! This is the modern half of the font problem. A `.tfm` says how wide a
//! character is and a `.pk` says what its pixels are, but a document set in an
//! OpenType font names the file itself, and everything a driver needs is
//! inside it: how many units to the em, how wide each glyph is, which glyph a
//! character maps to, and what the glyphs are called.
//!
//! The container is Apple's `sfnt`: a directory of tables, each with a
//! four-character tag, an offset and a length. The tables that matter here are
//! the ones `xdvipdfmx` reads before it can put a glyph on a page --
//! `head`, `hhea`, `maxp`, `hmtx`, `name`, `cmap` and `post`. The outlines
//! themselves (`glyf` or `CFF`) are not read: a driver embeds those, it does
//! not interpret them, and texrs has nothing to embed them into yet.

use std::collections::BTreeMap;
use std::path::Path;

/// One entry of the table directory.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub tag: String,
    pub checksum: u32,
    pub offset: usize,
    pub length: usize,
}

/// One string a font carries: which platform and language it is for, which of
/// the standard names it is, and the text.
#[derive(Debug, Clone, PartialEq)]
pub struct NameRecord {
    pub platform: u16,
    pub encoding: u16,
    pub language: u16,
    /// 1 family, 2 subfamily, 4 full name, 5 version, 6 PostScript name.
    pub id: u16,
    pub text: String,
}

/// `head`: what the whole font is measured in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Head {
    /// The em, in font units. Everything else in the font is in these.
    pub units_per_em: u16,
    pub x_min: i16,
    pub y_min: i16,
    pub x_max: i16,
    pub y_max: i16,
    /// Whether `loca` holds shorts or longs, which is how `glyf` is found.
    pub long_loca: bool,
}

/// `hhea`: how the line is laid out, and how much of `hmtx` is widths.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hhea {
    pub ascender: i16,
    pub descender: i16,
    pub line_gap: i16,
    pub number_of_h_metrics: u16,
}

/// A font.
///
/// The bytes are kept: a table is a range of them, and a driver reads several
/// tables of a font it has opened once.
pub struct Sfnt {
    bytes: Vec<u8>,
    /// The signature: `0x00010000` for TrueType outlines, `OTTO` for CFF ones.
    pub signature: u32,
    pub tables: Vec<Table>,
}

/// A big-endian read that says where it ran out rather than panicking.
fn number(bytes: &[u8], at: usize, width: usize) -> Result<u64, String> {
    bytes
        .get(at..at + width)
        .map(|region| {
            region
                .iter()
                .fold(0u64, |value, &b| (value << 8) | b as u64)
        })
        .ok_or_else(|| format!("byte {at} is past the end of the font"))
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16, String> {
    Ok(number(bytes, at, 2)? as u16)
}

fn i16_at(bytes: &[u8], at: usize) -> Result<i16, String> {
    Ok(number(bytes, at, 2)? as u16 as i16)
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32, String> {
    Ok(number(bytes, at, 4)? as u32)
}

fn tag_at(bytes: &[u8], at: usize) -> Result<String, String> {
    Ok(bytes
        .get(at..at + 4)
        .ok_or("a tag past the end of the font")?
        .iter()
        .map(|&b| b as char)
        .collect())
}

impl std::fmt::Debug for Sfnt {
    /// Without the bytes: a font is megabytes, and what a person wants to see
    /// is which tables it holds.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sfnt")
            .field("signature", &format_args!("0x{:08x}", self.signature))
            .field("bytes", &self.bytes.len())
            .field(
                "tables",
                &self
                    .tables
                    .iter()
                    .map(|t| t.tag.as_str())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl Sfnt {
    pub fn open(path: impl AsRef<Path>) -> Result<Sfnt, String> {
        let path = path.as_ref();
        let bytes =
            std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Sfnt::parse(bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Read the table directory.
    ///
    /// A font collection (`ttcf`) holds several fonts in one file with their
    /// tables shared; this reads the first, as a driver does when a document
    /// names the file rather than a face inside it.
    pub fn parse(bytes: Vec<u8>) -> Result<Sfnt, String> {
        let mut signature = u32_at(&bytes, 0)?;
        let mut directory = 0usize;
        if &tag_at(&bytes, 0)? == "ttcf" {
            // A collection: the offsets of its fonts follow the count.
            let count = u32_at(&bytes, 8)?;
            if count == 0 {
                return Err("a font collection holding no fonts".into());
            }
            directory = u32_at(&bytes, 12)? as usize;
            signature = u32_at(&bytes, directory)?;
        }
        match signature {
            // TrueType outlines, CFF outlines, and Apple's older signature.
            0x0001_0000 | 0x4f54_544f | 0x7472_7565 => {}
            other => {
                return Err(format!(
                    "0x{other:08x} is not a font signature (00010000, OTTO or true)"
                ))
            }
        }

        let count = u16_at(&bytes, directory + 4)? as usize;
        // A directory entry is sixteen bytes, and a font with thousands of
        // tables is a damaged file rather than an ambitious one.
        if count > 512 {
            return Err(format!("{count} tables is not a font"));
        }
        let mut tables = Vec::with_capacity(count);
        for i in 0..count {
            let at = directory + 12 + i * 16;
            tables.push(Table {
                tag: tag_at(&bytes, at)?,
                checksum: u32_at(&bytes, at + 4)?,
                offset: u32_at(&bytes, at + 8)? as usize,
                length: u32_at(&bytes, at + 12)? as usize,
            });
        }
        Ok(Sfnt {
            bytes,
            signature,
            tables,
        })
    }

    /// A table's bytes, or `None` when the font has no such table.
    pub fn table(&self, tag: &str) -> Option<&[u8]> {
        // A tag is four characters, so `CFF` is stored as `CFF `; either
        // spelling finds it.
        let entry = self.tables.iter().find(|t| t.tag.trim() == tag.trim())?;
        // A length that runs past the end is a damaged font, and the bytes
        // that are there are still worth reading.
        let end = (entry.offset + entry.length).min(self.bytes.len());
        self.bytes.get(entry.offset..end)
    }

    /// Whether the outlines are CFF (`OTTO`) rather than TrueType.
    pub fn is_cff(&self) -> bool {
        self.signature == 0x4f54_544f
    }

    pub fn head(&self) -> Result<Head, String> {
        let head = self.table("head").ok_or("the font has no head table")?;
        Ok(Head {
            units_per_em: u16_at(head, 18)?,
            x_min: i16_at(head, 36)?,
            y_min: i16_at(head, 38)?,
            x_max: i16_at(head, 40)?,
            y_max: i16_at(head, 42)?,
            long_loca: u16_at(head, 50)? == 1,
        })
    }

    pub fn hhea(&self) -> Result<Hhea, String> {
        let hhea = self.table("hhea").ok_or("the font has no hhea table")?;
        Ok(Hhea {
            ascender: i16_at(hhea, 4)?,
            descender: i16_at(hhea, 6)?,
            line_gap: i16_at(hhea, 8)?,
            number_of_h_metrics: u16_at(hhea, 34)?,
        })
    }

    /// How many glyphs the font holds.
    pub fn num_glyphs(&self) -> Result<u16, String> {
        let maxp = self.table("maxp").ok_or("the font has no maxp table")?;
        u16_at(maxp, 4)
    }

    /// Every glyph's advance width, in font units.
    ///
    /// `hmtx` stops repeating itself once the widths do: after
    /// `number_of_h_metrics` entries the last width holds for every glyph that
    /// follows, which is how a monospaced font stores one width for thousands
    /// of glyphs.
    pub fn advance_widths(&self) -> Result<Vec<u16>, String> {
        let hmtx = self.table("hmtx").ok_or("the font has no hmtx table")?;
        let metrics = self.hhea()?.number_of_h_metrics as usize;
        let glyphs = self.num_glyphs()? as usize;
        if metrics == 0 {
            return Err("hhea says the font has no widths".into());
        }
        let mut out = Vec::with_capacity(glyphs);
        let mut last = 0u16;
        for glyph in 0..glyphs {
            if glyph < metrics {
                last = u16_at(hmtx, glyph * 4)?;
            }
            out.push(last);
        }
        Ok(out)
    }

    /// The font's CFF outlines, when it has them.
    pub fn cff(&self) -> Option<crate::cff::Cff> {
        crate::cff::Cff::parse(self.table("CFF")?).ok()
    }

    /// The `name` table, as it is stored.
    pub fn names(&self) -> Result<Vec<NameRecord>, String> {
        let name = self.table("name").ok_or("the font has no name table")?;
        let count = u16_at(name, 2)? as usize;
        let strings = u16_at(name, 4)? as usize;
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let at = 6 + i * 12;
            let platform = u16_at(name, at)?;
            let encoding = u16_at(name, at + 2)?;
            let language = u16_at(name, at + 4)?;
            let id = u16_at(name, at + 6)?;
            let length = u16_at(name, at + 8)? as usize;
            let offset = u16_at(name, at + 10)? as usize;
            let Some(raw) = name.get(strings + offset..strings + offset + length) else {
                continue;
            };
            // Windows and Unicode strings are UTF-16 big-endian; Macintosh
            // ones are a single byte each, and are ASCII in practice.
            let text = match platform == 1 {
                true => raw.iter().map(|&b| b as char).collect(),
                false => {
                    // Two bytes to a unit, big-endian, as everything in an
                    // sfnt is.
                    let units: Vec<u16> = raw
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|pair| u16::from_be_bytes(*pair))
                        .collect();
                    String::from_utf16_lossy(&units)
                }
            };
            out.push(NameRecord {
                platform,
                encoding,
                language,
                id,
                text,
            });
        }
        Ok(out)
    }

    /// One name, preferring the Windows entry a driver reads first.
    ///
    /// The ids are the ones every font carries: 1 family, 2 subfamily, 4 full
    /// name, 5 version, 6 PostScript name.
    pub fn name(&self, id: u16) -> Option<String> {
        let names = self.names().ok()?;
        names
            .iter()
            .find(|record| record.platform == 3 && record.id == id)
            .or_else(|| names.iter().find(|record| record.id == id))
            .map(|record| record.text.clone())
    }

    /// What character maps to what glyph.
    ///
    /// A font carries several `cmap` subtables for different platforms; this
    /// reads the one a driver would, preferring the Unicode ones that can say
    /// more than 65535 characters.
    pub fn cmap(&self) -> Result<BTreeMap<u32, u16>, String> {
        let cmap = self.table("cmap").ok_or("the font has no cmap table")?;
        let count = u16_at(cmap, 2)? as usize;
        let mut best: Option<(u8, usize)> = None;
        for i in 0..count {
            let at = 4 + i * 8;
            let platform = u16_at(cmap, at)?;
            let encoding = u16_at(cmap, at + 2)?;
            let offset = u32_at(cmap, at + 4)? as usize;
            // Windows full Unicode, then Windows basic Unicode, then Unicode,
            // then Macintosh -- the order xdvipdfmx tries them in.
            let rank = match (platform, encoding) {
                (3, 10) => 4,
                (3, 1) => 3,
                (0, _) => 2,
                (1, 0) => 1,
                _ => 0,
            };
            if rank > 0 && best.is_none_or(|(best_rank, _)| rank > best_rank) {
                best = Some((rank, offset));
            }
        }
        let (_, offset) = best.ok_or("the font's cmap holds no subtable this can read")?;
        let sub = cmap
            .get(offset..)
            .ok_or("a cmap subtable past the end of the table")?;
        let mut out = BTreeMap::new();
        match u16_at(sub, 0)? {
            // A byte to a glyph, which is what an old Macintosh font used.
            0 => {
                for code in 0..256usize {
                    let glyph = *sub.get(6 + code).ok_or("a short format 0 subtable")? as u16;
                    if glyph != 0 {
                        out.insert(code as u32, glyph);
                    }
                }
            }
            // Segments, with a delta or an array per segment. This is the one
            // nearly every font uses, and the one with the pointer arithmetic
            // worth getting right.
            4 => {
                let segments = u16_at(sub, 6)? as usize / 2;
                let ends = 14;
                let starts = ends + segments * 2 + 2;
                let deltas = starts + segments * 2;
                let ranges = deltas + segments * 2;
                for segment in 0..segments {
                    let end = u16_at(sub, ends + segment * 2)?;
                    let start = u16_at(sub, starts + segment * 2)?;
                    let delta = u16_at(sub, deltas + segment * 2)?;
                    let range = u16_at(sub, ranges + segment * 2)? as usize;
                    if start > end {
                        continue;
                    }
                    for code in start..=end {
                        if code == 0xffff {
                            continue;
                        }
                        let glyph = match range == 0 {
                            true => code.wrapping_add(delta),
                            false => {
                                // The offset is from the range entry itself,
                                // which is what makes this table compact and
                                // its readers wrong.
                                let at = ranges + segment * 2 + range + (code - start) as usize * 2;
                                match u16_at(sub, at)? {
                                    0 => 0,
                                    glyph => glyph.wrapping_add(delta),
                                }
                            }
                        };
                        if glyph != 0 {
                            out.insert(code as u32, glyph);
                        }
                    }
                }
            }
            // A run of codes from a first one.
            6 => {
                let first = u16_at(sub, 6)? as u32;
                let count = u16_at(sub, 8)? as u32;
                for i in 0..count {
                    let glyph = u16_at(sub, 10 + i as usize * 2)?;
                    if glyph != 0 {
                        out.insert(first + i, glyph);
                    }
                }
            }
            // Groups, for the characters above 65535.
            12 => {
                let groups = u32_at(sub, 12)? as usize;
                for group in 0..groups {
                    let at = 16 + group * 12;
                    let start = u32_at(sub, at)?;
                    let end = u32_at(sub, at + 4)?;
                    let glyph = u32_at(sub, at + 8)?;
                    if start > end || end - start > 0x10_ffff {
                        continue;
                    }
                    for code in start..=end {
                        let glyph = glyph + (code - start);
                        if glyph != 0 && glyph <= u16::MAX as u32 {
                            out.insert(code, glyph as u16);
                        }
                    }
                }
            }
            other => return Err(format!("cmap format {other} is not one this reads")),
        }
        Ok(out)
    }

    /// What the glyphs are called.
    ///
    /// A TrueType font carries its names in `post`; a CFF font carries them in
    /// the CFF charset, which [`crate::cff`] reads. Either way this answers,
    /// because a driver asking what a glyph is called does not care which kind
    /// of font it has.
    pub fn glyph_names(&self) -> Result<Vec<String>, String> {
        // The CFF is asked first, because a CFF font's `post` is version 3.0
        // and carries no names at all.
        if let Some(table) = self.table("CFF") {
            return Ok(crate::cff::Cff::parse(table)?.glyph_names);
        }
        let post = self.table("post").ok_or("the font has no post table")?;
        let version = u32_at(post, 0)?;
        if version != 0x0002_0000 {
            return Err(format!(
                "post version {}.{} carries no names",
                version >> 16,
                version & 0xffff
            ));
        }
        let count = u16_at(post, 32)? as usize;
        let mut indices = Vec::with_capacity(count);
        for glyph in 0..count {
            indices.push(u16_at(post, 34 + glyph * 2)? as usize);
        }
        // The names that are not standard follow, as Pascal strings.
        let mut extra: Vec<String> = Vec::new();
        let mut at = 34 + count * 2;
        while at < post.len() {
            let length = post[at] as usize;
            at += 1;
            let Some(raw) = post.get(at..at + length) else {
                break;
            };
            extra.push(raw.iter().map(|&b| b as char).collect());
            at += length;
        }
        Ok(indices
            .iter()
            .map(|&index| match index < MAC_GLYPH_NAMES.len() {
                true => MAC_GLYPH_NAMES[index].to_string(),
                false => extra
                    .get(index - MAC_GLYPH_NAMES.len())
                    .cloned()
                    .unwrap_or_else(|| format!("glyph{index}")),
            })
            .collect())
    }

    /// A summary a person reads.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        for (label, id) in [
            ("family", 1u16),
            ("subfamily", 2),
            ("full name", 4),
            ("version", 5),
            ("postscript", 6),
        ] {
            if let Some(text) = self.name(id) {
                out.push_str(&format!("{label:<13} {}\n", text.trim()));
            }
        }
        out.push_str(&format!(
            "outlines      {}\n",
            match self.is_cff() {
                true => "CFF",
                false => "TrueType",
            }
        ));
        if let Ok(head) = self.head() {
            out.push_str(&format!("units per em  {}\n", head.units_per_em));
            out.push_str(&format!(
                "bounding box  {} {} {} {}\n",
                head.x_min, head.y_min, head.x_max, head.y_max
            ));
        }
        if let Ok(hhea) = self.hhea() {
            out.push_str(&format!(
                "ascender      {}  descender {}  line gap {}\n",
                hhea.ascender, hhea.descender, hhea.line_gap
            ));
        }
        if let Ok(glyphs) = self.num_glyphs() {
            out.push_str(&format!("glyphs        {glyphs}\n"));
        }
        if let Ok(cmap) = self.cmap() {
            out.push_str(&format!("characters    {}\n", cmap.len()));
        }
        out.push_str(&format!("tables        {}\n", self.tables.len()));
        for table in &self.tables {
            out.push_str(&format!("  {:<5} {:>8} bytes\n", table.tag, table.length));
        }
        out
    }

    /// What one character becomes: the glyph it maps to, its number, its name
    /// where the font says one, and how wide it is.
    pub fn describe(&self, code: u32) -> String {
        let Ok(cmap) = self.cmap() else {
            return "the font has no cmap this reads\n".to_string();
        };
        let shown = match char::from_u32(code) {
            Some(c) if c.is_ascii_graphic() => format!("'{c}'"),
            _ => format!("U+{code:04X}"),
        };
        let Some(&glyph) = cmap.get(&code) else {
            return format!("{shown}: the font has no glyph for it\n");
        };
        let name = self
            .glyph_names()
            .ok()
            .and_then(|names| names.get(glyph as usize).cloned());
        let width = self
            .advance_widths()
            .ok()
            .and_then(|widths| widths.get(glyph as usize).copied());
        let units = self.head().map(|h| h.units_per_em).unwrap_or(1000);
        let mut out = format!("{shown}  glyph {glyph}");
        if let Some(name) = name {
            out.push_str(&format!(" ({name})"));
        }
        if let Some(width) = width {
            out.push_str(&format!(
                "  width {width} of {units} ({:.4} em)",
                width as f64 / units as f64
            ));
        }
        out.push('\n');
        out
    }
}

/// The 258 names a `post` table numbers rather than spells, from `tt_post.c`.
/// A font that uses only these carries no strings at all.
pub const MAC_GLYPH_NAMES: [&str; 258] = [
    ".notdef",
    ".null",
    "nonmarkingreturn",
    "space",
    "exclam",
    "quotedbl",
    "numbersign",
    "dollar",
    "percent",
    "ampersand",
    "quotesingle",
    "parenleft",
    "parenright",
    "asterisk",
    "plus",
    "comma",
    "hyphen",
    "period",
    "slash",
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "colon",
    "semicolon",
    "less",
    "equal",
    "greater",
    "question",
    "at",
    "A",
    "B",
    "C",
    "D",
    "E",
    "F",
    "G",
    "H",
    "I",
    "J",
    "K",
    "L",
    "M",
    "N",
    "O",
    "P",
    "Q",
    "R",
    "S",
    "T",
    "U",
    "V",
    "W",
    "X",
    "Y",
    "Z",
    "bracketleft",
    "backslash",
    "bracketright",
    "asciicircum",
    "underscore",
    "grave",
    "a",
    "b",
    "c",
    "d",
    "e",
    "f",
    "g",
    "h",
    "i",
    "j",
    "k",
    "l",
    "m",
    "n",
    "o",
    "p",
    "q",
    "r",
    "s",
    "t",
    "u",
    "v",
    "w",
    "x",
    "y",
    "z",
    "braceleft",
    "bar",
    "braceright",
    "asciitilde",
    "Adieresis",
    "Aring",
    "Ccedilla",
    "Eacute",
    "Ntilde",
    "Odieresis",
    "Udieresis",
    "aacute",
    "agrave",
    "acircumflex",
    "adieresis",
    "atilde",
    "aring",
    "ccedilla",
    "eacute",
    "egrave",
    "ecircumflex",
    "edieresis",
    "iacute",
    "igrave",
    "icircumflex",
    "idieresis",
    "ntilde",
    "oacute",
    "ograve",
    "ocircumflex",
    "odieresis",
    "otilde",
    "uacute",
    "ugrave",
    "ucircumflex",
    "udieresis",
    "dagger",
    "degree",
    "cent",
    "sterling",
    "section",
    "bullet",
    "paragraph",
    "germandbls",
    "registered",
    "copyright",
    "trademark",
    "acute",
    "dieresis",
    "notequal",
    "AE",
    "Oslash",
    "infinity",
    "plusminus",
    "lessequal",
    "greaterequal",
    "yen",
    "mu",
    "partialdiff",
    "summation",
    "product",
    "pi",
    "integral",
    "ordfeminine",
    "ordmasculine",
    "Omega",
    "ae",
    "oslash",
    "questiondown",
    "exclamdown",
    "logicalnot",
    "radical",
    "florin",
    "approxequal",
    "Delta",
    "guillemotleft",
    "guillemotright",
    "ellipsis",
    "nonbreakingspace",
    "Agrave",
    "Atilde",
    "Otilde",
    "OE",
    "oe",
    "endash",
    "emdash",
    "quotedblleft",
    "quotedblright",
    "quoteleft",
    "quoteright",
    "divide",
    "lozenge",
    "ydieresis",
    "Ydieresis",
    "fraction",
    "currency",
    "guilsinglleft",
    "guilsinglright",
    "fi",
    "fl",
    "daggerdbl",
    "periodcentered",
    "quotesinglbase",
    "quotedblbase",
    "perthousand",
    "Acircumflex",
    "Ecircumflex",
    "Aacute",
    "Edieresis",
    "Egrave",
    "Iacute",
    "Icircumflex",
    "Idieresis",
    "Igrave",
    "Oacute",
    "Ocircumflex",
    "apple",
    "Ograve",
    "Uacute",
    "Ucircumflex",
    "Ugrave",
    "dotlessi",
    "circumflex",
    "tilde",
    "macron",
    "breve",
    "dotaccent",
    "ring",
    "cedilla",
    "hungarumlaut",
    "ogonek",
    "caron",
    "Lslash",
    "lslash",
    "Scaron",
    "scaron",
    "Zcaron",
    "zcaron",
    "brokenbar",
    "Eth",
    "eth",
    "Yacute",
    "yacute",
    "Thorn",
    "thorn",
    "minus",
    "multiply",
    "onesuperior",
    "twosuperior",
    "threesuperior",
    "onehalf",
    "onequarter",
    "threequarters",
    "franc",
    "Gbreve",
    "gbreve",
    "Idotaccent",
    "Scedilla",
    "scedilla",
    "Cacute",
    "cacute",
    "Ccaron",
    "ccaron",
    "dcroat",
];

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

    /// Latin Modern, which is Computer Modern as an OpenType font -- the one a
    /// document that says `\usepackage{lmodern}` and runs on a modern engine
    /// is set in.
    #[test]
    fn an_opentype_font_says_what_it_holds() {
        let Some(bytes) = installed("lmroman10-regular.otf") else {
            return;
        };
        let font = Sfnt::parse(bytes).expect("lmroman reads");

        assert!(font.is_cff(), "Latin Modern carries CFF outlines");
        assert_eq!(font.name(1).as_deref(), Some("LM Roman 10"));
        assert_eq!(font.name(6).as_deref(), Some("LMRoman10-Regular"));

        let head = font.head().expect("a head table");
        assert_eq!(head.units_per_em, 1000, "a CFF font counts in thousandths");
        assert!(head.x_max > head.x_min && head.y_max > head.y_min);

        // The characters a Latin font must have map to glyphs, and the widths
        // are the ones the metrics say.
        let cmap = font.cmap().expect("a cmap");
        let widths = font.advance_widths().expect("widths");
        for c in ['A', 'z', '0', ' ', '\u{e9}'] {
            let glyph = *cmap
                .get(&(c as u32))
                .unwrap_or_else(|| panic!("no glyph for {c:?}"));
            assert!(
                (glyph as usize) < widths.len(),
                "{c:?} maps past the end of hmtx"
            );
        }
        // An A in Computer Modern is 750 thousandths of an em, as its .tfm
        // says.
        let a = widths[cmap[&(b'A' as u32)] as usize];
        assert_eq!(a, 750, "an A is 0.75em");
        // A space is narrower than an m.
        assert!(widths[cmap[&(b' ' as u32)] as usize] < widths[cmap[&(b'm' as u32)] as usize]);
    }

    /// A TrueType font, which carries its glyph names where an OpenType one
    /// does not.
    #[test]
    fn a_truetype_font_carries_its_glyph_names() {
        let path = "/usr/local/texlive/2026/texmf-dist/fonts/truetype/intel/clearsans/ClearSans-Regular.ttf";
        let Ok(bytes) = std::fs::read(path) else {
            return;
        };
        let font = Sfnt::parse(bytes).expect("reads");
        assert!(!font.is_cff());

        let names = font.glyph_names().expect("post 2.0 names");
        assert_eq!(names.len(), font.num_glyphs().expect("maxp") as usize);
        assert_eq!(names[0], ".notdef", "glyph zero is always the empty one");

        // The name of the glyph an A maps to is the name an A has.
        let cmap = font.cmap().expect("a cmap");
        let a = cmap[&(b'A' as u32)] as usize;
        assert_eq!(names[a], "A");
    }

    /// What is not a font is refused rather than read as one.
    #[test]
    fn what_is_not_a_font_is_refused() {
        assert!(Sfnt::parse(Vec::new()).is_err());
        assert!(Sfnt::parse(b"not a font at all".to_vec()).is_err());
        // A .pk begins with 247, which is not a signature.
        let e = Sfnt::parse(vec![247, 89, 0, 0, 0, 0]).unwrap_err();
        assert!(e.contains("not a font signature"), "{e}");
        // The right signature and a directory that promises more tables than
        // the file holds.
        let mut lying = vec![0, 1, 0, 0, 0x01, 0xff];
        lying.extend([0; 10]);
        assert!(Sfnt::parse(lying).is_err());
    }
}
