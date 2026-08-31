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
    /// ... with those words falling on the same lines, which is where line
    /// breaking and glue setting show themselves.
    Lines,
    /// ... set in the same fonts, embedded the same way.
    Fonts,
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
            Rung::Lines => "LINES",
            Rung::Fonts => "FONTS",
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
            "LINES" => Rung::Lines,
            "FONTS" => Rung::Fonts,
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

/// The lines a PDF's pages carry, with the words on each.
///
/// `-layout` keeps the physical arrangement, so two files agree here only if
/// their line breaking put the same words on the same lines. Leading and
/// trailing space is dropped and runs of spaces collapse: the rung is about
/// which words share a line, not about the column they start in, which the
/// next rung up covers.
pub fn lines(pdf: &[u8]) -> Option<Vec<String>> {
    let out = run_tool("pdftotext-layout", pdf)?;
    Some(
        out.lines()
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|l| !l.is_empty())
            .collect(),
    )
}

/// The fonts a PDF names, as `name type embedded`.
///
/// This is where the two engines part company most visibly: luatex embeds a
/// subsetted `CMR10`, texrs names a non-embedded `Helvetica`. Byte equality is
/// unreachable while the typeface differs, so it is worth its own rung.
pub fn fonts(pdf: &[u8]) -> Option<Vec<String>> {
    let out = run_tool("pdffonts", pdf)?;
    Some(
        out.lines()
            .skip(2)
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                let f: Vec<&str> = l.split_whitespace().collect();
                // The subset prefix is arbitrary per run, so compare the face
                // rather than the tag: KJJYRX+CMR10 and ABCDEF+CMR10 are the
                // same font embedded twice.
                let name = f.first().copied().unwrap_or("");
                let face = name.split_once('+').map(|(_, r)| r).unwrap_or(name);
                format!("{face} {}", f.get(1).copied().unwrap_or(""))
            })
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
        // The same, keeping the physical layout so lines are comparable.
        "pdftotext-layout" => {
            cmd = Command::new("pdftotext");
            cmd.arg("-q").arg("-layout").arg(&path).arg("-");
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
    // Which engine wrote nothing is the whole diagnosis, and reporting one
    // message for all three cases got it backwards: for an empty document it
    // is LUATEX that writes no PDF ("no pages of output") while texrs writes an
    // empty one, and the harness blamed texrs for it.
    let (r, s) = match (reference, subject) {
        (None, None) => {
            return (
                Rung::Bytes,
                "neither engine wrote a PDF: no pages of output".to_string(),
            )
        }
        (Some(_), None) => return (Rung::None, "texrs wrote no PDF, luatex did".to_string()),
        (None, Some(s)) => {
            return (
                Rung::None,
                format!("texrs wrote a {}-byte PDF, luatex wrote none", s.len()),
            )
        }
        (Some(r), Some(s)) => (r, s),
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
    let (Some(rl), Some(sl)) = (lines(r), lines(s)) else {
        return (Rung::Text, "pdftotext -layout unavailable".to_string());
    };
    if rl != sl {
        let at = rl
            .iter()
            .zip(sl.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(rl.len().min(sl.len()));
        return (
            Rung::Text,
            format!(
                "lines: {} vs {}, first difference at {at} ({:?} vs {:?})",
                rl.len(),
                sl.len(),
                rl.get(at),
                sl.get(at)
            ),
        );
    }
    let (Some(rf), Some(sf)) = (fonts(r), fonts(s)) else {
        return (Rung::Lines, "pdffonts unavailable".to_string());
    };
    if rf != sf {
        return (Rung::Lines, format!("fonts: luatex {rf:?}, texrs {sf:?}"));
    }
    (
        Rung::Fonts,
        format!("bytes: luatex {}, texrs {}", r.len(), s.len()),
    )
}
