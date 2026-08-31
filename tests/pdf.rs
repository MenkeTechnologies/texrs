//! What this writes, against the programs that read PDF.
//!
//! A PDF is a graph with a table of byte offsets, and the file is refused
//! whole if one offset is wrong -- so "does it parse" is the test, and the only
//! honest way to ask is to hand the file to readers that had no part in
//! writing it. Three of them, that fail differently: `pdfinfo` reads the
//! trailer and the page tree, `pdftotext` reads the content streams and the
//! font encodings, and Ghostscript interprets the whole thing.

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
        assert!(report.contains("Pages:          1"), "{report}");
        // 612 by 792 points is 8.5 by 11 inches, which is what was asked for.
        assert!(report.contains("612 x 792"), "{report}");
        assert!(report.contains("PDF version:    1.7"), "{report}");
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
        assert!(report.contains("Pages:          5"), "{report}");
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
    assert!(report.contains("Pages:          1"));

    // Move every offset in the table along by one byte. The table is found by
    // `\nxref\n` rather than `xref`, because `startxref` ends in one too --
    // and on the bytes rather than on a string, because the header's binary
    // comment is not UTF-8 and every offset would shift.
    let at = good
        .windows(6)
        .rposition(|window| window == b"\nxref\n")
        .expect("a table");
    let mut broken = good.clone();
    let mut cursor = at + 6;
    // Past the `0 6` line that says which objects follow.
    cursor += broken[cursor..]
        .iter()
        .position(|&b| b == b'\n')
        .expect("a subsection header")
        + 1;
    while let Some(line) = broken[cursor..].iter().position(|&b| b == b'\n') {
        let entry = &broken[cursor..cursor + line];
        if entry.len() != 19 {
            break;
        }
        if let Ok(offset) = String::from_utf8_lossy(&entry[..10]).parse::<usize>() {
            let moved = format!("{:010}", offset + 1);
            broken[cursor..cursor + 10].copy_from_slice(moved.as_bytes());
        }
        cursor += line + 1;
    }
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

    // And the file really carries the font rather than naming it: a PFB is
    // tens of kilobytes, and a page that named a font would be two.
    let size = std::fs::metadata(&path).expect("the file").len();
    assert!(size > 20_000, "{size} bytes is too small to hold a font");

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
    page.text_in(Font::Embedded(Box::new(cmr10)), 12.0, 72.0, 700.0, "A");
    let bytes = document(&[page]);
    let text = String::from_utf8_lossy(&bytes).into_owned();

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
        assert!(report.contains("Pages:          1"), "{report}");
    }

    // What a reader finds inside it. `-listonly` is xpdf's spelling: `-list`
    // alone wants somewhere to write the pictures too.
    let Ok(listed) = Command::new("pdfimages")
        .arg("-listonly")
        .arg(&path)
        .output()
    else {
        return;
    };
    let listed = String::from_utf8_lossy(&listed.stdout).to_string();
    let rows: Vec<&str> = listed
        .lines()
        .filter(|line| line.contains("width="))
        .collect();
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
        assert!(report.contains("Pages:          1"), "{report}");
    }

    // A reader sees two pictures: the colour and its mask, the same size.
    let Ok(listed) = Command::new("pdfimages")
        .arg("-listonly")
        .arg(&path)
        .output()
    else {
        return;
    };
    let listed = String::from_utf8_lossy(&listed.stdout).to_string();
    let rows: Vec<&str> = listed
        .lines()
        .filter(|line| line.contains("width="))
        .collect();
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
