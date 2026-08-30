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
