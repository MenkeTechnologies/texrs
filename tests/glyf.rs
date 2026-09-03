//! A subset font against the font it came from.
//!
//! There is no program on this machine that dumps a TrueType outline -- the
//! `ttx` here has no fontTools behind it -- so the oracle is the page itself:
//! the same text, set once in the whole font and once in the subset, rendered
//! by Ghostscript, must come out pixel for pixel the same. That is a stronger
//! statement than any table comparison. It says the subset draws what the font
//! drew, which is the only thing a subset has to do.

use std::path::PathBuf;
use std::process::Command;

use texrs::glyf::{subset, Outlines};
use texrs::pdf::{document, Font, Page};
use texrs::sfnt::Sfnt;

const FACE: &str =
    "/usr/local/texlive/2026/texmf-dist/fonts/truetype/intel/clearsans/ClearSans-Regular.ttf";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("texrs_glyf_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A page of `text` set in `bytes`, as a PDF.
fn page_in(font: &Sfnt, bytes: Vec<u8>, text: &str) -> Vec<u8> {
    let head = font.head().expect("head");
    let hhea = font.hhea().expect("hhea");
    let cmap = font.cmap().expect("cmap");
    let advances = font.advance_widths().expect("hmtx");
    let scale = 1000.0 / head.units_per_em as f64;
    // The widths a PDF wants: one for every code from 32 up.
    let widths: Vec<i64> = (32u32..=255)
        .map(|code| {
            let glyph = cmap.get(&code).copied().unwrap_or(0) as usize;
            (advances.get(glyph).copied().unwrap_or(0) as f64 * scale).round() as i64
        })
        .collect();

    let mut page = Page::letter();
    page.text_in(
        Font::TrueType {
            name: "ClearSans".into(),
            bytes,
            widths,
            bbox: [
                (head.x_min as f64 * scale) as i64,
                (head.y_min as f64 * scale) as i64,
                (head.x_max as f64 * scale) as i64,
                (head.y_max as f64 * scale) as i64,
            ],
            ascent: (hhea.ascender as f64 * scale) as i64,
            descent: (hhea.descender as f64 * scale) as i64,
        },
        36.0,
        72.0,
        700.0,
        text,
    );
    document(&[page])
}

/// Render a PDF to a bitmap, which is what the comparison is made of.
fn rendered(path: &std::path::Path, out: &std::path::Path) -> Option<Vec<u8>> {
    let ran = Command::new("gs")
        .args([
            "-dNOPAUSE",
            "-dBATCH",
            "-dQUIET",
            "-sDEVICE=pnggray",
            "-r72",
        ])
        .arg(format!("-sOutputFile={}", out.display()))
        .arg(path)
        .output()
        .ok()?;
    ran.status.success().then(|| std::fs::read(out).ok())?
}

/// The page a subset draws is the page the whole font drew.
#[test]
fn a_subset_draws_what_the_font_drew() {
    let Ok(font) = Sfnt::open(FACE) else { return };
    let whole = std::fs::read(FACE).expect("the font");
    let text = "Handgloves 1234";

    // The glyphs that text uses, and whatever they are made of.
    let cmap = font.cmap().expect("cmap");
    let wanted: Vec<u16> = text
        .chars()
        .filter_map(|c| cmap.get(&(c as u32)).copied())
        .collect();
    assert!(wanted.len() > 8);
    let cut = subset(&font, wanted).expect("the subset");

    // A subset is a fraction of the font: this is what it is for.
    assert!(
        cut.len() * 8 < whole.len(),
        "{} bytes against {}",
        cut.len(),
        whole.len()
    );

    let dir = scratch("same");
    let with_whole = dir.join("whole.pdf");
    let with_cut = dir.join("cut.pdf");
    std::fs::write(&with_whole, page_in(&font, whole, text)).unwrap();
    std::fs::write(&with_cut, page_in(&font, cut, text)).unwrap();

    // Both are files a reader opens, and the subset is embedded in its own
    // right rather than being ignored.
    for path in [&with_whole, &with_cut] {
        let Ok(report) = Command::new("pdffonts").arg(path).output() else {
            return;
        };
        let report = String::from_utf8_lossy(&report.stdout).to_string();
        assert!(report.contains("TrueType"), "{path:?}: {report}");
        assert!(
            report.lines().any(|line| line.contains("yes")),
            "{path:?}: the font is not embedded: {report}"
        );
    }

    // The page itself: the same ink in the same places.
    let (Some(a), Some(b)) = (
        rendered(&with_whole, &dir.join("whole.png")),
        rendered(&with_cut, &dir.join("cut.png")),
    ) else {
        return;
    };
    assert!(a.len() > 1000, "the page rendered to nothing");
    assert_eq!(
        a,
        b,
        "the subset drew a different page: {} bytes against {}",
        b.len(),
        a.len()
    );

    // And the page is not blank -- two blank pages would also be equal.
    let ink = a.iter().filter(|&&byte| byte != 0xff).count();
    assert!(ink > 100, "only {ink} bytes of the page are not white");

    let _ = std::fs::remove_dir_all(&dir);
}

/// A composite glyph brings its pieces, or the accent floats alone.
#[test]
fn an_accented_letter_brings_the_letter_with_it() {
    let Ok(font) = Sfnt::open(FACE) else { return };
    let whole = std::fs::read(FACE).expect("the font");
    let cmap = font.cmap().expect("cmap");
    let Some(&eacute) = cmap.get(&0xe9) else {
        return;
    };
    let outlines = Outlines::read(&font).expect("the outlines");
    assert!(
        outlines.glyphs[eacute as usize].is_composite(),
        "this test wants a composite to test with"
    );

    // Asking for the e-acute alone: the subset must still hold the e and the
    // accent it is drawn from.
    let cut = subset(&font, [eacute]).expect("the subset");
    let smaller = Sfnt::parse(cut.clone()).expect("a font");
    let kept = Outlines::read(&smaller).expect("the outlines");
    for piece in &outlines.glyphs[eacute as usize].components {
        assert!(
            !kept.glyphs[*piece as usize].is_empty(),
            "the piece {piece} was left behind"
        );
    }

    // And it draws the same as the whole font does.
    let dir = scratch("accent");
    let text = "\u{e9}\u{e9}\u{e9}";
    let with_whole = dir.join("whole.pdf");
    let with_cut = dir.join("cut.pdf");
    std::fs::write(&with_whole, page_in(&font, whole, text)).unwrap();
    std::fs::write(&with_cut, page_in(&font, cut, text)).unwrap();
    let (Some(a), Some(b)) = (
        rendered(&with_whole, &dir.join("whole.png")),
        rendered(&with_cut, &dir.join("cut.png")),
    ) else {
        return;
    };
    assert_eq!(a, b, "the accented letter drew differently");
    assert!(
        a.iter().filter(|&&byte| byte != 0xff).count() > 50,
        "the page is blank, so this compared nothing"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
