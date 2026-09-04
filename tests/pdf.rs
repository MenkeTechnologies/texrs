//! What this writes, against the programs that read PDF.
//!
//! A PDF is a graph with a table saying where each object is, and the file is
//! refused whole if one entry is wrong -- so "does it parse" is the test, and
//! the only honest way to ask is to hand the file to readers that had no part
//! in writing it. Three of them, that fail differently: `pdfinfo` reads the
//! table and the page tree, `pdftotext` reads the content streams and the
//! font encodings, and Ghostscript interprets the whole thing.
//!
//! What the table IS is PDF 1.5's `/XRef` stream, and most objects are inside
//! an `/ObjStm` rather than in the file's own bytes -- so the tests here that
//! look at a dictionary inflate first, and the one that breaks the table on
//! purpose has to re-encode it.

use std::path::PathBuf;
use std::process::Command;

use texrs::pdf::{document, Font, Page};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("texrs_pdf_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write `pages` to a file and hand back where it is.
fn write(dir: &std::path::Path, pages: &[Page]) -> PathBuf {
    let path = dir.join("out.pdf");
    std::fs::write(&path, document(pages)).expect("write the pdf");
    path
}

/// `pdfinfo`'s report, or `None` when this machine has no poppler.
///
/// A reader's complaints are on stderr and its exit status is often zero
/// anyway: xpdf reconstructs a file whose table is wrong and tells you so
/// rather than refusing. So a quiet stderr is part of the answer, and a test
/// that only read stdout would pass on a file every reader had to repair.
fn info(path: &std::path::Path) -> Option<String> {
    let out = Command::new("pdfinfo").arg(path).output().ok()?;
    let complaints = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        complaints.trim().is_empty(),
        "pdfinfo had to repair the file: {complaints}"
    );
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

/// How many pages `pdfinfo` reports, read off its `Pages:` line.
///
/// Read as FIELDS rather than matched as a literal. Poppler pads that column to
/// a width of its own choosing and widened it by one space somewhere before
/// 26.08, which failed five assertions here that are about the page COUNT and
/// not about poppler's alignment -- they had been failing since that version
/// landed on the machine, whatever the writer did.
fn pages_reported(report: &str) -> Option<usize> {
    report
        .lines()
        .find_map(|line| line.strip_prefix("Pages:"))
        .and_then(|rest| rest.trim().parse().ok())
}

/// What a reader says is inside the file, one line a picture, in the
/// `width=200 height=100 colorspace=DeviceRGB bpc=8` shape the assertions here
/// read.
///
/// The listing flag is not the same in the two readers that provide this
/// program: it is `-listonly` in xpdf and `-list` in poppler, and poppler
/// answers the flag it does not know by treating it as the FILE to open --
/// `I/O Error: Couldn't open file '-listonly'`, rc 1, nothing on stdout. The
/// tests below then found no pictures in a file that has two, so both were
/// failing on the reader rather than on the writer. Both spellings are tried.
///
/// Poppler also prints a COLUMN table where xpdf prints `key=value` pairs, so
/// its header is read and each row rewritten into the pairs the callers expect.
/// `None` when neither reader is installed.
fn listed_images(path: &std::path::Path) -> Option<Vec<String>> {
    for flag in ["-listonly", "-list"] {
        let Ok(out) = Command::new("pdfimages").arg(flag).arg(path).output() else {
            return None;
        };
        if !out.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        // xpdf's spelling needs no translation.
        let pairs: Vec<String> = text
            .lines()
            .filter(|line| line.contains("width="))
            .map(str::to_string)
            .collect();
        if !pairs.is_empty() {
            return Some(pairs);
        }
        // Poppler's: a header of column names, then a row per picture. The
        // names are not the keys the callers ask for, so the four that are
        // read here are renamed and the rest passed through under their own.
        let mut lines = text.lines().filter(|line| !line.trim().is_empty());
        let Some(header) = lines.next() else {
            continue;
        };
        let columns: Vec<&str> = header.split_whitespace().collect();
        if !columns.contains(&"width") {
            continue;
        }
        return Some(
            lines
                .filter(|line| !line.starts_with("---"))
                .filter_map(|line| {
                    let fields: Vec<&str> = line.split_whitespace().collect();
                    (fields.len() >= columns.len()).then(|| {
                        columns
                            .iter()
                            .zip(fields.iter())
                            .map(|(name, value)| {
                                // Poppler abbreviates both the column and the
                                // value: `color` where xpdf says `colorspace`,
                                // and `rgb` where it says `DeviceRGB`.
                                let (name, value) = match *name {
                                    "color" => (
                                        "colorspace",
                                        match *value {
                                            "gray" => "DeviceGray",
                                            "rgb" => "DeviceRGB",
                                            "cmyk" => "DeviceCMYK",
                                            other => other,
                                        },
                                    ),
                                    other => (other, *value),
                                };
                                format!("{name}={value}")
                            })
                            .collect::<Vec<String>>()
                            .join(" ")
                    })
                })
                .collect(),
        );
    }
    None
}

/// The text `pdftotext` finds.
fn extracted(path: &std::path::Path) -> Option<String> {
    let out = Command::new("pdftotext").arg(path).arg("-").output().ok()?;
    let complaints = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        complaints.trim().is_empty(),
        "pdftotext had to repair the file: {complaints}"
    );
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

/// A document of one page, read by three programs that did not write it.
#[test]
fn a_page_this_writes_is_a_page_a_reader_opens() {
    let dir = scratch("one");
    let mut page = Page::letter();
    page.text("Helvetica", 24.0, 72.0, 700.0, "Hello from texrs");
    page.text(
        "Times-Roman",
        12.0,
        72.0,
        660.0,
        "A second line (parenthesised)",
    );
    page.rule(72.0, 640.0, 200.0, 2.0);
    let path = write(&dir, &[page]);

    if let Some(report) = info(&path) {
        assert_eq!(pages_reported(&report), Some(1), "{report}");
        // 612 by 792 points is 8.5 by 11 inches, which is what was asked for.
        assert!(report.contains("612 x 792"), "{report}");
        // Fields rather than a literal, for the reason `pages_reported` gives:
        // the padding is poppler's and it has changed.
        assert_eq!(
            report
                .lines()
                .find_map(|line| line.strip_prefix("PDF version:"))
                .map(str::trim),
            Some("1.7"),
            "{report}"
        );
    }

    if let Some(text) = extracted(&path) {
        assert!(text.contains("Hello from texrs"), "{text:?}");
        // The parentheses were escaped in the stream; if they had not been,
        // the string would have ended early and this would be truncated.
        assert!(text.contains("A second line (parenthesised)"), "{text:?}");
    }

    // Ghostscript interprets the file rather than reading its structure, so it
    // is the one that complains about a content stream that does not run.
    if let Ok(out) = Command::new("gs")
        .args(["-dNOPAUSE", "-dBATCH", "-dQUIET", "-sDEVICE=nullpage"])
        .arg(&path)
        .output()
    {
        let said = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "ghostscript refused it: {said}");
        assert!(
            !said.contains("Error") && !said.contains("**"),
            "ghostscript complained: {said}"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// Several pages, each with its own text, in order.
#[test]
fn every_page_is_there_and_holds_what_was_drawn_on_it() {
    let dir = scratch("many");
    let pages: Vec<Page> = (1..=5)
        .map(|number| {
            let mut page = Page::letter();
            page.text(
                "Courier",
                14.0,
                100.0,
                700.0,
                &format!("page number {number}"),
            );
            page
        })
        .collect();
    let path = write(&dir, &pages);

    if let Some(report) = info(&path) {
        assert_eq!(pages_reported(&report), Some(5), "{report}");
    }
    if let Some(text) = extracted(&path) {
        for number in 1..=5 {
            assert!(text.contains(&format!("page number {number}")), "{text:?}");
        }
        // In order, which is the page tree being right rather than the pages
        // merely being present.
        let first = text.find("page number 1").expect("the first page");
        let last = text.find("page number 5").expect("the last page");
        assert!(first < last, "{text:?}");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// The table is what a reader trusts, so this breaks it on purpose and checks
/// that the readers notice -- which is what says the passing tests above mean
/// something.
#[test]
fn a_reader_refuses_a_file_whose_table_is_wrong() {
    let dir = scratch("broken");
    let mut page = Page::letter();
    page.text("Helvetica", 12.0, 72.0, 700.0, "Hello");
    let good = document(&[page]);

    // Only run this where a reader is strict enough to be worth asking.
    let path = dir.join("good.pdf");
    std::fs::write(&path, &good).unwrap();
    let Some(report) = info(&path) else {
        return;
    };
    assert_eq!(pages_reported(&report), Some(1));

    // Is this reader strict enough for the question to mean anything? Ask it
    // the easiest version: the same file with `startxref` pointing past the end
    // of it, so there is no table to read at all. A reader that reports THAT is
    // one whose silence about the subtler damage below is a verdict.
    //
    // Measured, poppler 26.08.0 is not such a reader: it reconstructs the file
    // by scanning for object headers, says nothing on stderr and exits 0
    // (`pdfinfo` on a texrs PDF with `startxref 999999` prints the full report,
    // rc=0). Every writer's output passes through that repair, so the test can
    // only report the reader's leniency, and it reports it by not asking.
    let probe = dir.join("nostart.pdf");
    let at = good
        .windows(9)
        .rposition(|window| window == b"startxref")
        .expect("startxref");
    let mut nostart = good[..at].to_vec();
    nostart.extend(format!("startxref\n{}\n%%EOF\n", good.len() + 4096).as_bytes());
    std::fs::write(&probe, &nostart).unwrap();
    let said = Command::new("pdfinfo")
        .arg(&probe)
        .output()
        .expect("pdfinfo");
    let complained =
        !said.status.success() || !String::from_utf8_lossy(&said.stderr).trim().is_empty();
    if !complained {
        return;
    }

    // Move every offset in the table along by one byte. The table is an
    // `/XRef` stream (§7.5.8), so this is not a text edit: `startxref` names
    // the object holding it, its entries are inflated, the rows that ARE byte
    // offsets -- type 1 -- are bumped, and the stream is deflated and written
    // back with the length it now has.
    let at = good
        .windows(9)
        .rposition(|window| window == b"startxref")
        .expect("startxref");
    let table: usize = String::from_utf8_lossy(&good[at + 9..])
        .split_whitespace()
        .next()
        .expect("an offset")
        .parse()
        .expect("a number");
    let widths = xref_widths(&good, table);
    let mut data = stream_data(&good, table);
    for entry in data.chunks_mut(1 + widths[1] + widths[2]) {
        if entry[0] != 1 {
            continue;
        }
        let field = &mut entry[1..1 + widths[1]];
        let mut offset = 0usize;
        for byte in field.iter() {
            offset = offset << 8 | *byte as usize;
        }
        offset += 1;
        let width = field.len();
        for (i, byte) in field.iter_mut().enumerate() {
            *byte = (offset >> ((width - 1 - i) * 8)) as u8;
        }
    }
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut encoder, &data).expect("deflate");
    let deflated = encoder.finish().expect("deflate");

    // The object is the last one in the file, so only its own dictionary and
    // the bytes after it are rebuilt -- and its `/Length`, which the new
    // deflate is unlikely to have left alone.
    let tail = String::from_utf8_lossy(&good[table..]).into_owned();
    let close = tail.find(">>").expect("the dictionary ends");
    let dict = &tail[..close + 2];
    let length: String = dict
        .split("/Length ")
        .nth(1)
        .expect("a length")
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let dict = dict.replace(
        &format!("/Length {length}"),
        &format!("/Length {}", deflated.len()),
    );
    let mut broken = good[..table].to_vec();
    broken.extend(dict.as_bytes());
    broken.extend(b"\nstream\n");
    broken.extend(&deflated);
    broken.extend(b"\nendstream\nendobj\n");
    broken.extend(format!("startxref\n{table}\n%%EOF\n").as_bytes());
    assert_ne!(broken, good, "the table was not changed");

    let path = dir.join("broken.pdf");
    std::fs::write(&path, &broken).unwrap();
    let out = Command::new("pdfinfo")
        .arg(&path)
        .output()
        .expect("pdfinfo");
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !out.status.success() || said.contains("Error") || said.contains("damaged"),
        "a reader accepted a file whose table is wrong, so the oracle proves nothing: {said}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// `pdffonts` on a document carrying a font in it.
fn fonts_in(path: &std::path::Path) -> Option<String> {
    let out = Command::new("pdffonts").arg(path).output().ok()?;
    let complaints = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        complaints.trim().is_empty(),
        "pdffonts had to repair the file: {complaints}"
    );
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

/// A document set in Computer Modern, carried in the file.
///
/// This is what a TeX document needs and a base-14 name cannot give: nobody
/// has Computer Modern installed, so the font travels with the document. The
/// test is whether a reader that did not write the file will take the font out
/// of it and draw with it -- `pdffonts` says whether it is there and embedded,
/// and Ghostscript refuses a `FontFile` whose three lengths do not describe the
/// font, because it has to decrypt it to draw anything.
#[test]
fn a_font_carried_in_the_file_is_one_a_reader_accepts() {
    let Ok(found) = Command::new("kpsewhich").arg("cmr10.pfb").output() else {
        return;
    };
    let pfb = String::from_utf8_lossy(&found.stdout).trim().to_string();
    if pfb.is_empty() {
        return;
    }
    let cmr10 = texrs::type1::Type1::open(&pfb).expect("the font reads");

    let dir = scratch("embedded");
    let mut page = Page::letter();
    page.text_in(
        Font::Embedded(Box::new(cmr10.clone())),
        24.0,
        72.0,
        700.0,
        "Hello from Computer Modern",
    );
    // Code 11 is an ff ligature in a TeX font and a vertical tab in anyone
    // else's, which is what the encoding in the file is for.
    page.text_in(
        Font::Embedded(Box::new(cmr10)),
        24.0,
        72.0,
        660.0,
        "o\u{b}ice",
    );
    let path = write(&dir, &[page]);

    if let Some(report) = fonts_in(&path) {
        // The name the font calls itself, the kind it is, and -- the part that
        // matters -- that it is in the file.
        assert!(report.contains("CMR10"), "{report}");
        assert!(report.contains("Type 1"), "{report}");
        let line = report
            .lines()
            .find(|line| line.contains("CMR10"))
            .expect("the font");
        let columns: Vec<&str> = line.split_whitespace().collect();
        assert!(
            columns.contains(&"yes"),
            "the font is not embedded: {line:?}"
        );
        // pdffonts' `uni` column says whether the file tells a reader what
        // each code MEANS rather than only which glyph to draw. It is `no`
        // without a ToUnicode map, and `no` again if the map's values are
        // malformed -- writing the codepoints three hex digits wide turns it
        // back off -- so this is the reader saying it read the map.
        let uni = columns.get(columns.len() - 3);
        assert_eq!(
            uni,
            Some(&"yes"),
            "the font carries no usable ToUnicode map: {line:?}"
        );
    }

    // What the text comes back as, through two engines that share no code:
    // xpdf reads the glyph names out of the encoding this wrote, and
    // Ghostscript draws the page and reports what it drew. Either of them
    // getting words back means the codes in the content stream reached the
    // right glyphs of the embedded font.
    if let Some(text) = extracted(&path) {
        assert!(text.contains("Hello from Computer Modern"), "{text:?}");
    }
    if let Ok(out) = Command::new("gs")
        .args([
            "-dNOPAUSE",
            "-dBATCH",
            "-dQUIET",
            "-sDEVICE=txtwrite",
            "-o",
            "-",
        ])
        .arg(&path)
        .output()
    {
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(
            text.contains("Hello from Computer Modern"),
            "ghostscript drew {text:?}"
        );
    }

    // Ghostscript decrypts the embedded font to draw with it, so it is the one
    // that catches a font this took apart wrongly.
    if let Ok(out) = Command::new("gs")
        .args(["-dNOPAUSE", "-dBATCH", "-dQUIET", "-sDEVICE=nullpage"])
        .arg(&path)
        .output()
    {
        let said = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "ghostscript refused it: {said}");
        assert!(!said.contains("Error"), "ghostscript complained: {said}");
    }

    // What no reader here checks: `Length1`, `Length2` and `Length3`. Both
    // xpdf and Ghostscript find the parts of a Type 1 font by reading it
    // rather than by trusting the numbers, and accept a file whose lengths are
    // wrong by a byte. The lengths are pinned instead by the test in
    // `type1.rs` that takes the font apart and checks each part is what it
    // should be -- a property, since there is no oracle for it.

    // And the file really carries the font rather than naming it -- a page that
    // named one is two kilobytes -- while carrying only the glyphs it drew.
    // cmr10.pfb is a quarter of a megabyte with its 132 outlines in it, and a
    // page setting one line of it goes in at a fraction of that; a file as big
    // as the font is one the subset did not happen for.
    let size = std::fs::metadata(&path).expect("the file").len();
    let whole = std::fs::metadata(&pfb).expect("the font").len();
    assert!(size > 8_000, "{size} bytes is too small to hold a font");
    assert!(
        size < whole,
        "{size} bytes carries the whole {whole}-byte font rather than a subset"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The widths and the encoding a reader is given for an embedded font.
#[test]
fn the_widths_and_the_encoding_are_the_fonts_own() {
    let Ok(found) = Command::new("kpsewhich").arg("cmr10.pfb").output() else {
        return;
    };
    let pfb = String::from_utf8_lossy(&found.stdout).trim().to_string();
    if pfb.is_empty() {
        return;
    }
    let cmr10 = texrs::type1::Type1::open(&pfb).expect("the font reads");

    let mut page = Page::letter();
    // An A and, at code 11, the ff ligature -- which is a vertical tab to
    // anyone but a TeX font, and is the reason the encoding is written out.
    // The font goes into the file cut to the glyphs the page drew, so the
    // ligature has to be one of them for the encoding to be asked about it.
    page.text_in(Font::Embedded(Box::new(cmr10)), 12.0, 72.0, 700.0, "A\u{b}");
    let bytes = document(&[page]);
    // Inflated, because the font dictionary and its descriptor are packed into
    // an object stream now and are not in the file's own bytes.
    let text = String::from_utf8_lossy(&texrs::pdf::inflate_streams(&bytes)).into_owned();

    // An A is 750 thousandths of an em, which is what the .tfm says too.
    assert!(text.contains("/FirstChar"), "{}", &text[..400]);
    assert!(text.contains("750"), "the widths are not in the file");
    // The encoding is written as differences, so a code means what the font
    // says and not what the reader assumes.
    assert!(text.contains("/Differences"), "no encoding");
    assert!(text.contains("/ff"), "the ligature is not in the encoding");
    // The three lengths that let a reader take the font apart.
    for key in ["/Length1", "/Length2", "/Length3"] {
        assert!(text.contains(key), "{key} is missing");
    }
    // Symbolic, because a TeX font is in none of the standard encodings.
    assert!(text.contains("/Flags 4"), "the font is not marked symbolic");
}

/// Text set through a ligature comes back as the letters it stands for.
///
/// A TeX font puts `ff` at code 11, where nobody else has a printable
/// character at all. Copying that out of a PDF means knowing that the glyph
/// called `ff` is two f's, which is what the ToUnicode map written beside the
/// font says.
#[test]
fn a_ligature_extracts_as_the_letters_it_stands_for() {
    let Ok(found) = Command::new("kpsewhich").arg("cmr10.pfb").output() else {
        return;
    };
    let pfb = String::from_utf8_lossy(&found.stdout).trim().to_string();
    if pfb.is_empty() {
        return;
    }
    let cmr10 = texrs::type1::Type1::open(&pfb).expect("the font reads");

    let dir = scratch("ligature");
    let mut page = Page::letter();
    // As TeX sets it: o, the ff ligature, i, c, e.
    page.text_in(
        Font::Embedded(Box::new(cmr10)),
        24.0,
        72.0,
        700.0,
        "o\u{b}ice",
    );
    let path = write(&dir, &[page]);

    if let Some(text) = extracted(&path) {
        let word: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        // Either spelling is right: the map says U+FB00, and a reader may hand
        // that back or the two letters it stands for.
        assert!(
            word.starts_with("office") || word.starts_with("o\u{fb00}ice"),
            "the ligature came out as {word:?}"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// A picture put into a PDF is the picture a reader takes out.
///
/// Nothing is decoded on the way in -- a JPEG becomes a `/DCTDecode` stream and
/// a PNG's data a `/FlateDecode` one with PNG's own row filters as the
/// predictor -- so the test is whether a reader that decodes for real agrees
/// about what arrived. `pdfimages -list` says what it found; `pdfimages -png`
/// takes it out again.
#[test]
fn a_picture_arrives_as_the_picture_it_was() {
    let dir = scratch("image");

    // A PNG, made by Ghostscript so the test does not carry one.
    let made = Command::new("gs")
        .args([
            "-dNOPAUSE",
            "-dBATCH",
            "-dQUIET",
            "-sDEVICE=png16m",
            "-r36",
            "-g200x100",
        ])
        .arg(format!("-sOutputFile={}", dir.join("in.png").display()))
        .arg("-c")
        .arg("0 0 moveto 200 100 lineto 4 setlinewidth stroke showpage")
        .output();
    let Ok(made) = made else { return };
    if !made.status.success() {
        return;
    }
    let png = texrs::image::open(dir.join("in.png")).expect("the png reads");
    assert_eq!((png.width, png.height), (200, 100));

    let mut page = Page::letter();
    page.image(png, 72.0, 500.0, 200.0, 100.0);
    // A JPEG beside it, if the installation has one.
    let jpeg = texrs::image::open("/usr/local/texlive/2026/texmf-dist/doc/eplain/xhyper.jpg");
    if let Ok(jpeg) = &jpeg {
        page.image(jpeg.clone(), 72.0, 300.0, 106.0, 116.0);
    }
    let path = write(&dir, &[page]);

    // The file still opens, and nothing had to be repaired.
    if let Some(report) = info(&path) {
        assert_eq!(pages_reported(&report), Some(1), "{report}");
    }

    // What a reader finds inside it. Both readers' spellings of the listing
    // are tried and translated to one shape: see `listed_images`.
    let Some(rows) = listed_images(&path) else {
        return;
    };
    let listed = rows.join("\n");
    assert_eq!(
        rows.len(),
        1 + jpeg.is_ok() as usize,
        "the reader found {} pictures: {listed}",
        rows.len()
    );

    // The PNG, by the numbers its header stated: a reader that had to guess
    // any of these would report something else.
    let png_row = rows
        .iter()
        .find(|row| row.contains("width=200"))
        .unwrap_or_else(|| panic!("no 200-wide picture: {listed}"));
    assert!(png_row.contains("height=100"), "{png_row}");
    assert!(png_row.contains("colorspace=DeviceRGB"), "{png_row}");
    assert!(png_row.contains("bpc=8"), "{png_row}");
    if let Ok(jpeg) = &jpeg {
        let row = rows
            .iter()
            .find(|row| row.contains(&format!("width={}", jpeg.width)))
            .unwrap_or_else(|| panic!("no {}-wide picture: {listed}", jpeg.width));
        assert!(row.contains(&format!("height={}", jpeg.height)), "{row}");
    }

    // And the pictures come out again. A JPEG comes out AS a JPEG, which is
    // the proof it went in whole rather than being decoded and recompressed.
    let root = dir.join("out");
    let taken = Command::new("pdfimages")
        .arg("-j")
        .arg(&path)
        .arg(root.to_string_lossy().as_ref())
        .output();
    if let Ok(taken) = taken {
        assert!(taken.status.success());
        let written: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
            .expect("the directory")
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("out-"))
            })
            .collect();
        assert!(!written.is_empty(), "the reader wrote no pictures");
        // Whatever it wrote, one of them is the picture that went in. xpdf
        // writes anything that is not a JPEG as a PPM, whose header is three
        // numbers after a magic -- easier to read here than to teach the image
        // reader a format nothing else needs.
        let sizes: Vec<(u32, u32)> = written
            .iter()
            .filter_map(|path| match texrs::image::open(path) {
                Ok(image) => Some((image.width, image.height)),
                Err(_) => {
                    let bytes = std::fs::read(path).ok()?;
                    let head = String::from_utf8_lossy(&bytes[..40.min(bytes.len())]).to_string();
                    let mut words = head.split_ascii_whitespace();
                    let magic = words.next()?;
                    if magic != "P6" && magic != "P5" && magic != "P4" {
                        return None;
                    }
                    Some((words.next()?.parse().ok()?, words.next()?.parse().ok()?))
                }
            })
            .collect();
        assert!(sizes.contains(&(200, 100)), "the reader wrote {sizes:?}");

        // And the PIXELS, not only the size. The picture is a black line on
        // white, so nearly every byte of it is 0xff -- and a reader that had
        // been told the wrong predictor or the wrong row width would decode
        // the row filter bytes as colour and hand back noise, at exactly the
        // same size. This is the only assertion here that looks past the
        // dictionary into what the stream actually decodes to.
        let ppm = written
            .iter()
            .find(|path| path.extension().is_some_and(|e| e == "ppm"))
            .and_then(|path| std::fs::read(path).ok());
        if let Some(ppm) = ppm {
            let white = ppm.iter().filter(|&&b| b == 0xff).count();
            assert!(
                white * 10 > ppm.len() * 9,
                "only {white} of {} bytes came out white, so the pixels decoded wrongly",
                ppm.len()
            );
        }
        if let Ok(jpeg) = &jpeg {
            assert!(
                written
                    .iter()
                    .any(|path| path.extension().is_some_and(|e| e == "jpg")),
                "the JPEG did not come out as a JPEG, so it was recompressed"
            );
            assert!(sizes.contains(&(jpeg.width, jpeg.height)), "{sizes:?}");
        }
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// A picture with an alpha channel becomes a picture and a soft mask.
///
/// This is the one case where the pixels are taken apart rather than copied:
/// PNG interleaves the alpha with the colour and PDF wants it separate. So the
/// data is inflated, un-filtered, split and deflated again -- four chances to
/// produce something the right size and full of noise, which is what the
/// pixel check below is for.
#[test]
fn a_transparent_picture_keeps_its_transparency() {
    let dir = scratch("alpha");
    // Ghostscript's pngalpha device writes RGBA, with the page transparent
    // where nothing was drawn.
    let made = Command::new("gs")
        .args([
            "-dNOPAUSE",
            "-dBATCH",
            "-dQUIET",
            "-sDEVICE=pngalpha",
            "-r36",
            "-g120x80",
        ])
        .arg(format!("-sOutputFile={}", dir.join("in.png").display()))
        .arg("-c")
        .arg("0 0 0 setrgbcolor 10 10 100 60 rectfill showpage")
        .output();
    let Ok(made) = made else { return };
    if !made.status.success() {
        return;
    }
    let png = texrs::image::open(dir.join("in.png")).expect("the png reads");
    assert_eq!((png.width, png.height), (120, 80));
    let alpha = png
        .alpha
        .clone()
        .expect("the picture carries an alpha channel");
    assert!(!alpha.is_empty());
    // The colour is three components a pixel now, not four: the alpha is out
    // of it.
    assert_eq!(png.colours, texrs::image::Colours::Rgb);

    let mut page = Page::letter();
    page.image(png, 72.0, 500.0, 120.0, 80.0);
    let path = write(&dir, &[page]);

    if let Some(report) = info(&path) {
        assert_eq!(pages_reported(&report), Some(1), "{report}");
    }

    // A reader sees two pictures: the colour and its mask, the same size.
    let Some(rows) = listed_images(&path) else {
        return;
    };
    let listed = rows.join("\n");
    assert_eq!(rows.len(), 2, "the reader found {listed}");
    assert!(
        rows.iter()
            .all(|row| row.contains("width=120") && row.contains("height=80")),
        "{listed}"
    );
    // One of them is grey and one is colour: that is the mask and the picture.
    assert!(
        rows.iter().any(|row| row.contains("DeviceGray")),
        "no soft mask: {listed}"
    );
    assert!(
        rows.iter().any(|row| row.contains("DeviceRGB")),
        "no picture: {listed}"
    );

    // And the pixels. The picture is a black rectangle on a transparent
    // ground, so the colour comes out mostly black and the mask mostly white
    // where the rectangle is. A split that went wrong -- a filter undone with
    // the wrong neighbour, a row read at the wrong offset -- gives noise at
    // exactly this size, so this is the assertion that means anything.
    let root = dir.join("out");
    let taken = Command::new("pdfimages")
        .arg(&path)
        .arg(root.to_string_lossy().as_ref())
        .output();
    if let Ok(taken) = taken {
        assert!(taken.status.success());
        let mut checked = 0usize;
        for entry in std::fs::read_dir(&dir).expect("the directory") {
            let path = entry.expect("an entry").path();
            if !path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("out-"))
            {
                continue;
            }
            let bytes = std::fs::read(&path).expect("the picture");
            // The mask is a PGM of one byte a pixel; the picture a PPM of
            // three. Either way the ink is at one end of the range and the
            // ground at the other, and noise would be spread across it.
            let pixels = &bytes[bytes.len().saturating_sub(120 * 60)..];
            let flat = pixels.iter().filter(|&&b| b == 0x00 || b == 0xff).count();
            assert!(
                flat * 10 > pixels.len() * 9,
                "{}: only {flat} of {} bytes are black or white, so the split went wrong",
                path.display(),
                pixels.len()
            );
            checked += 1;
        }
        assert_eq!(checked, 2, "the reader wrote {checked} pictures");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// The file is PDF 1.5's serialisation, which is the one LuaTeX writes.
///
/// Two things at once, because they are one decision (§7.5.7 and §7.5.8): the
/// objects that MAY be packed go into an `/ObjStm` and stop appearing in the
/// file's own bytes, and the classic table and trailer are replaced by an
/// `/XRef` stream. Measured on `Hello world.`, LuaTeX's first object is
/// `3 0 obj` -- a content stream -- and its page, resources, font, descriptor,
/// widths, page tree and catalogue are seven objects inside one `/ObjStm`,
/// with a `/Type /XRef` object last and no `trailer` keyword anywhere.
///
/// This walks the table rather than grepping for its keys: every entry has to
/// LAND somewhere, which is the part a reader refuses the file over.
#[test]
fn the_objects_are_packed_and_the_table_is_a_stream() {
    let mut page = Page::letter();
    page.text("Helvetica", 12.0, 72.0, 700.0, "Hello world.");
    let bytes = document(&[page]);
    let raw = String::from_utf8_lossy(&bytes).into_owned();

    // The classic form is gone: no `xref` table, no `trailer` dictionary.
    assert!(
        !raw.contains("\nxref\n"),
        "a classic cross-reference table is still written"
    );
    assert!(
        !raw.contains("trailer"),
        "a classic trailer is still written"
    );

    // The page dictionary is not in the file's bytes any more -- it is inside
    // the object stream, which is why `pdf_parity::shape` has to ask pdfinfo
    // rather than scan for `/Type /Page`.
    assert!(
        !raw.contains("/Type /Page"),
        "the page dictionary was not packed"
    );
    // The table. `startxref` names the object that holds it, and that object
    // is an `/XRef` stream with the three field widths of §7.5.8.
    let at = raw.rfind("startxref").expect("startxref");
    let start: usize = raw[at + "startxref".len()..]
        .split_whitespace()
        .next()
        .expect("an offset")
        .parse()
        .expect("a number");
    let head = String::from_utf8_lossy(&bytes[start..(start + 200).min(bytes.len())]).into_owned();
    assert!(head.contains(" 0 obj"), "startxref points at {head:?}");
    assert!(head.contains("/Type /XRef"), "startxref points at {head:?}");
    assert!(head.contains("/W [ 1 "), "no field widths: {head:?}");

    // And every entry lands where it says. A type 1 entry is a byte offset and
    // an object begins there; a type 2 entry names an object stream and an
    // index into it, and that stream's own header has to agree.
    let table = xref_entries(&bytes, start);
    assert!(
        table.len() > 5,
        "a one-page document has more objects than {}",
        table.len()
    );
    assert_eq!(table[0], (0, 0, 255), "object zero heads the free list");
    let mut packed = 0;
    let mut streams: Vec<usize> = Vec::new();
    for (number, entry) in table.iter().enumerate().skip(1) {
        let (kind, field2, field3) = *entry;
        match kind {
            1 => {
                let here = String::from_utf8_lossy(&bytes[field2..(field2 + 24).min(bytes.len())])
                    .into_owned();
                assert!(
                    here.starts_with(&format!("{number} 0 obj")),
                    "entry {number} points at {here:?}"
                );
            }
            2 => {
                packed += 1;
                let (kind, offset, _) = table[field2];
                assert_eq!(kind, 1, "object {number}'s object stream is itself packed");
                if !streams.contains(&offset) {
                    streams.push(offset);
                }
                let head = objstm_header(&bytes, offset);
                assert_eq!(
                    head.get(field3).map(|(n, _)| *n),
                    Some(number as u32),
                    "object {number} is not at index {field3} of its stream: {head:?}"
                );
            }
            other => panic!("object {number} has entry type {other}"),
        }
    }
    assert!(packed >= 3, "only {packed} objects were packed");

    // And what was packed is the document's structure: the page, the tree it
    // hangs off and the catalogue a reader starts at.
    let inside: String = streams
        .iter()
        .map(|offset| String::from_utf8_lossy(&stream_data(&bytes, *offset)).into_owned())
        .collect();
    for key in ["/Type /Page", "/Type /Pages", "/Type /Catalog"] {
        assert!(inside.contains(key), "{key} is in no object stream");
    }
}

/// The font descriptor's heights are the ones the font's own metrics state.
///
/// §9.8.1 asks a descriptor for `/Ascent`, `/Descent`, `/CapHeight` and
/// `/XHeight`, and a Type 1 font states none of them: they are in the `.afm`
/// beside it. Written off the bounding box instead -- which is what this did --
/// they are the extremes of the OUTLINES rather than the heights of the
/// letters, and for CMR10 that is 750 and -250 where the answers are 694 and
/// -194.
///
/// Measured against LuaTeX on `tests/pdf_cases/two_words.tex`: it writes
/// `/Ascent 694 /CapHeight 683 /Descent -194 /ItalicAngle 0 /StemV 69
/// /XHeight 431`, and cmr10.afm states `Ascender 694`, `CapHeight 683`,
/// `Descender -194`, `XHeight 431`. This reads that file and compares, so the
/// oracle is the metrics rather than a number copied into an assertion.
#[test]
fn the_descriptor_states_what_the_fonts_metrics_state() {
    let found = |name: &str| {
        let out = Command::new("kpsewhich").arg(name).output().ok()?;
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!path.is_empty()).then_some(path)
    };
    let (Some(pfb), Some(afm)) = (found("cmr10.pfb"), found("cmr10.afm")) else {
        return;
    };
    let metrics = std::fs::read_to_string(&afm).expect("the metrics read");
    let stated = |key: &str| -> f64 {
        metrics
            .lines()
            .take_while(|line| !line.starts_with("StartCharMetrics"))
            .find_map(|line| line.strip_prefix(&format!("{key} ")))
            .unwrap_or_else(|| panic!("{afm} states no {key}"))
            .trim()
            .parse()
            .expect("a number")
    };

    let cmr10 = texrs::type1::Type1::open(&pfb).expect("the font reads");
    let mut page = Page::letter();
    page.text_in(Font::Embedded(Box::new(cmr10)), 10.0, 72.0, 700.0, "Hello");
    let text =
        String::from_utf8_lossy(&texrs::pdf::inflate_streams(&document(&[page]))).into_owned();

    for (key, from) in [
        ("Ascent", "Ascender"),
        ("Descent", "Descender"),
        ("CapHeight", "CapHeight"),
        ("XHeight", "XHeight"),
    ] {
        let want = format!("/{key} {}", stated(from));
        assert!(
            text.contains(&want),
            "the descriptor does not say {want}: {}",
            text.split("/FontDescriptor")
                .nth(1)
                .unwrap_or(&text)
                .chars()
                .take(200)
                .collect::<String>()
        );
    }
    // The bounding box is still the bounding box, which is a different question
    // and a different pair of numbers.
    assert!(
        text.contains("/FontBBox [ -40 -250 1009 750 ]"),
        "the bounding box moved: {text}"
    );
    // §9.8.1's `/CharSet`, spelled the way LuaTeX spells it: a space before
    // each name. The glyphs the program carries, which are the ones the page
    // drew -- `Hello` is an H, an e, an l and an o -- because the font goes in
    // cut to them. Measured, luatex writes `/CharSet( /H /d /e /l /o /one
    // /period /r /w)` for `Hello world.` and its folio, which is the same list
    // for the same reason.
    assert!(
        text.contains("/CharSet ( /H /e /l /o)"),
        "no CharSet, or not in LuaTeX's spelling: {text}"
    );
}

/// A graphics state the content stream names is one the page's `/Resources`
/// carries.
///
/// Constant alpha has no operator: §11.6.4.4 puts it in a dictionary and
/// `/name gs` selects it. So `0.5 CA` does not exist, `/pgf@CA0.5 gs` is how a
/// half-transparent stroke is asked for, and a page emitting that name without
/// the dictionary behind it draws the stroke opaque -- silently, since an
/// unresolved resource name is not an error a reader reports.
///
/// This is what TikZ's `opacity=` needs from the writer: `Picture::ext_gstates`
/// says which dictionaries a picture's operators name, and until now there was
/// nowhere on a `Page` to put them.
#[test]
fn a_named_graphics_state_reaches_the_pages_resources() {
    let dir = scratch("gstate");
    let mut page = Page::letter();
    page.ext_gstate("pgf@CA0.5", "CA", 0.5);
    page.ext_gstate("pgf@ca0.25", "ca", 0.25);
    // Naming the same one twice is what a picture with two half-transparent
    // paths does, and the resource dictionary is keyed by the name.
    page.ext_gstate("pgf@CA0.5", "CA", 0.5);
    assert_eq!(page.ext_gstates.len(), 2);
    page.content
        .push_str("q /pgf@CA0.5 gs /pgf@ca0.25 gs 100 100 200 200 re B Q\n");
    let bytes = document(&[page]);

    // The page dictionary is packed into an object stream, so this reads the
    // inflated file rather than its own bytes.
    let text = String::from_utf8_lossy(&texrs::pdf::inflate_streams(&bytes)).into_owned();
    assert!(text.contains("/ExtGState"), "no ExtGState: {text}");
    assert!(
        text.contains("/pgf@CA0.5 << /CA 0.5 >>"),
        "the stroking state is not the one the operator names: {text}"
    );
    assert!(
        text.contains("/pgf@ca0.25 << /ca 0.25 >>"),
        "the non-stroking state is not the one the operator names: {text}"
    );

    // And a reader opens it without complaint, which is the part a malformed
    // resource dictionary fails.
    let path = dir.join("out.pdf");
    std::fs::write(&path, &bytes).unwrap();
    if let Some(report) = info(&path) {
        assert_eq!(pages_reported(&report), Some(1), "{report}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A TrueType face goes into the file carrying the glyphs its codes can name
/// and no others, and a reader still draws the text.
///
/// A simple `/TrueType` font is addressed by CHARACTER: one byte, WinAnsi says
/// what it means, and the font's own `cmap` says which glyph draws it. So the
/// glyphs worth keeping are the ones those 224 codes reach, and everything else
/// in the face -- Arimo carries about ten times as many -- is weight the
/// document cannot use. The test that matters is not that the file shrank but
/// that it still WORKS after shrinking: a subset with a broken `loca` is
/// smaller and draws nothing.
#[test]
fn a_truetype_face_is_embedded_subsetted_and_still_drawn() {
    let path = std::path::Path::new(
        "/usr/local/texlive/2026/texmf-dist/fonts/truetype/google/arimo/Arimo-Regular.ttf",
    );
    if !path.exists() {
        return;
    }
    let whole = std::fs::read(path).expect("the font file");
    let font = texrs::sfnt::Sfnt::parse(whole.clone()).expect("the font reads");
    let head = font.head().expect("head");
    let hhea = font.hhea().expect("hhea");
    let cmap = font.cmap().expect("cmap");
    let advances = font.advance_widths().expect("hmtx");
    let scale = |n: i64| n * 1000 / head.units_per_em as i64;
    // Codes 32..=255, in the thousandths of an em a PDF states widths in.
    let widths: Vec<i64> = (32u8..=255)
        .map(|code| {
            texrs::typeset::winansi_unicode(code)
                .and_then(|ch| cmap.get(&(ch as u32)))
                .and_then(|&glyph| advances.get(glyph as usize))
                .map(|&w| scale(w as i64))
                .unwrap_or(0)
        })
        .collect();

    let mut page = Page::letter();
    page.text_in(
        Font::TrueType {
            name: "Arimo".to_string(),
            bytes: whole.clone(),
            widths,
            bbox: [
                scale(head.x_min as i64),
                scale(head.y_min as i64),
                scale(head.x_max as i64),
                scale(head.y_max as i64),
            ],
            ascent: scale(hhea.ascender as i64),
            descent: scale(hhea.descender as i64),
        },
        12.0,
        72.0,
        700.0,
        "Handgloves",
    );
    let dir = scratch("subset");
    let path = write(&dir, &[page]);
    let bytes = std::fs::read(&path).expect("the pdf");

    // Smaller than the face it came from, by a lot: this is the whole point.
    assert!(
        bytes.len() * 2 < whole.len(),
        "the file is {} bytes and the font alone is {}, so nothing was dropped",
        bytes.len(),
        whole.len()
    );
    // `/Length1` is the font program's length, so it says what actually went in.
    let text = String::from_utf8_lossy(&texrs::pdf::inflate_streams(&bytes)).into_owned();
    let length1: usize = text
        .split("/Length1 ")
        .nth(1)
        .expect("a Length1")
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .expect("a number");
    assert!(
        length1 < whole.len(),
        "the embedded program is {length1} bytes, the whole font {}",
        whole.len()
    );

    // A reader agrees it is there, embedded, and a subset.
    if let Ok(out) = Command::new("pdffonts").arg(&path).output() {
        let listed = String::from_utf8_lossy(&out.stdout).to_string();
        let row = listed
            .lines()
            .find(|line| line.contains("Arimo"))
            .unwrap_or_else(|| panic!("Arimo is not in the file: {listed}"));
        let fields: Vec<&str> = row.split_whitespace().collect();
        assert!(row.contains("TrueType"), "{row}");
        // `... emb sub uni ...`: three yes/no columns before the object number.
        let object = fields
            .iter()
            .rposition(|f| *f == "yes" || *f == "no")
            .expect("the yes/no columns");
        assert_eq!(fields[object - 2], "yes", "not embedded: {row}");
        assert_eq!(fields[object - 1], "yes", "not a subset: {row}");
    }

    // And the glyphs still draw: a subset whose `loca` no longer lines up with
    // its `glyf` is smaller, valid to a table reader, and blank on the page.
    if let Some(found) = extracted(&path) {
        assert!(found.contains("Handgloves"), "the text is gone: {found}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The trailer names an information dictionary and an identifier, and the same
/// document written twice under a pinned clock is the same bytes.
///
/// LuaTeX writes both for every file it produces -- measured on
/// `tests/pdf_cases/two_words.tex`, `11 0 obj << /Producer (LuaTeX-1.24.0)
/// /Creator (TeX) /CreationDate (D:19691231190000-05'00') ... /Trapped /False >>`
/// and `/ID [ <23826C1D...> <23826C1D...> ]` in the `/XRef` dictionary --
/// and texrs wrote neither. Reproducibility is the reason to care: `/ID` is a
/// digest and `/CreationDate` a clock, and if either is drawn from the run
/// rather than from the document then no two builds of it are the same file and
/// byte parity is a question that cannot be asked.
#[test]
fn the_trailer_names_an_information_dictionary_and_a_stable_identifier() {
    let build = || {
        let mut page = Page::letter();
        page.text("Helvetica", 12.0, 72.0, 700.0, "Hello");
        document(&[page])
    };
    // SAFETY: single-threaded here, and the value is what the harness pins.
    unsafe { std::env::set_var("SOURCE_DATE_EPOCH", "0") };
    let once = build();
    let twice = build();
    assert_eq!(once, twice, "two builds of one document are two files");

    let text = String::from_utf8_lossy(&texrs::pdf::inflate_streams(&once)).into_owned();
    for key in ["/Producer", "/Creator", "/CreationDate", "/ModDate"] {
        assert!(text.contains(key), "{key} is not in the file");
    }
    // "This shall be the name False, not the boolean value false" (Table 317).
    assert!(text.contains("/Trapped /False"), "no /Trapped: {text}");
    // Epoch zero, as §7.9.4 spells a date. UT rather than a local offset, so
    // the same document on two machines is the same file.
    assert!(
        text.contains("(D:19700101000000Z)"),
        "the clock was not pinned: {text}"
    );
    // §14.4: two byte strings, equal when a file is first written.
    let id = text.split("/ID [").nth(1).expect("an /ID").to_string();
    let id = id.split(']').next().expect("the array ends");
    let halves: Vec<&str> = id.split_whitespace().collect();
    assert_eq!(halves.len(), 2, "/ID is not a pair: {id}");
    assert_eq!(halves[0], halves[1], "the two identifiers differ: {id}");
    assert_eq!(halves[0].len(), 34, "not sixteen bytes of hex: {id}");

    // The digest is of the CONTENTS, so a document that differs has a different
    // name for its bytes -- which is what §14.4 asks the identifier to be.
    let mut other = Page::letter();
    other.text("Helvetica", 12.0, 72.0, 700.0, "Goodbye");
    let other =
        String::from_utf8_lossy(&texrs::pdf::inflate_streams(&document(&[other]))).into_owned();
    let other = other.split("/ID [").nth(1).expect("an /ID").to_string();
    assert_ne!(
        halves[0],
        other.split_whitespace().next().expect("a first half"),
        "two different documents share one identifier"
    );
}

/// The entries of the `/XRef` stream that begins at `start`, as
/// `(type, field 2, field 3)`.
fn xref_entries(pdf: &[u8], start: usize) -> Vec<(usize, usize, usize)> {
    let widths = xref_widths(pdf, start);
    let data = stream_data(pdf, start);
    let row: usize = widths.iter().sum();
    assert_eq!(
        data.len() % row,
        0,
        "the table is not a whole number of rows"
    );
    data.chunks(row)
        .map(|entry| {
            let mut fields = [0usize; 3];
            let mut at = 0;
            for (i, width) in widths.iter().enumerate() {
                for byte in &entry[at..at + width] {
                    fields[i] = fields[i] << 8 | *byte as usize;
                }
                at += width;
            }
            (fields[0], fields[1], fields[2])
        })
        .collect()
}

/// The three field widths the `/XRef` stream at `start` declares.
fn xref_widths(pdf: &[u8], start: usize) -> Vec<usize> {
    let dict = String::from_utf8_lossy(&pdf[start..(start + 400).min(pdf.len())]).into_owned();
    let at = dict.find("/W [").expect("field widths");
    let widths: Vec<usize> = dict[at + 4..]
        .split(']')
        .next()
        .expect("a closed array")
        .split_whitespace()
        .map(|w| w.parse().expect("a width"))
        .collect();
    assert_eq!(widths.len(), 3, "{dict:?}");
    widths
}

/// The `(object number, offset)` pairs the `/ObjStm` at `start` begins with.
fn objstm_header(pdf: &[u8], start: usize) -> Vec<(u32, usize)> {
    let data = stream_data(pdf, start);
    let text = String::from_utf8_lossy(&data).into_owned();
    let head = text.split('\n').next().expect("a header line").to_string();
    head.split_whitespace()
        .collect::<Vec<&str>>()
        .chunks(2)
        .filter(|pair| pair.len() == 2)
        .map(|pair| {
            (
                pair[0].parse().expect("an object number"),
                pair[1].parse().expect("an offset"),
            )
        })
        .collect()
}

/// The inflated data of the stream object beginning at `start`.
fn stream_data(pdf: &[u8], start: usize) -> Vec<u8> {
    let at = pdf[start..]
        .windows(6)
        .position(|window| window == b"stream")
        .expect("a stream")
        + start
        + 6;
    let at = at + usize::from(pdf[at] == b'\r');
    let at = at + usize::from(pdf[at] == b'\n');
    let end = pdf[at..]
        .windows(9)
        .position(|window| window == b"endstream")
        .expect("an endstream")
        + at;
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&pdf[at..end]), &mut out)
        .expect("the stream inflates");
    out
}
