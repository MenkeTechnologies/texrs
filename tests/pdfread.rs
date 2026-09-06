//! Reading PDFs that three different programs wrote.
//!
//! A PDF reader is only as good as the files it has been pointed at, and the
//! three here disagree about nearly everything a reader has to cope with:
//! pdftex writes a cross-reference stream and packs 46 of its 50 objects into
//! object streams; Ghostscript writes the older table of twenty-byte lines;
//! and this crate's own writer writes a table too, but a different one, with
//! no compression anywhere. What they agree about is the answer -- how many
//! pages, how big, and what is drawn on them -- which is what the reader is
//! held to.

use std::path::PathBuf;
use std::process::Command;

use texrs::pdf::Object;
use texrs::pdfread::Document;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("texrs_pdfread_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// What `pdfinfo` says, which is the oracle for the page count and the size.
fn info(path: &std::path::Path) -> Option<String> {
    let out = Command::new("pdfinfo").arg(path).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

/// A PDF from pdftex: a cross-reference stream, and objects inside objects.
#[test]
fn a_pdftex_file_reads_though_its_objects_are_packed_inside_others() {
    let dir = scratch("pdftex");
    std::fs::write(
        dir.join("t.tex"),
        "\\catcode`\\{=1 \\catcode`\\}=2\n\\pdfoutput=1\n\
         Hello from pdftex, and a rule: \\vrule width 40pt height 6pt\n\
         \\vfill\\eject\n Another page.\n\\bye\n",
    )
    .unwrap();
    let ran = Command::new("pdftex")
        .arg("-interaction=batchmode")
        .arg("t.tex")
        .current_dir(&dir)
        .output();
    let Ok(_) = ran else { return };
    let Ok(bytes) = std::fs::read(dir.join("t.pdf")) else {
        return;
    };

    let document = Document::read(bytes).expect("the pdf reads");
    let summary = document.summary();

    // Every object the trailer says the file holds is an object this can
    // fetch. That is the whole of what a cross-reference is for, and it is the
    // assertion that would fail if either form of it were read wrongly.
    let Some(Object::Integer(size)) = document.trailer.get("Size") else {
        panic!("the trailer states no size: {summary}");
    };
    let missing: Vec<u32> = (1..*size as u32)
        .filter(|number| document.object(*number).is_none())
        .collect();
    assert!(missing.is_empty(), "objects {missing:?} were not found");
    assert!(document.len() >= *size as usize - 1, "{summary}");

    // And most of them were inside other objects rather than at an offset: a
    // reader that stopped at byte offsets would find six of the fifteen.
    let packed: usize = summary
        .lines()
        .find_map(|line| line.strip_prefix("in streams")?.trim().parse().ok())
        .unwrap_or(0);
    assert!(
        packed * 2 > document.len(),
        "only {packed} of {} objects were packed away",
        document.len()
    );

    // Two pages, of the size pdfinfo reports.
    let pages = document.pages();
    assert_eq!(pages.len(), 2, "{summary}");
    if let Some(report) = info(&dir.join("t.pdf")) {
        assert!(report.contains("Pages:          2"), "{report}");
        // pdfinfo prints the size in points, to two places.
        let stated = report
            .lines()
            .find(|line| line.starts_with("Page size:"))
            .expect("a page size");
        let width: f64 = stated
            .split_whitespace()
            .nth(2)
            .and_then(|word| word.parse().ok())
            .expect("a width");
        let Object::Array(box_) = document.resolve(pages[0].get("MediaBox").expect("a MediaBox"))
        else {
            panic!("the MediaBox is not an array");
        };
        let ours = match box_[2] {
            Object::Integer(value) => value as f64,
            Object::Real(value) => value,
            _ => panic!("the MediaBox holds something odd"),
        };
        assert!((ours - width).abs() < 0.02, "{ours} against {width}");
    }

    // The content is text, and it is the text of the page: a content stream
    // that had not been inflated would be bytes nobody can read.
    let content = document.content(&pages[0]).expect("the content");
    let text = String::from_utf8_lossy(&content).to_string();
    assert!(
        text.contains("BT"),
        "no text object: {:?}",
        &text[..80.min(text.len())]
    );
    assert!(text.contains("Tf"), "no font is selected");
    assert!(
        text.contains("re") || text.contains(" l\n") || text.contains("Do"),
        "the rule is not drawn: {text}"
    );

    // The fonts a page names are objects this can follow to.
    let Object::Dict(resources) = document.resolve(pages[0].get("Resources").expect("resources"))
    else {
        panic!("the resources are not a dictionary");
    };
    let Object::Dict(fonts) = document.resolve(resources.get("Font").expect("a font")) else {
        panic!("the fonts are not a dictionary");
    };
    assert!(!fonts.is_empty());
    for (_, font) in fonts.iter() {
        let Object::Dict(font) = document.resolve(font) else {
            panic!("a font is not a dictionary");
        };
        assert_eq!(
            font.get("Type"),
            Some(&Object::Name("Font".into())),
            "what a page called a font is not one"
        );
        assert!(font.get("BaseFont").is_some());
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// A PDF from Ghostscript: the older cross-reference, and a different shape
/// throughout.
#[test]
fn a_ghostscript_file_reads_too() {
    let dir = scratch("gs");
    let made = Command::new("gs")
        .args(["-dNOPAUSE", "-dBATCH", "-dQUIET", "-sDEVICE=pdfwrite"])
        .arg(format!("-sOutputFile={}", dir.join("g.pdf").display()))
        .arg("-c")
        .arg("/Helvetica findfont 24 scalefont setfont 72 700 moveto (From Ghostscript) show showpage")
        .output();
    let Ok(made) = made else { return };
    if !made.status.success() {
        return;
    }

    let document = Document::open(dir.join("g.pdf")).expect("the pdf reads");
    let pages = document.pages();
    assert_eq!(pages.len(), 1, "{}", document.summary());
    assert!(document.len() > 3, "only {} objects", document.len());

    let content = document.content(&pages[0]).expect("the content");
    let text = String::from_utf8_lossy(&content).to_string();
    assert!(text.contains("BT"), "{text}");
    // Ghostscript writes the words as a string in the content stream.
    assert!(
        text.contains("From Ghostscript") || text.contains("Tj"),
        "{text}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The writer in this crate, read back by the reader in this crate.
///
/// The two halves have to agree, and this is where a disagreement shows up
/// first: every object written must be an object found, with the value it was
/// given.
#[test]
fn what_this_crate_writes_is_what_it_reads() {
    use texrs::pdf::{document, Page};

    let mut page = Page::letter();
    page.text("Helvetica", 18.0, 72.0, 700.0, "A page with (parentheses)");
    page.rule(72.0, 650.0, 200.0, 3.0);
    let mut second = Page::letter();
    second.text("Times-Roman", 12.0, 72.0, 700.0, "and a second page");
    let bytes = document(&[page, second]);

    let read = Document::read(bytes).expect("what this wrote reads");
    let pages = read.pages();
    assert_eq!(pages.len(), 2);

    // The size that was asked for, and the text that was drawn.
    let Object::Array(box_) = read.resolve(pages[0].get("MediaBox").expect("a MediaBox")) else {
        panic!("no MediaBox");
    };
    assert_eq!(box_.len(), 4);
    let content = String::from_utf8_lossy(&read.content(&pages[0]).expect("content")).to_string();
    // The parentheses were escaped on the way in, and come back as they were.
    assert!(
        content.contains("(A page with \\(parentheses\\)) Tj"),
        "{content}"
    );
    assert!(content.contains("72 650 200 3 re f"), "{content}");
    let second = String::from_utf8_lossy(&read.content(&pages[1]).expect("content")).to_string();
    assert!(second.contains("and a second page"), "{second}");

    // The catalogue and the page tree really point at each other.
    let catalog = read.catalog().expect("a catalogue");
    assert_eq!(catalog.get("Type"), Some(&Object::Name("Catalog".into())));
    let Object::Dict(tree) = read.resolve(catalog.get("Pages").expect("a page tree")) else {
        panic!("the page tree is not a dictionary");
    };
    assert_eq!(tree.get("Count"), Some(&Object::Integer(2)));
}

/// What is not a PDF, and one that has been cut short.
#[test]
fn what_cannot_be_read_says_so() {
    assert!(Document::read(Vec::new()).is_err());
    assert!(Document::read(b"not a pdf".to_vec())
        .unwrap_err()
        .contains("%PDF-"));
    // The right beginning and nothing else.
    let e = Document::read(b"%PDF-1.7\nnothing else at all\n".to_vec()).unwrap_err();
    assert!(e.contains("cross-reference"), "{e}");

    // A real file with its tail cut off, which is how a truncated download
    // arrives.
    use texrs::pdf::{document, Page};
    let mut page = Page::letter();
    page.text("Helvetica", 12.0, 72.0, 700.0, "Hello");
    let bytes = document(&[page]);
    let cut = bytes[..bytes.len() / 2].to_vec();
    assert!(Document::read(cut).is_err());
}
