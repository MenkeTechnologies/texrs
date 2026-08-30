//! Reading the `CFF ` table, ported from `cff.c` and `cff_dict.c` in tectonic's
//! `xdvipdfmx`.
//!
//! [`crate::sfnt`] reads an OpenType font's tables and stops at the outlines.
//! For a font with TrueType outlines that is nearly everything a driver needs;
//! for one with CFF outlines it leaves a hole, because a CFF font keeps its
//! glyph *names* inside the CFF rather than in a `post` table. Latin Modern is
//! a CFF font, and so is nearly every OpenType font a TeX document is set in,
//! so the hole is where the names of the glyphs a document uses would be.
//!
//! CFF is a compact format in the literal sense: everything that can be shared
//! is. Strings are numbered, and the first 391 numbers are a table every font
//! has rather than anything the font stores. Values are packed by size, in a
//! scheme close to but not the same as Type 1's. Arrays are `INDEX`es: a count,
//! a width, a list of offsets, and the data, so nothing needs a terminator.
//! Dictionaries put their operands before their operators, so a `DICT` is read
//! like a stack machine rather than like a table.
//!
//! What is read here is what names a glyph and what says how wide it is: the
//! header, the four top-level `INDEX`es, the Top and Private `DICT`s, the
//! charset in all three of its forms, and the width at the front of a Type 2
//! charstring. The outlines are not interpreted -- a driver embeds them.

use std::collections::BTreeMap;

/// A CFF font.
#[derive(Debug, Clone, Default)]
pub struct Cff {
    /// What the font calls itself, from the Name INDEX.
    pub name: String,
    /// One name per glyph, in glyph order.
    pub glyph_names: Vec<String>,
    /// Every glyph's advance width, in the units the font is drawn in.
    pub widths: Vec<f64>,
    /// Whether this is a CID-keyed font, whose "names" are numbers.
    pub is_cid: bool,
    /// Each glyph's charstring, as it was. A driver embeds these; nothing here
    /// draws them.
    pub charstrings: Vec<Vec<u8>>,
}

/// One `INDEX`: a count, then offsets, then the data they cut up.
#[derive(Debug, Default)]
struct Index {
    items: Vec<(usize, usize)>,
    /// Where the INDEX ends, which is where the next one begins.
    end: usize,
}

fn number(bytes: &[u8], at: usize, width: usize) -> Result<usize, String> {
    bytes
        .get(at..at + width)
        .map(|region| {
            region
                .iter()
                .fold(0usize, |value, &b| (value << 8) | b as usize)
        })
        .ok_or_else(|| format!("byte {at} is past the end of the CFF"))
}

/// Read the INDEX beginning at `at`.
fn index(bytes: &[u8], at: usize) -> Result<Index, String> {
    let count = number(bytes, at, 2)?;
    if count == 0 {
        // §5: an empty INDEX is two bytes and nothing else.
        return Ok(Index {
            items: Vec::new(),
            end: at + 2,
        });
    }
    let width = number(bytes, at + 2, 1)?;
    if !(1..=4).contains(&width) {
        return Err(format!("{width} is not an offset size"));
    }
    let offsets = at + 3;
    let data = offsets + (count + 1) * width - 1;
    let mut items = Vec::with_capacity(count);
    for i in 0..count {
        let from = data + number(bytes, offsets + i * width, width)?;
        let to = data + number(bytes, offsets + (i + 1) * width, width)?;
        if to < from || to > bytes.len() {
            return Err(format!("an INDEX entry runs from {from} to {to}"));
        }
        items.push((from, to));
    }
    let end = items.last().map(|&(_, to)| to).unwrap_or(data);
    Ok(Index { items, end })
}

/// A `DICT`: operands, then the operator they belong to.
///
/// §4: an operator is one byte, or `12` and another. Operands are integers in
/// four sizes and a fixed-point real written as nybbles, which is the one part
/// that looks like nothing else.
fn dict(bytes: &[u8]) -> Result<BTreeMap<u16, Vec<f64>>, String> {
    let mut out = BTreeMap::new();
    let mut operands: Vec<f64> = Vec::new();
    let mut at = 0usize;
    while at < bytes.len() {
        let b0 = bytes[at];
        at += 1;
        match b0 {
            // An operator, which takes everything on the stack.
            0..=21 => {
                let operator = match b0 == 12 {
                    true => {
                        let b1 = *bytes.get(at).ok_or("a two-byte operator cut short")?;
                        at += 1;
                        0x0c00 | b1 as u16
                    }
                    false => b0 as u16,
                };
                out.insert(operator, std::mem::take(&mut operands));
            }
            28 => {
                operands.push(number(bytes, at, 2)? as i16 as f64);
                at += 2;
            }
            29 => {
                operands.push(number(bytes, at, 4)? as i32 as f64);
                at += 4;
            }
            // A real, as nybbles: digits, and 10 to 15 for `.`, `E`, `E-`,
            // nothing, `-` and the end.
            30 => {
                let mut text = String::new();
                'digits: while at < bytes.len() {
                    let byte = bytes[at];
                    at += 1;
                    for nybble in [byte >> 4, byte & 0xf] {
                        match nybble {
                            0..=9 => text.push((b'0' + nybble) as char),
                            0xa => text.push('.'),
                            0xb => text.push('E'),
                            0xc => text.push_str("E-"),
                            0xe => text.push('-'),
                            0xf => break 'digits,
                            _ => {}
                        }
                    }
                }
                operands.push(text.parse().unwrap_or(0.0));
            }
            32..=246 => operands.push(b0 as f64 - 139.0),
            247..=250 => {
                let b1 = *bytes.get(at).ok_or("a two-byte operand cut short")?;
                at += 1;
                operands.push((b0 as f64 - 247.0) * 256.0 + b1 as f64 + 108.0);
            }
            251..=254 => {
                let b1 = *bytes.get(at).ok_or("a two-byte operand cut short")?;
                at += 1;
                operands.push(-(b0 as f64 - 251.0) * 256.0 - b1 as f64 - 108.0);
            }
            other => return Err(format!("{other} is not a DICT byte")),
        }
    }
    Ok(out)
}

/// The operators this reads, by the numbers §9 gives them.
const CHARSET: u16 = 15;
const CHAR_STRINGS: u16 = 17;
const PRIVATE: u16 = 18;
const ROS: u16 = 0x0c1e;
const DEFAULT_WIDTH_X: u16 = 20;
const NOMINAL_WIDTH_X: u16 = 21;
const SUBRS: u16 = 19;

impl Cff {
    /// Read a `CFF ` table.
    pub fn parse(bytes: &[u8]) -> Result<Cff, String> {
        // §6: the header says how long it is, so a later version can add to it.
        let header = *bytes.get(2).ok_or("shorter than a CFF header")? as usize;
        if header < 4 {
            return Err(format!("{header} is not a header size"));
        }
        let names = index(bytes, header)?;
        let tops = index(bytes, names.end)?;
        let strings = index(bytes, tops.end)?;
        // The fourth INDEX is the global subroutines, which a charstring calls
        // by a number biased by how many there are.
        let gsubrs = index(bytes, strings.end)?;

        let name = names
            .items
            .first()
            .map(|&(from, to)| bytes[from..to].iter().map(|&b| b as char).collect())
            .unwrap_or_default();
        let &(from, to) = tops.items.first().ok_or("the font has no Top DICT")?;
        let top = dict(&bytes[from..to])?;

        let charstrings_at = top
            .get(&CHAR_STRINGS)
            .and_then(|values| values.first())
            .copied()
            .ok_or("the Top DICT names no CharStrings")? as usize;
        let charstrings = index(bytes, charstrings_at)?;
        let glyphs = charstrings.items.len();

        // A string above 390 is stored in the font, and its number is its
        // place in the String INDEX past the standard ones.
        let string = |sid: usize| -> String {
            match sid < STANDARD_STRINGS.len() {
                true => STANDARD_STRINGS[sid].to_string(),
                false => strings
                    .items
                    .get(sid - STANDARD_STRINGS.len())
                    .map(|&(from, to)| bytes[from..to].iter().map(|&b| b as char).collect())
                    .unwrap_or_else(|| format!("sid{sid}")),
            }
        };

        let is_cid = top.contains_key(&ROS);
        let charset_at = top
            .get(&CHARSET)
            .and_then(|values| values.first())
            .copied()
            .unwrap_or(0.0) as usize;
        let glyph_names = read_charset(bytes, charset_at, glyphs, &string)?;

        // The Private DICT holds the two numbers a Type 2 charstring's width is
        // expressed against, and the local subroutines.
        let (mut default_width, mut nominal_width) = (0.0, 0.0);
        let mut subrs = Index::default();
        if let Some(private) = top.get(&PRIVATE) {
            if let [size, offset] = private[..] {
                let (from, to) = (offset as usize, offset as usize + size as usize);
                if to <= bytes.len() {
                    let private = dict(&bytes[from..to])?;
                    default_width = private
                        .get(&DEFAULT_WIDTH_X)
                        .and_then(|v| v.first())
                        .copied()
                        .unwrap_or(0.0);
                    nominal_width = private
                        .get(&NOMINAL_WIDTH_X)
                        .and_then(|v| v.first())
                        .copied()
                        .unwrap_or(0.0);
                    // §15: the local subroutines' offset is from the start of
                    // the Private DICT, not of the font.
                    if let Some(local) = private.get(&SUBRS).and_then(|v| v.first()) {
                        subrs = index(bytes, from + *local as usize)?;
                    }
                }
            }
        }

        let reader = Reader {
            bytes,
            gsubrs: &gsubrs,
            subrs: &subrs,
            default_width,
            nominal_width,
        };
        let widths = charstrings
            .items
            .iter()
            .map(|&(from, to)| reader.width_of(&bytes[from..to]))
            .collect();

        Ok(Cff {
            name,
            glyph_names,
            widths,
            is_cid,
            charstrings: charstrings
                .items
                .iter()
                .map(|&(from, to)| bytes[from..to].to_vec())
                .collect(),
        })
    }

    pub fn len(&self) -> usize {
        self.glyph_names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.glyph_names.is_empty()
    }
}

/// The charset: which name belongs to which glyph.
///
/// §13: glyph zero is always `.notdef` and is not in the table. Format 0 is a
/// name per glyph; formats 1 and 2 are runs, a first name and how many follow
/// it, which is what makes an alphabet cost four bytes.
fn read_charset(
    bytes: &[u8],
    at: usize,
    glyphs: usize,
    string: &dyn Fn(usize) -> String,
) -> Result<Vec<String>, String> {
    // The three predefined charsets, of which only ISOAdobe is common: the
    // glyphs are the standard strings in order.
    if at <= 2 {
        return Ok((0..glyphs).map(string).collect());
    }
    let format = *bytes.get(at).ok_or("a charset past the end of the CFF")?;
    let mut names = vec![".notdef".to_string()];
    let mut cursor = at + 1;
    match format {
        0 => {
            while names.len() < glyphs {
                names.push(string(number(bytes, cursor, 2)?));
                cursor += 2;
            }
        }
        1 | 2 => {
            let extra = match format == 1 {
                true => 1,
                false => 2,
            };
            while names.len() < glyphs {
                let first = number(bytes, cursor, 2)?;
                let left = number(bytes, cursor + 2, extra)?;
                cursor += 2 + extra;
                for i in 0..=left {
                    if names.len() >= glyphs {
                        break;
                    }
                    names.push(string(first + i));
                }
            }
        }
        other => return Err(format!("charset format {other} is not one this reads")),
    }
    Ok(names)
}

/// Enough of a Type 2 interpreter to find a glyph's width.
///
/// §3.1: a glyph whose width is the font's default says nothing at all; one
/// whose width differs puts a single extra operand before the first
/// stack-clearing operator, expressed as a difference from `nominalWidthX`. So
/// the width is found by counting operands, and a charstring that begins by
/// calling a subroutine has to be followed into it -- which most of Latin
/// Modern's glyphs do, because the shared parts of a letter are what a
/// subroutine is for. A reader that stopped at the first `callsubr` would find
/// a width for the handful of glyphs that have none and miss every other.
struct Reader<'a> {
    bytes: &'a [u8],
    gsubrs: &'a Index,
    subrs: &'a Index,
    default_width: f64,
    nominal_width: f64,
}

/// §4.7: a subroutine number is biased by how many there are, so the common
/// ones can be reached with a one-byte operand.
fn bias(count: usize) -> i64 {
    match count {
        0..=1239 => 107,
        1240..=33899 => 1131,
        _ => 32768,
    }
}

impl Reader<'_> {
    fn width_of(&self, charstring: &[u8]) -> f64 {
        let mut stack: Vec<f64> = Vec::new();
        self.scan(charstring, &mut stack, 0)
            .unwrap_or(self.default_width)
    }

    /// Walk until something says whether there is a width. `None` means the
    /// charstring ended without saying, which is a glyph of the default width.
    fn scan(&self, charstring: &[u8], stack: &mut Vec<f64>, depth: usize) -> Option<f64> {
        if depth > 10 {
            return None;
        }
        let mut at = 0usize;
        while at < charstring.len() {
            let b0 = charstring[at];
            at += 1;
            match b0 {
                32..=246 => stack.push(b0 as f64 - 139.0),
                247..=250 => {
                    let &b1 = charstring.get(at)?;
                    at += 1;
                    stack.push((b0 as f64 - 247.0) * 256.0 + b1 as f64 + 108.0);
                }
                251..=254 => {
                    let &b1 = charstring.get(at)?;
                    at += 1;
                    stack.push(-(b0 as f64 - 251.0) * 256.0 - b1 as f64 - 108.0);
                }
                28 => {
                    stack.push(number(charstring, at, 2).ok()? as i16 as f64);
                    at += 2;
                }
                255 => {
                    // A 255 here is 16.16 fixed point, where in a Type 1
                    // charstring it was a plain integer.
                    stack.push(number(charstring, at, 4).ok()? as i32 as f64 / 65536.0);
                    at += 4;
                }
                // callsubr and callgsubr: the width may be inside.
                10 | 29 => {
                    let which = stack.pop()? as i64;
                    let index = match b0 == 10 {
                        true => self.subrs,
                        false => self.gsubrs,
                    };
                    let at = which + bias(index.items.len());
                    let &(from, to) = index.items.get(usize::try_from(at).ok()?)?;
                    if let Some(width) = self.scan(&self.bytes[from..to], stack, depth + 1) {
                        return Some(width);
                    }
                }
                // A subroutine that returns hands the stack back as it is.
                11 => return None,
                // The stack-clearing operators, and how many operands each
                // takes when no width is in front of it.
                //   hstem, vstem, hstemhm, vstemhm, hintmask, cntrmask: even
                1 | 3 | 18 | 23 | 19 | 20 => {
                    return Some(match stack.len() % 2 == 1 {
                        true => self.nominal_width + stack[0],
                        false => self.default_width,
                    })
                }
                // rmoveto: two.
                21 => {
                    return Some(match stack.len() > 2 {
                        true => self.nominal_width + stack[0],
                        false => self.default_width,
                    })
                }
                // hmoveto and vmoveto: one.
                22 | 4 => {
                    return Some(match stack.len() > 1 {
                        true => self.nominal_width + stack[0],
                        false => self.default_width,
                    })
                }
                // endchar: none, or four for a deprecated composite.
                14 => {
                    return Some(match stack.len() == 1 || stack.len() == 5 {
                        true => self.nominal_width + stack[0],
                        false => self.default_width,
                    })
                }
                // Anything else is drawing, and drawing means the width was
                // settled before it.
                _ => return None,
            }
        }
        None
    }
}

/// The 391 strings every CFF font shares, from Appendix A of the
/// specification. A glyph name below 391 is one of these and is not stored
/// in the font at all, which is most of why a CFF font is small.
pub const STANDARD_STRINGS: [&str; 391] = [
    ".notdef",
    "space",
    "exclam",
    "quotedbl",
    "numbersign",
    "dollar",
    "percent",
    "ampersand",
    "quoteright",
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
    "quoteleft",
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
    "exclamdown",
    "cent",
    "sterling",
    "fraction",
    "yen",
    "florin",
    "section",
    "currency",
    "quotesingle",
    "quotedblleft",
    "guillemotleft",
    "guilsinglleft",
    "guilsinglright",
    "fi",
    "fl",
    "endash",
    "dagger",
    "daggerdbl",
    "periodcentered",
    "paragraph",
    "bullet",
    "quotesinglbase",
    "quotedblbase",
    "quotedblright",
    "guillemotright",
    "ellipsis",
    "perthousand",
    "questiondown",
    "grave",
    "acute",
    "circumflex",
    "tilde",
    "macron",
    "breve",
    "dotaccent",
    "dieresis",
    "ring",
    "cedilla",
    "hungarumlaut",
    "ogonek",
    "caron",
    "emdash",
    "AE",
    "ordfeminine",
    "Lslash",
    "Oslash",
    "OE",
    "ordmasculine",
    "ae",
    "dotlessi",
    "lslash",
    "oslash",
    "oe",
    "germandbls",
    "onesuperior",
    "logicalnot",
    "mu",
    "trademark",
    "Eth",
    "onehalf",
    "plusminus",
    "Thorn",
    "onequarter",
    "divide",
    "brokenbar",
    "degree",
    "thorn",
    "threequarters",
    "twosuperior",
    "registered",
    "minus",
    "eth",
    "multiply",
    "threesuperior",
    "copyright",
    "Aacute",
    "Acircumflex",
    "Adieresis",
    "Agrave",
    "Aring",
    "Atilde",
    "Ccedilla",
    "Eacute",
    "Ecircumflex",
    "Edieresis",
    "Egrave",
    "Iacute",
    "Icircumflex",
    "Idieresis",
    "Igrave",
    "Ntilde",
    "Oacute",
    "Ocircumflex",
    "Odieresis",
    "Ograve",
    "Otilde",
    "Scaron",
    "Uacute",
    "Ucircumflex",
    "Udieresis",
    "Ugrave",
    "Yacute",
    "Ydieresis",
    "Zcaron",
    "aacute",
    "acircumflex",
    "adieresis",
    "agrave",
    "aring",
    "atilde",
    "ccedilla",
    "eacute",
    "ecircumflex",
    "edieresis",
    "egrave",
    "iacute",
    "icircumflex",
    "idieresis",
    "igrave",
    "ntilde",
    "oacute",
    "ocircumflex",
    "odieresis",
    "ograve",
    "otilde",
    "scaron",
    "uacute",
    "ucircumflex",
    "udieresis",
    "ugrave",
    "yacute",
    "ydieresis",
    "zcaron",
    "exclamsmall",
    "Hungarumlautsmall",
    "dollaroldstyle",
    "dollarsuperior",
    "ampersandsmall",
    "Acutesmall",
    "parenleftsuperior",
    "parenrightsuperior",
    "twodotenleader",
    "onedotenleader",
    "zerooldstyle",
    "oneoldstyle",
    "twooldstyle",
    "threeoldstyle",
    "fouroldstyle",
    "fiveoldstyle",
    "sixoldstyle",
    "sevenoldstyle",
    "eightoldstyle",
    "nineoldstyle",
    "commasuperior",
    "threequartersemdash",
    "periodsuperior",
    "questionsmall",
    "asuperior",
    "bsuperior",
    "centsuperior",
    "dsuperior",
    "esuperior",
    "isuperior",
    "lsuperior",
    "msuperior",
    "nsuperior",
    "osuperior",
    "rsuperior",
    "ssuperior",
    "tsuperior",
    "ff",
    "ffi",
    "ffl",
    "parenleftinferior",
    "parenrightinferior",
    "Circumflexsmall",
    "hyphensuperior",
    "Gravesmall",
    "Asmall",
    "Bsmall",
    "Csmall",
    "Dsmall",
    "Esmall",
    "Fsmall",
    "Gsmall",
    "Hsmall",
    "Ismall",
    "Jsmall",
    "Ksmall",
    "Lsmall",
    "Msmall",
    "Nsmall",
    "Osmall",
    "Psmall",
    "Qsmall",
    "Rsmall",
    "Ssmall",
    "Tsmall",
    "Usmall",
    "Vsmall",
    "Wsmall",
    "Xsmall",
    "Ysmall",
    "Zsmall",
    "colonmonetary",
    "onefitted",
    "rupiah",
    "Tildesmall",
    "exclamdownsmall",
    "centoldstyle",
    "Lslashsmall",
    "Scaronsmall",
    "Zcaronsmall",
    "Dieresissmall",
    "Brevesmall",
    "Caronsmall",
    "Dotaccentsmall",
    "Macronsmall",
    "figuredash",
    "hypheninferior",
    "Ogoneksmall",
    "Ringsmall",
    "Cedillasmall",
    "questiondownsmall",
    "oneeighth",
    "threeeighths",
    "fiveeighths",
    "seveneighths",
    "onethird",
    "twothirds",
    "zerosuperior",
    "foursuperior",
    "fivesuperior",
    "sixsuperior",
    "sevensuperior",
    "eightsuperior",
    "ninesuperior",
    "zeroinferior",
    "oneinferior",
    "twoinferior",
    "threeinferior",
    "fourinferior",
    "fiveinferior",
    "sixinferior",
    "seveninferior",
    "eightinferior",
    "nineinferior",
    "centinferior",
    "dollarinferior",
    "periodinferior",
    "commainferior",
    "Agravesmall",
    "Aacutesmall",
    "Acircumflexsmall",
    "Atildesmall",
    "Adieresissmall",
    "Aringsmall",
    "AEsmall",
    "Ccedillasmall",
    "Egravesmall",
    "Eacutesmall",
    "Ecircumflexsmall",
    "Edieresissmall",
    "Igravesmall",
    "Iacutesmall",
    "Icircumflexsmall",
    "Idieresissmall",
    "Ethsmall",
    "Ntildesmall",
    "Ogravesmall",
    "Oacutesmall",
    "Ocircumflexsmall",
    "Otildesmall",
    "Odieresissmall",
    "OEsmall",
    "Oslashsmall",
    "Ugravesmall",
    "Uacutesmall",
    "Ucircumflexsmall",
    "Udieresissmall",
    "Yacutesmall",
    "Thornsmall",
    "Ydieresissmall",
    "001.000",
    "001.001",
    "001.002",
    "001.003",
    "Black",
    "Bold",
    "Book",
    "Light",
    "Medium",
    "Regular",
    "Roman",
    "Semibold",
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

    fn latin_modern() -> Option<Cff> {
        let bytes = installed("lmroman10-regular.otf")?;
        let font = crate::sfnt::Sfnt::parse(bytes).ok()?;
        let table = font.table("CFF")?.to_vec();
        Some(Cff::parse(&table).expect("the CFF reads"))
    }

    /// Latin Modern's outlines, which is where its glyph names live.
    #[test]
    fn a_cff_gives_up_the_names_a_post_table_would_have_held() {
        let Some(cff) = latin_modern() else { return };

        assert_eq!(cff.name, "LMRoman10-Regular");
        assert!(!cff.is_cid, "Latin Modern is not CID-keyed");
        assert!(cff.len() > 800, "{} glyphs", cff.len());
        assert_eq!(cff.glyph_names[0], ".notdef", "glyph zero is always that");

        // The names are names rather than the numbers they are stored as.
        assert!(
            cff.glyph_names.iter().all(|name| name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))),
            "a name with rubbish in it means the strings were read wrongly"
        );
        // A font this size uses both the standard strings and its own.
        assert!(cff.glyph_names.iter().any(|n| n == "A"));
        assert!(cff.glyph_names.iter().any(|n| n.contains('.')));

        // Every glyph has a width, and they are not all the same number.
        assert_eq!(cff.widths.len(), cff.len());
        let distinct: std::collections::BTreeSet<i64> =
            cff.widths.iter().map(|&w| w as i64).collect();
        assert!(distinct.len() > 20, "only {} widths", distinct.len());
    }

    /// The pieces of the format, on their own.
    #[test]
    fn an_index_and_a_dict_are_read_the_way_the_specification_says() {
        // An INDEX of two items, with one-byte offsets: two bytes of count,
        // one of offset size, three offsets, then the data. The offsets count
        // from one, and from the byte before the data -- which is why the data
        // begins at 6 and the first item is [6, 8).
        let bytes = [0, 2, 1, 1, 3, 5, b'a', b'b', b'c', b'd'];
        let read = index(&bytes, 0).expect("an INDEX");
        assert_eq!(read.items, [(6, 8), (8, 10)]);
        assert_eq!(&bytes[6..8], b"ab");
        assert_eq!(&bytes[8..10], b"cd");
        assert_eq!(read.end, 10);

        // An empty INDEX is two bytes.
        let empty = index(&[0, 0, 9, 9], 0).expect("an empty INDEX");
        assert!(empty.items.is_empty());
        assert_eq!(empty.end, 2);

        // A DICT: operands before their operator, in every size.
        //   139 -> 0, 247 0 -> 108, 251 0 -> -108, then operator 17.
        let read = dict(&[139, 247, 0, 251, 0, 17]).expect("a DICT");
        assert_eq!(
            read.get(&17).map(Vec::as_slice),
            Some([0.0, 108.0, -108.0].as_slice())
        );
        // 28 is a two-byte integer, and it is signed.
        assert_eq!(
            dict(&[28, 0xff, 0xff, 15]).expect("a DICT").get(&15),
            Some(&vec![-1.0])
        );
        // 12 and another byte is one operator.
        assert!(dict(&[139, 12, 30]).expect("a DICT").contains_key(&0x0c1e));
        // A real: nybbles, ending in 15. `-2.5` is e 2 a 5 f.
        let real = dict(&[30, 0xe2, 0xa5, 0xff, 15]).expect("a DICT");
        assert_eq!(real.get(&15), Some(&vec![-2.5]));
    }

    /// What is not a CFF is refused.
    #[test]
    fn what_is_not_a_cff_is_refused() {
        assert!(Cff::parse(b"").is_err());
        // A header that says it is shorter than a header.
        assert!(Cff::parse(&[1, 0, 2, 4])
            .unwrap_err()
            .contains("header size"));
        // A plausible header and nothing after it.
        assert!(Cff::parse(&[1, 0, 4, 4]).is_err());
    }
}
