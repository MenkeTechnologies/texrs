//! PDF parity: the same document through `luatex` and through texrs, compared
//! as far as they agree.
//!
//! The goal is byte-identical output. That is a long way off — texrs writes a
//! 624-byte PDF where luatex writes 11,729 for the same two words — so a
//! harness that only answered "identical?" would say "no" every day and tell
//! nobody anything. This one reports how far up a LADDER each document gets,
//! and a floor file records the rung each currently reaches, so a change that
//! drops one is a failure even while the top rung is out of reach.
//!
//! Byte equality is only meaningful with `SOURCE_DATE_EPOCH` pinned: measured,
//! luatex reproduces itself exactly when it is set and differs run to run when
//! it is not, because the PDF carries `/CreationDate` and an `/ID`. Every run
//! here pins it, and texrs must honour it too before the top rung can be
//! claimed.

use std::path::Path;
use std::process::Command;

/// The engine texrs is measured against.
pub struct Oracle {
    pub program: String,
    pub version: String,
}

/// The epoch both engines are pinned to, so `/CreationDate` and `/ID` are
/// fixed and a byte comparison means what it says.
pub const EPOCH: &str = "0";

/// `luatex`, if it is installed, with the version it reports.
pub fn oracle() -> Option<Oracle> {
    let out = Command::new("luatex").arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let version = text
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .nth(4)
        .unwrap_or("unknown")
        .trim_end_matches(',')
        .to_string();
    Some(Oracle {
        program: "luatex".to_string(),
        version,
    })
}

/// How far up the ladder a document's two PDFs agree.
///
/// Each rung implies the ones below it, so a single value says everything that
/// currently holds. They are ordered, and the floor file compares with `>=`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Rung {
    /// texrs produced no PDF at all.
    None,
    /// Both engines produced a PDF.
    Produced,
    /// ... with the same number of pages.
    Pages,
    /// ... on the same page size.
    PageSize,
    /// ... carrying the same words, in the same order.
    Text,
    /// ... byte for byte. The goal.
    Bytes,
}

impl Rung {
    pub fn name(self) -> &'static str {
        match self {
            Rung::None => "NONE",
            Rung::Produced => "PRODUCED",
            Rung::Pages => "PAGES",
            Rung::PageSize => "PAGESIZE",
            Rung::Text => "TEXT",
            Rung::Bytes => "BYTES",
        }
    }

    pub fn parse(s: &str) -> Option<Rung> {
        Some(match s {
            "NONE" => Rung::None,
            "PRODUCED" => Rung::Produced,
            "PAGES" => Rung::Pages,
            "PAGESIZE" => Rung::PageSize,
            "TEXT" => Rung::Text,
            "BYTES" => Rung::Bytes,
            _ => return None,
        })
    }
}

/// Run one engine on `case` in its own directory and return the PDF it wrote.
///
/// Separate directories because both engines name their output after the
/// input: run in one place and the second would read the first's file.
fn build(case: &Path, engine: &str, program: &str) -> Option<Vec<u8>> {
    let dir = std::env::temp_dir().join(format!(
        "texrs-pdfparity-{}-{}-{}",
        std::process::id(),
        engine,
        case.file_stem()?.to_string_lossy()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).ok()?;
    let src = dir.join("case.tex");
    std::fs::copy(case, &src).ok()?;

    let mut cmd = Command::new(program);
    match engine {
        "texrs" => {
            cmd.arg("--pdf").arg("case.tex");
        }
        _ => {
            cmd.args(["-interaction=nonstopmode", "case.tex"]);
        }
    }
    let ok = cmd
        .env("SOURCE_DATE_EPOCH", EPOCH)
        .env("max_print_line", "8000")
        .current_dir(&dir)
        .output()
        .ok()
        .is_some();
    let pdf = match ok {
        true => std::fs::read(dir.join("case.pdf")).ok(),
        false => None,
    };
    let _ = std::fs::remove_dir_all(&dir);
    pdf
}

/// What luatex writes for `case`.
pub fn reference(oracle: &Oracle, case: &Path) -> Option<Vec<u8>> {
    build(case, "luatex", &oracle.program)
}

/// What texrs writes for the same document.
///
/// In process, like `parity::subject`: there is no binary to find and no
/// second build to keep in step with the library under test.
pub fn subject(case: &Path) -> Option<Vec<u8>> {
    let src = std::fs::read_to_string(case).ok()?;
    crate::run_pdf_at(case.parent(), &src).ok()
}

/// The number of pages and the page size a PDF declares.
///
/// Read with `pdfinfo` rather than by scanning the bytes: luatex compresses its
/// objects into streams, so `/Type /Page` does not appear in the file at all,
/// and a naive scan of texrs's uncompressed output counts `/Type /Pages` as a
/// page too. A hand parser that got both of those wrong reported "luatex 0,
/// texrs 2" for a one-page document.
pub fn shape(pdf: &[u8]) -> Option<(usize, String)> {
    let out = run_tool("pdfinfo", pdf)?;
    let mut pages = None;
    let mut size = String::new();
    for line in out.lines() {
        if let Some(v) = line.strip_prefix("Pages:") {
            pages = v.trim().parse::<usize>().ok();
        }
        if let Some(v) = line.strip_prefix("Page size:") {
            size = v.trim().to_string();
        }
    }
    Some((pages?, size))
}

/// The words a PDF's pages carry, in order.
pub fn words(pdf: &[u8]) -> Option<Vec<String>> {
    let out = run_tool("pdftotext", pdf)?;
    Some(
        out.split_whitespace()
            .map(|w| {
                w.chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect::<String>()
            })
            .filter(|w| !w.is_empty())
            .collect(),
    )
}

/// Run a poppler tool over `pdf` and return its stdout.
///
/// The arguments are spelled out per tool rather than templated: a template
/// that substituted the path for every `-` turned `pdftotext -q FILE -` into
/// `pdftotext -q FILE FILE`, which writes the text to a file and leaves stdout
/// empty. Both engines then produced an empty word list, the lists compared
/// equal, and the harness reported a match that was not there.
///
/// `None` when the tool is not installed, which the caller reports as a rung it
/// cannot judge rather than as agreement.
fn run_tool(tool: &str, pdf: &[u8]) -> Option<String> {
    let path = std::env::temp_dir().join(format!(
        "texrs-pdfparity-{}-{}.pdf",
        std::process::id(),
        tool
    ));
    std::fs::write(&path, pdf).ok()?;
    let mut cmd = Command::new(tool);
    match tool {
        // Text to stdout, quietly.
        "pdftotext" => {
            cmd.arg("-q").arg(&path).arg("-");
        }
        _ => {
            cmd.arg(&path);
        }
    }
    let out = cmd.output().ok();
    let _ = std::fs::remove_file(&path);
    let out = out?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// How far the two PDFs agree, and a line saying where they stop.
pub fn verdict(reference: Option<&Vec<u8>>, subject: Option<&Vec<u8>>) -> (Rung, String) {
    let (Some(r), Some(s)) = (reference, subject) else {
        return (Rung::None, "texrs wrote no PDF".to_string());
    };
    if r == s {
        return (Rung::Bytes, String::new());
    }
    let (Some((rp, rm)), Some((sp, sm))) = (shape(r), shape(s)) else {
        return (Rung::Produced, "pdfinfo not installed".to_string());
    };
    if rp != sp {
        return (Rung::Produced, format!("pages: luatex {rp}, texrs {sp}"));
    }
    if rm != sm {
        return (Rung::Pages, format!("page size: luatex {rm}, texrs {sm}"));
    }
    let (Some(rw), Some(sw)) = (words(r), words(s)) else {
        return (Rung::PageSize, "pdftotext not installed".to_string());
    };
    if rw != sw {
        let at = rw
            .iter()
            .zip(sw.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(rw.len().min(sw.len()));
        return (
            Rung::PageSize,
            format!(
                "words: {} vs {}, first difference at {at} ({:?} vs {:?})",
                rw.len(),
                sw.len(),
                rw.get(at),
                sw.get(at)
            ),
        );
    }
    (
        Rung::Text,
        format!("bytes: luatex {}, texrs {}", r.len(), s.len()),
    )
}
