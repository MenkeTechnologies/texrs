//! A page of one PDF drawn on a page of another, against the page it came
//! from.
//!
//! The oracle is the one the subsetters use, and for the same reason: what
//! matters is not which objects were copied but whether the picture arrives.
//! So a document is made by pdftex, its first page is included into a new file
//! at the same size and place, and the two are rendered and compared pixel for
//! pixel. Anything the copy left behind -- a font, its descriptor, the file
//! inside that -- shows up as a difference in the ink.

use std::path::PathBuf;
use std::process::Command;

use texrs::pdf::{Dict, Object, Pdf};
use texrs::pdfread::Document;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("texrs_include_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A document from pdftex: real text, in a font the file carries.
fn source(dir: &std::path::Path, body: &str) -> Option<PathBuf> {
    std::fs::write(
        dir.join("s.tex"),
        format!("\\catcode`\\{{=1 \\catcode`\\}}=2\n\\pdfoutput=1\n{body}\n\\bye\n"),
    )
    .ok()?;
    let ran = Command::new("pdftex")
        .arg("-interaction=batchmode")
        .arg("s.tex")
        .current_dir(dir)
        .output()
        .ok()?;
    let _ = ran;
    dir.join("s.pdf").exists().then(|| dir.join("s.pdf"))
}

/// A one-page PDF that draws `form` at `(x, y)`, scaled by `scale`.
fn drawing(pdf: &mut Pdf, form: Object, x: f64, y: f64, scale: f64) -> Vec<u8> {
    let tree = pdf.reserve();
    let content = pdf.add(Object::Stream {
        dict: Dict::new(),
        data: format!("q {scale} 0 0 {scale} {x} {y} cm /Fig Do Q\n").into_bytes(),
    });
    let page = pdf.add(Object::Dict(Dict::from([
        ("Type", Object::name("Page")),
        ("Parent", Object::Reference(tree)),
        (
            "MediaBox",
            Object::Array(vec![
                Object::Integer(0),
                Object::Integer(0),
                Object::Integer(612),
                Object::Integer(792),
            ]),
        ),
        (
            "Resources",
            Object::Dict(Dict::from([(
                "XObject",
                Object::Dict(Dict::from([("Fig", form)])),
            )])),
        ),
        ("Contents", content),
    ])));
    pdf.fill(
        tree,
        Object::Dict(Dict::from([
            ("Type", Object::name("Pages")),
            ("Count", Object::Integer(1)),
            ("Kids", Object::Array(vec![page])),
        ])),
    );
    let catalog = pdf.add(Object::Dict(Dict::from([
        ("Type", Object::name("Catalog")),
        ("Pages", Object::Reference(tree)),
    ])));
    if let Object::Reference(number) = catalog {
        pdf.set_catalog(number);
    }
    pdf.finish()
}

/// Render a PDF to a raw bitmap -- raw, because comparing compressed bytes
/// says nothing about pixels.
fn rendered(path: &std::path::Path, out: &std::path::Path) -> Option<Vec<u8>> {
    let ran = Command::new("gs")
        .args(["-dNOPAUSE", "-dBATCH", "-sDEVICE=pgmraw", "-r72"])
        .arg(format!("-sOutputFile={}", out.display()))
        .arg(path)
        .output()
        .ok()?;
    let said = String::from_utf8_lossy(&ran.stderr).to_string();
    assert!(
        !said.contains("Error") && !said.contains("invalid"),
        "ghostscript refused {}: {said}",
        path.display()
    );
    ran.status.success().then(|| std::fs::read(out).ok())?
}

fn ink(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&byte| byte < 200).count()
}

/// An included page draws what the page drew.
#[test]
fn an_included_page_is_the_page_it_came_from() {
    let dir = scratch("same");
    let Some(path) = source(
        &dir,
        "Hello from the figure, with a rule: \\vrule width 60pt height 8pt",
    ) else {
        return;
    };
    let document = Document::open(&path).expect("the source reads");
    assert_eq!(document.pages().len(), 1);

    // The page is Letter, so drawing it at the origin unscaled puts every mark
    // exactly where it was.
    let (width, height) = texrs::include::size(&document, 1).expect("a size");
    assert!((width - 612.0).abs() < 1.0 && (height - 792.0).abs() < 1.0);

    let mut pdf = Pdf::new();
    let form = texrs::include::page(&mut pdf, &document, 1).expect("the page");
    let bytes = drawing(&mut pdf, form, 0.0, 0.0, 1.0);
    let ours = dir.join("ours.pdf");
    std::fs::write(&ours, bytes).unwrap();

    let (Some(a), Some(b)) = (
        rendered(&path, &dir.join("source.pgm")),
        rendered(&ours, &dir.join("ours.pgm")),
    ) else {
        return;
    };
    assert!(ink(&a) > 200, "the source page is blank: {} dark", ink(&a));
    assert_eq!(a, b, "the included page drew differently");

    // The font came across with it: the new file carries what the old one did.
    let Ok(fonts) = Command::new("pdffonts").arg(&ours).output() else {
        return;
    };
    let fonts = String::from_utf8_lossy(&fonts.stdout).to_string();
    let embedded = fonts
        .lines()
        .filter(|line| line.split_whitespace().any(|word| word == "yes"))
        .count();
    assert!(embedded >= 1, "no font was carried across: {fonts}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// The page is a drawing, so it can be put anywhere and at any size.
#[test]
fn an_included_page_can_be_put_where_the_document_wants_it() {
    let dir = scratch("scaled");
    let Some(path) = source(&dir, "A figure to be shrunk.") else {
        return;
    };
    let document = Document::open(&path).expect("the source reads");

    // Half size, in the middle of the page.
    let mut pdf = Pdf::new();
    let form = texrs::include::page(&mut pdf, &document, 1).expect("the page");
    let bytes = drawing(&mut pdf, form, 150.0, 200.0, 0.5);
    let ours = dir.join("half.pdf");
    std::fs::write(&ours, bytes).unwrap();

    let (Some(a), Some(b)) = (
        rendered(&path, &dir.join("source.pgm")),
        rendered(&ours, &dir.join("half.pgm")),
    ) else {
        return;
    };
    // It is the same drawing, so there is ink; it is somewhere else and
    // smaller, so it is not the same page.
    assert!(ink(&b) > 50, "the shrunk figure is blank");
    assert_ne!(a, b, "the figure was drawn at full size after all");
    // Half the size is roughly a quarter of the ink, and a page that had
    // failed to draw would have none.
    assert!(
        ink(&b) * 2 < ink(&a),
        "{} against {} dark pixels",
        ink(&b),
        ink(&a)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A page number that is not there, and a file that is not a PDF.
#[test]
fn a_page_that_is_not_there_says_so() {
    let dir = scratch("absent");
    let Some(path) = source(&dir, "One page only.") else {
        return;
    };
    let document = Document::open(&path).expect("the source reads");
    let mut pdf = Pdf::new();
    let e = texrs::include::page(&mut pdf, &document, 2).unwrap_err();
    assert!(e.contains("1 pages"), "{e}");
    assert!(texrs::include::size(&document, 2).is_none());

    let _ = std::fs::remove_dir_all(&dir);
}
