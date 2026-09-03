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

use std::collections::{BTreeMap, BTreeSet};

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
    /// Where the INDEX began, which is what a subset copies from.
    start: usize,
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
            start: at,
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
    Ok(Index {
        items,
        start: at,
        end,
    })
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
const ENCODING: u16 = 16;
/// The Top DICT operators whose operands are string ids rather than numbers:
/// version, Notice, FullName, FamilyName, Weight, Copyright, PostScript,
/// BaseFontName and FontName.
const SID_OPERATORS: [u16; 9] = [0, 1, 2, 3, 4, 0x0c00, 0x0c15, 0x0c16, 0x0c26];
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

/// The Top DICT's operators and operands, for looking at what a rebuild
/// changed.
pub fn top_dict_of(bytes: &[u8]) -> Result<Vec<(u16, Vec<f64>)>, String> {
    let header = *bytes.get(2).ok_or("shorter than a CFF header")? as usize;
    let names = index(bytes, header)?;
    let tops = index(bytes, names.end)?;
    let &(from, to) = tops.items.first().ok_or("no Top DICT")?;
    Ok(dict(&bytes[from..to])?.into_iter().collect())
}

/// Cut a `CFF ` down to the glyphs a document used.
///
/// The counterpart of [`crate::glyf::subset`] for the other kind of outline,
/// and the one that matters for a TeX document: Latin Modern is a CFF font,
/// and a paper uses a fraction of it.
///
/// A CFF cannot simply have its outlines deleted, because everything in the
/// file is addressed by an offset from the start of it and those offsets are
/// written in the Top DICT. So the file is rebuilt: the same header, name and
/// strings; a CharStrings INDEX in which a glyph that was not asked for is
/// replaced by a single `endchar`, which draws nothing; the charset and the
/// private data copied across; and a Top DICT written with the new offsets.
/// Glyph numbers do not change, for the reason they do not change in a
/// TrueType subset -- the charset still names the right glyph, and so does a
/// PDF's encoding.
///
/// The operands of the offsets in the Top DICT are written in the five-byte
/// form whatever their value, so the DICT is the same length whichever
/// offsets it ends up holding. That is what makes the layout solvable in one
/// pass rather than by iterating until it settles.
pub fn subset(bytes: &[u8], keep: &BTreeSet<u16>) -> Result<Vec<u8>, String> {
    let header = *bytes.get(2).ok_or("shorter than a CFF header")? as usize;
    let names = index(bytes, header)?;
    let tops = index(bytes, names.end)?;
    let strings = index(bytes, tops.end)?;
    let gsubrs = index(bytes, strings.end)?;
    let &(from, to) = tops.items.first().ok_or("the font has no Top DICT")?;
    let top = dict(&bytes[from..to])?;

    let charstrings_at = top
        .get(&CHAR_STRINGS)
        .and_then(|values| values.first())
        .copied()
        .ok_or("the Top DICT names no CharStrings")? as usize;
    let charstrings = index(bytes, charstrings_at)?;

    // §  : a glyph that draws nothing is one `endchar`. Keeping the numbers
    // means keeping the entries.
    let kept: Vec<Vec<u8>> = charstrings
        .items
        .iter()
        .enumerate()
        .map(
            |(number, &(from, to))| match keep.contains(&(number as u16)) {
                true => bytes[from..to].to_vec(),
                false => vec![14],
            },
        )
        .collect();

    // The charset and the strings. A glyph's name is a number into the
    // standard strings or into the font's own, and the font's own are most of
    // what is left once the outlines are gone -- eight hundred names for a
    // paper that used six. So the charset is written afresh: the glyphs that
    // were kept keep their names, and the rest are `.notdef`, which lets every
    // string nobody needs be left out.
    let charset_at = top
        .get(&CHARSET)
        .and_then(|values| values.first())
        .copied()
        .unwrap_or(0.0) as usize;
    let sids = charset_sids(bytes, charset_at, charstrings.items.len())?;

    let mut new_strings: Vec<Vec<u8>> = Vec::new();
    let mut new_sids: Vec<u16> = Vec::with_capacity(sids.len());
    for (number, &sid) in sids.iter().enumerate() {
        if !keep.contains(&(number as u16)) {
            new_sids.push(0);
            continue;
        }
        // A standard string is the same number in every font and stays as it
        // is; one of the font's own is written again and renumbered.
        if (sid as usize) < STANDARD_STRINGS.len() {
            new_sids.push(sid);
            continue;
        }
        let Some(&(from, to)) = strings.items.get(sid as usize - STANDARD_STRINGS.len()) else {
            new_sids.push(0);
            continue;
        };
        let text = bytes[from..to].to_vec();
        let at = match new_strings.iter().position(|it| it == &text) {
            Some(at) => at,
            None => {
                new_strings.push(text);
                new_strings.len() - 1
            }
        };
        new_sids.push((STANDARD_STRINGS.len() + at) as u16);
    }

    // The Top DICT names things too -- its version, its notice, the font's
    // full name -- and those are string ids like a glyph's. Renumbering the
    // strings without renumbering these leaves them pointing at whatever now
    // has that number, or past the end, and a reader refuses the font. That is
    // not a hypothetical: Ghostscript said "an embedded font is invalid" and
    // drew the page in a substitute face.
    let mut intern = |sid: f64| -> f64 {
        let sid = sid as usize;
        if sid < STANDARD_STRINGS.len() {
            return sid as f64;
        }
        let Some(&(from, to)) = strings.items.get(sid - STANDARD_STRINGS.len()) else {
            return 0.0;
        };
        let text = bytes[from..to].to_vec();
        let at = match new_strings.iter().position(|it| it == &text) {
            Some(at) => at,
            None => {
                new_strings.push(text);
                new_strings.len() - 1
            }
        };
        (STANDARD_STRINGS.len() + at) as f64
    };
    let top: BTreeMap<u16, Vec<f64>> = top
        .into_iter()
        .map(|(operator, values)| {
            let values = match SID_OPERATORS.contains(&operator) {
                true => values.iter().map(|value| intern(*value)).collect(),
                false => values,
            };
            (operator, values)
        })
        .collect();

    // Format 0: a name per glyph, past the first, which is always `.notdef`.
    let mut charset = vec![0u8];
    for sid in new_sids.iter().skip(1) {
        charset.extend(sid.to_be_bytes());
    }

    // The private data: the DICT and, right behind it, the local subroutines
    // it points at. The pointer inside the DICT is relative to the DICT's own
    // start, so as long as the two stay together in the same order it is still
    // right.
    let (private_dict, private_size) = match private_offset_of(&top, bytes)? {
        Some((from, local)) => {
            let size = top
                .get(&PRIVATE)
                .and_then(|values| values.first())
                .copied()
                .unwrap_or(0.0) as usize;
            let dict_bytes = bytes
                .get(from..from + size)
                .ok_or("the Private DICT is past the end of the CFF")?
                .to_vec();
            // Whatever sits between the DICT and its subroutines travels too.
            let gap = bytes
                .get(from + size..from + local)
                .ok_or("the local subroutines are past the end of the CFF")?
                .to_vec();
            let mut all = dict_bytes;
            all.extend(gap);
            (all, size)
        }
        None => match top.get(&PRIVATE).map(|values| values[..].to_vec()) {
            Some(values) if values.len() == 2 => {
                let (size, offset) = (values[0] as usize, values[1] as usize);
                (
                    bytes
                        .get(offset..offset + size)
                        .ok_or("the Private DICT is past the end of the CFF")?
                        .to_vec(),
                    size,
                )
            }
            _ => (Vec::new(), 0),
        },
    };

    // The subroutines. Most of what is left once the outlines are gone is
    // them: a font shares the parts of letters between its glyphs, and Latin
    // Modern's global subroutines are fourteen kilobytes. The ones a kept
    // glyph calls have to stay -- and so do the ones THOSE call -- but the
    // rest can be emptied. They are not removed: a subroutine is called by a
    // number biased by how many there are, so taking one away would renumber
    // the others and every charstring that calls them. An emptied one is a
    // single `return`, which is a byte.
    let local = match private_offset_of(&top, bytes)? {
        Some((from, local)) => index(bytes, from + local)?,
        None => Index::default(),
    };
    let used = Used::of(bytes, &kept, &gsubrs, &local);
    let gsubr_index = write_index(&emptied(bytes, &gsubrs, &used.global));
    let local_index = write_index(&emptied(bytes, &local, &used.local));

    // Everything before the Top DICT is unchanged, and the Top DICT is a fixed
    // length, so the rest can be laid out by adding up.
    let head_bytes = &bytes[..header];
    let name_index = raw_index(bytes, &names);
    let string_index = write_index(&new_strings);
    // The global subroutines stay: a charstring that was kept may call any of
    // them, and which ones it calls is not known without running it.

    let top_dict = |charset_at: usize, charstrings_at: usize, private_at: usize| -> Vec<u8> {
        let mut out = Vec::new();
        for (operator, values) in &top {
            match *operator {
                CHARSET => {
                    out.extend(offset_operand(charset_at as i64));
                    out.extend(operator_bytes(CHARSET));
                }
                CHAR_STRINGS => {
                    out.extend(offset_operand(charstrings_at as i64));
                    out.extend(operator_bytes(CHAR_STRINGS));
                }
                PRIVATE => {
                    out.extend(offset_operand(private_size as i64));
                    out.extend(offset_operand(private_at as i64));
                    out.extend(operator_bytes(PRIVATE));
                }
                // An Encoding is left out: a PDF says what each code means
                // through its own /Differences, and the standard encoding is
                // what a reader falls back to.
                ENCODING => {}
                other => {
                    for value in values {
                        out.extend(offset_operand(*value as i64));
                    }
                    out.extend(operator_bytes(other));
                }
            }
        }
        out
    };

    // The Top DICT's length does not depend on the offsets, so one pass with
    // zeros gives the length, and a second gives the file.
    // The Top DICT is the same length whatever offsets it holds, so the INDEX
    // around it can be measured with the offsets still zero.
    let top_index_length = write_index(&[top_dict(0, 0, 0)]).len();
    let mut at =
        header + name_index.len() + top_index_length + string_index.len() + gsubr_index.len();
    let charset_offset = at;
    at += charset.len();
    let charstrings_offset = at;
    let new_charstrings = write_index(&kept);
    at += new_charstrings.len();
    let private_offset = at;

    let mut out = Vec::new();
    out.extend(head_bytes);
    out.extend(name_index);
    out.extend(write_index(&[top_dict(
        charset_offset,
        charstrings_offset,
        private_offset,
    )]));
    out.extend(string_index);
    out.extend(gsubr_index);
    out.extend(&charset);
    out.extend(new_charstrings);
    out.extend(private_dict);
    out.extend(local_index);
    Ok(out)
}

/// Where the Private DICT begins and how far past it its subroutines are.
fn private_offset_of(
    top: &BTreeMap<u16, Vec<f64>>,
    bytes: &[u8],
) -> Result<Option<(usize, usize)>, String> {
    let Some(values) = top.get(&PRIVATE) else {
        return Ok(None);
    };
    let [size, offset] = values[..] else {
        return Ok(None);
    };
    let (from, to) = (offset as usize, offset as usize + size as usize);
    let Some(region) = bytes.get(from..to) else {
        return Err("the Private DICT is past the end of the CFF".into());
    };
    Ok(dict(region)?
        .get(&SUBRS)
        .and_then(|values| values.first())
        .map(|local| (from, *local as usize)))
}

/// Which subroutines the kept glyphs call, and which those call.
#[derive(Default)]
struct Used {
    global: BTreeSet<usize>,
    local: BTreeSet<usize>,
}

impl Used {
    fn of(bytes: &[u8], charstrings: &[Vec<u8>], gsubrs: &Index, local: &Index) -> Used {
        let mut used = Used::default();
        for charstring in charstrings {
            used.walk(bytes, charstring, gsubrs, local, 0);
        }
        used
    }

    /// Walk a charstring far enough to see which subroutines it calls.
    ///
    /// Only the operands matter, and only the last of them at a call: this is
    /// not an interpreter, and a subroutine whose number is computed rather
    /// than written would be missed -- which no font does, because the number
    /// has to be a constant for the bias to make sense.
    fn walk(
        &mut self,
        bytes: &[u8],
        charstring: &[u8],
        gsubrs: &Index,
        local: &Index,
        depth: usize,
    ) {
        if depth > 10 {
            return;
        }
        let mut stack: Vec<i64> = Vec::new();
        let mut at = 0usize;
        while at < charstring.len() {
            let b0 = charstring[at];
            at += 1;
            match b0 {
                32..=246 => stack.push(b0 as i64 - 139),
                247..=250 => {
                    let Some(&b1) = charstring.get(at) else {
                        return;
                    };
                    at += 1;
                    stack.push((b0 as i64 - 247) * 256 + b1 as i64 + 108);
                }
                251..=254 => {
                    let Some(&b1) = charstring.get(at) else {
                        return;
                    };
                    at += 1;
                    stack.push(-(b0 as i64 - 251) * 256 - b1 as i64 - 108);
                }
                28 => {
                    let Ok(value) = number(charstring, at, 2) else {
                        return;
                    };
                    at += 2;
                    stack.push(value as i16 as i64);
                }
                255 => {
                    let Ok(value) = number(charstring, at, 4) else {
                        return;
                    };
                    at += 4;
                    stack.push(value as i32 as i64 / 65536);
                }
                // callsubr and callgsubr.
                10 | 29 => {
                    let Some(which) = stack.pop() else { return };
                    let index = match b0 == 10 {
                        true => local,
                        false => gsubrs,
                    };
                    let at = which + bias(index.items.len());
                    let Ok(at) = usize::try_from(at) else {
                        continue;
                    };
                    let Some(&(from, to)) = index.items.get(at) else {
                        continue;
                    };
                    let fresh = match b0 == 10 {
                        true => self.local.insert(at),
                        false => self.global.insert(at),
                    };
                    if fresh {
                        self.walk(bytes, &bytes[from..to], gsubrs, local, depth + 1);
                    }
                }
                // hintmask and cntrmask carry a mask whose length depends on
                // how many stems have been declared; the operands before them
                // are stems, so counting them is enough to step over it.
                19 | 20 => {
                    at += (stack.len() / 2).max(1).div_ceil(8);
                    stack.clear();
                }
                12 => {
                    at += 1;
                    stack.clear();
                }
                _ => stack.clear(),
            }
        }
    }
}

/// The subroutines, with the ones nobody calls replaced by a `return`.
fn emptied(bytes: &[u8], index: &Index, used: &BTreeSet<usize>) -> Vec<Vec<u8>> {
    index
        .items
        .iter()
        .enumerate()
        .map(|(number, &(from, to))| match used.contains(&number) {
            true => bytes[from..to].to_vec(),
            false => vec![11],
        })
        .collect()
}

/// An INDEX as it was, bytes and all.
fn raw_index(bytes: &[u8], index: &Index) -> Vec<u8> {
    bytes[index.start..index.end].to_vec()
}

/// Write an INDEX around `items`: a count, the width of an offset, one offset
/// per item and one past the last, then the items.
fn write_index(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend((items.len() as u16).to_be_bytes());
    if items.is_empty() {
        // §5: an empty INDEX is its count and nothing else, not even a width.
        return out;
    }
    let total: usize = items.iter().map(Vec::len).sum();
    let width = match total + 1 {
        n if n <= 0xff => 1usize,
        n if n <= 0xffff => 2,
        n if n <= 0xff_ffff => 3,
        _ => 4,
    };
    out.push(width as u8);
    // The offsets count from one, and there is one more of them than there are
    // items, so the last says where the data ends.
    let mut offset = 1usize;
    for item in items {
        out.extend(&offset.to_be_bytes()[8 - width..]);
        offset += item.len();
    }
    out.extend(&offset.to_be_bytes()[8 - width..]);
    for item in items {
        out.extend(item);
    }
    out
}

/// A five-byte integer, whatever its value, so a DICT's length does not depend
/// on the offsets in it.
fn offset_operand(value: i64) -> Vec<u8> {
    let mut out = vec![29];
    out.extend((value as i32).to_be_bytes());
    out
}

fn operator_bytes(operator: u16) -> Vec<u8> {
    match operator > 0xff {
        true => vec![12, (operator & 0xff) as u8],
        false => vec![operator as u8],
    }
}

/// Which string names each glyph, in glyph order.
///
/// The three forms say the same thing in different ways: one name per glyph,
/// or a first name and how many follow it. What comes back is the plain list,
/// which is what a subset needs before it can write a new one.
fn charset_sids(bytes: &[u8], at: usize, glyphs: usize) -> Result<Vec<u16>, String> {
    // A predefined charset is the standard strings in order.
    if at <= 2 {
        return Ok((0..glyphs as u16).collect());
    }
    let format = *bytes.get(at).ok_or("a charset past the end of the CFF")?;
    let mut out = vec![0u16];
    let mut cursor = at + 1;
    match format {
        0 => {
            while out.len() < glyphs {
                out.push(number(bytes, cursor, 2)? as u16);
                cursor += 2;
            }
        }
        1 | 2 => {
            let extra = match format == 1 {
                true => 1,
                false => 2,
            };
            while out.len() < glyphs {
                let first = number(bytes, cursor, 2)?;
                let left = number(bytes, cursor + 2, extra)?;
                cursor += 2 + extra;
                for i in 0..=left {
                    if out.len() >= glyphs {
                        break;
                    }
                    out.push((first + i) as u16);
                }
            }
        }
        other => return Err(format!("charset format {other} is not one this reads")),
    }
    Ok(out)
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

    /// A subset holds the glyphs asked for, and the rest draw nothing.
    #[test]
    fn a_subset_keeps_the_charstrings_asked_for() {
        let Some(bytes) = installed("lmroman10-regular.otf") else {
            return;
        };
        let font = crate::sfnt::Sfnt::parse(bytes).expect("the font reads");
        let table = font.table("CFF").expect("a CFF").to_vec();
        let whole = Cff::parse(&table).expect("the CFF reads");

        // The glyphs of a word, by name, plus glyph zero.
        let wanted: BTreeSet<u16> = ["H", "e", "l", "o", "space"]
            .iter()
            .filter_map(|name| {
                whole
                    .glyph_names
                    .iter()
                    .position(|it| it == name)
                    .map(|at| at as u16)
            })
            .chain([0])
            .collect();
        assert!(wanted.len() >= 5, "{wanted:?}");

        let cut = subset(&table, &wanted).expect("the subset");
        assert!(
            cut.len() * 4 < table.len(),
            "{} of {}",
            cut.len(),
            table.len()
        );

        // It is a CFF, with the same glyphs numbered the same way.
        let smaller = Cff::parse(&cut).expect("the subset is a CFF");
        assert_eq!(smaller.len(), whole.len(), "the glyph count changed");
        assert_eq!(smaller.name, whole.name);
        // The glyphs that were kept keep their names; the rest are `.notdef`,
        // which is what lets their names be left out of the file.
        for glyph in &wanted {
            assert_eq!(
                smaller.glyph_names[*glyph as usize], whole.glyph_names[*glyph as usize],
                "glyph {glyph} lost its name"
            );
        }
        assert!(
            smaller
                .glyph_names
                .iter()
                .filter(|name| *name == ".notdef")
                .count()
                > whole.len() - 10,
            "the names of the dropped glyphs are still in the file"
        );

        // Every glyph asked for is there, byte for byte; every other draws
        // nothing.
        for glyph in 0..whole.len() as u16 {
            let was = &whole.charstrings[glyph as usize];
            let is = &smaller.charstrings[glyph as usize];
            match wanted.contains(&glyph) {
                true => assert_eq!(is, was, "glyph {glyph} changed"),
                false => assert_eq!(is, &[14], "glyph {glyph} is still drawn"),
            }
        }

        // And the widths of the kept glyphs still read, which means the
        // private data and its subroutines came across with their offsets
        // intact.
        for glyph in &wanted {
            assert_eq!(
                smaller.widths[*glyph as usize], whole.widths[*glyph as usize],
                "glyph {glyph} changed width"
            );
        }
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
