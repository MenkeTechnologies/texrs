//! `\pdfvariable`, `\pdffeedback` and `\pdfextension`, against luatex.
//!
//! The frontend half -- what the three expand to and what they number -- is
//! held against luatex 1.24.0's own output, frozen here, so it runs anywhere.
//! The file half is held against luatex itself where it is installed: the same
//! document through both engines, object by object, must agree in every object
//! but the three that are not the backend's -- the page's content stream (the
//! typesetter's), the font program (the subsetter's) and the information
//! dictionary (which names the engine and the clock).

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

/// Parameters set, read, grouped and `\edef`ed; feedback numbers before and
/// after objects are made; an outline's action and title numbered; a colour
/// stack initialised; an unknown key.
const FEEDBACK: &str = r"\catcode`\{=1 \catcode`\}=2 \catcode`\#=6
\pdfvariable compresslevel=3 \message{[\the\pdfvariable compresslevel]}
{\pdfvariable compresslevel=5 }\message{[\the\pdfvariable compresslevel]}
\edef\x{\pdfvariable decimaldigits}\x=2 \message{[\the\x]}
\pdfvariable pageresources{/Foo 1 0 R}\message{[\the\pdfvariable pageresources]}
\pdfvariable horigin=2in \message{[\the\pdfvariable horigin]}
\message{[\pdffeedback version][\pdffeedback revision][\pdffeedback lastobj]}
\immediate\pdfextension obj{<< /A 1 >>}\message{[\pdffeedback lastobj]}
\pdfextension obj reserveobjnum \message{[\pdffeedback lastobj]}
\pdfextension obj {<< /B 2 >>}\message{[\pdffeedback lastobj]}
\pdfextension outline goto name{sec1} count 1 {One}\message{[\pdffeedback lastobj]}
\pdfextension outline goto name{sec11} {One.One}
\pdfextension literal{1 0 0 rg}\pdfextension save \pdfextension restore
\message{[\pdffeedback colorstackinit page direct{0 g 0 G}]}
\end
";

/// What luatex 1.24.0 printed for it, message by message.
const PRINTED: &str =
    "[3] [3] [2] [/Foo 1 0 R] [144.54pt] [140][0][0] [1] [3] [4] [4] [1]";

/// A page with every document-level backend feature on it: an immediate
/// object, the information dictionary, the catalogue, two nested outline
/// entries whose actions name destinations no page sets, the name tree, the
/// trailer, and the page and page-tree attributes.
const DOCUMENT: &str = r"\catcode`\{=1 \catcode`\}=2 \catcode`\#=6
\immediate\pdfextension obj{<< /A 1 >>}
\pdfextension info{/Title (T)}\pdfextension catalog{/PageMode /UseOutlines}
\pdfextension outline goto name{sec1} count 1 {One}
\pdfextension outline goto name{sec11} {One.One}
\pdfextension names{/JavaScript 1 0 R}
\pdfextension trailer{/Foo 2}
\pdfvariable pagesattr{/Rotate 0}
\pdfvariable pageattr{/Group 1 0 R}
Hellofoo.
\end
";

fn run(program: &str, args: &[&str], dir: &Path, src: &str) -> (String, Option<Vec<u8>>) {
    std::fs::write(dir.join("case.tex"), src).expect("write the document");
    let out = Command::new(program)
        .args(args)
        .arg("case.tex")
        .env("SOURCE_DATE_EPOCH", "0")
        .current_dir(dir)
        .output()
        .expect("run the engine");
    let pdf = std::fs::read(dir.join("case.pdf")).ok();
    (String::from_utf8_lossy(&out.stdout).into_owned(), pdf)
}

fn luatex() -> bool {
    Command::new("luatex")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn the_backend_numbers_and_answers_as_luatex_does() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (shown, _) = run(env!("CARGO_BIN_EXE_texrs"), &[], dir.path(), FEEDBACK);
    let shown: String = shown.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(shown.contains(PRINTED), "{shown}");
}

/// Every object of a PDF by number, inflated and unpacked.
fn objects(pdf: &[u8]) -> BTreeMap<u32, String> {
    let text = String::from_utf8_lossy(&texrs::pdf::unpacked(pdf)).into_owned();
    let mut out = BTreeMap::new();
    for chunk in text.split("\nendobj") {
        let Some(at) = chunk.find(" 0 obj\n") else {
            continue;
        };
        let number = chunk[..at].rsplit(['\n', '\r']).next().unwrap_or("").trim();
        if let Ok(n) = number.parse() {
            out.insert(n, chunk[at + 7..].to_string());
        }
    }
    out
}

/// The objects that are the backend's: all but the content stream, the font
/// program, the information dictionary and the cross reference.
fn backend_objects(pdf: &[u8]) -> BTreeMap<u32, String> {
    let all = objects(pdf);
    let contents: Vec<String> = all
        .values()
        .filter_map(|b| b.split("/Contents ").nth(1))
        .filter_map(|r| r.split(' ').next().map(str::to_string))
        .collect();
    all.into_iter()
        .filter(|(n, b)| {
            !contents.contains(&n.to_string())
                && !b.contains("/Length1")
                && !b.contains("/Producer")
                && !b.contains("/Type /XRef")
        })
        .collect()
}

/// The document-level objects, as luatex 1.24.0 wrote them for `DOCUMENT`:
/// numbered before any page, so they are the same whatever font the page is
/// set in, and on any machine.
#[test]
fn outlines_destinations_and_names_are_written_as_luatex_writes_them() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_, pdf) = run(env!("CARGO_BIN_EXE_texrs"), &["--pdf"], dir.path(), DOCUMENT);
    let all = objects(&pdf.expect("a PDF"));
    for (n, body) in [
        (1, "<< /A 1 >>"),
        (3, "<< /S /GoTo /D (sec1) >>"),
        (6, "(One)"),
        (7, "<< /S /GoTo /D (sec11) >>"),
        (10, "(One.One)"),
    ] {
        assert_eq!(all.get(&n).map(|b| b.trim()), Some(body), "object {n}");
    }
    let text: String = all.values().cloned().collect::<Vec<_>>().join("\n");
    for want in [
        "<< /Names [ (sec1) 4 0 R (sec11) 8 0 R ] /Limits [ (sec1) (sec11) ] >>",
        "/JavaScript 1 0 R >>",
        "/PageMode /UseOutlines >>",
        "/Group 1 0 R /Parent",
        "<< /Type /Pages /Rotate 0   /Count 1",
        "/Count 1 >>",
        "<< /Title (T) /Producer",
        "/Foo 2 /ID",
    ] {
        assert!(text.contains(want), "{want} is not in the file: {text}");
    }
    // The two destinations no page set point at the last page.
    assert_eq!(all.get(&4).map(|b| b.trim()), all.get(&8).map(|b| b.trim()));
    assert!(all[&4].trim().ends_with("0 R /Fit ]"), "{}", all[&4]);
}

#[test]
fn every_backend_object_agrees_with_luatex_when_it_is_installed() {
    if !luatex() {
        eprintln!("skipped: no luatex on PATH");
        return;
    }
    let ours = tempfile::tempdir().expect("tempdir");
    let theirs = tempfile::tempdir().expect("tempdir");
    let (_, a) = run("luatex", &["-interaction=nonstopmode"], theirs.path(), DOCUMENT);
    let (_, b) = run(env!("CARGO_BIN_EXE_texrs"), &["--pdf"], ours.path(), DOCUMENT);
    let (a, b) = (a.expect("luatex wrote one"), b.expect("texrs wrote one"));
    // Where TeX is installed texrs embeds the same CMR10 luatex does, so the
    // font's own objects are part of the comparison too.
    assert_eq!(backend_objects(&a), backend_objects(&b));
}
