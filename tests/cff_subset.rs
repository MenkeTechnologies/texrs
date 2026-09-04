//! A subset CFF against the font it came from.
//!
//! The same oracle the TrueType subsetter has, and for the same reason: no
//! program here dumps a charstring, but a page can be rendered. The same text
//! set once in the whole font and once in the subset, drawn by Ghostscript,
//! must come out pixel for pixel identical.
//!
//! The PDF is built here rather than through a helper, because a CFF font
//! program is embedded differently from a TrueType one: it goes in as a
//! `FontFile3` of subtype `Type1C`, which is the bare CFF and not the OpenType
//! container it was cut out of.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;
use texrs::pdf::Dict;

use texrs::cff::{subset, Cff};
use texrs::pdf::{Object, Pdf};
use texrs::sfnt::Sfnt;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("texrs_cffsub_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn latin_modern() -> Option<(Sfnt, Vec<u8>)> {
    let found = Command::new("kpsewhich")
        .arg("lmroman10-regular.otf")
        .output()
        .ok()?;
    let path = String::from_utf8_lossy(&found.stdout).trim().to_string();
    let font = Sfnt::open(&path).ok()?;
    let table = font.table("CFF")?.to_vec();
    Some((font, table))
}

/// One page of `text`, set in `program` -- a bare CFF -- through the glyph
/// names `names` gives for each code.
fn page(program: Vec<u8>, names: &[(u8, String, f64)], text: &str) -> Vec<u8> {
    let mut pdf = Pdf::new();
    let tree = pdf.reserve();

    let file = pdf.add(Object::Stream {
        dict: Dict::from([("Subtype".to_string(), Object::name("Type1C"))]),
        data: program,
    });
    let descriptor = pdf.add(Object::dict([
        ("Type", Object::name("FontDescriptor")),
        ("FontName", Object::name("LMRoman10-Regular")),
        ("Flags", Object::Integer(4)),
        (
            "FontBBox",
            Object::Array(vec![
                Object::Integer(-430),
                Object::Integer(-290),
                Object::Integer(1417),
                Object::Integer(1127),
            ]),
        ),
        ("ItalicAngle", Object::Integer(0)),
        ("Ascent", Object::Integer(1127)),
        ("Descent", Object::Integer(-290)),
        ("CapHeight", Object::Integer(700)),
        ("StemV", Object::Integer(80)),
        ("FontFile3", file),
    ]));

    let first = names.first().map(|(code, _, _)| *code).unwrap_or(32);
    let mut differences = vec![Object::Integer(first as i64)];
    differences.extend(names.iter().map(|(_, name, _)| Object::name(name)));
    let encoding = pdf.add(Object::dict([
        ("Type", Object::name("Encoding")),
        ("Differences", Object::Array(differences)),
    ]));
    let font = pdf.add(Object::dict([
        ("Type", Object::name("Font")),
        ("Subtype", Object::name("Type1")),
        ("BaseFont", Object::name("LMRoman10-Regular")),
        ("FirstChar", Object::Integer(first as i64)),
        (
            "LastChar",
            Object::Integer(first as i64 + names.len() as i64 - 1),
        ),
        (
            "Widths",
            Object::Array(names.iter().map(|(_, _, w)| Object::Real(*w)).collect()),
        ),
        ("Encoding", encoding),
        ("FontDescriptor", descriptor),
    ]));

    let escaped: String = text
        .chars()
        .flat_map(|c| match c {
            '(' | ')' | '\\' => vec!['\\', c],
            other => vec![other],
        })
        .collect();
    let content = pdf.add(Object::Stream {
        dict: Dict::new(),
        data: format!("BT /F1 36 Tf 1 0 0 1 72 700 Tm ({escaped}) Tj ET\n").into_bytes(),
    });
    let page = pdf.add(Object::dict([
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
            Object::dict([("Font", Object::Dict(Dict::from([("F1".to_string(), font)])))]),
        ),
        ("Contents", content),
    ]));
    pdf.fill(
        tree,
        Object::dict([
            ("Type", Object::name("Pages")),
            ("Count", Object::Integer(1)),
            ("Kids", Object::Array(vec![page])),
        ]),
    );
    let catalog = pdf.add(Object::dict([
        ("Type", Object::name("Catalog")),
        ("Pages", Object::Reference(tree)),
    ]));
    if let Object::Reference(number) = catalog {
        pdf.set_catalog(number);
    }
    pdf.finish()
}

fn rendered(pdf: &[u8], dir: &std::path::Path, name: &str) -> Option<Vec<u8>> {
    let file = dir.join(format!("{name}.pdf"));
    std::fs::write(&file, pdf).ok()?;
    let out = dir.join(format!("{name}.pgm"));
    let ran = Command::new("gs")
        .args(["-dNOPAUSE", "-dBATCH", "-dQUIET", "-sDEVICE=pgmraw", "-r72"])
        .arg(format!("-sOutputFile={}", out.display()))
        .arg(&file)
        .output()
        .ok()?;
    // A reader that refuses the font draws the page in a substitute and says
    // so; the page would then differ, but this says which of the two it was.
    let said = String::from_utf8_lossy(&ran.stderr).to_string();
    assert!(
        !said.contains("invalid") && !said.contains("substitute"),
        "ghostscript refused the font: {said}"
    );
    ran.status.success().then(|| std::fs::read(out).ok())?
}

/// The page a subset draws is the page the whole font drew.
#[test]
fn a_cff_subset_draws_what_the_font_drew() {
    let Some((font, table)) = latin_modern() else {
        return;
    };
    let whole = Cff::parse(&table).expect("the CFF reads");
    let text = "Hello";

    // The glyphs that text needs, by the names the font gives them.
    let wanted: Vec<(u8, String, f64)> = text
        .bytes()
        .collect::<BTreeSet<u8>>()
        .into_iter()
        .filter_map(|code| {
            let cmap = font.cmap().ok()?;
            let glyph = *cmap.get(&(code as u32))? as usize;
            Some((
                code,
                whole.glyph_names.get(glyph)?.clone(),
                whole.widths.get(glyph).copied()?,
            ))
        })
        .collect();
    assert!(wanted.len() >= 4, "{wanted:?}");

    let keep: BTreeSet<u16> = font
        .cmap()
        .expect("cmap")
        .iter()
        .filter(|(code, _)| text.bytes().any(|b| b as u32 == **code))
        .map(|(_, glyph)| *glyph)
        .chain([0])
        .collect();
    let cut = subset(&table, &keep).expect("the subset");
    assert!(
        cut.len() * 5 < table.len(),
        "{} bytes against {}",
        cut.len(),
        table.len()
    );

    // The codes the page uses are not the ASCII ones the font maps: the
    // encoding written into the PDF says what each code means, so the page
    // sets 32, 33, ... in the order the names were collected.
    let recoded: String = (0..wanted.len() as u8).map(|i| (32 + i) as char).collect();
    let text: String = text
        .bytes()
        .map(|byte| {
            let at = wanted
                .iter()
                .position(|(code, _, _)| *code == byte)
                .unwrap();
            recoded.as_bytes()[at] as char
        })
        .collect();
    let names: Vec<(u8, String, f64)> = wanted
        .iter()
        .enumerate()
        .map(|(i, (_, name, width))| (32 + i as u8, name.clone(), *width))
        .collect();

    let dir = scratch("same");
    let (Some(a), Some(b)) = (
        rendered(&page(table.clone(), &names, &text), &dir, "whole"),
        rendered(&page(cut, &names, &text), &dir, "cut"),
    ) else {
        return;
    };
    // The page is a raw bitmap rather than a PNG on purpose: comparing
    // compressed bytes says nothing about pixels, and a difference of a dozen
    // bytes in a PNG stream can be a page drawn in another font entirely.
    let ink = |bytes: &[u8]| bytes.iter().filter(|&&byte| byte < 200).count();
    assert!(
        ink(&a) > 300,
        "the page has {} dark pixels, so this compared nothing",
        ink(&a)
    );
    assert_eq!(a, b, "the subset drew a different page");

    let _ = std::fs::remove_dir_all(&dir);
}
