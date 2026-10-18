//! `fontloader-font-cff.lua`'s reading of a `CFF ` table: the dictionaries,
//! the charset that names the glyphs, and every charstring run far enough to
//! give the glyph its advance and its bounding box.
//!
//! The box is the fontloader's own: the extremes of every point a charstring
//! moves to, lines to, or uses as a CURVE CONTROL POINT
//! (font-cff.lua:888-910's `xycurveto` takes all three), rounded half away
//! from zero (`math.round`, fontloader-l-math.lua:25). It is not the ink box
//! HarfBuzz reports, and that difference is exactly why a node-mode glyph
//! and a harf-mode glyph can have different heights.
//!
//! Ported with its quirks, since they decide numbers: a 32-bit charstring
//! number is `n1 + n2/0xFFFF` (:1882), a dictionary `long` treats values from
//! `0x8000000` as negative (:602), the charset loops read one SID more than
//! there are glyphs (:2394, :2400), and an operator the interpreter does not
//! handle empties the stack.

use std::collections::BTreeMap;

use super::otr::FontData;
use super::stream::Stream;

/// `defaultstrings` (font-cff.lua:81-147): the 391 standard strings.
const DEFAULT_STRINGS: [&str; 391] = [
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

/// What `readers.cff` keeps about the font (font-cff.lua:2657-2668).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CffInfo {
    pub familyname: Option<String>,
    pub fullname: Option<String>,
    pub weight: Option<String>,
    pub italicangle: f64,
    pub monospaced: bool,
    pub defaultwidth: f64,
    pub nominalwidth: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CidInfo {
    pub registry: String,
    pub ordering: String,
    pub supplement: f64,
}

/// `math.round` (fontloader-l-math.lua:25): half away from zero.
pub fn round(x: f64) -> f64 {
    if x < 0.0 {
        -(-x + 0.5).floor()
    } else {
        (x + 0.5).floor()
    }
}

/// A Top or Private DICT as `parsedictionaries` / `parseprivates` leave it.
#[derive(Clone, Debug, Default)]
struct Dict {
    fullname: Option<String>,
    familyname: Option<String>,
    weight: Option<String>,
    italicangle: f64,
    monospaced: bool,
    charstringtype: f64,
    charset: f64,
    charstrings: Option<f64>,
    private: Option<(f64, f64)>,
    subroutines: f64,
    defaultwidthx: f64,
    nominalwidthx: f64,
    cid_registry: Option<String>,
    cid_ordering: Option<String>,
    cid_supplement: f64,
    cid_fdarray: Option<f64>,
    cid_fdselect: Option<f64>,
    has_cid: bool,
}

struct Strings<'a> {
    custom: &'a [String],
}

impl Strings<'_> {
    fn get(&self, sid: f64) -> Option<String> {
        if sid < 0.0 || sid.fract() != 0.0 {
            return None;
        }
        let sid = sid as usize;
        match sid < DEFAULT_STRINGS.len() {
            true => Some(DEFAULT_STRINGS[sid].to_string()),
            false => self.custom.get(sid - DEFAULT_STRINGS.len()).cloned(),
        }
    }
}

/// The real-number nibbles (font-cff.lua:531-562): each byte is two nibbles
/// spelled out, the last byte's low or high `f` ends it, and Lua's
/// `tonumber` reads the text (0 when it cannot).
fn nibbles(data: &[u8], at: &mut usize) -> f64 {
    const NIBBLE: [&str; 16] = [
        "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", ".", "E", "E-", "", "-", "",
    ];
    let mut text = String::new();
    loop {
        let Some(&b) = data.get(*at) else {
            break;
        };
        *at += 1;
        let hi = (b >> 4) as usize;
        let lo = (b & 0x0F) as usize;
        let last1 = lo == 0xF && hi <= 0xB;
        let last2 = hi == 0xF;
        if last2 {
            break;
        }
        if last1 {
            // remap_2 has only \x0F..\x9F; \xAF and \xBF stay the raw byte,
            // which no number survives.
            match hi <= 9 {
                true => text.push_str(NIBBLE[hi]),
                false => text.push('\u{FFFD}'),
            }
            break;
        }
        if hi >= 0xE {
            // \xE0..\xEF are not in remap_1 and stay raw.
            text.push('\u{FFFD}');
        } else {
            text.push_str(NIBBLE[hi]);
            text.push_str(NIBBLE[lo]);
        }
    }
    lua_tonumber(&text).unwrap_or(0.0)
}

/// Lua's `tonumber` on a decimal string.
fn lua_tonumber(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty()
        || t.contains(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '-' | '+')))
    {
        return None;
    }
    t.parse::<f64>().ok()
}

/// `parsedictionaries` (font-cff.lua:299-667) over one DICT's bytes.
fn parse_dict(data: &[u8], strings: &Strings, private_defaults: bool) -> Dict {
    let mut d = Dict {
        charstringtype: 2.0,
        ..Dict::default()
    };
    if private_defaults {
        d.charstringtype = 0.0;
    }
    let mut stack: Vec<f64> = Vec::new();
    let top = |s: &Vec<f64>| s.last().copied().unwrap_or(0.0);
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        match b {
            32..=246 => {
                stack.push(b as f64 - 139.0);
                i += 1;
            }
            247..=250 => {
                let b1 = *data.get(i + 1).unwrap_or(&0) as f64;
                stack.push((b as f64 - 247.0) * 256.0 + b1 + 108.0);
                i += 2;
            }
            251..=254 => {
                let b1 = *data.get(i + 1).unwrap_or(&0) as f64;
                stack.push(-(b as f64 - 251.0) * 256.0 - b1 - 108.0);
                i += 2;
            }
            28 => {
                let n = 0x100 * *data.get(i + 1).unwrap_or(&0) as i64
                    + *data.get(i + 2).unwrap_or(&0) as i64;
                stack.push(match n >= 0x8000 {
                    true => (n - 0xFFFF - 1) as f64,
                    false => n as f64,
                });
                i += 3;
            }
            29 => {
                let g = |k: usize| *data.get(i + k).unwrap_or(&0) as i64;
                let n = 0x100_0000 * g(1) + 0x1_0000 * g(2) + 0x100 * g(3) + g(4);
                // font-cff.lua:602 compares with 0x8000000, one zero short.
                stack.push(match n >= 0x800_0000 {
                    true => (n - 0xFFFF_FFFF - 1) as f64,
                    false => n as f64,
                });
                i += 5;
            }
            30 => {
                i += 1;
                stack.push(nibbles(data, &mut i));
            }
            12 => {
                let op = data.get(i + 1).copied();
                let handled = match op {
                    Some(1) => {
                        d.monospaced = top(&stack) == 1.0;
                        true
                    }
                    Some(2) => {
                        d.italicangle = top(&stack);
                        true
                    }
                    Some(6) => {
                        d.charstringtype = top(&stack);
                        true
                    }
                    Some(30) => {
                        let n = stack.len();
                        let at = |k: usize| {
                            n.checked_sub(k)
                                .and_then(|j| stack.get(j))
                                .copied()
                                .unwrap_or(0.0)
                        };
                        d.cid_registry = strings.get(at(3));
                        d.cid_ordering = strings.get(at(2));
                        d.cid_supplement = at(1);
                        d.has_cid = true;
                        true
                    }
                    Some(36) => {
                        d.cid_fdarray = Some(top(&stack));
                        d.has_cid = true;
                        true
                    }
                    Some(37) => {
                        d.cid_fdselect = Some(top(&stack));
                        d.has_cid = true;
                        true
                    }
                    Some(
                        0 | 3 | 4 | 5 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 20 | 21 | 22 | 31 | 32 | 33
                        | 34 | 35 | 38,
                    ) => true,
                    _ => false,
                };
                if handled {
                    stack.clear();
                    i += 2;
                } else {
                    // p_double did not match: p_unsupported eats the 12.
                    stack.clear();
                    i += 1;
                }
            }
            0..=21 | 24 | 25 => {
                match b {
                    2 => d.fullname = strings.get(top(&stack)),
                    3 => d.familyname = strings.get(top(&stack)),
                    4 => d.weight = strings.get(top(&stack)),
                    15 => d.charset = top(&stack),
                    17 => d.charstrings = Some(top(&stack)),
                    18 => {
                        let n = stack.len();
                        let size = n.checked_sub(2).map(|j| stack[j]).unwrap_or(0.0);
                        d.private = Some((size, top(&stack)));
                    }
                    19 => d.subroutines = top(&stack),
                    20 => d.defaultwidthx = top(&stack),
                    21 => d.nominalwidthx = top(&stack),
                    _ => {}
                }
                stack.clear();
                i += 1;
            }
            _ => {
                // 22, 23, 26, 27, 31, 255: p_unsupported.
                stack.clear();
                i += 1;
            }
        }
    }
    d
}

/// `readlengths` (font-cff.lua:223-247): an INDEX's item lengths, leaving
/// the stream at the first item.
fn read_lengths(f: &mut Stream, long: bool) -> Vec<usize> {
    let count = match long {
        true => f.u32(),
        false => f.u16(),
    } as usize;
    if count == 0 {
        return Vec::new();
    }
    let osize = f.u8();
    let read = |f: &mut Stream| match osize {
        1 => f.u8(),
        2 => f.u16(),
        3 => f.u24(),
        4 => f.u32(),
        _ => 0,
    };
    if !(1..=4).contains(&osize) {
        return Vec::new();
    }
    let mut previous = read(f) as i64;
    (0..count)
        .map(|_| {
            let offset = read(f) as i64;
            let length = (offset - previous).max(0) as usize;
            previous = offset;
            length
        })
        .collect()
}

fn read_index(f: &mut Stream, long: bool) -> Vec<Vec<u8>> {
    read_lengths(f, long)
        .into_iter()
        .map(|n| f.bytes(n))
        .collect()
}

/// The charstring interpreter's state (font-cff.lua:713-743), for one glyph.
struct Interp<'a> {
    stack: Vec<f64>,
    top: usize,
    x: f64,
    y: f64,
    width: Width,
    xmin: f64,
    xmax: f64,
    ymin: f64,
    ymax: f64,
    checked: bool,
    stems: usize,
    popped: f64,
    globals: &'a [Vec<u8>],
    locals: &'a [Vec<u8>],
    globalbias: i64,
    localbias: i64,
    depth: usize,
}

/// `width`: `false` until a width is seen, `true` when the charstring has
/// none, else the value.
#[derive(Clone, Copy, PartialEq)]
enum Width {
    Unset,
    None,
    Value(f64),
}

impl<'a> Interp<'a> {
    fn at(&self, k: usize) -> f64 {
        // `stack[k]`, 1-based; a slot never pushed is nil, read as 0 here
        // where the Lua would fail on arithmetic.
        if k == 0 || k > self.stack.len() {
            0.0
        } else {
            self.stack[k - 1]
        }
    }

    fn push(&mut self, v: f64) {
        self.top += 1;
        if self.stack.len() < self.top {
            self.stack.resize(self.top, 0.0);
        }
        self.stack[self.top - 1] = v;
    }

    fn point(&mut self, x: f64, y: f64) {
        if self.checked {
            if x > self.xmax {
                self.xmax = x;
            } else if x < self.xmin {
                self.xmin = x;
            }
            if y > self.ymax {
                self.ymax = y;
            } else if y < self.ymin {
                self.ymin = y;
            }
        } else {
            self.xmin = x;
            self.ymin = y;
            self.xmax = x;
            self.ymax = y;
            self.checked = true;
        }
    }

    /// `xmoveto`/`xlineto`: only x can have moved.
    fn xpoint(&mut self) {
        if !self.checked {
            self.point(self.x, self.y);
        } else if self.x > self.xmax {
            self.xmax = self.x;
        } else if self.x < self.xmin {
            self.xmin = self.x;
        }
    }

    fn ypoint(&mut self) {
        if !self.checked {
            self.point(self.x, self.y);
        } else if self.y > self.ymax {
            self.ymax = self.y;
        } else if self.y < self.ymin {
            self.ymin = self.y;
        }
    }

    /// `xycurveto` (font-cff.lua:888-910): the first point initialises the
    /// box; the other two only extend it.
    fn curve(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, x3: f64, y3: f64) {
        self.point(x1, y1);
        for (x, y) in [(x2, y2), (x3, y3)] {
            if x > self.xmax {
                self.xmax = x;
            } else if x < self.xmin {
                self.xmin = x;
            }
            if y > self.ymax {
                self.ymax = y;
            } else if y < self.ymin {
                self.ymin = y;
            }
        }
    }

    fn backtrack_width(&mut self, more_than: usize) {
        if self.width == Width::Unset {
            self.width = match self.top > more_than {
                true => Width::Value(self.at(1)),
                false => Width::None,
            };
        }
    }

    /// `remove(stack, 1)`: the width in front of a stem list.
    fn shift_width(&mut self) {
        if self.top == 0 {
            return;
        }
        if self.top % 2 != 0 {
            let first = if self.stack.is_empty() {
                0.0
            } else {
                self.stack.remove(0)
            };
            if self.width == Width::Unset {
                self.width = Width::Value(first);
            }
            self.top -= 1;
        }
    }

    fn stem(&mut self) {
        self.shift_width();
        self.stems += self.top / 2;
        self.top = 0;
    }

    /// `getmask`: the number of mask bytes to skip.
    fn mask(&mut self) -> usize {
        self.shift_width();
        self.stems += self.top / 2;
        self.top = 0;
        match self.stems {
            0 => 0,
            1..=8 => 1,
            n => (n + 7) / 8,
        }
    }

    fn action(&mut self, t: u8) -> usize {
        let top = self.top;
        match t {
            1 | 3 | 18 | 23 => self.stem(),
            19 | 20 => return self.mask(),
            4 => {
                self.backtrack_width(1);
                self.y += self.at(top);
                self.top = 0;
                self.ypoint();
            }
            21 => {
                self.backtrack_width(2);
                self.x += self.at(top - 1);
                self.y += self.at(top);
                self.top = 0;
                self.point(self.x, self.y);
            }
            22 => {
                self.backtrack_width(1);
                self.x += self.at(top);
                self.top = 0;
                self.xpoint();
            }
            5 => {
                let mut i = 1;
                while i <= top {
                    self.x += self.at(i);
                    self.y += self.at(i + 1);
                    self.point(self.x, self.y);
                    i += 2;
                }
                self.top = 0;
            }
            6 | 7 => {
                // hlineto starts horizontal, vlineto vertical.
                if top == 1 {
                    if t == 6 {
                        self.x += self.at(1);
                        self.xpoint();
                    } else {
                        self.y += self.at(1);
                        self.ypoint();
                    }
                } else {
                    let mut horizontal = t == 6;
                    for i in 1..=top {
                        if horizontal {
                            self.x += self.at(i);
                            self.xpoint();
                        } else {
                            self.y += self.at(i);
                            self.ypoint();
                        }
                        horizontal = !horizontal;
                    }
                }
                self.top = 0;
            }
            8 => {
                let mut i = 1;
                while i <= top {
                    let ax = self.x + self.at(i);
                    let ay = self.y + self.at(i + 1);
                    let bx = ax + self.at(i + 2);
                    let by = ay + self.at(i + 3);
                    self.x = bx + self.at(i + 4);
                    self.y = by + self.at(i + 5);
                    self.curve(ax, ay, bx, by, self.x, self.y);
                    i += 6;
                }
                self.top = 0;
            }
            27 => {
                // hhcurveto
                let mut s = 1;
                if top % 2 != 0 {
                    self.y += self.at(1);
                    s = 2;
                }
                if top == 4 {
                    let ax = self.x + self.at(1);
                    let ay = self.y;
                    let bx = ax + self.at(2);
                    let by = ay + self.at(3);
                    self.x = bx + self.at(4);
                    self.y = by;
                    self.curve(ax, ay, bx, by, self.x, self.y);
                } else {
                    let mut i = s;
                    while i <= top {
                        let ax = self.x + self.at(i);
                        let ay = self.y;
                        let bx = ax + self.at(i + 1);
                        let by = ay + self.at(i + 2);
                        self.x = bx + self.at(i + 3);
                        self.y = by;
                        self.curve(ax, ay, bx, by, self.x, self.y);
                        i += 4;
                    }
                }
                self.top = 0;
            }
            26 => {
                // vvcurveto
                let mut s = 1;
                let mut d = 0.0;
                if top % 2 != 0 {
                    d = self.at(1);
                    s = 2;
                }
                if top == 4 {
                    let ax = self.x + d;
                    let ay = self.y + self.at(1);
                    let bx = ax + self.at(2);
                    let by = ay + self.at(3);
                    self.x = bx;
                    self.y = by + self.at(4);
                    self.curve(ax, ay, bx, by, self.x, self.y);
                } else {
                    let mut i = s;
                    while i <= top {
                        let ax = self.x + d;
                        let ay = self.y + self.at(i);
                        let bx = ax + self.at(i + 1);
                        let by = ay + self.at(i + 2);
                        self.x = bx;
                        self.y = by + self.at(i + 3);
                        self.curve(ax, ay, bx, by, self.x, self.y);
                        d = 0.0;
                        i += 4;
                    }
                }
                self.top = 0;
            }
            30 | 31 => self.xxcurveto(t == 31),
            24 => {
                // rcurveline
                let mut i = 1;
                while i + 2 <= top {
                    let ax = self.x + self.at(i);
                    let ay = self.y + self.at(i + 1);
                    let bx = ax + self.at(i + 2);
                    let by = ay + self.at(i + 3);
                    self.x = bx + self.at(i + 4);
                    self.y = by + self.at(i + 5);
                    self.curve(ax, ay, bx, by, self.x, self.y);
                    i += 6;
                }
                self.x += self.at(top - 1);
                self.y += self.at(top);
                self.point(self.x, self.y);
                self.top = 0;
            }
            25 => {
                // rlinecurve
                if top > 6 {
                    let mut i = 1;
                    while i <= top - 6 {
                        self.x += self.at(i);
                        self.y += self.at(i + 1);
                        self.point(self.x, self.y);
                        i += 2;
                    }
                }
                let ax = self.x + self.at(top - 5);
                let ay = self.y + self.at(top - 4);
                let bx = ax + self.at(top - 3);
                let by = ay + self.at(top - 2);
                self.x = bx + self.at(top - 1);
                self.y = by + self.at(top);
                self.curve(ax, ay, bx, by, self.x, self.y);
                self.top = 0;
            }
            13 => {
                // hsbw: a Type 1 operator, which a CFF ("cff") honours.
                self.width = Width::Value(self.at(top));
                self.top = 0;
            }
            15 => {
                // setvsindex (CFF2): no regions without variation data.
                self.top = self.top.saturating_sub(1);
            }
            16 => {
                // blend with no axes: `top - nofregions * n` with no regions
                // drops only the count.
                self.top = self.top.saturating_sub(1);
            }
            _ => self.top = 0,
        }
        0
    }

    /// `xxcurveto` (font-cff.lua:1122-1185): `swap` is hvcurveto.
    fn xxcurveto(&mut self, mut swap: bool) {
        let mut top = self.top;
        let last = match top % 4 != 0 {
            true => Some(self.at(top)),
            false => None,
        };
        if last.is_some() {
            top -= 1;
        }
        if top == 4 {
            let (ax, ay, bx, by);
            if swap {
                ax = self.x + self.at(1);
                ay = self.y;
                bx = ax + self.at(2);
                by = ay + self.at(3);
                self.y = by + self.at(4);
                self.x = bx + last.unwrap_or(0.0);
            } else {
                ax = self.x;
                ay = self.y + self.at(1);
                bx = ax + self.at(2);
                by = ay + self.at(3);
                self.x = bx + self.at(4);
                self.y = by + last.unwrap_or(0.0);
            }
            self.curve(ax, ay, bx, by, self.x, self.y);
        } else {
            let mut i = 1;
            while i <= top {
                let (ax, ay, bx, by);
                if swap {
                    ax = self.x + self.at(i);
                    ay = self.y;
                    bx = ax + self.at(i + 1);
                    by = ay + self.at(i + 2);
                    self.y = by + self.at(i + 3);
                    self.x = match (last, i + 3 == top) {
                        (Some(l), true) => bx + l,
                        _ => bx,
                    };
                    swap = false;
                } else {
                    ax = self.x;
                    ay = self.y + self.at(i);
                    bx = ax + self.at(i + 1);
                    by = ay + self.at(i + 2);
                    self.x = bx + self.at(i + 3);
                    self.y = match (last, i + 3 == top) {
                        (Some(l), true) => by + l,
                        _ => by,
                    };
                    swap = true;
                }
                self.curve(ax, ay, bx, by, self.x, self.y);
                i += 4;
            }
        }
        self.top = 0;
    }

    /// The flex family and the other two-byte operators (font-cff.lua:1689).
    fn subaction(&mut self, t: u8) {
        let s = |k: usize| self.at(k);
        match t {
            34 => {
                // hflex
                let ax = self.x + s(1);
                let ay = self.y;
                let bx = ax + s(2);
                let by = ay + s(3);
                let cx = bx + s(4);
                let cy = by;
                let dx = cx + s(5);
                let dy = by;
                let ex = dx + s(6);
                let ey = self.y;
                let nx = ex + s(7);
                self.curve(ax, ay, bx, by, cx, cy);
                self.x = nx;
                self.curve(dx, dy, ex, ey, self.x, self.y);
                self.top = 0;
            }
            35 => {
                // flex
                let ax = self.x + s(1);
                let ay = self.y + s(2);
                let bx = ax + s(3);
                let by = ay + s(4);
                let cx = bx + s(5);
                let cy = by + s(6);
                let dx = cx + s(7);
                let dy = cy + s(8);
                let ex = dx + s(9);
                let ey = dy + s(10);
                let nx = ex + s(11);
                let ny = ey + s(12);
                self.curve(ax, ay, bx, by, cx, cy);
                self.x = nx;
                self.y = ny;
                self.curve(dx, dy, ex, ey, self.x, self.y);
                self.top = 0;
            }
            36 => {
                // hflex1
                let ax = self.x + s(1);
                let ay = self.y + s(2);
                let bx = ax + s(3);
                let by = ay + s(4);
                let cx = bx + s(5);
                let cy = by;
                let dx = cx + s(6);
                let dy = by;
                let ex = dx + s(7);
                let ey = dy + s(8);
                let nx = ex + s(9);
                self.curve(ax, ay, bx, by, cx, cy);
                self.x = nx;
                self.curve(dx, dy, ex, ey, self.x, self.y);
                self.top = 0;
            }
            37 => {
                // flex1
                let ax = self.x + s(1);
                let ay = self.y + s(2);
                let bx = ax + s(3);
                let by = ay + s(4);
                let cx = bx + s(5);
                let cy = by + s(6);
                let dx = cx + s(7);
                let dy = cy + s(8);
                let ex = dx + s(9);
                let ey = dy + s(10);
                let last = s(11);
                self.curve(ax, ay, bx, by, cx, cy);
                if (ex - self.x).abs() > (ey - self.y).abs() {
                    self.x = ex + last;
                } else {
                    self.y = ey + last;
                }
                self.curve(dx, dy, ex, ey, self.x, self.y);
                self.top = 0;
            }
            1 | 2 => self.top = 0,
            6 => self.top = 0,
            7 => {
                // sbw: the fontloader runs a CFF table's charstrings with
                // `version == "cff"`, which honours the Type 1 operators.
                self.width = Width::Value(self.at(self.top - 1));
                self.top = 0;
            }
            12 => {
                // divide, leaving the stack one shorter and not cleared
                let d = self.at(self.top);
                self.top = self.top.saturating_sub(1);
                if self.top >= 1 {
                    let v = self.at(self.top) / d;
                    self.stack[self.top - 1] = v;
                }
            }
            16 => {
                // callothersubr
                let top = self.top;
                if self.at(top) == 3.0 {
                    self.popped = self.at(top - 2);
                } else {
                    self.popped = 3.0;
                }
                let t = self.at(top - 1);
                let drop = t as i64 + 2;
                self.top = (top as i64 - drop).max(0) as usize;
            }
            17 => {
                // pop
                let p = self.popped;
                self.push(p);
            }
            33 => {
                self.x += self.at(self.top - 1);
                self.y += self.at(self.top);
                self.top = 0;
            }
            _ => self.top = 0,
        }
    }

    fn call(&mut self, global: bool) {
        self.depth += 1;
        if self.top == 0 {
            self.top = 0;
        } else {
            let bias = match global {
                true => self.globalbias,
                false => self.localbias,
            };
            // `index = stack[top] + bias` into a 1-based list.
            let index = self.at(self.top) as i64 + bias;
            self.top -= 1;
            let list = match global {
                true => self.globals,
                false => self.locals,
            };
            match (index >= 1).then(|| list.get(index as usize - 1)).flatten() {
                Some(tab) if self.depth < 64 => {
                    self.process(tab);
                }
                _ => self.top = 0,
            }
        }
        self.depth -= 1;
    }

    /// `process` (font-cff.lua:1846-2087) without the pass-through branch.
    /// Returns true at `endchar`.
    fn process(&mut self, tab: &[u8]) -> bool {
        let mut i = 0;
        let n = tab.len();
        let b = |k: usize| *tab.get(k).unwrap_or(&0) as f64;
        while i < n {
            let t = tab[i];
            if t >= 32 {
                let v = if t <= 246 {
                    i += 1;
                    t as f64 - 139.0
                } else if t <= 250 {
                    let v = t as f64 * 256.0 - 63124.0 + b(i + 1);
                    i += 2;
                    v
                } else if t <= 254 {
                    let v = -(t as f64) * 256.0 + 64148.0 - b(i + 1);
                    i += 2;
                    v
                } else {
                    let mut n1 = 0x100 as f64 * b(i + 1) + b(i + 2);
                    let n2 = 0x100 as f64 * b(i + 3) + b(i + 4);
                    if n1 >= 0x8000 as f64 {
                        n1 -= 0x10000 as f64;
                    }
                    i += 5;
                    n1 + n2 / 0xFFFF as f64
                };
                self.push(v);
            } else if t == 28 {
                let v = 0x100 as f64 * b(i + 1) + b(i + 2);
                self.push(match v >= 0x8000 as f64 {
                    true => v - 0x10000 as f64,
                    false => v,
                });
                i += 3;
            } else if t == 11 {
                return false;
            } else if t == 10 {
                self.call(false);
                i += 1;
            } else if t == 14 {
                if self.width == Width::Unset {
                    self.width = match self.top > 0 {
                        true => Width::Value(self.at(1)),
                        false => Width::None,
                    };
                }
                return true;
            } else if t == 29 {
                self.call(true);
                i += 1;
            } else if t == 12 {
                i += 1;
                let sub = *tab.get(i).unwrap_or(&0);
                self.subaction(sub);
                i += 1;
            } else {
                let s = self.action(t);
                i += s + 1;
            }
        }
        false
    }
}

/// `setbias` (font-cff.lua:2127-2137): the bias plus one, for 1-based lists.
fn bias(n: usize) -> i64 {
    (match n {
        _ if n < 1240 => 107,
        _ if n < 33900 => 1131,
        _ => 32768,
    }) + 1
}

/// `processshape` (font-cff.lua:2139-2241) for every glyph a private DICT
/// governs, writing the box, the width and the charset name.
#[allow(clippy::too_many_arguments)]
fn process_shape(
    fontdata: &mut FontData,
    tab: Option<&[u8]>,
    index: u32,
    globals: &[Vec<u8>],
    locals: &[Vec<u8>],
    nominalwidth: f64,
    defaultwidth: f64,
    charset: Option<&BTreeMap<u32, String>>,
) {
    let Some(tab) = tab else {
        let glyph = fontdata.glyph(index);
        glyph.boundingbox = Some([0.0; 4]);
        glyph.width = Some(0.0);
        glyph.name = charset.and_then(|c| c.get(&index).cloned());
        return;
    };
    let mut it = Interp {
        stack: Vec::new(),
        top: 0,
        x: 0.0,
        y: 0.0,
        width: Width::Unset,
        xmin: 0.0,
        xmax: 0.0,
        ymin: 0.0,
        ymax: 0.0,
        checked: false,
        stems: 0,
        popped: 3.0,
        globals,
        locals,
        globalbias: bias(globals.len()),
        localbias: bias(locals.len()),
        depth: 1,
    };
    it.process(tab);
    let bb = [
        round(it.xmin),
        round(it.ymin),
        round(it.xmax),
        round(it.ymax),
    ];
    let width = match it.width {
        Width::Unset | Width::None => defaultwidth,
        Width::Value(w) => nominalwidth + w,
    };
    let glyph = fontdata.glyph(index);
    glyph.boundingbox = Some(bb);
    if glyph.width.is_none() {
        glyph.width = Some(width);
    }
    if glyph.name.is_none() {
        if let Some(c) = charset {
            glyph.name = c.get(&index).cloned();
        }
    }
}

/// `readcharsets` (font-cff.lua:2383-2418).
fn read_charset(
    f: &mut Stream,
    header: usize,
    offset: f64,
    nofglyphs: usize,
    strings: &Strings,
) -> Option<BTreeMap<u32, String>> {
    if offset == 0.0 {
        return None;
    }
    f.set(header + offset as usize);
    let format = f.u8();
    let mut charset = BTreeMap::new();
    charset.insert(0, ".notdef".to_string());
    if format == 0 {
        for i in 1..=nofglyphs {
            if let Some(s) = strings.get(f.u16() as f64) {
                charset.insert(i as u32, s);
            }
        }
    } else if format == 1 || format == 2 {
        let mut i = 1;
        while i <= nofglyphs {
            let sid = f.u16();
            let n = match format {
                1 => f.u8(),
                _ => f.u16(),
            };
            for s in sid..=sid + n {
                if let Some(name) = strings.get(s as f64) {
                    charset.insert(i as u32, name);
                }
                i += 1;
                if i > nofglyphs {
                    break;
                }
            }
        }
    }
    Some(charset)
}

/// The Private DICT at `(size, offset)` relative to `header`, and its local
/// subroutines (`readprivates`, `parseprivates`, `readlocals`).
fn read_private(
    f: &mut Stream,
    header: usize,
    private: Option<(f64, f64)>,
    strings: &Strings,
) -> (Dict, Vec<Vec<u8>>) {
    let Some((size, offset)) = private else {
        return (Dict::default(), Vec::new());
    };
    f.set(header + offset as usize);
    let data = f.bytes(size.max(0.0) as usize);
    let dict = parse_dict(&data, strings, true);
    let locals = match dict.subroutines != 0.0 {
        true => {
            f.set(header + offset as usize + dict.subroutines as usize);
            read_index(f, false)
        }
        false => Vec::new(),
    };
    (dict, locals)
}

/// `readers.cff` (font-cff.lua:2632-2705).
pub fn read(f: &mut Stream, fontdata: &mut FontData) {
    let Some(t) = fontdata.tables.get("cff").copied() else {
        return;
    };
    f.set(t.offset);
    let header = f.pos();
    let major = f.u8();
    let _minor = f.u8();
    let size = f.u8() as usize;
    if major != 1 {
        return;
    }
    f.set(header + size);
    let _names = read_index(f, false);
    let dictionaries = read_index(f, false);
    let custom: Vec<String> = read_index(f, false)
        .into_iter()
        .map(|s| String::from_utf8_lossy(&s).into_owned())
        .collect();
    let globals = read_index(f, false);
    let strings = Strings { custom: &custom };
    let Some(top) = dictionaries.first() else {
        return;
    };
    let dic = parse_dict(top, &strings, false);
    let mut info = CffInfo {
        familyname: dic.familyname.clone(),
        fullname: dic.fullname.clone(),
        weight: dic.weight.clone(),
        italicangle: dic.italicangle,
        monospaced: dic.monospaced,
        defaultwidth: dic.defaultwidthx,
        nominalwidth: dic.nominalwidthx,
    };
    if dic.has_cid {
        fontdata.cidinfo = Some(CidInfo {
            registry: dic.cid_registry.clone().unwrap_or_default(),
            ordering: dic.cid_ordering.clone().unwrap_or_default(),
            supplement: dic.cid_supplement,
        });
    }
    // readcharstrings
    let charstrings = match (dic.charstrings, dic.charstringtype == 2.0) {
        (Some(offset), true) => {
            f.set(header + offset as usize);
            read_index(f, false)
        }
        _ => Vec::new(),
    };
    let nofglyphs = charstrings.len();
    if let (true, Some(select)) = (dic.has_cid, dic.cid_fdselect) {
        // readfdselect (font-cff.lua:2535-2616)
        f.set(header + select as usize);
        let format = f.u8();
        let mut fdindex: BTreeMap<usize, u32> = BTreeMap::new();
        let mut maxindex: i64 = -1;
        if format == 1 {
            for i in 0..=nofglyphs {
                let index = f.u8();
                fdindex.insert(i, index);
                maxindex = maxindex.max(index as i64);
            }
        } else if format == 3 {
            let _nofranges = f.u16();
            let mut first = f.u16() as usize;
            let mut index = f.u8();
            loop {
                let last = f.u16() as usize;
                maxindex = maxindex.max(index as i64);
                for i in first..=last {
                    fdindex.insert(i, index);
                }
                if last >= nofglyphs || f.overrun() {
                    break;
                }
                first = last + 1;
                index = f.u8();
            }
        }
        if maxindex >= 0 {
            if let Some(array) = dic.cid_fdarray {
                f.set(header + array as usize);
                let fds: Vec<Dict> = read_index(f, false)
                    .iter()
                    .map(|d| parse_dict(d, &strings, false))
                    .collect();
                let privates: Vec<(Dict, Vec<Vec<u8>>)> = fds
                    .iter()
                    .map(|d| read_private(f, header, d.private, &strings))
                    .collect();
                for (i, cs) in charstrings.iter().enumerate() {
                    // `dictionaries[fdindex[i]+1]` with the 1-based i: the
                    // fd of the NEXT glyph's slot, as font-cff.lua:2600 reads it.
                    let fd = fdindex.get(&(i + 1)).copied();
                    if let Some((p, locals)) = fd.and_then(|fd| privates.get(fd as usize)) {
                        process_shape(
                            fontdata,
                            Some(cs),
                            i as u32,
                            &globals,
                            locals,
                            p.nominalwidthx,
                            p.defaultwidthx,
                            None,
                        );
                        // `parsecharstring` ends `return glyphs[index]` with the
                        // 1-based index: the autovivifying table gains the
                        // glyph AFTER this one (font-cff.lua:2364-2366), which is
                        // how a CID font ends up with one glyph more than it has.
                        fontdata.glyph(i as u32 + 1);
                    }
                }
            }
        }
    } else {
        let charset = read_charset(f, header, dic.charset, nofglyphs, &strings);
        let mut private = dic.private;
        if dic.has_cid && dic.private.is_none() {
            // readnoselect: a CID font without FDSelect takes the first
            // FDArray dictionary's private.
            if let Some(array) = dic.cid_fdarray {
                f.set(header + array as usize);
                let fds = read_index(f, false);
                if let Some(first) = fds.first() {
                    private = parse_dict(first, &strings, false).private;
                }
            }
        }
        let (p, locals) = read_private(f, header, private, &strings);
        for (i, cs) in charstrings.iter().enumerate() {
            process_shape(
                fontdata,
                Some(cs),
                i as u32,
                &globals,
                &locals,
                p.nominalwidthx,
                p.defaultwidthx,
                charset.as_ref(),
            );
        }
        if private.is_some() {
            info.defaultwidth = p.defaultwidthx;
            info.nominalwidth = p.nominalwidthx;
        }
    }
    fontdata.cffinfo = Some(info);
}
