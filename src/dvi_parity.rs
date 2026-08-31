//! DVI parity: the same document through `tex` and through texrs.
//!
//! The second axis, and the attainable one. A DVI file is a short uncompressed
//! stream of drawing commands with no fonts inside it and no compression, so
//! byte-identical output is a realistic goal: for `Hello world.` tex writes 224
//! bytes and texrs writes 260, where the same document in PDF is 11,729 against
//! 615. The two files already agree on everything but the preamble.
//!
//! The structural comparison is `Dvi::compare`, which `-X dvi` already needed —
//! this adds the ladder and the oracle around it rather than a second way of
//! diffing two DVI files.

use crate::dvi::{Difference, Dvi};
use std::path::Path;
use std::process::Command;

/// A number no other call in this process will use.
///
/// The scratch directory used to be named by process id and case name alone,
/// so two tests running the same document in parallel threads shared one
/// directory and removed it under each other. That failed only when the tests
/// ran together, which is the worst way for a harness to fail.
fn unique_suffix() -> usize {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// The engine texrs is measured against here: real `tex`, not luatex, because
/// DVI is what tex writes natively.
pub struct Oracle {
    pub program: String,
    pub version: String,
}

/// `tex`, if it is installed.
pub fn oracle() -> Option<Oracle> {
    let out = Command::new("tex").arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let version = text
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .nth(1)
        .unwrap_or("unknown")
        .to_string();
    Some(Oracle {
        program: "tex".to_string(),
        version,
    })
}

/// How far up the ladder two DVI files agree.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Rung {
    /// One engine wrote a DVI and the other did not.
    None,
    /// Both wrote one, and both parse.
    Parses,
    /// ... with the same number of pages.
    Pages,
    /// ... setting the same characters.
    Text,
    /// ... asking for the same fonts, and drawing the same rules and specials:
    /// everything `Dvi::compare` can see.
    Structure,
    /// ... byte for byte. The goal, and a reachable one.
    Bytes,
}

impl Rung {
    pub fn name(self) -> &'static str {
        match self {
            Rung::None => "NONE",
            Rung::Parses => "PARSES",
            Rung::Pages => "PAGES",
            Rung::Text => "TEXT",
            Rung::Structure => "STRUCTURE",
            Rung::Bytes => "BYTES",
        }
    }

    pub fn parse(s: &str) -> Option<Rung> {
        Some(match s {
            "NONE" => Rung::None,
            "PARSES" => Rung::Parses,
            "PAGES" => Rung::Pages,
            "TEXT" => Rung::Text,
            "STRUCTURE" => Rung::Structure,
            "BYTES" => Rung::Bytes,
            _ => return None,
        })
    }
}

/// What `tex` writes for `case`.
pub fn reference(oracle: &Oracle, case: &Path) -> Option<Vec<u8>> {
    let dir = std::env::temp_dir().join(format!(
        "texrs-dviparity-{}-{}-{}",
        std::process::id(),
        unique_suffix(),
        case.file_stem()?.to_string_lossy()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).ok()?;
    std::fs::copy(case, dir.join("case.tex")).ok()?;
    let _ = Command::new(&oracle.program)
        .args(["-interaction=nonstopmode", "case.tex"])
        .env("SOURCE_DATE_EPOCH", "0")
        .current_dir(&dir)
        .output();
    let dvi = std::fs::read(dir.join("case.dvi")).ok();
    let _ = std::fs::remove_dir_all(&dir);
    dvi
}

/// What texrs writes for the same document, in process.
pub fn subject(case: &Path) -> Option<Vec<u8>> {
    let src = std::fs::read_to_string(case).ok()?;
    let font = crate::typeset::find_font("cmr10")?;
    crate::run_dvi(&src, &font, &crate::typeset::Layout::default()).ok()
}

/// How far the two files agree, and a line saying where they stop.
pub fn verdict(reference: Option<&Vec<u8>>, subject: Option<&Vec<u8>>) -> (Rung, String) {
    let (r, s) = match (reference, subject) {
        (None, None) => return (Rung::Bytes, "neither engine wrote a DVI".to_string()),
        (Some(_), None) => return (Rung::None, "texrs wrote no DVI, tex did".to_string()),
        (None, Some(_)) => return (Rung::None, "texrs wrote a DVI, tex wrote none".to_string()),
        (Some(r), Some(s)) => (r, s),
    };
    if r == s {
        return (Rung::Bytes, String::new());
    }
    let (Ok(rd), Ok(sd)) = (Dvi::parse(r), Dvi::parse(s)) else {
        return (
            Rung::None,
            "one of the two files does not parse".to_string(),
        );
    };

    // `Dvi::compare` is the reader `-X dvi` already needed. Its findings map
    // onto the ladder in order, so the lowest one that fires is where the two
    // files stop agreeing.
    let differences = rd.compare(&sd);
    for d in &differences {
        if let Difference::Pages { left, right } = d {
            return (Rung::Parses, format!("pages: tex {left}, texrs {right}"));
        }
    }
    for d in &differences {
        if let Difference::Text { at, left, right } = d {
            return (
                Rung::Pages,
                format!(
                    "text differs at {at}: tex {:?}, texrs {:?}",
                    snippet(left, *at),
                    snippet(right, *at)
                ),
            );
        }
    }
    if !differences.is_empty() {
        return (
            Rung::Text,
            format!(
                "{} structural difference(s): {:?}",
                differences.len(),
                differences.first()
            ),
        );
    }
    (
        Rung::Structure,
        format!("bytes: tex {}, texrs {}", r.len(), s.len()),
    )
}

/// A few characters either side of `at`, for a message that fits on a line.
fn snippet(text: &str, at: usize) -> String {
    text.chars().skip(at.saturating_sub(8)).take(24).collect()
}

/// How faithfully texrs's reader and writer carry a file they did not write.
///
/// A third axis, and the one that needs no typesetting: parse a DVI real `tex`
/// produced, write it back, and compare. Everything else here asks whether
/// texrs SETS a document as tex does; this asks only whether it can carry
/// tex's own file through unchanged, which is a smaller question with a
/// definite answer.
pub fn roundtrip(dvi: &[u8]) -> Result<Vec<usize>, String> {
    let parsed = Dvi::parse(dvi)?;
    let out = parsed.rewrite();
    if out.len() != dvi.len() {
        return Err(format!(
            "length changed: {} in, {} out",
            dvi.len(),
            out.len()
        ));
    }
    Ok(dvi
        .iter()
        .zip(out.iter())
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i)
        .collect())
}

/// How far a file survives that round trip.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Trip {
    /// texrs could not parse what tex wrote.
    Unparsed,
    /// It parsed and came back at a DIFFERENT length: the writer does not
    /// choose the compact operand widths tex chose.
    Rewritten,
    /// ... at the same length, with bytes changed.
    SameLength,
    /// ... unchanged. The goal, and it needs no typesetting to reach.
    Identical,
}

impl Trip {
    pub fn name(self) -> &'static str {
        match self {
            Trip::Unparsed => "UNPARSED",
            Trip::Rewritten => "REWRITTEN",
            Trip::SameLength => "SAMELENGTH",
            Trip::Identical => "IDENTICAL",
        }
    }

    pub fn parse(s: &str) -> Option<Trip> {
        Some(match s {
            "UNPARSED" => Trip::Unparsed,
            "REWRITTEN" => Trip::Rewritten,
            "SAMELENGTH" => Trip::SameLength,
            "IDENTICAL" => Trip::Identical,
            _ => return None,
        })
    }
}

/// How far `dvi` survives the round trip, and a line saying what changed.
pub fn trip_verdict(dvi: &[u8]) -> (Trip, String) {
    match roundtrip(dvi) {
        Ok(d) if d.is_empty() => (Trip::Identical, String::new()),
        Ok(d) => (
            Trip::SameLength,
            format!(
                "{} of {} bytes differ, first at {}",
                d.len(),
                dvi.len(),
                d.first().copied().unwrap_or(0)
            ),
        ),
        Err(e) if e.starts_with("length changed") => (Trip::Rewritten, e),
        Err(e) => (Trip::Unparsed, e),
    }
}
