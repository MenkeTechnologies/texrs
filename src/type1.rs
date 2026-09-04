//! Reading a Type 1 font, ported from `type1.c` and `t1_char.c` in tectonic's
//! `xdvipdfmx`.
//!
//! This is the font Computer Modern actually ships as, and the one nearly
//! every `.dvi` from the last thirty years is drawn with. It is a PostScript
//! program in two halves: a cleartext header saying what the font is called
//! and how its characters are encoded, and an encrypted body holding the
//! outlines. The encryption is Adobe's, and it is not a secret -- the key is
//! published, and the point of it was never to hide the outlines but to keep
//! them from being edited by hand. It is applied twice over: once to the whole
//! body with key 55665, and again to each glyph's charstring with key 4330.
//!
//! What is read here is what a driver needs before it can place a glyph: the
//! font's name, its matrix, its own encoding, which glyphs it holds, and how
//! wide each one is. The width is in the charstring rather than beside it --
//! the first thing a Type 1 glyph does is `hsbw`, which declares its left side
//! bearing and its advance -- so getting it out means decrypting the
//! charstring and decoding the numbers at the front of it.
//!
//! The outlines themselves are kept as they came: a driver embeds a charstring,
//! it does not interpret one, and texrs has nothing to draw with yet.
//!
//! Reading is also what WRITING one needs. `Type1::subset` cuts a font down to
//! the glyphs a page drew and puts it back together -- a tagged `/FontName`, an
//! `/Encoding` array of the codes that survived, a `/CharStrings` dictionary of
//! the kept outlines -- and encrypts the body again, because a PDF carries the
//! font in the same two halves it was read from. That is the difference between
//! an 11 kB file and a 40 kB one for a page of a dozen words.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

/// One glyph: its name, what the font says it costs, and its charstring.
#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    pub name: String,
    /// The advance width, in the font's own units -- thousandths of an em for
    /// every Type 1 font in practice.
    pub width: f64,
    /// How far the outline starts to the right of the origin.
    pub left_side_bearing: f64,
    /// The decrypted charstring, as it was.
    pub charstring: Vec<u8>,
}

/// A Type 1 font.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Type1 {
    /// The font as a PDF embeds it: the cleartext header, the encrypted body,
    /// and the zeros that close the file. A `FontFile` stream is those three
    /// concatenated, with `Length1`, `Length2` and `Length3` saying where each
    /// ends -- which is why they are kept rather than reassembled.
    parts: (Vec<u8>, Vec<u8>, Vec<u8>),
    pub font_name: String,
    /// The matrix that takes the font's units to em units: `0.001 0 0 0.001 0
    /// 0` for a font counting in thousandths.
    pub font_matrix: [f64; 6],
    pub font_bbox: [f64; 4],
    /// The font's own encoding, when it carries one rather than naming
    /// StandardEncoding.
    pub encoding: BTreeMap<u8, String>,
    pub uses_standard_encoding: bool,
    /// `/ItalicAngle`, in degrees and negative for a face that leans right.
    /// A PDF font descriptor asks for it and this is where it is stated.
    pub italic_angle: f64,
    /// `/StdVW`, the dominant vertical stem width, out of the Private DICT --
    /// so it is inside the encrypted half and a font need not state it.
    ///
    /// This is what a descriptor's `/StemV` is asking for. LuaTeX does not
    /// answer it from the font: measured, it writes `/StemV 69` for both CMR10
    /// and CMTT10, whose stems are not the same width, so 69 is a constant in
    /// the writer rather than a measurement of the face.
    pub stem_v: Option<f64>,
    /// How many subroutines the private part holds. They are what a charstring
    /// calls, and a driver embeds them with it.
    pub subroutines: usize,
    /// The four heights a PDF font descriptor asks for and a Type 1 font does
    /// not state: `Ascender`, `Descender`, `CapHeight` and `XHeight`, in the
    /// order §9.8.1 wants them -- ascent, descent, cap height, x-height.
    ///
    /// They are in the `.afm` beside the font and nowhere in the `.pfb`, which
    /// is why they are `None` for a font read from bytes alone. Measured, that
    /// is where LuaTeX gets them too: for CMR10 it writes `/Ascent 694
    /// /CapHeight 683 /Descent -194 /XHeight 431`, and cmr10.afm states
    /// `Ascender 694`, `CapHeight 683`, `Descender -194`, `XHeight 431` -- the
    /// same four numbers, none of which appears in cmr10.pfb. Writing the
    /// bounding box instead, which is what this did, gave `/Ascent 750
    /// /CapHeight 750 /Descent -250`: the extremes of the outlines rather than
    /// the heights of the letters.
    pub afm_metrics: Option<AfmMetrics>,
    glyphs: BTreeMap<String, Glyph>,
    /// The eexec half, DECRYPTED: the Private DICT, the subroutines and the
    /// CharStrings dictionary, as PostScript. Kept because a subset is built by
    /// rewriting this and encrypting it again, and re-deriving it would mean
    /// decrypting the font a second time.
    private: Vec<u8>,
    /// How many bytes of random padding each charstring carries -- `/lenIV`,
    /// four unless the font says otherwise. A charstring is decrypted by
    /// dropping that many and encrypted by putting that many back.
    len_iv: usize,
    /// What this font calls the two operators around a charstring's bytes:
    /// `RD`/`ND` in a font written out in full, `-|`/`|-` in one squeezed for
    /// space. A subset has to spell them the way the font's own Private DICT
    /// defines them, since that is where they are defined.
    charstring_tokens: (String, String),
    /// Where `/CharStrings` begins in `private`, and where the entry for the
    /// last glyph ends. Everything before the first and after the `end` that
    /// follows the second is copied into a subset unchanged.
    charstrings: (usize, usize),
}

/// The four heights an `.afm` states about a face, in the font's own units.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AfmMetrics {
    pub ascender: f64,
    pub descender: f64,
    pub cap_height: f64,
    pub x_height: f64,
}

impl AfmMetrics {
    /// Read the four out of an `.afm`'s global section.
    ///
    /// Adobe's metrics format is `Key value` a line, and these four are single
    /// numbers in the header before `StartCharMetrics`. Only the header is
    /// read: a `C 72 ; WX 750 ; N H ;` line also begins with a key and a number
    /// and states nothing about the face.
    pub fn parse(text: &str) -> AfmMetrics {
        let mut out = AfmMetrics::default();
        for line in text.lines() {
            if line.starts_with("StartCharMetrics") {
                break;
            }
            let Some((key, value)) = line.split_once(char::is_whitespace) else {
                continue;
            };
            let Ok(value) = value.trim().parse::<f64>() else {
                continue;
            };
            match key {
                "Ascender" => out.ascender = value,
                "Descender" => out.descender = value,
                "CapHeight" => out.cap_height = value,
                "XHeight" => out.x_height = value,
                _ => {}
            }
        }
        out
    }
}

/// The key for the whole encrypted body (`eexec`), from the Type 1 book.
const EEXEC_KEY: u16 = 55665;
/// The key for one glyph's charstring.
const CHARSTRING_KEY: u16 = 4330;

/// Adobe's stream cipher, which both halves use: a running key, one byte at a
/// time, with the first `skip` bytes of plaintext thrown away because they are
/// random padding.
fn decrypt(bytes: &[u8], key: u16, skip: usize) -> Vec<u8> {
    let mut r = key;
    let mut out = Vec::with_capacity(bytes.len().saturating_sub(skip));
    for (i, &c) in bytes.iter().enumerate() {
        let plain = c ^ (r >> 8) as u8;
        r = (c as u16)
            .wrapping_add(r)
            .wrapping_mul(52845)
            .wrapping_add(22719);
        if i >= skip {
            out.push(plain);
        }
    }
    out
}

/// The same cipher the other way, which is what writing a subset needs.
///
/// `pad` bytes of leading plaintext go in front of the message and are thrown
/// away again by `decrypt`; they are meant to be random, and are a constant
/// here so that the same document twice gives the same file. `0x00` would be a
/// poor choice for the eexec half -- §7.2 of the Type 1 book asks that the
/// first four ciphertext bytes not all be hexadecimal digits, or a reader
/// cannot tell a binary body from a hexadecimal one -- so the padding is chosen
/// to make the first ciphertext byte fall outside `0-9A-Fa-f`.
fn encrypt(plain: &[u8], key: u16, pad: usize) -> Vec<u8> {
    let mut r = key;
    let mut out = Vec::with_capacity(plain.len() + pad);
    for &byte in std::iter::repeat_n(&0x58u8, pad).chain(plain) {
        let cipher = byte ^ (r >> 8) as u8;
        r = (cipher as u16)
            .wrapping_add(r)
            .wrapping_mul(52845)
            .wrapping_add(22719);
        out.push(cipher);
    }
    out
}

/// The metrics beside a font program: `cmr10.afm` for `cmr10.pfb`.
///
/// A TeX installation does not keep the two in one directory -- the fonts are
/// under `fonts/type1/` and the metrics under `fonts/afm/` -- so the sibling is
/// tried first for a font that came from somewhere else, and `kpsewhich`
/// second, which is how everything else in the tree finds a TeX file.
fn afm_beside(path: &Path) -> Option<AfmMetrics> {
    let read = |at: &Path| {
        std::fs::read_to_string(at)
            .ok()
            .map(|t| AfmMetrics::parse(&t))
    };
    if let Some(found) = read(&path.with_extension("afm")) {
        return Some(found);
    }
    let name = format!("{}.afm", path.file_stem()?.to_string_lossy());
    let out = std::process::Command::new("kpsewhich")
        .arg(&name)
        .output()
        .ok()?;
    let found = String::from_utf8_lossy(&out.stdout).trim().to_string();
    match found.is_empty() {
        true => None,
        false => read(Path::new(&found)),
    }
}

/// Where `needle` is in `haystack`, if anywhere.
fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (from..=haystack.len() - needle.len()).find(|&at| &haystack[at..at + needle.len()] == needle)
}

/// Where `needle` stands as a WORD of its own at or after `from`.
///
/// PostScript is whitespace-separated, and a plain search is not: looking for
/// `def` after `/Encoding` finds it inside `/.notdef put` in the very first
/// line of a font's encoding array, three hundred bytes before the `readonly
/// def` that actually closes it.
fn word_at(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    let mut at = from;
    loop {
        let found = find(haystack, needle, at)?;
        let before = found
            .checked_sub(1)
            .map(|i| haystack[i].is_ascii_whitespace());
        let after = haystack
            .get(found + needle.len())
            .map(|c| c.is_ascii_whitespace());
        if before.unwrap_or(true) && after.unwrap_or(true) {
            return Some(found);
        }
        at = found + 1;
    }
}

/// Replace what lies between `key` and the standalone `until` that follows it
/// with `replacement`, keeping the key and dropping the old terminator, which
/// `replacement` is expected to write again.
///
/// This is how a subset re-states `/FontName` and `/Encoding` without rewriting
/// the rest of a font's cleartext header, every line of which -- the copyright,
/// the `/UniqueID`, the `FontDirectory` guard -- is the font's own to keep.
fn replace_after(bytes: &[u8], key: &[u8], replacement: &str, until: &[u8]) -> Option<Vec<u8>> {
    let at = find(bytes, key, 0)? + key.len();
    let end = word_at(bytes, until, at)? + until.len();
    let mut out = bytes[..at].to_vec();
    out.extend(replacement.as_bytes());
    out.extend(&bytes[end..]);
    Some(out)
}

/// Where TeX keeps the font program `file`, e.g. `cmr10.pfb`.
///
/// `typeset::find_font` answers the same question for a `.tfm`, and cannot be
/// reused: it appends `.tfm` to the name it is given, and a `.pfb` is not on
/// the `.tfm` search path at all -- `kpsewhich cmr10.pfb` and
/// `kpsewhich cmr10.tfm` return two different directories. The answer is
/// remembered per name because kpsewhich reads TeX Live's `ls-R` databases and
/// costs the better part of a second, and the PDF writer asks for the same
/// face once per document.
pub fn installed(file: &str) -> Option<std::path::PathBuf> {
    static SEEN: std::sync::OnceLock<
        std::sync::Mutex<BTreeMap<String, Option<std::path::PathBuf>>>,
    > = std::sync::OnceLock::new();
    let seen = SEEN.get_or_init(Default::default);
    if let Some(hit) = seen.lock().ok().and_then(|m| m.get(file).cloned()) {
        return hit;
    }
    let found = std::process::Command::new("kpsewhich")
        .arg(file)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|p| !p.is_empty())
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists());
    if let Ok(mut m) = seen.lock() {
        m.insert(file.to_string(), found.clone());
    }
    found
}

impl Type1 {
    pub fn open(path: impl AsRef<Path>) -> Result<Type1, String> {
        let path = path.as_ref();
        let bytes =
            std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut font = Type1::parse(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        font.afm_metrics = afm_beside(path);
        Ok(font)
    }

    /// Read a `.pfb` (segmented binary) or a `.pfa` (all text, with the body in
    /// hexadecimal).
    pub fn parse(bytes: &[u8]) -> Result<Type1, String> {
        let joined = match bytes.first() {
            // §  : a PFB is segments, each with a two-byte marker and a
            // four-byte little-endian length -- the one place a font counts
            // backwards.
            Some(0x80) => {
                let mut out = Vec::with_capacity(bytes.len());
                let mut at = 0usize;
                while at + 2 <= bytes.len() && bytes[at] == 0x80 {
                    let kind = bytes[at + 1];
                    if kind == 3 {
                        break;
                    }
                    let length = bytes
                        .get(at + 2..at + 6)
                        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
                        .ok_or("a segment header past the end of the font")?;
                    let start = at + 6;
                    let end = start
                        .checked_add(length)
                        .filter(|&end| end <= bytes.len())
                        .ok_or("a segment longer than the font")?;
                    out.extend_from_slice(&bytes[start..end]);
                    at = end;
                }
                out
            }
            Some(b'%') => bytes.to_vec(),
            _ => return Err("does not begin with %! or a PFB segment".into()),
        };

        let eexec = find(&joined, b"eexec", 0).ok_or("the font has no eexec")?;
        let clear = &joined[..eexec];
        // The body starts after `eexec` and whatever whitespace follows it.
        let mut start = eexec + 5;
        while joined.get(start).is_some_and(|c| c.is_ascii_whitespace()) {
            start += 1;
        }
        let body = &joined[start..];
        // A PFA writes the body as hexadecimal; a PFB writes it as bytes. The
        // first four bytes are random, so a body whose first four are all hex
        // digits is text.
        let is_hex = body.iter().take(4).all(|c| c.is_ascii_hexdigit());
        let binary: Vec<u8> = match is_hex {
            false => body.to_vec(),
            true => {
                let digits: Vec<u8> = body
                    .iter()
                    .copied()
                    .filter(|c| c.is_ascii_hexdigit())
                    .collect();
                digits
                    .chunks(2)
                    .filter(|pair| pair.len() == 2)
                    .map(|pair| {
                        let value = |c: u8| (c as char).to_digit(16).unwrap_or(0) as u8;
                        value(pair[0]) << 4 | value(pair[1])
                    })
                    .collect()
            }
        };
        let private = decrypt(&binary, EEXEC_KEY, 4);

        // §  : a Type 1 file ends with 512 zeros written as text, and a PDF
        // counts them separately from the body.
        let (body, trailer) = split_trailer(&joined, start);

        let mut font = Type1 {
            font_name: text_after(clear, b"/FontName").unwrap_or_default(),
            parts: (joined[..start].to_vec(), body.to_vec(), trailer.to_vec()),
            ..Type1::default()
        };
        font.font_matrix = numbers_after(clear, b"/FontMatrix")
            .try_into()
            .unwrap_or([0.001, 0.0, 0.0, 0.001, 0.0, 0.0]);
        font.font_bbox = numbers_after(clear, b"/FontBBox")
            .try_into()
            .unwrap_or_default();
        font.italic_angle = numbers_after(clear, b"/ItalicAngle")
            .first()
            .copied()
            .unwrap_or(0.0);
        font.stem_v = numbers_after(&private, b"/StdVW").first().copied();
        font.read_encoding(clear);

        // §  : how many bytes of each charstring are random padding.
        let len_iv = find(&private, b"/lenIV", 0)
            .and_then(|at| numbers_after(&private[at..], b"/lenIV").first().copied())
            .unwrap_or(4.0) as usize;
        font.subroutines = find(&private, b"/Subrs", 0)
            .and_then(|at| numbers_after(&private[at..], b"/Subrs").first().copied())
            .unwrap_or(0.0) as usize;
        font.len_iv = len_iv;
        font.read_charstrings(&private, len_iv)?;
        font.private = private;
        Ok(font)
    }

    /// The font's own encoding: either it names StandardEncoding, or it builds
    /// an array with a `dup <code> /<name> put` for each character it has.
    fn read_encoding(&mut self, clear: &[u8]) {
        let Some(at) = find(clear, b"/Encoding", 0) else {
            return;
        };
        let rest = &clear[at..];
        if find(rest, b"StandardEncoding", 0).is_some_and(|found| found < 40) {
            self.uses_standard_encoding = true;
            return;
        }
        let end = find(rest, b"readonly def", 0)
            .or_else(|| find(rest, b" def", 0))
            .unwrap_or(rest.len());
        let mut from = 0usize;
        while let Some(found) = find(&rest[..end], b"dup ", from) {
            let after = &rest[found + 4..end];
            let text: String = after.iter().take(64).map(|&b| b as char).collect();
            let mut words = text.split_whitespace();
            let code = words.next().and_then(|w| w.parse::<i64>().ok());
            let name = words.next().and_then(|w| w.strip_prefix('/'));
            if let (Some(code), Some(name)) = (code, name) {
                if (0..=255).contains(&code) {
                    self.encoding.insert(code as u8, name.to_string());
                }
            }
            from = found + 4;
        }
    }

    /// The glyphs. Each is `/name <length> RD <bytes> ND`, where `RD` and `ND`
    /// are whatever the font called them -- `-|` and `|-` in a font squeezed
    /// for space.
    fn read_charstrings(&mut self, private: &[u8], len_iv: usize) -> Result<(), String> {
        let at = find(private, b"/CharStrings", 0)
            .ok_or("the decrypted body holds no /CharStrings, so the key was wrong")?;
        let mut from = at;
        // Each entry begins with a name, and the dictionary ends at `end`.
        while let Some(slash) = find(private, b"/", from + 1) {
            let mut cursor = slash + 1;
            let mut name = String::new();
            while let Some(&c) = private.get(cursor) {
                if c.is_ascii_whitespace() || c == b'(' || c == b'/' || c == b'{' {
                    break;
                }
                name.push(c as char);
                cursor += 1;
            }
            // The length, then the token that introduces the bytes.
            let text: String = private
                .get(cursor..cursor + 32)
                .unwrap_or_default()
                .iter()
                .map(|&b| b as char)
                .collect();
            let mut words = text.split_whitespace();
            // A name with no length after it is not an entry: the
            // dictionary's own name, or the `end` that closes it.
            let Some(length) = words.next().and_then(|w| w.parse::<usize>().ok()) else {
                from = slash;
                continue;
            };
            let Some(token) = words.next() else {
                from = slash;
                continue;
            };
            // Where the bytes begin: after the token and the single space that
            // follows it.
            let Some(token_at) = find(private, token.as_bytes(), cursor) else {
                from = slash;
                continue;
            };
            let start = token_at + token.len() + 1;
            let Some(bytes) = private.get(start..start + length) else {
                from = slash;
                continue;
            };
            let charstring = decrypt(bytes, CHARSTRING_KEY, len_iv);
            let (left_side_bearing, width) = width_of(&charstring);
            self.glyphs.insert(
                name.clone(),
                Glyph {
                    name,
                    width,
                    left_side_bearing,
                    charstring,
                },
            );
            // What this font spells the two operators around a charstring as,
            // and how far the dictionary has got. A subset writes its own
            // entries between these two offsets and copies the rest. The
            // closing operator is read from AFTER the bytes rather than from
            // the window the length was read out of, which stops at the
            // charstring and holds no text past it.
            let closing: String = private
                .get(start + length..start + length + 16)
                .unwrap_or_default()
                .iter()
                .map(|&b| b as char)
                .collect();
            self.charstring_tokens = (
                token.to_string(),
                closing
                    .split_whitespace()
                    .next()
                    .unwrap_or("ND")
                    .to_string(),
            );
            from = start + length;
            self.charstrings = (at, from);
        }
        match self.glyphs.is_empty() {
            true => Err("the font's /CharStrings holds no glyphs".into()),
            false => Ok(()),
        }
    }

    /// The font as a PDF embeds it: the bytes, and where the cleartext, the
    /// encrypted body and the closing zeros end.
    ///
    /// A `FontFile` stream is exactly the file as it was, so this hands back
    /// what was read rather than anything rebuilt: a font a driver re-encoded
    /// would no longer decrypt.
    pub fn embeddable(&self) -> (Vec<u8>, usize, usize, usize) {
        let (clear, body, trailer) = &self.parts;
        let mut bytes = clear.clone();
        bytes.extend(body);
        bytes.extend(trailer);
        (bytes, clear.len(), body.len(), trailer.len())
    }

    /// The same font cut down to the codes a document actually drew, under a
    /// subset name.
    ///
    /// This is what LuaTeX embeds and what makes the difference between an
    /// 11 kB file and a 40 kB one: measured on `Hello world.`, luatex's
    /// `/FontFile` is `/Length1 1510 /Length2 9354` where the whole cmr10.pfb is
    /// 4287 and 30900. Its cleartext names the font `KJJYRX+CMR10`, its
    /// `/Encoding` array carries the nine `dup <code> /<name> put` lines for the
    /// glyphs the page set and nothing else, and its `/CharStrings` holds those
    /// nine charstrings.
    ///
    /// Three parts are rebuilt and the rest is copied byte for byte:
    ///
    /// * the cleartext header, with `/FontName` tagged and the encoding array
    ///   replaced. Everything else it says -- the notice, the `/FontInfo`, the
    ///   `/UniqueID`, the `FontDirectory` guard -- is the font's own and stays.
    /// * the `/CharStrings` dictionary inside the encrypted half, which is
    ///   written out afresh with the kept glyphs re-encrypted. The Private DICT
    ///   and ALL of the subroutines in front of it are copied: a charstring
    ///   calls `callsubr` by index, and dropping a subroutine would renumber
    ///   every call in the font. That is where this is still bigger than
    ///   LuaTeX's -- measured on `Hello world.`, `/Length2 12183` against its
    ///   9354, which is cmr10's 102 subroutines against the handful ten glyphs
    ///   reach. Pruning them means keeping the array's length and stubbing the
    ///   unused entries, and finding which are unused means INTERPRETING every
    ///   kept charstring: hint replacement pushes a subroutine number, hands it
    ///   to OtherSubr 3, takes it back with `pop` and only then calls it, so a
    ///   scanner that reads the operand before `callsubr` does not see it and
    ///   would stub a subroutine the font goes on calling. `xdvipdfmx`'s
    ///   `t1_subset`, which this module is ported from, carries them all for
    ///   the same reason.
    /// * the eexec encryption itself, since the body it covers has changed.
    ///
    /// The 512 zeros and `cleartomark` that close the file are the original's.
    ///
    /// `None` when nothing would be kept, or when the font is one this cannot
    /// take apart -- the caller then embeds it whole, which is what every font
    /// did before there was a subset at all.
    pub fn subset(&self, keep: &std::collections::BTreeSet<String>, tag: &str) -> Option<Type1> {
        let kept: Vec<&Glyph> = keep
            .iter()
            .filter_map(|name| self.glyphs.get(name))
            .collect();
        if kept.is_empty() || self.private.is_empty() {
            return None;
        }
        let name = format!("{tag}+{}", self.font_name);

        // The cleartext header: the name it goes under, and the encoding array
        // cut to the codes that survived.
        let clear = replace_after(
            &self.parts.0,
            b"/FontName",
            &format!(" /{name} def"),
            b"def",
        )?;
        let mut array = String::from(" 256 array\n0 1 255 {1 index exch /.notdef put} for\n");
        for (code, glyph) in &self.encoding {
            if keep.contains(glyph) {
                let _ = writeln!(array, "dup {code} /{glyph} put");
            }
        }
        array.push_str("readonly def");
        let clear = replace_after(&clear, b"/Encoding", &array, b"def")?;

        // The encrypted half. `/CharStrings` is rewritten between the offsets
        // the parse recorded; the `end` that closed the old dictionary is where
        // the copied tail starts again.
        let (from, after_last) = self.charstrings;
        let tail = word_at(&self.private, b"end", after_last)?;
        let (rd, nd) = &self.charstring_tokens;
        let mut body = self.private[..from].to_vec();
        body.extend(format!("/CharStrings {} dict dup begin\n", kept.len()).into_bytes());
        for glyph in &kept {
            // The charstring goes back in the way it came out: `lenIV` bytes of
            // padding in front, encrypted with the charstring key.
            let bytes = encrypt(&glyph.charstring, CHARSTRING_KEY, self.len_iv);
            body.extend(format!("/{} {} {rd} ", glyph.name, bytes.len()).into_bytes());
            body.extend(bytes);
            body.extend(format!(" {nd}\n").into_bytes());
        }
        body.extend(self.private[tail..].to_vec());

        let mut cut = self.clone();
        cut.font_name = name;
        cut.glyphs = kept
            .iter()
            .map(|glyph| (glyph.name.clone(), (*glyph).clone()))
            .collect();
        cut.encoding.retain(|_, glyph| keep.contains(glyph));
        // No trailer. §9.9, Table 127: "If Length3 is 0, it indicates that the
        // 512 zeros and cleartomark have not been included in the FontFile
        // stream and shall be assumed by the consumer application." That is
        // what LuaTeX writes -- measured, `/Length1 1510 /Length2 9354
        // /Length3 0` -- and it saves the 545 bytes of zeros that a reader
        // supplies for itself.
        cut.parts = (clear, encrypt(&body, EEXEC_KEY, 4), Vec::new());
        cut.private = body;
        Some(cut)
    }

    /// The glyph of this name.
    pub fn glyph(&self, name: &str) -> Option<&Glyph> {
        self.glyphs.get(name)
    }

    /// Every glyph name the font holds, in order.
    pub fn glyph_names(&self) -> Vec<&str> {
        self.glyphs.keys().map(String::as_str).collect()
    }

    /// The glyph a character code means, through the font's own encoding.
    pub fn encoded(&self, code: u8) -> Option<&Glyph> {
        let name = match self.uses_standard_encoding {
            true => crate::sfnt::MAC_GLYPH_NAMES
                .get(code as usize)
                .copied()
                .unwrap_or_default(),
            false => self.encoding.get(&code)?.as_str(),
        };
        self.glyph(name)
    }

    /// A summary a person reads.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("font name     {}\n", self.font_name));
        out.push_str(&format!(
            "matrix        {}\n",
            self.font_matrix
                .iter()
                .map(|n| format!("{n}"))
                .collect::<Vec<_>>()
                .join(" ")
        ));
        out.push_str(&format!(
            "bounding box  {}\n",
            self.font_bbox
                .iter()
                .map(|n| format!("{n}"))
                .collect::<Vec<_>>()
                .join(" ")
        ));
        out.push_str(&format!("glyphs        {}\n", self.glyphs.len()));
        out.push_str(&format!("subroutines   {}\n", self.subroutines));
        out.push_str(&format!(
            "encoding      {}\n",
            match self.uses_standard_encoding {
                true => "StandardEncoding".to_string(),
                false => format!("{} characters, the font's own", self.encoding.len()),
            }
        ));
        out
    }

    /// One character: which glyph it is, how wide, and how long its charstring
    /// is.
    pub fn describe(&self, code: u8) -> String {
        let name = match self.uses_standard_encoding {
            true => crate::sfnt::MAC_GLYPH_NAMES
                .get(code as usize)
                .copied()
                .unwrap_or("")
                .to_string(),
            false => self.encoding.get(&code).cloned().unwrap_or_default(),
        };
        let shown = match (code as char).is_ascii_graphic() {
            true => format!("'{}'", code as char),
            false => format!("0o{code:o}"),
        };
        let Some(glyph) = self.glyph(&name) else {
            return format!("{shown}: the font's encoding gives no glyph for it\n");
        };
        format!(
            "{shown}  {}  width {}  side bearing {}  charstring {} bytes\n",
            glyph.name,
            glyph.width,
            glyph.left_side_bearing,
            glyph.charstring.len()
        )
    }
}

/// Where the encrypted body ends and the closing zeros begin.
///
/// A Type 1 file ends with 512 `0` characters and a `cleartomark`, written as
/// text. They are not part of the encrypted body, and a PDF counts them
/// separately, so the split is by looking for the run of zeros rather than by
/// trusting a length.
fn split_trailer(joined: &[u8], start: usize) -> (&[u8], &[u8]) {
    let body = &joined[start..];
    // The zeros are near the end but not at it: `cleartomark` follows them,
    // and it belongs to the trailer. So the search starts at the last zero and
    // walks back over the run.
    let Some(last) = body.iter().rposition(|&b| b == b'0') else {
        return (body, &body[body.len()..]);
    };
    let mut at = last + 1;
    let mut zeros = 0usize;
    while at > 0 {
        match body[at - 1] {
            b'0' => zeros += 1,
            c if c.is_ascii_whitespace() => {}
            _ => break,
        }
        at -= 1;
    }
    match zeros >= 512 {
        true => (&body[..at], &body[at..]),
        // A font with no trailer: a PFA cut short, or one already stripped.
        false => (body, &body[body.len()..]),
    }
}

/// The value of a name in the cleartext header: `/FontName /CMR10 def` gives
/// `CMR10`.
fn text_after(bytes: &[u8], key: &[u8]) -> Option<String> {
    let at = find(bytes, key, 0)? + key.len();
    let text: String = bytes
        .get(at..)?
        .iter()
        .take(128)
        .map(|&b| b as char)
        .collect();
    let word = text.split_whitespace().next()?;
    Some(word.trim_start_matches('/').to_string())
}

/// The numbers after a name: `/FontBBox {-40 -250 1009 750 }` gives four.
fn numbers_after(bytes: &[u8], key: &[u8]) -> Vec<f64> {
    let Some(at) = find(bytes, key, 0) else {
        return Vec::new();
    };
    let text: String = bytes[at + key.len()..]
        .iter()
        .take(128)
        .map(|&b| b as char)
        .collect();
    let text = text.replace(['[', ']', '{', '}'], " ");
    let mut out = Vec::new();
    for word in text.split_whitespace() {
        match word.parse::<f64>() {
            Ok(value) => out.push(value),
            // The numbers run until something that is not one: `readonly def`,
            // `array`, `dict`.
            Err(_) if !out.is_empty() => break,
            Err(_) => {}
        }
    }
    out
}

/// The side bearing and width a charstring declares.
///
/// A Type 1 glyph's first operator is `hsbw` (13), taking its left side
/// bearing and its advance; a glyph that moves vertically as well uses `sbw`
/// (12 7), which takes four. The numbers before them are in Type 1's own
/// encoding: one byte for the small ones, two for the middling, five for the
/// rest.
fn width_of(charstring: &[u8]) -> (f64, f64) {
    let mut stack: Vec<f64> = Vec::new();
    let mut at = 0usize;
    while at < charstring.len() {
        let byte = charstring[at];
        at += 1;
        match byte {
            32..=246 => stack.push(byte as f64 - 139.0),
            247..=250 => {
                let Some(&low) = charstring.get(at) else {
                    break;
                };
                at += 1;
                stack.push((byte as f64 - 247.0) * 256.0 + low as f64 + 108.0);
            }
            251..=254 => {
                let Some(&low) = charstring.get(at) else {
                    break;
                };
                at += 1;
                stack.push(-(byte as f64 - 251.0) * 256.0 - low as f64 - 108.0);
            }
            255 => {
                let Some(bytes) = charstring.get(at..at + 4) else {
                    break;
                };
                at += 4;
                stack.push(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64);
            }
            // hsbw: side bearing, width.
            13 => {
                return match stack.len() >= 2 {
                    true => (stack[stack.len() - 2], stack[stack.len() - 1]),
                    false => (0.0, 0.0),
                }
            }
            12 => {
                let Some(&second) = charstring.get(at) else {
                    break;
                };
                at += 1;
                // sbw: side bearing x and y, width x and y.
                if second == 7 {
                    return match stack.len() >= 4 {
                        true => (stack[stack.len() - 4], stack[stack.len() - 2]),
                        false => (0.0, 0.0),
                    };
                }
                stack.clear();
            }
            // Any other operator before hsbw means there is no width to read.
            _ => break,
        }
    }
    (0.0, 0.0)
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

    /// cmr10 as it actually ships: the Type 1 font every driver has drawn from
    /// for thirty years.
    #[test]
    fn a_type1_font_gives_up_its_names_and_widths() {
        let Some(bytes) = installed("cmr10.pfb") else {
            return;
        };
        let font = Type1::parse(&bytes).expect("cmr10 reads");

        assert_eq!(font.font_name, "CMR10");
        assert_eq!(font.font_matrix, [0.001, 0.0, 0.0, 0.001, 0.0, 0.0]);
        assert_eq!(font.font_bbox, [-40.0, -250.0, 1009.0, 750.0]);
        assert!(
            font.subroutines > 0,
            "a Computer Modern font has subroutines"
        );

        // The decryption worked if the glyph names are words.
        assert!(
            font.glyph_names().len() > 100,
            "{}",
            font.glyph_names().len()
        );
        assert!(font.glyph("A").is_some() && font.glyph("space").is_some());
        assert!(
            font.glyph_names()
                .iter()
                .all(|name| name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')),
            "a name with rubbish in it means the key was wrong"
        );

        // The width in the charstring is the width in the metrics: an A is 750
        // thousandths of an em, which is cmr10.tfm's 0.750002.
        let a = font.glyph("A").expect("an A");
        assert_eq!(a.width, 750.0);
        assert_eq!(a.left_side_bearing, 32.0);
        assert!(!a.charstring.is_empty());

        // The font carries its own encoding rather than naming Adobe's, which
        // is why TeX can put a ligature at position 11.
        assert!(!font.uses_standard_encoding);
        assert_eq!(font.encoding.get(&65).map(String::as_str), Some("A"));
        assert_eq!(font.encoding.get(&11).map(String::as_str), Some("ff"));
        assert_eq!(font.encoded(65).map(|g| g.name.as_str()), Some("A"));
    }

    /// The three parts a PDF embeds, and what they must add up to.
    #[test]
    fn the_font_comes_apart_into_the_lengths_a_pdf_wants() {
        let Some(bytes) = installed("cmr10.pfb") else {
            return;
        };
        let font = Type1::parse(&bytes).expect("cmr10 reads");
        let (embedded, clear, binary, trailer) = font.embeddable();

        assert_eq!(embedded.len(), clear + binary + trailer);
        // The cleartext is PostScript and ends where eexec does.
        assert!(embedded[..clear].starts_with(b"%!"));
        assert!(embedded[..clear].ends_with(b"eexec\r") || embedded[..clear].ends_with(b"eexec\n"));
        // The body is what decrypts, so it must not be text.
        assert!(
            embedded[clear..clear + binary]
                .iter()
                .any(|&b| b > 127 || b == 0),
            "the encrypted body reads as text, so the split is wrong"
        );
        // The trailer is 512 zeros and the words that close the file.
        assert!(trailer >= 512, "{trailer}");
        assert_eq!(
            embedded[clear + binary..]
                .iter()
                .filter(|&&b| b == b'0')
                .count(),
            512
        );
        // And the three parts together are the file as it was, because a font
        // that was rebuilt would no longer decrypt.
        assert!(bytes.windows(64).any(|w| w == &embedded[..64]));
    }

    /// What is not a Type 1 font, and a font whose body will not decrypt.
    #[test]
    fn what_is_not_a_type1_font_is_refused() {
        assert!(Type1::parse(b"").is_err());
        assert!(Type1::parse(b"not a font").is_err());
        // A PostScript file with no encrypted part.
        assert!(Type1::parse(b"%!PS-AdobeFont-1.0\n/FontName /X def\n")
            .unwrap_err()
            .contains("eexec"));

        // The right shape and the wrong key: rubbish after eexec decrypts to
        // rubbish, and rubbish holds no /CharStrings.
        let mut wrong = b"%!PS-AdobeFont-1.0\n/FontName /X def\neexec ".to_vec();
        wrong.extend([0u8; 64]);
        let e = Type1::parse(&wrong).unwrap_err();
        assert!(e.contains("/CharStrings"), "{e}");
    }
}
