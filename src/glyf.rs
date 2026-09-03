//! TrueType outlines and cutting a font down to the glyphs a document used,
//! ported from `tt_glyf.c` in tectonic's `xdvipdfmx`.
//!
//! A PDF that names a font is small and a PDF that carries one is not: a modern
//! TrueType face is three hundred kilobytes, of which a paper set in it uses
//! perhaps eighty glyphs. `xdvipdfmx` embeds the eighty. That is what a subset
//! is, and it is the difference between a document somebody can send and one
//! they cannot.
//!
//! Cutting a font up means reading two tables the rest of this crate leaves
//! alone. `loca` says where each glyph's outline begins and ends; `glyf` holds
//! the outlines. Neither has to be understood to copy a glyph -- an outline is
//! opaque and goes across as bytes -- with one exception that is the whole
//! difficulty: a *composite* glyph has no outline of its own, only a list of
//! other glyphs to draw and where to put them. An e-acute is an e and an
//! acute. Drop the e and the accent floats alone, so a subset has to be closed
//! over what its glyphs are made of.
//!
//! What comes out keeps the glyph numbers it went in with. A subsetter that
//! renumbered would have to rewrite the `cmap` and every reference to a glyph
//! anywhere else in the file; leaving the numbers alone costs a few bytes of
//! empty `loca` entries and cannot go wrong.

use std::collections::BTreeSet;

use crate::sfnt::Sfnt;

/// What a glyph is: an outline, or a list of other glyphs.
#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    /// Negative for a composite; zero or more contours for an outline.
    pub contours: i16,
    pub bbox: [i16; 4],
    /// The glyphs a composite is made of, in the order it draws them.
    pub components: Vec<u16>,
    /// Where the glyph is in `glyf`, and how long it is.
    pub at: usize,
    pub length: usize,
}

impl Glyph {
    pub fn is_composite(&self) -> bool {
        self.contours < 0
    }

    /// A glyph of no length is a space: it has no outline at all, which is not
    /// the same as an outline of nothing.
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }
}

/// A font's outlines.
#[derive(Debug, Clone)]
pub struct Outlines {
    pub glyphs: Vec<Glyph>,
}

fn number(bytes: &[u8], at: usize, width: usize) -> Result<u32, String> {
    bytes
        .get(at..at + width)
        .map(|region| {
            region
                .iter()
                .fold(0u32, |value, &b| (value << 8) | b as u32)
        })
        .ok_or_else(|| format!("byte {at} is past the end of the table"))
}

fn signed(bytes: &[u8], at: usize) -> Result<i16, String> {
    Ok(number(bytes, at, 2)? as u16 as i16)
}

/// The offsets `loca` states, one more than there are glyphs.
///
/// `head` says whether they are shorts or longs, and a short is half the real
/// offset -- which is why a font with an odd-length glyph pads it.
pub fn offsets(font: &Sfnt) -> Result<Vec<usize>, String> {
    let loca = font.table("loca").ok_or("the font has no loca table")?;
    let long = font.head()?.long_loca;
    let glyphs = font.num_glyphs()? as usize;
    let mut out = Vec::with_capacity(glyphs + 1);
    for i in 0..=glyphs {
        out.push(match long {
            true => number(loca, i * 4, 4)? as usize,
            false => number(loca, i * 2, 2)? as usize * 2,
        });
    }
    Ok(out)
}

impl Outlines {
    /// Read every glyph's header, and what a composite is made of.
    pub fn read(font: &Sfnt) -> Result<Outlines, String> {
        let glyf = font.table("glyf").ok_or("the font has no glyf table")?;
        let offsets = offsets(font)?;
        let mut glyphs = Vec::with_capacity(offsets.len() - 1);
        for pair in offsets.windows(2) {
            let (at, next) = (pair[0], pair[1]);
            if next < at {
                return Err(format!("loca runs backwards, from {at} to {next}"));
            }
            let length = next - at;
            // A glyph of no length is one with no outline: a space.
            if length == 0 {
                glyphs.push(Glyph {
                    contours: 0,
                    bbox: [0; 4],
                    components: Vec::new(),
                    at,
                    length,
                });
                continue;
            }
            if at + 10 > glyf.len() {
                return Err(format!("a glyph at {at} is past the end of glyf"));
            }
            let contours = signed(glyf, at)?;
            let bbox = [
                signed(glyf, at + 2)?,
                signed(glyf, at + 4)?,
                signed(glyf, at + 6)?,
                signed(glyf, at + 8)?,
            ];
            let components = match contours < 0 {
                true => components_of(glyf, at + 10, at + length)?,
                false => Vec::new(),
            };
            glyphs.push(Glyph {
                contours,
                bbox,
                components,
                at,
                length,
            });
        }
        Ok(Outlines { glyphs })
    }

    pub fn len(&self) -> usize {
        self.glyphs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }

    /// Everything `wanted` is made of, and everything those are made of.
    ///
    /// This is what a subset cannot do without: an e-acute is an e and an
    /// acute, and a composite may be made of composites.
    pub fn closure(&self, wanted: impl IntoIterator<Item = u16>) -> BTreeSet<u16> {
        let mut out = BTreeSet::new();
        let mut todo: Vec<u16> = wanted.into_iter().collect();
        // Glyph zero is what a reader draws for anything missing, so it always
        // travels.
        todo.push(0);
        while let Some(glyph) = todo.pop() {
            if !out.insert(glyph) {
                continue;
            }
            if let Some(found) = self.glyphs.get(glyph as usize) {
                todo.extend(found.components.iter().copied());
            }
        }
        out
    }
}

/// Which glyphs a composite draws.
///
/// §  : a component is a pair of flags and a glyph number, then arguments
/// whose size the flags give, then a transformation whose size they also give.
/// Nothing here needs the transformation -- the bytes are copied -- but its
/// length has to be known to find the next component.
fn components_of(glyf: &[u8], mut at: usize, end: usize) -> Result<Vec<u16>, String> {
    const ARGS_ARE_WORDS: u16 = 0x0001;
    const HAVE_SCALE: u16 = 0x0008;
    const MORE_COMPONENTS: u16 = 0x0020;
    const HAVE_X_AND_Y_SCALE: u16 = 0x0040;
    const HAVE_TWO_BY_TWO: u16 = 0x0080;

    let mut out = Vec::new();
    loop {
        if at + 4 > end {
            return Err("a composite glyph runs past its own end".into());
        }
        let flags = number(glyf, at, 2)? as u16;
        out.push(number(glyf, at + 2, 2)? as u16);
        at += 4;
        at += match flags & ARGS_ARE_WORDS != 0 {
            true => 4,
            false => 2,
        };
        at += match flags {
            f if f & HAVE_TWO_BY_TWO != 0 => 8,
            f if f & HAVE_X_AND_Y_SCALE != 0 => 4,
            f if f & HAVE_SCALE != 0 => 2,
            _ => 0,
        };
        if flags & MORE_COMPONENTS == 0 {
            return Ok(out);
        }
        // A font whose components never end is damaged, and a subsetter that
        // followed it would read the next glyph as a component list.
        if out.len() > 256 {
            return Err("a composite glyph of more than 256 components".into());
        }
    }
}

/// Cut a font down to `wanted` and whatever those glyphs are made of.
///
/// The glyph numbers do not change, so everything that refers to a glyph --
/// the `cmap`, a PDF's own encoding -- still refers to the right one. What
/// changes is `glyf`, which keeps only the outlines that are wanted, and
/// `loca`, which points the rest at nothing.
pub fn subset(font: &Sfnt, wanted: impl IntoIterator<Item = u16>) -> Result<Vec<u8>, String> {
    let outlines = Outlines::read(font)?;
    let keep = outlines.closure(wanted);
    let glyf = font.table("glyf").ok_or("the font has no glyf table")?;

    // The new outlines, in glyph order, each padded to a multiple of four so
    // the short form of `loca` can address it.
    let mut new_glyf: Vec<u8> = Vec::new();
    let mut new_loca: Vec<usize> = Vec::with_capacity(outlines.len() + 1);
    for (number, glyph) in outlines.glyphs.iter().enumerate() {
        new_loca.push(new_glyf.len());
        if !keep.contains(&(number as u16)) || glyph.is_empty() {
            continue;
        }
        let bytes = glyf
            .get(glyph.at..glyph.at + glyph.length)
            .ok_or_else(|| format!("glyph {number} is past the end of glyf"))?;
        new_glyf.extend_from_slice(bytes);
        while !new_glyf.len().is_multiple_of(4) {
            new_glyf.push(0);
        }
    }
    new_loca.push(new_glyf.len());

    // Long offsets, so a glyph may sit anywhere; `head` is told.
    let mut loca_bytes = Vec::with_capacity(new_loca.len() * 4);
    for offset in &new_loca {
        loca_bytes.extend((*offset as u32).to_be_bytes());
    }
    let mut head = font
        .table("head")
        .ok_or("the font has no head table")?
        .to_vec();
    if head.len() > 51 {
        head[50] = 0;
        head[51] = 1;
    }

    // The tables a reader needs to draw with the font, and no others: the
    // layout tables and the names are what a subset is for leaving behind.
    let mut tables: Vec<(String, Vec<u8>)> = vec![
        ("head".into(), head),
        ("glyf".into(), new_glyf),
        ("loca".into(), loca_bytes),
    ];
    for tag in ["hhea", "maxp", "hmtx", "cmap", "cvt ", "fpgm", "prep"] {
        if let Some(bytes) = font.table(tag) {
            tables.push((tag.to_string(), bytes.to_vec()));
        }
    }
    Ok(assemble(&tables))
}

/// Write a font out of its tables: a directory, then the tables themselves,
/// each starting on a four-byte boundary.
fn assemble(tables: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut tables: Vec<&(String, Vec<u8>)> = tables.iter().collect();
    // §  : the directory is sorted by tag, which is how a reader finds a table
    // by bisection.
    tables.sort_by(|a, b| a.0.cmp(&b.0));

    let count = tables.len();
    let mut out = Vec::new();
    out.extend(0x0001_0000u32.to_be_bytes());
    out.extend((count as u16).to_be_bytes());
    // The three numbers after the count are a search hint, and a reader that
    // trusts them rather than the count is the reason they are written
    // correctly rather than as zeros.
    let entry = 16u16 * 2u16.pow((count as f64).log2() as u32);
    out.extend(entry.to_be_bytes());
    out.extend(((count as f64).log2() as u16).to_be_bytes());
    out.extend((count as u16 * 16 - entry).to_be_bytes());

    let mut at = 12 + count * 16;
    let mut directory = Vec::new();
    for (tag, bytes) in &tables {
        directory.extend(tag.as_bytes());
        directory.extend(checksum(bytes).to_be_bytes());
        directory.extend((at as u32).to_be_bytes());
        directory.extend((bytes.len() as u32).to_be_bytes());
        at += bytes.len().next_multiple_of(4);
    }
    out.extend(directory);
    for (_, bytes) in &tables {
        out.extend(bytes.iter());
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
    }
    out
}

/// A table's checksum: its bytes as big-endian words, added up and allowed to
/// wrap.
fn checksum(bytes: &[u8]) -> u32 {
    let mut sum = 0u32;
    for chunk in bytes.chunks(4) {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum = sum.wrapping_add(u32::from_be_bytes(word));
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clear_sans() -> Option<Sfnt> {
        Sfnt::open("/usr/local/texlive/2026/texmf-dist/fonts/truetype/intel/clearsans/ClearSans-Regular.ttf")
            .ok()
    }

    /// The outlines of a real font, and what its composites are made of.
    #[test]
    fn a_font_says_where_its_outlines_are_and_what_they_are_made_of() {
        let Some(font) = clear_sans() else { return };
        let outlines = Outlines::read(&font).expect("the outlines read");

        assert_eq!(outlines.len(), font.num_glyphs().expect("maxp") as usize);

        // Every glyph lies inside glyf, and the offsets only go forwards --
        // which is what says the loca table was read in the right width.
        let glyf = font.table("glyf").expect("glyf");
        let mut previous = 0usize;
        for (number, glyph) in outlines.glyphs.iter().enumerate() {
            assert!(glyph.at >= previous, "glyph {number} goes backwards");
            assert!(
                glyph.at + glyph.length <= glyf.len(),
                "glyph {number} runs past glyf"
            );
            previous = glyph.at;
        }

        // A letter has contours and no components; a space has neither.
        let cmap = font.cmap().expect("cmap");
        let a = outlines.glyphs[cmap[&(b'A' as u32)] as usize].clone();
        assert!(a.contours > 0, "an A is drawn, not assembled");
        assert!(a.bbox[2] > a.bbox[0] && a.bbox[3] > a.bbox[1]);
        let space = outlines.glyphs[cmap[&(b' ' as u32)] as usize].clone();
        assert!(space.is_empty(), "a space has no outline");

        // An accented letter is a composite, and what it is made of is in the
        // font: the letter and the accent.
        let Some(&eacute) = cmap.get(&0xe9) else {
            return;
        };
        let eacute = outlines.glyphs[eacute as usize].clone();
        assert!(eacute.is_composite(), "an e-acute is assembled");
        assert!(
            eacute.components.len() >= 2,
            "out of {} pieces",
            eacute.components.len()
        );
        let e = cmap[&(b'e' as u32)];
        assert!(
            eacute.components.contains(&e),
            "an e-acute is made of an e: {:?}",
            eacute.components
        );

        // And the closure follows that: asking for the e-acute keeps the e.
        let keep = outlines.closure([cmap[&0xe9]]);
        assert!(keep.contains(&e), "the e was left behind");
        assert!(keep.contains(&0), "glyph zero always travels");
    }

    /// A subset holds the glyphs that were asked for and nothing else.
    #[test]
    fn a_subset_keeps_what_was_asked_for_and_drops_the_rest() {
        let Some(font) = clear_sans() else { return };
        let outlines = Outlines::read(&font).expect("the outlines read");
        let cmap = font.cmap().expect("cmap");

        // The glyphs of one word, and an accented letter to bring its pieces.
        let wanted: Vec<u16> = "Hello"
            .chars()
            .chain(['\u{e9}'])
            .filter_map(|c| cmap.get(&(c as u32)).copied())
            .collect();
        assert!(wanted.len() > 4);
        let cut = subset(&font, wanted.clone()).expect("the subset");

        // It is a font, and a much smaller one.
        let smaller = Sfnt::parse(cut.clone()).expect("the subset is a font");
        let whole = std::fs::metadata(
            "/usr/local/texlive/2026/texmf-dist/fonts/truetype/intel/clearsans/ClearSans-Regular.ttf",
        )
        .map(|it| it.len() as usize)
        .unwrap_or(usize::MAX);
        assert!(cut.len() * 4 < whole, "{} against {whole}", cut.len());

        // The same number of glyphs, because the numbers did not change --
        // which is what keeps the cmap and a PDF's encoding pointing at the
        // right ones.
        assert_eq!(
            smaller.num_glyphs().expect("maxp"),
            font.num_glyphs().unwrap()
        );

        // Every table the subset carries begins where its directory says. This
        // is checked here rather than left to a reader: Ghostscript repairs a
        // font whose directory is out by a byte and draws the right page
        // anyway, so a page comparison cannot see that mistake.
        assert_eq!(
            smaller.head().expect("head").units_per_em,
            font.head().unwrap().units_per_em,
            "the head table is not where the directory says"
        );
        assert_eq!(
            smaller.hhea().expect("hhea").number_of_h_metrics,
            font.hhea().unwrap().number_of_h_metrics,
            "the hhea table is not where the directory says"
        );
        assert_eq!(
            smaller.cmap().expect("cmap"),
            font.cmap().unwrap(),
            "the cmap table is not where the directory says"
        );
        // And the tables really are the ones asked for.
        let mut tags: Vec<&str> = smaller.tables.iter().map(|t| t.tag.trim()).collect();
        tags.sort();
        assert!(tags.contains(&"glyf") && tags.contains(&"loca") && tags.contains(&"head"));
        assert!(
            !tags.contains(&"GSUB") && !tags.contains(&"name"),
            "a subset leaves the layout tables and the names behind: {tags:?}"
        );

        // Every glyph asked for is there, byte for byte as it was.
        let kept = Outlines::read(&smaller).expect("the subset's outlines");
        let original = font.table("glyf").expect("glyf");
        let now = smaller.table("glyf").expect("glyf");
        for glyph in outlines.closure(wanted.clone()) {
            let was = &outlines.glyphs[glyph as usize];
            let is = &kept.glyphs[glyph as usize];
            // Each outline starts on a four-byte boundary, so what loca says
            // may be up to three bytes longer than the outline itself. The
            // outline must be there unchanged; the padding is the writer's.
            assert!(
                is.length >= was.length && is.length < was.length + 4,
                "glyph {glyph} is {} bytes where it was {}",
                is.length,
                was.length
            );
            assert_eq!(
                &now[is.at..is.at + was.length],
                &original[was.at..was.at + was.length],
                "glyph {glyph} changed"
            );
        }

        // And a glyph nobody asked for is gone.
        let dropped = (0..outlines.len() as u16)
            .find(|g| {
                !outlines.closure(wanted.clone()).contains(g)
                    && !outlines.glyphs[*g as usize].is_empty()
            })
            .expect("some glyph was dropped");
        assert!(
            kept.glyphs[dropped as usize].is_empty(),
            "glyph {dropped} is still there"
        );
    }

    /// What is not a font to cut up.
    #[test]
    fn a_font_without_outlines_is_refused() {
        // Latin Modern has CFF outlines, so it has no glyf to subset.
        let found = std::process::Command::new("kpsewhich")
            .arg("lmroman10-regular.otf")
            .output();
        let Ok(found) = found else { return };
        let path = String::from_utf8_lossy(&found.stdout).trim().to_string();
        let Ok(font) = Sfnt::open(&path) else { return };
        assert!(Outlines::read(&font).unwrap_err().contains("glyf"));
        assert!(subset(&font, [1u16]).unwrap_err().contains("glyf"));
    }
}
