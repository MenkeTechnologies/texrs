//! A CFF font's subset as luatex embeds it for a CID font
//! (`/FontFile3 /Subtype /CIDFontType0C`): `write_cff` and `write_cid_cff`
//! of LuaTeX 1.24's font/writecff.c, which `writetype0` calls for an
//! OpenType font with CFF outlines.
//!
//! A name-keyed font becomes CID-keyed (ROS Adobe-Identity-0) with the
//! glyph indices as CIDs; a CID-keyed one keeps its CIDs. Either way every
//! charstring has its subroutine calls replaced by the subroutines' code,
//! the subroutine indexes are dropped, and the font is written again with
//! its Top DICT, strings, charset, FDSelect, FDArray and Private DICTs
//! rebuilt in luatex's order. CFF2 is not handled here (luatex's own CFF2
//! path cannot blend either).

use std::collections::BTreeSet;

/// writecff.c:50-144: the standard strings.
const STDSTR: [&str; 391] = [
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArgType {
    Number,
    Boolean,
    Sid,
    Array,
    Delta,
    Ros,
    Offset,
    SzOff,
}

const LAST_OP1: usize = 26;
const LAST_OP: usize = LAST_OP1 + 39;

/// writecff.c:539-610: the DICT operators, `12 x` ones from 26.
fn operator(id: usize) -> Option<(&'static str, ArgType)> {
    use ArgType::*;
    const OPS: [Option<(&str, ArgType)>; LAST_OP] = [
        Some(("version", Sid)),
        Some(("Notice", Sid)),
        Some(("FullName", Sid)),
        Some(("FamilyName", Sid)),
        Some(("Weight", Sid)),
        Some(("FontBBox", Array)),
        Some(("BlueValues", Delta)),
        Some(("OtherBlues", Delta)),
        Some(("FamilyBlues", Delta)),
        Some(("FamilyOtherBlues", Delta)),
        Some(("StdHW", Number)),
        Some(("StdVW", Number)),
        None,
        Some(("UniqueID", Number)),
        Some(("XUID", Array)),
        Some(("charset", Offset)),
        Some(("Encoding", Offset)),
        Some(("CharStrings", Offset)),
        Some(("Private", SzOff)),
        Some(("Subrs", Offset)),
        Some(("defaultWidthX", Number)),
        Some(("nominalWidthX", Number)),
        None,
        None,
        Some(("vstore", Offset)),
        Some(("maxstack", Number)),
        Some(("Copyright", Sid)),
        Some(("IsFixedPitch", Boolean)),
        Some(("ItalicAngle", Number)),
        Some(("UnderlinePosition", Number)),
        Some(("UnderlineThickness", Number)),
        Some(("PaintType", Number)),
        Some(("CharstringType", Number)),
        Some(("FontMatrix", Array)),
        Some(("StrokeWidth", Number)),
        Some(("BlueScale", Number)),
        Some(("BlueShift", Number)),
        Some(("BlueFuzz", Number)),
        Some(("StemSnapH", Delta)),
        Some(("StemSnapV", Delta)),
        Some(("ForceBold", Boolean)),
        None,
        None,
        Some(("LanguageGroup", Number)),
        Some(("ExpansionFactor", Number)),
        Some(("InitialRandomSeed", Number)),
        Some(("SyntheticBase", Number)),
        Some(("PostScript", Sid)),
        Some(("BaseFontName", Sid)),
        Some(("BaseFontBlend", Delta)),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(("ROS", Ros)),
        Some(("CIDFontVersion", Number)),
        Some(("CIDFontRevision", Number)),
        Some(("CIDFontType", Number)),
        Some(("CIDCount", Number)),
        Some(("UIDBase", Number)),
        Some(("FDArray", Offset)),
        Some(("FDSelect", Offset)),
        Some(("FontName", Sid)),
    ];
    OPS.get(id).copied().flatten()
}

fn op_id(key: &str) -> Option<usize> {
    (0..LAST_OP).find(|&i| operator(i).is_some_and(|(k, _)| k == key))
}

#[derive(Clone, Debug)]
struct Entry {
    id: usize,
    values: Vec<f64>,
}

impl Entry {
    fn key(&self) -> &'static str {
        operator(self.id).map_or("", |o| o.0)
    }
}

#[derive(Clone, Debug, Default)]
struct Dict {
    entries: Vec<Entry>,
}

impl Dict {
    /// `cff_dict_known`.
    fn known(&self, key: &str) -> bool {
        self.entries
            .iter()
            .any(|e| e.key() == key && !e.values.is_empty())
    }
    /// `cff_dict_get`.
    fn get(&self, key: &str, i: usize) -> Result<f64, String> {
        let e = self
            .entries
            .iter()
            .find(|e| e.key() == key)
            .ok_or_else(|| format!("cff: DICT entry '{key}' not found"))?;
        e.values
            .get(i)
            .copied()
            .ok_or_else(|| "cff: invalid index number".to_string())
    }
    /// `cff_dict_set`.
    fn set(&mut self, key: &str, i: usize, v: f64) -> Result<(), String> {
        let e = self
            .entries
            .iter_mut()
            .find(|e| e.key() == key)
            .ok_or_else(|| format!("cff: DICT entry '{key}' not found"))?;
        match e.values.get_mut(i) {
            Some(x) => {
                *x = v;
                Ok(())
            }
            None => Err("cff: invalid index number".into()),
        }
    }
    /// `cff_dict_add`: a new entry of `count` zeros, unless there is one
    /// (which must have `count` values; a removed one has none).
    fn add(&mut self, key: &str, count: usize) -> Result<(), String> {
        let id = op_id(key).ok_or("cff: unknown DICT operator")?;
        if let Some(e) = self.entries.iter().find(|e| e.id == id) {
            if e.values.len() != count {
                return Err("cff: inconsistent DICT argument number".into());
            }
            return Ok(());
        }
        self.entries.push(Entry {
            id,
            values: vec![0.0; count],
        });
        Ok(())
    }
    /// `cff_dict_remove`: the entry stays, with no values.
    fn remove(&mut self, key: &str) {
        for e in self.entries.iter_mut().filter(|e| e.key() == key) {
            e.values.clear();
        }
    }
}

/// `cff_dict_unpack` (writecff.c:796-834).
fn dict_unpack(data: &[u8]) -> Result<Dict, String> {
    let mut dict = Dict::default();
    let mut stack: Vec<f64> = Vec::new();
    let mut p = 0;
    let end = data.len();
    while p < end {
        let b0 = data[p];
        if (b0 as usize) < LAST_OP1 {
            // add_dict
            let mut id = b0 as usize;
            if id == 0x0c {
                p += 1;
                if p >= end {
                    return Err("cff: parsing DICT failed".into());
                }
                id = data[p] as usize + LAST_OP1;
                if id >= LAST_OP {
                    return Err("cff: parsing DICT failed".into());
                }
            }
            let (name, argtype) = operator(id).ok_or("cff: parsing DICT failed")?;
            match argtype {
                ArgType::Number | ArgType::Boolean | ArgType::Sid | ArgType::Offset => {
                    let v = stack
                        .pop()
                        .ok_or("cff: parsing DICT failed (stack underflow)")?;
                    dict.entries.push(Entry {
                        id,
                        values: vec![v],
                    });
                }
                _ => {
                    // an operator without operands is ignored
                    if !stack.is_empty() {
                        let mut values = std::mem::take(&mut stack);
                        if values.len() > 3 && name == "FontMatrix" {
                            values[0] = 0.001;
                            values[3] = 0.001;
                        }
                        dict.entries.push(Entry { id, values });
                    }
                }
            }
            p += 1;
        } else if b0 == 30 {
            if stack.len() >= 64 {
                return Err("cff: parsing DICT failed (stack overflow)".into());
            }
            stack.push(get_real(data, &mut p)?);
        } else if b0 == 255 || (LAST_OP1 as u8..=27).contains(&b0) {
            p += 1;
        } else {
            if stack.len() >= 64 {
                return Err("cff: parsing DICT failed (stack overflow)".into());
            }
            stack.push(get_integer(data, &mut p)?);
        }
    }
    Ok(dict)
}

/// `get_integer` (writecff.c:614-651), with its bounds tests.
fn get_integer(data: &[u8], p: &mut usize) -> Result<f64, String> {
    let end = data.len();
    let b0 = data[*p];
    *p += 1;
    let b = |i: usize| data.get(i).copied().unwrap_or(0) as i64;
    let r = if b0 == 28 && (*p as i64) < end as i64 - 2 {
        let mut r = b(*p) * 256 + b(*p + 1);
        *p += 2;
        if r > 0x7fff {
            r -= 0x10000;
        }
        r
    } else if b0 == 29 && (*p as i64) < end as i64 - 4 {
        let mut r = b(*p);
        if r > 0x7f {
            r -= 0x100;
        }
        for i in 1..4 {
            r = r * 256 + b(*p + i);
        }
        *p += 4;
        r
    } else if (32..=246).contains(&b0) {
        b0 as i64 - 139
    } else if (247..=250).contains(&b0) {
        let r = (b0 as i64 - 247) * 256 + b(*p) + 108;
        *p += 1;
        r
    } else if (251..=254).contains(&b0) {
        let r = -(b0 as i64 - 251) * 256 - b(*p) - 108;
        *p += 1;
        r
    } else {
        return Err("cff: parsing DICT failed".into());
    };
    Ok(r as f64)
}

/// `get_real` (writecff.c:655-716): the nibbles read as `strtod` reads
/// them.
fn get_real(data: &[u8], p: &mut usize) -> Result<f64, String> {
    let end = data.len();
    if data[*p] != 30 || *p + 1 >= end {
        return Err("cff: parsing DICT failed".into());
    }
    *p += 1;
    let mut s = String::new();
    let mut pos = 0;
    let mut nibble = 0;
    while *p < end {
        nibble = if pos % 2 == 1 {
            let n = data[*p] & 0x0f;
            *p += 1;
            n
        } else {
            (data[*p] >> 4) & 0x0f
        };
        match nibble {
            0..=9 => s.push((b'0' + nibble) as char),
            0x0a => s.push('.'),
            0x0b => s.push('e'),
            0x0c => s.push_str("e-"),
            0x0e => s.push('-'),
            0x0d => {}
            0x0f => {
                if pos % 2 == 0 && data.get(*p) != Some(&0xff) {
                    return Err("cff: parsing DICT failed".into());
                }
                break;
            }
            _ => return Err("cff: parsing DICT failed".into()),
        }
        pos += 1;
    }
    if nibble != 0x0f {
        return Err("cff: parsing DICT failed".into());
    }
    // a real ending on the high nibble has its byte still to skip
    if pos % 2 == 0 {
        *p += 1;
    }
    strtod(&s).ok_or_else(|| "cff: parsing DICT failed".into())
}

/// `strtod` of the longest valid prefix.
fn strtod(s: &str) -> Option<f64> {
    (1..=s.len()).rev().find_map(|n| s[..n].parse::<f64>().ok())
}

#[derive(Clone, Debug, Default)]
struct Index {
    count: u32,
    offset: Vec<u32>,
    data: Vec<u8>,
}

impl Index {
    /// `cff_index_size`.
    fn size(&self) -> usize {
        if self.count == 0 {
            return 2;
        }
        let datalen = self.offset[self.count as usize] - 1;
        let offsize = offsize_of(datalen);
        3 + offsize * (self.count as usize + 1) + datalen as usize
    }
    /// `cff_pack_index`.
    fn pack(&self, out: &mut Vec<u8>) {
        if self.count == 0 {
            out.extend_from_slice(&[0, 0]);
            return;
        }
        let datalen = self.offset[self.count as usize] - 1;
        let offsize = offsize_of(datalen);
        out.extend_from_slice(&(self.count as u16).to_be_bytes());
        out.push(offsize as u8);
        for &o in &self.offset[..=self.count as usize] {
            out.extend_from_slice(&o.to_be_bytes()[4 - offsize..]);
        }
        out.extend_from_slice(&self.data[..datalen as usize]);
    }
    fn item(&self, i: usize) -> &[u8] {
        let a = self.offset[i] as usize - 1;
        let b = self.offset[i + 1] as usize - 1;
        &self.data[a..b]
    }
}

fn offsize_of(datalen: u32) -> usize {
    if datalen < 0xff {
        1
    } else if datalen < 0xffff {
        2
    } else if datalen < 0xff_ffff {
        3
    } else {
        4
    }
}

struct Reader<'a> {
    s: &'a [u8],
    p: usize,
}

impl Reader<'_> {
    fn card8(&mut self) -> u32 {
        let b = self.s.get(self.p).copied().unwrap_or(0);
        self.p += 1;
        b as u32
    }
    fn unsigned(&mut self, n: usize) -> u32 {
        (0..n).fold(0u32, |v, _| v.wrapping_mul(256).wrapping_add(self.card8()))
    }
    /// `cff_get_index`.
    fn index(&mut self) -> Result<Index, String> {
        let count = self.unsigned(2);
        if count == 0 {
            return Ok(Index::default());
        }
        let offsize = self.card8() as usize;
        if !(1..=4).contains(&offsize) {
            return Err("cff: invalid offsize data".into());
        }
        let offset: Vec<u32> = (0..=count).map(|_| self.unsigned(offsize)).collect();
        if offset[0] != 1 {
            return Err("cff: invalid index offset data".into());
        }
        let len = (offset[count as usize] - offset[0]) as usize;
        let data = self
            .s
            .get(self.p..self.p + len)
            .ok_or("cff: index past the end")?
            .to_vec();
        self.p += len;
        Ok(Index {
            count,
            offset,
            data,
        })
    }
    /// `cff_get_index_header`: the offsets only.
    fn index_header(&mut self) -> Result<Index, String> {
        let count = self.unsigned(2);
        if count == 0 {
            return Ok(Index::default());
        }
        let offsize = self.card8() as usize;
        if !(1..=4).contains(&offsize) {
            return Err("cff: invalid offsize data".into());
        }
        let offset: Vec<u32> = (0..=count).map(|_| self.unsigned(offsize)).collect();
        if offset[0] != 1 {
            return Err("cff: invalid index data".into());
        }
        Ok(Index {
            count,
            offset,
            data: Vec::new(),
        })
    }
}

/// `cff_font` as `read_cff` leaves it and the writers extend it.
struct Font<'a> {
    stream: &'a [u8],
    minor: u8,
    name: Index,
    topdict: Dict,
    string: Index,
    gsubr_offset: usize,
    num_glyphs: u32,
    cid: bool,
}

/// `read_cff` (writecff.c:1021-1130) for the first font of the set.
fn read_cff(stream: &[u8]) -> Result<Font<'_>, String> {
    let mut r = Reader { s: stream, p: 0 };
    let major = r.card8();
    let minor = r.card8() as u8;
    let hdr_size = r.card8() as usize;
    if major != 1 {
        return Err(format!("cff: major version {major} not supported here"));
    }
    let offsize = r.card8();
    if !(1..=4).contains(&offsize) {
        return Err("cff: invalid offsize data (4)".into());
    }
    r.p = hdr_size;
    let name = r.index()?;
    let top = r.index()?;
    if top.count == 0 {
        return Err("cff: top DICT not exist".into());
    }
    let topdict = dict_unpack(top.item(0))?;
    if topdict.known("CharstringType") && topdict.get("CharstringType", 0)? != 2.0 {
        return Err("cff: only type 2 charstrings supported".into());
    }
    if topdict.known("SyntheticBase") {
        return Err("cff: synthetic font not supported".into());
    }
    let string = r.index()?;
    let gsubr_offset = r.p;
    r.p = topdict.get("CharStrings", 0)? as usize;
    let num_glyphs = r.unsigned(2);
    let cid = topdict.known("ROS");
    Ok(Font {
        stream,
        minor,
        name,
        topdict,
        string,
        gsubr_offset,
        num_glyphs,
        cid,
    })
}

impl Font<'_> {
    /// `cff_get_string`.
    fn get_string(&self, id: usize) -> Option<String> {
        if id < 391 {
            return Some(STDSTR[id].to_string());
        }
        let i = id - 391;
        (i < self.string.count as usize)
            .then(|| String::from_utf8_lossy(self.string.item(i)).into_owned())
    }
}

/// The new string INDEX (`cff->_string`) being built.
#[derive(Default)]
struct Strings {
    list: Vec<String>,
}

impl Strings {
    /// `cff_add_string`: an existing new string, a standard one, or a new SID.
    fn add(&mut self, s: &str) -> usize {
        if let Some(i) = self.list.iter().position(|x| x == s) {
            return i + 391;
        }
        if let Some(i) = STDSTR.iter().position(|x| *x == s) {
            return i;
        }
        self.list.push(s.to_string());
        self.list.len() - 1 + 391
    }
    fn index(&self) -> Index {
        let mut offset = vec![1u32];
        let mut data = Vec::new();
        for s in &self.list {
            data.extend_from_slice(s.as_bytes());
            offset.push(data.len() as u32 + 1);
        }
        Index {
            count: self.list.len() as u32,
            offset,
            data,
        }
    }
    /// `cff_get_sid` against the new strings (after `cff_update_string`).
    fn sid(&self, s: &str) -> Option<usize> {
        self.list
            .iter()
            .position(|x| x == s)
            .map(|i| i + 391)
            .or_else(|| STDSTR.iter().position(|x| *x == s))
    }
}

/// `cff_dict_update`: a DICT's SIDs moved into the new strings.
fn dict_update(dict: &mut Dict, font: &Font, strings: &mut Strings) {
    for e in dict.entries.iter_mut().filter(|e| !e.values.is_empty()) {
        match operator(e.id).map(|o| o.1) {
            Some(ArgType::Sid) => {
                if let Some(s) = font.get_string(e.values[0] as usize) {
                    e.values[0] = strings.add(&s) as f64;
                }
            }
            Some(ArgType::Ros) => {
                for k in 0..2 {
                    if let Some(s) = font.get_string(e.values[k] as usize) {
                        e.values[k] = strings.add(&s) as f64;
                    }
                }
            }
            _ => {}
        }
    }
}

/// `pack_integer` (writecff.c:1134-1176).
fn pack_integer(out: &mut Vec<u8>, v: i64) {
    if (-107..=107).contains(&v) {
        out.push((v + 139) as u8);
    } else if (108..=1131).contains(&v) {
        let v = 0xf700 + v - 108;
        out.extend_from_slice(&[(v >> 8) as u8, v as u8]);
    } else if (-1131..=-108).contains(&v) {
        let v = 0xfb00 - v - 108;
        out.extend_from_slice(&[(v >> 8) as u8, v as u8]);
    } else if (-32768..=32767).contains(&v) {
        out.extend_from_slice(&[28, (v >> 8) as u8, v as u8]);
    } else {
        out.extend_from_slice(&[
            29,
            (v >> 24) as u8,
            (v >> 16) as u8,
            (v >> 8) as u8,
            v as u8,
        ]);
    }
}

/// C's `%1.14g` for a value in [1, 10).
fn g14(v: f64) -> String {
    let e = format!("{v:.13e}");
    let (mant, exp) = e.split_once('e').unwrap();
    let x: i32 = exp.parse().unwrap();
    if !(-4..14).contains(&x) {
        let mant = mant.trim_end_matches('0').trim_end_matches('.');
        return format!("{mant}e{}{:02}", if x < 0 { '-' } else { '+' }, x.abs());
    }
    let prec = (13 - x).max(0) as usize;
    let s = format!("{v:.prec$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// `pack_real` (writecff.c:1178-1292).
fn pack_real(out: &mut Vec<u8>, value: f64) {
    let mut buf = vec![0u8; 32];
    buf[0] = 30;
    if value == 0.0 {
        out.extend_from_slice(&[30, 0x0f]);
        return;
    }
    let mut v = value;
    let mut pos = 2usize;
    if v < 0.0 {
        buf[1] = 0xe0;
        v = -v;
        pos += 1;
    }
    let mut e: i64 = 0;
    if v >= 10.0 {
        while v >= 10.0 {
            v /= 10.0;
            e += 1;
        }
    } else if v < 1.0 {
        while v < 1.0 {
            v *= 10.0;
            e -= 1;
        }
    }
    let put = |buf: &mut Vec<u8>, pos: &mut usize, ch: u8| {
        if *pos % 2 == 1 {
            buf[*pos / 2] = buf[*pos / 2].wrapping_add(ch);
        } else {
            buf[*pos / 2] = ch << 4;
        }
        *pos += 1;
    };
    for c in g14(v).bytes().take(17) {
        let ch = match c {
            b'.' => 0x0a,
            b'0'..=b'9' => c - b'0',
            _ => break,
        };
        put(&mut buf, &mut pos, ch);
    }
    if e > 0 {
        put(&mut buf, &mut pos, 0x0b);
    } else if e < 0 {
        put(&mut buf, &mut pos, 0x0c);
        e = -e;
    }
    if e != 0 {
        for c in e.to_string().bytes() {
            put(&mut buf, &mut pos, c - b'0');
        }
    }
    if pos % 2 == 1 {
        buf[pos / 2] = buf[pos / 2].wrapping_add(0x0f);
        pos += 1;
    } else {
        buf[pos / 2] = 0xff;
        pos += 2;
    }
    out.extend_from_slice(&buf[..pos / 2]);
}

/// `cff_dict_put_number`.
fn put_number(out: &mut Vec<u8>, value: f64, offset: bool) {
    let nearint = (value + 0.5).floor();
    if offset {
        let l = value as i64;
        out.extend_from_slice(&[
            29,
            (l >> 24) as u8,
            (l >> 16) as u8,
            (l >> 8) as u8,
            l as u8,
        ]);
    } else if value > i32::MAX as f64 || value < i32::MIN as f64 || (value - nearint).abs() > 1.0e-5
    {
        pack_real(out, value);
    } else {
        pack_integer(out, nearint as i64);
    }
}

/// `put_dict_entry`.
fn put_entry(out: &mut Vec<u8>, e: &Entry) {
    if e.values.is_empty() {
        return;
    }
    let offset = matches!(
        operator(e.id).map(|o| o.1),
        Some(ArgType::Offset | ArgType::SzOff)
    );
    for &v in &e.values {
        put_number(out, v, offset);
    }
    if e.id < LAST_OP1 {
        out.push(e.id as u8);
    } else {
        out.extend_from_slice(&[12, (e.id - LAST_OP1) as u8]);
    }
}

/// `cff_dict_pack`: ROS first, then the rest in order.
fn dict_pack(d: &Dict) -> Vec<u8> {
    let mut out = Vec::new();
    if let Some(e) = d.entries.iter().find(|e| e.key() == "ROS") {
        put_entry(&mut out, e);
    }
    for e in d.entries.iter().filter(|e| e.key() != "ROS") {
        put_entry(&mut out, e);
    }
    out
}

/// The Type 2 charstring copier (writecff.c:1857-2460): operands and
/// operators passed through, subroutine calls replaced by their code.
struct Cs {
    stack: Vec<f64>,
    trn: [f64; 32],
    phase: u8,
    num_stems: usize,
    nest: usize,
    status: Status,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    SubrReturn,
    CharEnd,
}

impl Cs {
    /// `clear_stack`.
    fn clear_stack(&mut self, out: &mut Vec<u8>) -> Result<(), String> {
        for &value in &self.stack {
            let ivalue = (value + 0.5).floor() as i64;
            if value >= 32768.0 || value <= -32769.0 {
                return Err("cff: argument value too large (this is bug)".into());
            } else if (value - ivalue as f64).abs() > 3.0e-5 {
                let m = value.floor() as i64;
                let f = ((value - m as f64) * 65536.0) as i64;
                out.extend_from_slice(&[255, (m >> 8) as u8, m as u8, (f >> 8) as u8, f as u8]);
            } else if (-107..=107).contains(&ivalue) {
                out.push((ivalue + 139) as u8);
            } else if (108..=1131).contains(&ivalue) {
                let v = 0xf700 + ivalue - 108;
                out.extend_from_slice(&[(v >> 8) as u8, v as u8]);
            } else if (-1131..=-108).contains(&ivalue) {
                let v = 0xfb00 - ivalue - 108;
                out.extend_from_slice(&[(v >> 8) as u8, v as u8]);
            } else {
                out.extend_from_slice(&[28, (ivalue >> 8) as u8, ivalue as u8]);
            }
        }
        self.stack.clear();
        Ok(())
    }

    fn push(&mut self, v: f64) -> Result<(), String> {
        if self.stack.len() >= 48 {
            return Err("cff: charstring stack overflow".into());
        }
        self.stack.push(v);
        Ok(())
    }

    fn need(&self, n: usize) -> Result<(), String> {
        if self.stack.len() < n {
            return Err("cff: charstring stack underflow".into());
        }
        Ok(())
    }

    /// `do_operator1`.
    fn op1(&mut self, out: &mut Vec<u8>, data: &[u8], p: &mut usize) -> Result<(), String> {
        let op = data[*p];
        *p += 1;
        match op {
            1 | 3 | 18 | 23 => {
                self.num_stems += self.stack.len() / 2;
                self.clear_stack(out)?;
                out.push(op);
                self.phase = 1;
            }
            19 | 20 => {
                if self.phase < 2 {
                    self.num_stems += self.stack.len() / 2;
                }
                self.clear_stack(out)?;
                out.push(op);
                if self.num_stems > 0 {
                    let masklen = self.num_stems.div_ceil(8);
                    let mask = data
                        .get(*p..*p + masklen)
                        .ok_or("cff: parsing charstring failed")?;
                    out.extend_from_slice(mask);
                    *p += masklen;
                }
                self.phase = 2;
            }
            21 | 22 | 4 => {
                self.clear_stack(out)?;
                out.push(op);
                self.phase = 2;
            }
            14 => {
                match self.stack.len() {
                    1 => self.clear_stack(out)?,
                    4 | 5 => {
                        return Err("cff: 'seac' character deprecated in type 2 charstring".into())
                    }
                    _ => {}
                }
                out.push(op);
                self.status = Status::CharEnd;
            }
            15 | 16 => return Err("cff2: unsupported setvindex/blend operator".into()),
            5..=8 | 24..=27 | 30 | 31 => {
                if self.phase < 2 {
                    return Err("cff: broken type 2 charstring".into());
                }
                self.clear_stack(out)?;
                out.push(op);
            }
            10 | 11 | 29 => return Err("cff: unexpected subroutine operator".into()),
            _ => return Err(format!("cff: unknown charstring operator: 0x{op:02x}")),
        }
        Ok(())
    }

    /// `do_operator2`.
    fn op2(&mut self, out: &mut Vec<u8>, data: &[u8], p: &mut usize) -> Result<(), String> {
        *p += 1;
        let op = *data.get(*p).ok_or("cff: parsing charstring failed")?;
        *p += 1;
        let n = self.stack.len();
        match op {
            0 => return Err("cff: Operator 'dotsection' deprecated in type 2 charstring".into()),
            34..=37 => {
                if self.phase < 2 {
                    return Err("cff: broken type 2 charstring".into());
                }
                self.clear_stack(out)?;
                out.extend_from_slice(&[12, op]);
            }
            3 | 4 | 15 => {
                self.need(2)?;
                let (a, b) = (self.stack[n - 1], self.stack[n - 2]);
                self.stack.pop();
                let r = match op {
                    3 => a != 0.0 && b != 0.0,
                    4 => a != 0.0 || b != 0.0,
                    _ => a == b,
                };
                self.stack[n - 2] = if r { 1.0 } else { 0.0 };
            }
            5 => {
                self.need(1)?;
                self.stack[n - 1] = if self.stack[n - 1] != 0.0 { 0.0 } else { 1.0 };
            }
            9 => {
                self.need(1)?;
                self.stack[n - 1] = self.stack[n - 1].abs();
            }
            10 | 11 | 12 | 24 => {
                self.need(2)?;
                let b = self.stack.pop().unwrap();
                let a = &mut self.stack[n - 2];
                match op {
                    10 => *a += b,
                    11 => *a -= b,
                    12 => *a /= b,
                    _ => *a *= b,
                }
            }
            14 => {
                self.need(1)?;
                self.stack[n - 1] *= -1.0;
            }
            18 => {
                self.need(1)?;
                self.stack.pop();
            }
            20 => {
                self.need(2)?;
                let idx = self.stack.pop().unwrap() as i64;
                let v = self.stack.pop().unwrap();
                if !(0..32).contains(&idx) {
                    return Err("cff: charstring stack error".into());
                }
                self.trn[idx as usize] = v;
            }
            21 => {
                self.need(1)?;
                let idx = self.stack[n - 1] as i64;
                if !(0..32).contains(&idx) {
                    return Err("cff: charstring stack error".into());
                }
                self.stack[n - 1] = self.trn[idx as usize];
            }
            22 => {
                self.need(4)?;
                let top = n - 3;
                if self.stack[top + 1] > self.stack[top + 2] {
                    self.stack[top - 1] = self.stack[top];
                }
                self.stack.truncate(top);
            }
            26 => {
                self.need(1)?;
                self.stack[n - 1] = self.stack[n - 1].sqrt();
            }
            27 => {
                self.need(1)?;
                let v = self.stack[n - 1];
                self.push(v)?;
            }
            28 => {
                self.need(2)?;
                self.stack.swap(n - 2, n - 1);
            }
            29 => {
                self.need(2)?;
                let idx = self.stack[n - 1] as i64;
                if idx < 0 {
                    self.stack[n - 1] = self.stack[n - 2];
                } else {
                    self.need(idx as usize + 2)?;
                    self.stack[n - 1] = self.stack[n - idx as usize - 2];
                }
            }
            30 => {
                self.need(2)?;
                let j = self.stack.pop().unwrap() as i64;
                let nn = self.stack.pop().unwrap() as i64;
                self.need(nn.max(0) as usize)?;
                let top = self.stack.len();
                if nn > 0 {
                    let slice = &mut self.stack[top - nn as usize..];
                    if j > 0 {
                        slice.rotate_right((j % nn) as usize);
                    } else {
                        slice.rotate_left(((-j) % nn) as usize);
                    }
                }
            }
            23 => self.push(1.0)?,
            _ => return Err(format!("cff: unknown charstring operator: 0x0c{op:02x}")),
        }
        Ok(())
    }

    /// `cs2_get_integer` and `get_fixed`.
    fn number(&mut self, data: &[u8], p: &mut usize) -> Result<(), String> {
        let b0 = data[*p];
        *p += 1;
        let need = |p: usize, n: usize| -> Result<(), String> {
            if p + n > data.len() {
                Err("cff: parsing charstring failed".into())
            } else {
                Ok(())
            }
        };
        let v = if b0 == 255 {
            need(*p, 4)?;
            let i = data[*p] as i64 * 256 + data[*p + 1] as i64;
            let mut r = if i > 0x7fff { i - 0x10000 } else { i } as f64;
            r += (data[*p + 2] as f64 * 256.0 + data[*p + 3] as f64) / 65536.0;
            *p += 4;
            r
        } else if b0 == 28 {
            need(*p, 2)?;
            let mut r = data[*p] as i64 * 256 + data[*p + 1] as i64;
            if r > 0x7fff {
                r -= 0x10000;
            }
            *p += 2;
            r as f64
        } else if (32..=246).contains(&b0) {
            (b0 as i64 - 139) as f64
        } else if (247..=250).contains(&b0) {
            need(*p, 1)?;
            let r = (b0 as i64 - 247) * 256 + data[*p] as i64 + 108;
            *p += 1;
            r as f64
        } else if (251..=254).contains(&b0) {
            need(*p, 1)?;
            let r = -(b0 as i64 - 251) * 256 - data[*p] as i64 - 108;
            *p += 1;
            r as f64
        } else {
            return Err("cff: parsing charstring failed".into());
        };
        self.push(v)
    }

    /// `do_charstring`.
    fn run(
        &mut self,
        out: &mut Vec<u8>,
        data: &[u8],
        gsubr: &Index,
        subr: Option<&Index>,
    ) -> Result<(), String> {
        if self.nest > 10 {
            return Err("cff: subroutine nested too deeply".into());
        }
        self.nest += 1;
        let mut p = 0;
        while p < data.len() && self.status == Status::Ok {
            let b0 = data[p];
            if b0 == 255 {
                self.number(data, &mut p)?;
            } else if b0 == 11 {
                self.status = Status::SubrReturn;
            } else if b0 == 29 || b0 == 10 {
                let id = self.stack.pop().ok_or("cff: charstring stack error")? as i64;
                let idx = if b0 == 29 { Some(gsubr) } else { subr }
                    .ok_or("cff: subroutine called but no subroutine found")?;
                let count = idx.count as i64;
                let biased = id
                    + if count < 1240 {
                        107
                    } else if count < 33900 {
                        1131
                    } else {
                        32768
                    };
                if biased >= count || biased < 0 {
                    return Err(format!(
                        "cff: invalid subroutine index: {biased} (max={count})"
                    ));
                }
                let code = idx.item(biased as usize).to_vec();
                self.run(out, &code, gsubr, subr)?;
                p += 1;
            } else if b0 == 12 {
                self.op2(out, data, &mut p)?;
            } else if b0 < 32 && b0 != 28 {
                self.op1(out, data, &mut p)?;
            } else {
                self.number(data, &mut p)?;
            }
        }
        if self.status == Status::SubrReturn {
            self.status = Status::Ok;
        }
        self.nest -= 1;
        Ok(())
    }
}

/// `cs_copy_charstring` with its state reset.
fn copy_charstring(data: &[u8], gsubr: &Index, subr: Option<&Index>) -> Result<Vec<u8>, String> {
    let mut cs = Cs {
        stack: Vec::new(),
        trn: [0.0; 32],
        phase: 0,
        num_stems: 0,
        nest: 0,
        status: Status::Ok,
    };
    let mut out = Vec::new();
    cs.run(&mut out, data, gsubr, subr)?;
    Ok(out)
}

/// What one subset gives luatex's writer.
#[derive(Clone, Debug, PartialEq)]
pub struct CffSubset {
    pub fontfile: Vec<u8>,
    pub cidset: Vec<u8>,
}

/// `writetype0`'s subset of `used` glyph indices from the OpenType font in
/// `data` (`subfont` counting from 1 in a collection). `tag` and
/// `fontname` make the `TAG+fontname` the font is renamed to.
pub fn make_cff_subset(
    data: &[u8],
    subfont: u32,
    used: &BTreeSet<u16>,
    tag: &str,
    fontname: &str,
) -> Result<CffSubset, String> {
    let (cff_offset, cff_len) = cff_table(data, subfont)?;
    let stream = data
        .get(cff_offset..cff_offset + cff_len)
        .ok_or("cff: table past the end")?;
    let font = read_cff(stream)?;
    let fullname = format!("{tag}+{fontname}");
    match font.cid {
        false => write_cff(font, used, &fullname),
        true => write_cid_cff(font, used, &fullname),
    }
}

/// The `CFF ` table of the face (`ttf_read_tabdir`/`otc_read_tabdir`).
fn cff_table(data: &[u8], subfont: u32) -> Result<(usize, usize), String> {
    let mut r = Reader { s: data, p: 0 };
    let tag = r.unsigned(4);
    if tag == 0x7474_6366 {
        r.p = 8;
        let num = r.unsigned(4);
        let i = subfont.saturating_sub(1).min(num.saturating_sub(1));
        r.p = 12 + 4 * i as usize;
        let off = r.unsigned(4) as usize;
        r.p = off + 4;
    }
    let n = r.unsigned(2);
    r.p += 6;
    for _ in 0..n {
        let t = r.unsigned(4);
        let _sum = r.unsigned(4);
        let off = r.unsigned(4) as usize;
        let len = r.unsigned(4) as usize;
        if t == u32::from_be_bytes(*b"CFF ") {
            return Ok((off, len));
        }
    }
    Err("type 0: no CFF table".into())
}

/// The private DICT of a top or FD DICT (`cff_read_private`), and its
/// Subrs (`cff_read_subrs`).
fn read_private(font: &Font, fd: &Dict) -> Result<(Option<Dict>, Option<Index>), String> {
    if !fd.known("Private") {
        return Ok((None, None));
    }
    let size = fd.get("Private", 0)? as usize;
    if size == 0 {
        return Ok((None, None));
    }
    let offset = fd.get("Private", 1)? as usize;
    let bytes = font
        .stream
        .get(offset..offset + size)
        .ok_or("cff: private past the end")?;
    let private = dict_unpack(bytes)?;
    let subrs = match private.known("Subrs") {
        true => {
            let at = offset + private.get("Subrs", 0)? as usize;
            let mut r = Reader {
                s: font.stream,
                p: at,
            };
            Some(r.index()?)
        }
        false => None,
    };
    Ok((Some(private), subrs))
}

fn gsubr(font: &Font) -> Result<Index, String> {
    Reader {
        s: font.stream,
        p: font.gsubr_offset,
    }
    .index()
}

fn charstrings_header(font: &Font) -> Result<(Index, usize), String> {
    let mut r = Reader {
        s: font.stream,
        p: font.topdict.get("CharStrings", 0)? as usize,
    };
    let idx = r.index_header()?;
    if idx.count < 2 {
        return Err("cff: no valid charstring data found".into());
    }
    Ok((idx, r.p))
}

/// `write_cff` (writecff.c:2665-2910): a name-keyed font made CID-keyed.
fn write_cff(mut font: Font, used: &BTreeSet<u16>, fullname: &str) -> Result<CffSubset, String> {
    let mut strings = Strings::default();
    let (mut private, subrs) = read_private(&font, &font.topdict)?;
    let gsub = gsubr(&font)?;
    let mut gl: BTreeSet<u16> = used.clone();
    gl.insert(0);
    let last_cid = gl.iter().next_back().copied().unwrap_or(0);
    let num_glyphs = gl.len();
    // charset format 0 of the CIDs, FDSelect format 3 of one range
    let charset: Vec<u16> = gl.iter().copied().filter(|&g| g != 0).collect();
    let top = &mut font.topdict;
    top.add("CIDCount", 1)?;
    top.set("CIDCount", 0, last_cid as f64 + 1.0)?;
    let mut fdarray = Dict::default();
    fdarray.add("FontName", 1)?;
    let fid = strings.add(fullname);
    fdarray.set("FontName", 0, fid as f64)?;
    fdarray.add("Private", 2)?;
    top.add("FDArray", 1)?;
    top.add("FDSelect", 1)?;
    for k in ["UniqueID", "XUID", "Private", "Encoding"] {
        top.remove(k);
    }
    let (cs_idx, cs_data) = charstrings_header(&font)?;
    let mut cstrings: Vec<Vec<u8>> = Vec::new();
    for i in 0..cs_idx.count as usize {
        if !gl.contains(&(i as u16)) {
            continue;
        }
        let a = cs_data + cs_idx.offset[i] as usize - 1;
        let b = cs_data + cs_idx.offset[i + 1] as usize - 1;
        let code = font
            .stream
            .get(a..b)
            .ok_or("cff: charstring past the end")?;
        cstrings.push(copy_charstring(code, &gsub, subrs.as_ref())?);
    }
    let mut cidset = vec![0u8; last_cid as usize / 8 + 1];
    for cid in 1..=last_cid {
        if gl.contains(&cid) {
            cidset[cid as usize / 8] |= 1 << (7 - (cid % 8));
        }
    }
    let n = cstrings.len().min(num_glyphs);
    cstrings.truncate(n);
    if let Some(p) = private.as_mut() {
        p.remove("Subrs");
    }
    let mut top = std::mem::take(&mut font.topdict);
    dict_update(&mut top, &font, &mut strings);
    font.topdict = top;
    strings.add("Adobe");
    strings.add("Identity");
    if let Some(p) = private.as_mut() {
        dict_update(p, &font, &mut strings);
    }
    let top = &mut font.topdict;
    top.add("ROS", 3)?;
    top.set("ROS", 0, strings.sid("Adobe").unwrap_or(0) as f64)?;
    top.set("ROS", 1, strings.sid("Identity").unwrap_or(0) as f64)?;
    top.set("ROS", 2, 0.0)?;
    let fdselect = vec![(0u16, 0u8)];
    let fontfile = write_fontfile(
        &font,
        fullname,
        &strings,
        &charset,
        &fdselect,
        n as u32,
        &cstrings,
        &mut [fdarray],
        &[private],
    )?;
    Ok(CffSubset { fontfile, cidset })
}

/// `write_cid_cff` (writecff.c:2915-3118): a CID-keyed font subset.
fn write_cid_cff(
    mut font: Font,
    used: &BTreeSet<u16>,
    fullname: &str,
) -> Result<CffSubset, String> {
    let mut gl: BTreeSet<u16> = used.clone();
    gl.insert(0);
    let last_cid = gl.iter().next_back().copied().unwrap_or(0);
    if last_cid as u32 >= font.num_glyphs {
        return Err(format!("cff font: bad glyph index {last_cid}"));
    }
    let mut cidset = vec![0u8; last_cid as usize / 8 + 1];
    cidset[0] |= 1 << 7;
    for &cid in gl.iter().filter(|&&c| c >= 1) {
        cidset[cid as usize / 8] |= 1 << (7 - (cid % 8));
    }
    // FDSelect, FDArray, the privates and their Subrs
    let mut r = Reader {
        s: font.stream,
        p: font.topdict.get("FDSelect", 0)? as usize,
    };
    let format = r.card8();
    let fds: Vec<u8> = match format {
        0 => (0..font.num_glyphs).map(|_| r.card8() as u8).collect(),
        3 => {
            let n = r.unsigned(2) as usize;
            let ranges: Vec<(u32, u8)> = (0..n).map(|_| (r.unsigned(2), r.card8() as u8)).collect();
            let mut fds = vec![0u8; font.num_glyphs as usize];
            for (k, &(first, fd)) in ranges.iter().enumerate() {
                let end = ranges.get(k + 1).map_or(font.num_glyphs, |x| x.0);
                for g in first..end.min(font.num_glyphs) {
                    fds[g as usize] = fd;
                }
            }
            fds
        }
        _ => return Err("cff: unknown FDSelect format".into()),
    };
    let mut r = Reader {
        s: font.stream,
        p: font.topdict.get("FDArray", 0)? as usize,
    };
    let fda = r.index()?;
    let mut fdarray: Vec<Dict> = Vec::new();
    for i in 0..fda.count as usize {
        let item = fda.item(i);
        fdarray.push(if item.is_empty() {
            Dict::default()
        } else {
            dict_unpack(item)?
        });
    }
    let mut privates = Vec::new();
    let mut subrs = Vec::new();
    for fd in &fdarray {
        let (p, s) = read_private(&font, fd)?;
        privates.push(p);
        subrs.push(s);
    }
    let gsub = gsubr(&font)?;
    let (cs_idx, cs_data) = charstrings_header(&font)?;
    let mut charset: Vec<u16> = Vec::new();
    let mut fdselect: Vec<(u16, u8)> = Vec::new();
    let mut cstrings = Vec::new();
    let mut prev: i32 = -1;
    for (gid, &cid) in gl.iter().enumerate() {
        let fd = *fds.get(cid as usize).ok_or("cff: invalid glyph index")?;
        if fd as usize >= fdarray.len() {
            return Err("cff: invalid Font DICT index".into());
        }
        let a = cs_data + cs_idx.offset[cid as usize] as usize - 1;
        let b = cs_data + cs_idx.offset[cid as usize + 1] as usize - 1;
        let code = font
            .stream
            .get(a..b)
            .ok_or("cff: charstring past the end")?;
        cstrings.push(copy_charstring(code, &gsub, subrs[fd as usize].as_ref())?);
        if cid > 0 {
            charset.push(cid);
        }
        if fd as i32 != prev {
            fdselect.push((gid as u16, fd));
            prev = fd as i32;
        }
    }
    for p in privates.iter_mut().flatten() {
        p.remove("Subrs");
    }
    // the string INDEX stays the font's own
    let strings = Strings {
        list: (0..font.string.count as usize)
            .map(|i| String::from_utf8_lossy(font.string.item(i)).into_owned())
            .collect(),
    };
    let n = cstrings.len() as u32;
    font.num_glyphs = n;
    write_fontfile(
        &font,
        fullname,
        &strings,
        &charset,
        &fdselect,
        n,
        &cstrings,
        &mut fdarray,
        &privates,
    )
    .map(|fontfile| CffSubset { fontfile, cidset })
}

/// `write_fontfile` (writecff.c:2563-2663).
#[allow(clippy::too_many_arguments)]
fn write_fontfile(
    font: &Font,
    fullname: &str,
    strings: &Strings,
    charset: &[u16],
    fdselect: &[(u16, u8)],
    num_glyphs: u32,
    cstrings: &[Vec<u8>],
    fdarray: &mut [Dict],
    privates: &[Option<Dict>],
) -> Result<Vec<u8>, String> {
    let mut top = font.topdict.clone();
    for k in [
        "UniqueID", "XUID", "Private", "Encoding", "vstore", "maxstack",
    ] {
        top.remove(k);
    }
    // the Private sizes, a Private of only Subrs dropped
    let mut private_sizes = Vec::new();
    for (i, p) in privates.iter().enumerate() {
        let size = p.as_ref().map_or(0, |p| dict_pack(p).len());
        if p.is_some() && size < 1 {
            fdarray[i].remove("Private");
        }
        private_sizes.push(size);
    }
    let mut out = Vec::new();
    // header
    out.extend_from_slice(&[1, font.minor, 4, 4]);
    // Name
    let name = Index {
        count: 1,
        offset: vec![1, fullname.len() as u32 + 1],
        data: fullname.as_bytes().to_vec(),
    };
    let _ = &font.name;
    name.pack(&mut out);
    // Top DICT: its size now, its offsets filled in below
    let topdict_offset = out.len();
    let topdict_len = dict_pack(&top).len();
    let topdict_size = Index {
        count: 1,
        offset: vec![1, topdict_len as u32 + 1],
        data: vec![0; topdict_len],
    }
    .size();
    out.resize(out.len() + topdict_size, 0);
    strings.index().pack(&mut out);
    // the global subrs: none
    Index::default().pack(&mut out);
    top.set("charset", 0, out.len() as f64)?;
    out.push(0);
    for &c in charset {
        out.extend_from_slice(&c.to_be_bytes());
    }
    top.set("FDSelect", 0, out.len() as f64)?;
    out.push(3);
    let fdsel_start = out.len();
    out.extend_from_slice(&[0, 0]);
    for &(first, fd) in fdselect {
        out.extend_from_slice(&first.to_be_bytes());
        out.push(fd);
    }
    out.extend_from_slice(&(num_glyphs as u16).to_be_bytes());
    let entries = ((out.len() - fdsel_start + 1) / 3 - 1) as u16;
    out[fdsel_start..fdsel_start + 2].copy_from_slice(&entries.to_be_bytes());
    top.set("CharStrings", 0, out.len() as f64)?;
    let mut cs = Index {
        count: num_glyphs,
        offset: vec![1],
        data: Vec::new(),
    };
    for c in cstrings.iter().take(num_glyphs as usize) {
        cs.data.extend_from_slice(c);
        cs.offset.push(cs.data.len() as u32 + 1);
    }
    cs.pack(&mut out);
    top.set("FDArray", 0, out.len() as f64)?;
    let fdarray_offset = out.len();
    let fd_lens: Vec<usize> = fdarray.iter().map(|d| dict_pack(d).len()).collect();
    let fd_index_size = Index {
        count: fdarray.len() as u32,
        offset: std::iter::once(1)
            .chain(fd_lens.iter().scan(1u32, |acc, &l| {
                *acc += l as u32;
                Some(*acc)
            }))
            .collect(),
        data: vec![0; fd_lens.iter().sum()],
    }
    .size();
    out.resize(out.len() + fd_index_size, 0);
    let mut fdata = Vec::new();
    let mut foffsets = vec![1u32];
    for (i, fd) in fdarray.iter_mut().enumerate() {
        let size = private_sizes[i];
        if let (Some(p), true) = (&privates[i], size > 0) {
            let packed = dict_pack(p);
            let at = out.len();
            out.extend_from_slice(&packed);
            fd.set("Private", 0, size as f64)?;
            fd.set("Private", 1, at as f64)?;
        }
        fdata.extend_from_slice(&dict_pack(fd));
        foffsets.push(fdata.len() as u32 + 1);
    }
    let mut fdx = Vec::new();
    Index {
        count: fdarray.len() as u32,
        offset: foffsets,
        data: fdata,
    }
    .pack(&mut fdx);
    out[fdarray_offset..fdarray_offset + fdx.len()].copy_from_slice(&fdx);
    // the Top DICT, now that its offsets are known
    let mut tx = Vec::new();
    let packed = dict_pack(&top);
    Index {
        count: 1,
        offset: vec![1, packed.len() as u32 + 1],
        data: packed,
    }
    .pack(&mut tx);
    out[topdict_offset..topdict_offset + tx.len()].copy_from_slice(&tx);
    Ok(out)
}
