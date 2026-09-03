//! A face goes into a file as the glyphs the document set, not whole.
//!
//! A modern TrueType face is half a megabyte and a book sets perhaps two
//! hundred of its glyphs, so embedding the file entire is most of the
//! difference between what texrs writes and what `lualatex` writes for the
//! same document: measured on `scifi2/docs/book.tex`, four faces came to
//! 1,215,388 bytes of font program in a 3,580,772-byte file whose LuaTeX
//! reference is 963,081. LuaTeX subsets, and names what it wrote with a
//! six-letter tag (PDF 32000 S9.6.4) so a reader never takes one cut of a face
//! for another.
//!
//! Font availability is not a property of the code, so every test here says
//! which face it needs and returns rather than failing on a machine without
//! one.

use texrs::pdf::{document, inflate_streams, Font, Page};

/// An installed family with TrueType outlines, as texrs would embed it.
///
/// CFF-flavoured OpenType is not a `/FontFile2` and `embed_family` refuses it,
/// so whatever comes back here has a `glyf` to cut.
fn a_face() -> Option<Font> {
    [
        "DejaVu Sans",
        "Arial Unicode MS",
        "Helvetica",
        "Verdana",
        "Tahoma",
        "Georgia",
        "Menlo",
        "Liberation Sans",
        "FreeSerif",
        "Noto Sans",
    ]
    .into_iter()
    .find_map(|family| match texrs::typeset::embed_family(family) {
        // Small enough to be a face nothing can be saved on: the point of the
        // test is a face with more in it than one page draws.
        Some(Font::TrueType { bytes, .. }) if bytes.len() < 60_000 => None,
        found => found,
    })
}

/// The `/Length1` of every embedded font program in a file, which is how long
/// the program is before any filter.
///
/// The file's dictionaries live in a deflated `/ObjStm`, so the bytes have to
/// be inflated before anything can be read out of them -- but a font program
/// is a stream of its own and its dictionary is in the file's own bytes either
/// way; inflating first is what makes this find both.
fn programs(pdf: &[u8]) -> Vec<usize> {
    let text = String::from_utf8_lossy(&inflate_streams(pdf)).into_owned();
    let mut out = Vec::new();
    for piece in text.split("/Length1").skip(1) {
        let digits: String = piece
            .trim_start()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if let Ok(length) = digits.parse() {
            out.push(length);
        }
    }
    out
}

/// A page of `text`, set in `font`, as a whole PDF.
fn page_of(font: Font, text: &str) -> Vec<u8> {
    let mut page = Page::letter();
    page.text_in(font, 12.0, 72.0, 700.0, text);
    document(&[page])
}

/// What goes into the file is what was drawn, and not the rest of the face.
#[test]
fn a_page_carries_the_glyphs_it_drew_and_not_the_face() {
    let Some(font) = a_face() else {
        eprintln!("skipping: no installed family embeds as TrueType");
        return;
    };
    let Font::TrueType { bytes: whole, .. } = &font else {
        unreachable!("a_face only returns TrueType")
    };
    let whole = whole.len();

    let pdf = page_of(font, "Handgloves");
    let embedded = programs(&pdf);
    assert_eq!(embedded.len(), 1, "one font, one program: {embedded:?}");

    // Ten letters out of a face of thousands. The face is what a document
    // NAMES; what it draws is the size the file should be.
    assert!(
        embedded[0] * 4 < whole,
        "the face went in whole: {} bytes of a {whole}-byte file",
        embedded[0]
    );
    // And it is still a font, rather than a smaller thing that is not one.
    assert!(embedded[0] > 500, "{} bytes is not a font", embedded[0]);
}

/// A subsetted font is named for its subset, in both places that name it.
///
/// S9.6.4: the PostScript name begins with six uppercase letters and a plus
/// sign. `/BaseFont` and the descriptor's `/FontName` are the same font and
/// must say the same name, or a reader has two fonts where the file has one.
#[test]
fn a_subset_is_named_for_itself() {
    let Some(font) = a_face() else {
        return;
    };
    let pdf = page_of(font, "Handgloves");
    let text = String::from_utf8_lossy(&inflate_streams(&pdf)).into_owned();

    let named = |key: &str| -> String {
        text.split(key)
            .nth(1)
            .map(|rest| rest.trim_start().trim_start_matches('/'))
            .map(|rest| {
                rest.chars()
                    .take_while(|c| !c.is_whitespace() && *c != '/' && *c != '>')
                    .collect()
            })
            .unwrap_or_default()
    };
    let base = named("/BaseFont");
    let descriptor = named("/FontName");
    assert_eq!(base, descriptor, "the two names of one font disagree");

    let (tag, face) = base.split_once('+').unwrap_or_else(|| {
        panic!("{base} carries no subset tag, so the face went in whole");
    });
    assert_eq!(tag.len(), 6, "a tag is six letters: {tag}");
    assert!(
        tag.chars().all(|c| c.is_ascii_uppercase()),
        "a tag is uppercase letters: {tag}"
    );
    assert!(!face.is_empty(), "the face lost its own name: {base}");
}

/// Two documents drawing different glyphs of one face get different tags, and
/// two drawing the same get the same.
///
/// This is what the tag is FOR: a reader that saw `ABCDEF+Face` in one file and
/// a different cut of `Face` under the same name in another would take the two
/// for one font.
#[test]
fn the_tag_says_which_subset_it_is() {
    let Some(font) = a_face() else {
        return;
    };
    // Six letters and a plus, or nothing: a name with no plus in it carries no
    // tag, and comparing two of those would compare the face's own name to
    // itself and hold for a file that subsets nothing.
    let tag_of = |pdf: &[u8]| -> Option<String> {
        let text = String::from_utf8_lossy(&inflate_streams(pdf)).into_owned();
        let name = text.split("/BaseFont /").nth(1)?;
        let tag: String = name.chars().take(6).collect();
        (name.chars().nth(6) == Some('+') && tag.chars().all(|c| c.is_ascii_uppercase()))
            .then_some(tag)
    };
    let one = tag_of(&page_of(font.clone(), "Handgloves"));
    let same = tag_of(&page_of(font.clone(), "Handgloves"));
    let other = tag_of(&page_of(font, "quartz jockeys"));
    assert!(one.is_some(), "no tag on the name to compare");
    assert_eq!(one, same, "the same subset must be named the same twice");
    assert_ne!(one, other, "two different subsets share a name: {one:?}");
}

/// The program in the file is a font, holding the outlines that were drawn and
/// not the ones that were not.
#[test]
fn the_program_in_the_file_holds_the_outlines_that_were_drawn() {
    let Some(font) = a_face() else {
        return;
    };
    let Font::TrueType { bytes, .. } = &font else {
        unreachable!("a_face only returns TrueType")
    };
    let whole = texrs::sfnt::Sfnt::parse(bytes.clone()).expect("the face parses");
    let map = whole.cmap().expect("cmap");
    let outline = |sfnt: &texrs::sfnt::Sfnt, glyph: u16| -> usize {
        let loca = sfnt.table("loca").expect("loca");
        let long = sfnt.head().expect("head").long_loca;
        let at = |g: usize| match long {
            true => u32::from_be_bytes(loca[g * 4..g * 4 + 4].try_into().unwrap()) as usize,
            false => u16::from_be_bytes(loca[g * 2..g * 2 + 2].try_into().unwrap()) as usize * 2,
        };
        at(glyph as usize + 1) - at(glyph as usize)
    };

    let text = "Handgloves";
    let pdf = page_of(font.clone(), text);
    // The program itself, out of the file's own BYTES: a font is not text and
    // a lossy string of it would not be one, so the search is over the bytes
    // and the length is what the dictionary said.
    let find = |hay: &[u8], needle: &[u8], from: usize| -> Option<usize> {
        hay[from..]
            .windows(needle.len())
            .position(|w| w == needle)
            .map(|at| at + from)
    };
    let dict = find(&pdf, b"/Length1", 0).expect("an embedded program");
    let body = find(&pdf, b"stream\n", dict).expect("its stream") + 7;
    let length: usize = programs(&pdf)[0];
    let cut =
        texrs::sfnt::Sfnt::parse(pdf[body..body + length].to_vec()).expect("the program is a font");

    // A code still finds its glyph, because a simple TrueType font is resolved
    // through the face's own cmap -- a subset that dropped it would draw a
    // blank page.
    let smaller = cut.cmap().expect("the subset keeps its cmap");
    for ch in text.chars() {
        let glyph = *map.get(&(ch as u32)).expect("the face has it");
        assert_eq!(
            smaller.get(&(ch as u32)),
            Some(&glyph),
            "{ch} no longer finds its glyph"
        );
        assert!(outline(&cut, glyph) > 0, "{ch} lost its outline");
    }
    // And a glyph nothing drew is gone: that is what made the file smaller.
    let drawn: Vec<u16> = text
        .chars()
        .filter_map(|c| map.get(&(c as u32)).copied())
        .collect();
    let untouched = (1..whole.num_glyphs().expect("maxp"))
        .find(|g| !drawn.contains(g) && outline(&whole, *g) > 0)
        .expect("some other glyph has an outline");
    assert_eq!(
        outline(&cut, untouched),
        0,
        "glyph {untouched} was kept though nothing drew it"
    );
}

/// How much was asked for does not change what is drawn.
///
/// The oracle is the page: one page draws a few glyphs and another draws those
/// same glyphs and every printable code besides, off the bottom of the sheet
/// where no ink lands. The two cuts of the face are very different sizes, and
/// Ghostscript must render the visible line pixel for pixel the same out of
/// both. That is what catches a subsetter whose correctness depends on the size
/// of the request -- the short-against-long `loca` switch and the `hmtx`
/// truncation both turn on it, and both are silent until something draws.
#[test]
fn what_is_drawn_does_not_depend_on_how_much_was_asked_for() {
    let Some(font) = a_face() else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("texrs_subset_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }

    let text = "Handgloves quartz jockeys";
    let few = dir.join("few.pdf");
    let many = dir.join("many.pdf");
    if std::fs::write(&few, page_of(font.clone(), text)).is_err() {
        return;
    }
    let mut page = Page::letter();
    page.text_in(font.clone(), 12.0, 72.0, 700.0, text);
    // Off the bottom of the page: the codes are carried, and no ink lands.
    let all: String = (32u8..=126).map(|c| c as char).collect();
    page.text_in(font, 12.0, 72.0, -400.0, &all);
    let _ = std::fs::write(&many, document(&[page]));
    assert!(
        programs(&std::fs::read(&many).unwrap())[0] > programs(&std::fs::read(&few).unwrap())[0],
        "the two pages asked for the same glyphs, so this compared nothing"
    );

    let render = |input: &std::path::Path, out: &std::path::Path| -> Option<Vec<u8>> {
        let ran = std::process::Command::new("gs")
            .args([
                "-dNOPAUSE",
                "-dBATCH",
                "-dQUIET",
                "-sDEVICE=pnggray",
                "-r72",
            ])
            .arg(format!("-sOutputFile={}", out.display()))
            .arg(input)
            .output()
            .ok()?;
        ran.status.success().then(|| std::fs::read(out).ok())?
    };
    let (Some(a), Some(b)) = (
        render(&many, &dir.join("many.png")),
        render(&few, &dir.join("few.png")),
    ) else {
        eprintln!("skipping: no ghostscript to render with");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    };
    // Two blank pages are also equal, so the page has to have ink on it.
    let ink = a.iter().filter(|&&byte| byte != 0xff).count();
    assert!(ink > 100, "only {ink} bytes of the page are not white");
    assert_eq!(a, b, "the smaller cut drew a different page");
    let _ = std::fs::remove_dir_all(&dir);
}
