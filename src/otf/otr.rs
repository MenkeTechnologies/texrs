//! The table readers of `fontloader-font-otr.lua` and the `glyf`/`loca` half
//! of `fontloader-font-ttf.lua`: what luaotfload's node mode knows about a
//! font before any feature is read.
//!
//! The order the tables are read in is `readdata`'s (font-otr.lua:2093-2238),
//! and it matters: `hmtx` needs `hhea`'s count, `loca` needs `head`'s format
//! and `maxp`'s glyph count, and `cmap` hands out the first Unicode each
//! glyph is reached by.

use std::collections::{BTreeMap, BTreeSet};

use super::stream::Stream;

/// One glyph as the reader leaves it (font-otr.lua:2073's autovivified
/// entries).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Glyph {
    pub index: u32,
    pub name: Option<String>,
    /// The first code point `cmap` maps to this glyph.
    pub unicode: Option<u32>,
    /// The advance, from `hmtx` (a zero advance is left unset there) or the
    /// CFF charstring.
    pub width: Option<f64>,
    /// `{ xmin, ymin, xmax, ymax }`.
    pub boundingbox: Option<[f64; 4]>,
    /// From GDEF: `base`, `ligature`, `mark` or `component`.
    pub class: Option<String>,
}

/// A table directory entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TableEntry {
    pub offset: usize,
    pub length: usize,
}

/// A name record the filter kept (font-otr.lua:903).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NameEntry {
    pub content: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WindowsMetrics {
    pub version: u32,
    pub averagewidth: i32,
    pub weightclass: u32,
    pub widthclass: u32,
    pub fstype: u32,
    pub panose: [u32; 10],
    pub fsselection: u32,
    pub typoascender: i32,
    pub typodescender: i32,
    pub typolinegap: i32,
    pub winascent: u32,
    pub windescent: u32,
    pub xheight: Option<i32>,
    pub capheight: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontHeader {
    pub units: u32,
    pub xmin: i32,
    pub ymin: i32,
    pub xmax: i32,
    pub ymax: i32,
    pub macstyle: u32,
    pub indextolocformat: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HorizontalHeader {
    pub ascender: i32,
    pub descender: i32,
    pub linegap: i32,
    pub nofmetrics: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PostScript {
    pub version: u32,
    pub italicangle: f64,
    pub underlineposition: i32,
    pub underlinethickness: i32,
    pub monospaced: u32,
}

/// What `readdata` builds (font-otr.lua:2036-2047 and the readers after).
#[derive(Clone, Debug, Default)]
pub struct FontData {
    pub tables: BTreeMap<String, TableEntry>,
    /// `opentype` when there is a CFF or CFF2 table, else `truetype`.
    pub format: &'static str,
    pub names: BTreeMap<String, NameEntry>,
    pub windowsmetrics: Option<WindowsMetrics>,
    pub fontheader: Option<FontHeader>,
    pub horizontalheader: Option<HorizontalHeader>,
    pub postscript: Option<PostScript>,
    pub nofglyphs: usize,
    /// Index to glyph, grown on any access as the Lua table is.
    pub glyphs: Vec<Glyph>,
    /// Index to the first code point that reached it (`mapping`).
    pub mapping: BTreeMap<u32, u32>,
    /// A primary code point to the other code points mapped to its glyph.
    pub duplicates: BTreeMap<u32, BTreeSet<u32>>,
    /// Format 14 variation selectors: selector -> code point -> glyph.
    pub variants: BTreeMap<u32, BTreeMap<u32, u32>>,
    pub cffinfo: Option<super::cff::CffInfo>,
    pub cidinfo: Option<super::cff::CidInfo>,
    /// `loca`'s glyph locations, absolute in the file.
    locations: Vec<usize>,
    pub filesize: usize,
    pub badfont: bool,
}

impl FontData {
    /// `glyphs[k]`, created on first use as `prepareglyps`'s metatable does.
    pub fn glyph(&mut self, k: u32) -> &mut Glyph {
        let k = k as usize;
        if k >= self.glyphs.len() {
            let start = self.glyphs.len();
            self.glyphs.extend((start..=k).map(|i| Glyph {
                index: i as u32,
                ..Glyph::default()
            }));
        }
        &mut self.glyphs[k]
    }

    fn table(&self, tag: &str) -> Option<TableEntry> {
        self.tables.get(tag).copied()
    }

    /// `getname(fontdata, key)` (font-otr.lua:959): the content if it is
    /// valid UTF-8.
    pub fn getname(&self, key: &str) -> Option<String> {
        let v = self.names.get(key)?;
        String::from_utf8(v.content.clone()).ok()
    }
}

/// `reservednames` (font-otr.lua:181-208).
const RESERVED_NAMES: [&str; 26] = [
    "copyright",
    "family",
    "subfamily",
    "uniqueid",
    "fullname",
    "version",
    "postscriptname",
    "trademark",
    "manufacturer",
    "designer",
    "description",
    "vendorurl",
    "designerurl",
    "license",
    "licenseurl",
    "reserved",
    "typographicfamily",
    "typographicsubfamily",
    "compatiblefullname",
    "sampletext",
    "cidfindfontname",
    "wwsfamily",
    "wwssubfamily",
    "lightbackgroundpalette",
    "darkbackgroundpalette",
    "variationspostscriptnameprefix",
];

/// The platforms, encodings and languages `readers.name` accepts
/// (font-otr.lua:218-470): only the English ones -- every other language in
/// the tables there is commented out.
fn platform(p: u32) -> Option<&'static str> {
    Some(match p {
        0 => "unicode",
        1 => "macintosh",
        2 => "iso",
        3 => "windows",
        4 => "custom",
        _ => return None,
    })
}

fn encoding(platform: &str, e: u32) -> Option<&'static str> {
    const UNICODE: [&str; 7] = [
        "unicode 1.0 semantics",
        "unicode 1.1 semantics",
        "iso/iec 10646",
        "unicode 2.0 bmp",
        "unicode 2.0 full",
        "unicode variation sequences",
        "unicode full repertoire",
    ];
    const MACINTOSH: [&str; 33] = [
        "roman",
        "japanese",
        "chinese (traditional)",
        "korean",
        "arabic",
        "hebrew",
        "greek",
        "russian",
        "rsymbol",
        "devanagari",
        "gurmukhi",
        "gujarati",
        "oriya",
        "bengali",
        "tamil",
        "telugu",
        "kannada",
        "malayalam",
        "sinhalese",
        "burmese",
        "khmer",
        "thai",
        "laotian",
        "georgian",
        "armenian",
        "chinese (simplified)",
        "tibetan",
        "mongolian",
        "geez",
        "slavic",
        "vietnamese",
        "sindhi",
        "uninterpreted",
    ];
    const ISO: [&str; 3] = ["7-bit ascii", "iso 10646", "iso 8859-1"];
    const WINDOWS: [&str; 11] = [
        "symbol",
        "unicode bmp",
        "shiftjis",
        "prc",
        "big5",
        "wansung",
        "johab",
        "reserved 7",
        "reserved 8",
        "reserved 9",
        "unicode ucs-4",
    ];
    let e = e as usize;
    match platform {
        "unicode" => UNICODE.get(e).copied(),
        "macintosh" => MACINTOSH.get(e).copied(),
        "iso" => ISO.get(e).copied(),
        "windows" => WINDOWS.get(e).copied(),
        _ => None,
    }
}

fn language(platform: &str, l: u32) -> Option<&'static str> {
    match (platform, l) {
        ("unicode", 0) => Some("english"),
        ("macintosh", 0) => Some("english"),
        ("windows", 0x0409) => Some("english - united states"),
        _ => None,
    }
}

/// `utf16_to_utf8_be` (fontloader-l-unicode.lua:926-971), for the Windows
/// encodings that have a decoder: a byte-order mark is dropped, a surrogate
/// pair joined, and a high surrogate waits for its partner.
fn utf16_be(s: &[u8]) -> Vec<u8> {
    if s.is_empty() {
        return Vec::new();
    }
    let s = match s.starts_with(&[0xFE, 0xFF]) {
        true => &s[2..],
        false => s,
    };
    let mut out = String::new();
    let mut more = 0u32;
    for pair in s.chunks_exact(2) {
        let now = 256 * pair[0] as u32 + pair[1] as u32;
        if more > 0 {
            let c = (more - 0xD800) * 0x400 + now.wrapping_sub(0xDC00) + 0x10000;
            more = 0;
            if let Some(c) = char::from_u32(c) {
                out.push(c);
            }
        } else if (0xD800..=0xDBFF).contains(&now) {
            more = now;
        } else if let Some(c) = char::from_u32(now) {
            out.push(c);
        }
    }
    out.into_bytes()
}

fn decode(platform: &str, encoding: &str, content: Vec<u8>) -> Vec<u8> {
    match (platform, encoding) {
        (
            "windows",
            "unicode semantics"
            | "unicode bmp"
            | "unicode full"
            | "unicode 1.0 semantics"
            | "unicode 1.1 semantics"
            | "unicode 2.0 bmp"
            | "unicode 2.0 full"
            | "unicode variation sequences"
            | "unicode full repertoire",
        ) => utf16_be(&content),
        _ => content,
    }
}

/// `loadtables` (font-otr.lua:2028-2071).
fn load_tables(f: &mut Stream, offset: usize, fontdata: &mut FontData) {
    f.set(offset);
    let _version = f.bytes(4);
    let noftables = f.u16();
    f.skip_u16(3);
    for _ in 0..noftables {
        let tag = f.bytes(4);
        let tag = String::from_utf8_lossy(&tag).trim().to_lowercase();
        let _checksum = f.u32();
        let offset = f.u32() as usize;
        let length = f.u32() as usize;
        fontdata.tables.insert(tag, TableEntry { offset, length });
    }
    fontdata.format =
        match fontdata.tables.contains_key("cff") || fontdata.tables.contains_key("cff2") {
            true => "opentype",
            false => "truetype",
        };
}

/// `readers.name` (font-otr.lua:808-954).
fn read_name(f: &mut Stream, fontdata: &mut FontData) {
    let Some(t) = fontdata.table("name") else {
        return;
    };
    f.set(t.offset);
    let _format = f.u16();
    let nofnames = f.u16();
    let offset = f.u16() as usize;
    let start = t.offset + offset;
    struct Record {
        platform: &'static str,
        encoding: &'static str,
        language: &'static str,
        name: Option<&'static str>,
        length: usize,
        offset: usize,
    }
    let mut lists: BTreeMap<&'static str, Vec<Record>> = BTreeMap::new();
    for _ in 0..nofnames {
        let p = f.u16();
        match platform(p).filter(|p| matches!(*p, "unicode" | "windows" | "macintosh")) {
            Some(p) => {
                let e = f.u16();
                let l = f.u16();
                match (encoding(p, e), language(p, l)) {
                    (Some(encoding), Some(language)) => {
                        let index = f.u16() as usize;
                        let length = f.u16() as usize;
                        let offset = start + f.u16() as usize;
                        lists.entry(p).or_default().push(Record {
                            platform: p,
                            encoding,
                            language,
                            name: RESERVED_NAMES.get(index).copied(),
                            length,
                            offset,
                        });
                    }
                    _ => f.skip_u16(3),
                }
            }
            None => f.skip_u16(5),
        }
    }
    let mut done: BTreeSet<String> = BTreeSet::new();
    let mut filter =
        |f: &mut Stream, fontdata: &mut FontData, p: &str, e: Option<&str>, l: Option<&str>| {
            let Some(list) = lists.get(p) else {
                return;
            };
            for (i, r) in list.iter().enumerate() {
                let key = r
                    .name
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("#{i}"));
                if done.contains(&key) {
                    continue;
                }
                if e.is_none_or(|e| e == r.encoding) && l.is_none_or(|l| l == r.language) {
                    f.set(r.offset);
                    let content = decode(r.platform, r.encoding, f.bytes(r.length));
                    if let Some(name) = r.name {
                        fontdata
                            .names
                            .insert(name.to_string(), NameEntry { content });
                    }
                    done.insert(key);
                }
            }
        };
    filter(
        f,
        fontdata,
        "windows",
        Some("unicode bmp"),
        Some("english - united states"),
    );
    filter(f, fontdata, "macintosh", Some("roman"), Some("english"));
    filter(f, fontdata, "windows", None, None);
    filter(f, fontdata, "macintosh", None, None);
    filter(f, fontdata, "unicode", None, None);
}

/// `readers["os/2"]` (font-otr.lua:974-1032).
fn read_os2(f: &mut Stream, fontdata: &mut FontData) {
    let Some(t) = fontdata.table("os/2") else {
        fontdata.windowsmetrics = Some(WindowsMetrics::default());
        return;
    };
    f.set(t.offset);
    let mut m = WindowsMetrics {
        version: f.u16(),
        averagewidth: f.i16(),
        weightclass: f.u16(),
        widthclass: f.u16(),
        fstype: f.u16(),
        ..WindowsMetrics::default()
    };
    // sub- and superscript sizes and offsets, strikeout, family class
    for _ in 0..11 {
        f.i16();
    }
    for k in 0..10 {
        m.panose[k] = f.u8();
    }
    f.u32s(4);
    f.bytes(4);
    m.fsselection = f.u16();
    f.u16();
    f.u16();
    m.typoascender = f.i16();
    m.typodescender = f.i16();
    m.typolinegap = f.i16();
    m.winascent = f.u16();
    m.windescent = f.u16();
    if m.version >= 1 {
        f.u32s(2);
    }
    if m.version >= 2 {
        m.xheight = Some(f.i16());
        m.capheight = Some(f.i16());
    }
    fontdata.windowsmetrics = Some(m);
}

/// `readers.head` (font-otr.lua:1034-1065).
fn read_head(f: &mut Stream, fontdata: &mut FontData) {
    if let Some(t) = fontdata.table("head") {
        f.set(t.offset);
        f.u32();
        f.u32();
        f.u32();
        f.u32();
        let _flags = f.u16();
        let units = f.u16();
        f.bytes(16);
        let h = FontHeader {
            units,
            xmin: f.i16(),
            ymin: f.i16(),
            xmax: f.i16(),
            ymax: f.i16(),
            macstyle: f.u16(),
            indextolocformat: {
                f.u16();
                f.i16();
                f.i16()
            },
        };
        fontdata.fontheader = Some(h);
    } else {
        fontdata.fontheader = Some(FontHeader::default());
    }
    fontdata.nofglyphs = 0;
}

/// `readers.maxp` (font-otr.lua:1133-1169).
fn read_maxp(f: &mut Stream, fontdata: &mut FontData) {
    if let Some(t) = fontdata.table("maxp") {
        f.set(t.offset);
        let version = f.u32();
        let nofglyphs = f.u16() as usize;
        fontdata.nofglyphs = nofglyphs;
        if version != 0x0000_5000 && version != 0x0001_0000 {
            // "fontdata.maximumprofile can be bad": the profile records no
            // glyphs, which `loca` then has to count itself.
            fontdata.nofglyphs = nofglyphs;
        }
    }
}

/// `readers.hhea` (font-otr.lua:1070-1097).
fn read_hhea(f: &mut Stream, fontdata: &mut FontData) {
    let Some(t) = fontdata.table("hhea") else {
        fontdata.horizontalheader = Some(HorizontalHeader::default());
        return;
    };
    f.set(t.offset);
    f.u32();
    let ascender = f.i16();
    let descender = f.i16();
    let linegap = f.i16();
    // maxadvancewidth .. reserved_4
    for _ in 0..11 {
        f.u16();
    }
    let _metricdataformat = f.i16();
    let nofmetrics = f.u16();
    fontdata.horizontalheader = Some(HorizontalHeader {
        ascender,
        descender,
        linegap,
        nofmetrics,
    });
}

/// `readers.hmtx` (font-otr.lua:1174-1209): a zero advance is not recorded,
/// and the last advance repeats for the glyphs past `nofmetrics`.
fn read_hmtx(f: &mut Stream, fontdata: &mut FontData) {
    let Some(t) = fontdata.table("hmtx") else {
        return;
    };
    f.set(t.offset);
    let nofmetrics = fontdata
        .horizontalheader
        .as_ref()
        .map_or(0, |h| h.nofmetrics);
    let nofglyphs = fontdata.nofglyphs as u32;
    let mut width = 0;
    for i in 0..nofmetrics {
        width = f.i16();
        let _lsb = f.i16();
        let glyph = fontdata.glyph(i);
        if width != 0 {
            glyph.width = Some(width as f64);
        }
    }
    for i in nofmetrics..nofglyphs {
        let glyph = fontdata.glyph(i);
        if width != 0 {
            glyph.width = Some(width as f64);
        }
    }
}

/// `standardromanencoding` (font-otr.lua:633-671), the 258 Macintosh names.
pub const STANDARD_ROMAN: [&str; 258] = [
    "notdef",
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

/// `readers.post` (font-otr.lua:1251-1308).
fn read_post(f: &mut Stream, fontdata: &mut FontData) {
    let Some(t) = fontdata.table("post") else {
        fontdata.postscript = Some(PostScript::default());
        return;
    };
    f.set(t.offset);
    let version = f.u32();
    let italicangle = f.fixed();
    let underlineposition = f.i16();
    let underlinethickness = f.i16();
    let monospaced = f.u32();
    f.u32s(4);
    fontdata.postscript = Some(PostScript {
        version,
        italicangle,
        underlineposition,
        underlinethickness,
        monospaced,
    });
    if version == 0x0001_0000 {
        for (index, name) in STANDARD_ROMAN.iter().enumerate() {
            fontdata.glyph(index as u32).name = Some(name.to_string());
        }
    } else if version == 0x0002_0000 {
        let nofglyphs = f.u16();
        let mut indices: BTreeMap<u32, u32> = BTreeMap::new();
        let mut maxnames = 0;
        for i in 0..nofglyphs {
            let nameindex = f.u16();
            if nameindex >= 258 {
                maxnames += 1;
                indices.insert(nameindex - 257, i);
            } else {
                fontdata.glyph(i).name = Some(STANDARD_ROMAN[nameindex as usize].to_string());
            }
        }
        for i in 1..=maxnames {
            let Some(&mapping) = indices.get(&i) else {
                // "quit post name fetching ... no index"
                break;
            };
            let length = f.u8() as usize;
            if length > 0 {
                let name = f.bytes(length);
                fontdata.glyph(mapping).name = Some(String::from_utf8_lossy(&name).into_owned());
            }
        }
    }
}

/// The cmap subtables `readers.cmap` tries, in order (font-otr.lua:1322-1338).
const CMAP_SEQUENCE: [(u32, u32, u32); 12] = [
    (3, 1, 4),
    (3, 10, 12),
    (0, 3, 4),
    (0, 3, 12),
    (0, 1, 4),
    (0, 1, 12),
    (0, 0, 6),
    (3, 0, 6),
    (3, 0, 4),
    (0, 5, 14),
    (0, 4, 12),
    (3, 10, 13),
];

/// The private use offset (`fonts.privateoffsets.textbase`).
pub const PRIVATE: u32 = 0xF0000;

/// A code point reaching a glyph: the first one is its `unicode`, a later
/// different one a duplicate of it (font-otr.lua:1398-1420).
fn assign(fontdata: &mut FontData, index: u32, unicode: u32, duplicates_too: bool) -> u32 {
    let glyph = fontdata.glyph(index);
    let mut done = 0;
    match glyph.unicode {
        None => {
            glyph.unicode = Some(unicode);
            done = 1;
        }
        Some(gu) if gu != unicode && duplicates_too => {
            fontdata.duplicates.entry(gu).or_default().insert(unicode);
        }
        _ => {}
    }
    fontdata.mapping.entry(index).or_insert(unicode);
    done
}

/// `formatreaders[4]` (font-otr.lua:1358-1461).
fn cmap4(f: &mut Stream, fontdata: &mut FontData, offset: usize) -> u32 {
    f.set(offset + 2);
    let length = f.u16() as i64;
    let _language = f.u16();
    let nofsegments = (f.u16() / 2) as usize;
    f.skip_u16(3);
    let endchars = f.u16s(nofsegments);
    let _reserved = f.u16();
    let startchars = f.u16s(nofsegments);
    let deltas = f.u16s(nofsegments);
    let offsets = f.u16s(nofsegments);
    let size = (length - 2 * 2 - 5 * 2 - 4 * 2 * nofsegments as i64) / 2;
    // readcardinaltable(f,size-1,ushort): a 1-based array.
    let count = (size - 1).max(0) as usize;
    let indices = f.u16s(count);
    let mut nofdone = 0;
    for segment in 1..=nofsegments {
        let startchar = startchars[segment - 1];
        let endchar = endchars[segment - 1];
        let off = offsets[segment - 1];
        let delta = deltas[segment - 1];
        if startchar == 0xFFFF && endchar == 0xFFFF {
        } else if startchar == 0xFFFF && off == 0 {
        } else if off == 0xFFFF {
        } else if off == 0 {
            for unicode in startchar..=endchar {
                let index = (unicode + delta) % 65536;
                if index > 0 {
                    nofdone += assign(fontdata, index, unicode, true);
                }
            }
        } else {
            // `shift = (segment-nofsegments+offset/2) - startchar`, a slot in
            // the 1-based `indices`.
            let shift = (segment as i64 - nofsegments as i64 + off as i64 / 2) - startchar as i64;
            for unicode in startchar..=endchar {
                let slot = shift + unicode as i64;
                let index = match slot >= 1 {
                    true => indices.get(slot as usize - 1).copied(),
                    false => None,
                };
                if let Some(index) = index.filter(|i| *i > 0) {
                    let index = (index + delta) % 65536;
                    nofdone += assign(fontdata, index, unicode, true);
                }
            }
        }
    }
    nofdone
}

/// `formatreaders[6]` (font-otr.lua:1463-1499): no duplicates recorded.
fn cmap6(f: &mut Stream, fontdata: &mut FontData, offset: usize) -> u32 {
    f.set(offset);
    f.u16();
    f.u16();
    f.u16();
    let start = f.u16();
    let count = f.u16();
    let mut nofdone = 0;
    for unicode in start..start + count {
        let index = f.u16();
        if index > 0 {
            nofdone += assign(fontdata, index, unicode, false);
        }
    }
    nofdone
}

/// `formatreaders[12]` (font-otr.lua:1501-1539).
fn cmap12(f: &mut Stream, fontdata: &mut FontData, offset: usize) -> u32 {
    f.set(offset + 2 + 2 + 4 + 4);
    let nofgroups = f.u32();
    let mut nofdone = 0;
    for _ in 0..nofgroups {
        let first = f.u32();
        let last = f.u32();
        let mut index = f.u32();
        for unicode in first..=last {
            nofdone += assign(fontdata, index, unicode, true);
            index += 1;
        }
    }
    nofdone
}

/// `formatreaders[13]` (font-otr.lua:1541-1586): many code points, one glyph.
fn cmap13(f: &mut Stream, fontdata: &mut FontData, offset: usize) -> u32 {
    f.set(offset + 2 + 2 + 4 + 4);
    let nofgroups = f.u32();
    let mut nofdone = 0;
    for _ in 0..nofgroups {
        let mut first = f.u32();
        let mut last = f.u32();
        let index = f.u32();
        if first < PRIVATE {
            let glyph = fontdata.glyph(index);
            let unicode = match glyph.unicode {
                Some(u) => u,
                None => {
                    glyph.unicode = Some(first);
                    first += 1;
                    first - 1
                }
            };
            fontdata.mapping.insert(index, unicode);
            let list = fontdata.duplicates.entry(unicode).or_default();
            if last >= PRIVATE {
                last = PRIVATE - 1;
            }
            for u in first..=last {
                list.insert(u);
            }
            nofdone += last.saturating_sub(first) + 1;
        }
    }
    nofdone
}

/// `formatreaders[14]` (font-otr.lua:1588-1640): non-default variation
/// sequences only.
fn cmap14(f: &mut Stream, fontdata: &mut FontData, offset: usize) -> u32 {
    if offset == 0 {
        return 0;
    }
    f.set(offset);
    f.u16();
    f.u32();
    let nofrecords = f.u32();
    let records: Vec<(u32, u32, u32)> = (0..nofrecords)
        .map(|_| (f.u24(), f.u32(), f.u32()))
        .collect();
    let mut nofdone = 0;
    for (selector, _default, other) in records {
        if other != 0 {
            f.set(offset + other as usize);
            let count = f.u32();
            let mut mapping = BTreeMap::new();
            for _ in 0..count {
                let u = f.u24();
                let g = f.u16();
                mapping.insert(u, g);
            }
            nofdone += count;
            fontdata.variants.insert(selector, mapping);
        }
    }
    nofdone
}

/// `readers.cmap` (font-otr.lua:1687-1788).
fn read_cmap(f: &mut Stream, fontdata: &mut FontData) {
    let Some(t) = fontdata.table("cmap") else {
        return;
    };
    let tableoffset = t.offset;
    f.set(tableoffset);
    let _version = f.u16();
    let noftables = f.u16();
    // platform -> encoding -> the subtables' offsets, in file order
    let mut records: BTreeMap<u32, BTreeMap<u32, Vec<usize>>> = BTreeMap::new();
    for _ in 0..noftables {
        let p = f.u16();
        let e = f.u16();
        let o = f.u32() as usize;
        records.entry(p).or_default().entry(e).or_default().push(o);
    }
    // formats[format] = offset: a later subtable of the same format wins.
    let mut formats: BTreeMap<(u32, u32), BTreeMap<u32, usize>> = BTreeMap::new();
    for (&p, encs) in &records {
        for (&e, offsets) in encs {
            let entry = formats.entry((p, e)).or_default();
            for &o in offsets {
                let offset = tableoffset + o;
                f.set(offset);
                entry.insert(f.u16(), offset);
            }
        }
    }
    let mut done: BTreeSet<(u32, u32, u32)> = BTreeSet::new();
    for &(p, e, fmt) in CMAP_SEQUENCE.iter() {
        let Some(&offset) = formats.get(&(p, e)).and_then(|m| m.get(&fmt)) else {
            continue;
        };
        if !done.insert((p, e, fmt)) {
            continue;
        }
        let _ = match fmt {
            4 => cmap4(f, fontdata, offset),
            6 => cmap6(f, fontdata, offset),
            12 => cmap12(f, fontdata, offset),
            13 => cmap13(f, fontdata, offset),
            14 => cmap14(f, fontdata, offset),
            _ => 0,
        };
    }
}

/// `readers.loca` (font-ttf.lua:1131-1162).
fn read_loca(f: &mut Stream, fontdata: &mut FontData) {
    let (Some(loca), Some(glyf)) = (fontdata.table("loca"), fontdata.table("glyf")) else {
        return;
    };
    let format = fontdata
        .fontheader
        .as_ref()
        .map_or(0, |h| h.indextolocformat);
    let mut nofglyphs = fontdata.nofglyphs;
    f.set(loca.offset);
    let mut locations = Vec::new();
    if format == 1 {
        if nofglyphs == 0 {
            nofglyphs = (loca.length / 4).saturating_sub(1);
        }
        for _ in 0..=nofglyphs {
            locations.push(glyf.offset + f.u32() as usize);
        }
    } else {
        if nofglyphs == 0 {
            nofglyphs = (loca.length / 2).saturating_sub(1);
        }
        for _ in 0..=nofglyphs {
            locations.push(glyf.offset + f.u16() as usize * 2);
        }
    }
    fontdata.nofglyphs = nofglyphs;
    fontdata.locations = locations;
}

/// `readers.glyf` (font-ttf.lua:1164-1231), the bounding boxes only: the
/// shapes are read for embedding or an instance, which node mode's metrics do
/// not need.
fn read_glyf(f: &mut Stream, fontdata: &mut FontData) {
    if fontdata.table("glyf").is_none() || fontdata.locations.is_empty() {
        return;
    }
    let nofglyphs = fontdata.nofglyphs;
    let filesize = fontdata.filesize;
    for index in 0..nofglyphs {
        let location = fontdata.locations[index];
        let next = fontdata
            .locations
            .get(index + 1)
            .copied()
            .unwrap_or(location);
        let length = next as i64 - location as i64;
        if location >= filesize {
            // "discarding %s glyphs due to glyph location bug"
            fontdata.nofglyphs = index.saturating_sub(1);
            fontdata.badfont = true;
            break;
        } else if length > 0 {
            f.set(location);
            let _contours = f.i16();
            let bb = [
                f.i16() as f64,
                f.i16() as f64,
                f.i16() as f64,
                f.i16() as f64,
            ];
            fontdata.glyph(index as u32).boundingbox = Some(bb);
        } else {
            fontdata.glyph(index as u32).boundingbox = Some([0.0; 4]);
        }
    }
}

/// `readdata` (font-otr.lua:2093-2238) for the tables node mode's metrics
/// read, from the font at `offset` (a collection's face).
pub fn read(data: &[u8], offset: usize) -> (FontData, super::dsp::Layout) {
    let mut f = Stream::new(data);
    let mut fontdata = FontData {
        filesize: data.len(),
        ..FontData::default()
    };
    let mut layout = super::dsp::Layout::default();
    load_tables(&mut f, offset, &mut fontdata);
    read_name(&mut f, &mut fontdata);
    read_os2(&mut f, &mut fontdata);
    read_head(&mut f, &mut fontdata);
    read_maxp(&mut f, &mut fontdata);
    read_hhea(&mut f, &mut fontdata);
    read_hmtx(&mut f, &mut fontdata);
    read_post(&mut f, &mut fontdata);
    super::dsp::read_gdef(&mut f, &mut fontdata, &mut layout);
    super::cff::read(&mut f, &mut fontdata);
    read_cmap(&mut f, &mut fontdata);
    read_loca(&mut f, &mut fontdata);
    read_glyf(&mut f, &mut fontdata);
    super::dsp::read_scripts(&mut f, &mut fontdata, &mut layout, "gsub");
    super::dsp::read_scripts(&mut f, &mut fontdata, &mut layout, "gpos");
    super::dsp::read_kern(&mut f, &mut fontdata, &mut layout);
    super::dsp::read_math(&mut f, &mut fontdata, &mut layout);
    (fontdata, layout)
}

/// The faces of a file: one offset, or a collection's (font-otr.lua:2254-2305).
pub fn face_offsets(data: &[u8]) -> Vec<usize> {
    let mut f = Stream::new(data);
    match &f.bytes(4)[..] {
        b"OTTO" | b"true" | [0, 1, 0, 0] => vec![0],
        b"ttcf" => {
            let _version = f.u32();
            let n = f.u32() as usize;
            f.u32s(n).into_iter().map(|o| o as usize).collect()
        }
        _ => Vec::new(),
    }
}
