//! The oracle: what real `tex` says about a document, and whether we agree.
//!
//! Three callers ask that question — `tests/differential.rs` over the committed
//! corpus, `src/bin/parity.rs` for a person running the same comparison by
//! hand, and `src/bin/parity_fuzz.rs` over generated programs — and they must
//! ask it the SAME way. Two harnesses that extract the message stream
//! differently are asking two different questions, and the one that is wrong
//! reports divergences that are not there, or parity that is not.
//!
//! It lives in the library rather than in a test module for that reason: a test
//! module can only be shared by tests, and one of the three consumers is a
//! binary. The shell harness this replaced kept the same logic in bash and perl
//! beside the Rust copy, which is exactly the arrangement that drifts.

use std::path::Path;
use std::process::Command;

/// The reference engine, resolved and version-checked.
pub struct Oracle {
    /// The program to run, from `TEX_ORACLE` or `tex`.
    pub program: String,
    /// What `--version` reported.
    pub version: String,
}

/// The version every expectation in the tree was measured against, read out of
/// `BUGS.md`.
///
/// Single-sourced from the document that quotes it so the number cannot drift
/// from the prose. `TEX_VERSION_EXPECT` overrides it for a deliberate
/// cross-version run.
pub fn pinned_version(repo: &Path) -> Option<String> {
    if let Ok(v) = std::env::var("TEX_VERSION_EXPECT") {
        return Some(v);
    }
    let bugs = std::fs::read_to_string(repo.join("BUGS.md")).ok()?;
    bugs.lines()
        .find_map(|l| l.split("measured against **tex ").nth(1))
        .and_then(|rest| rest.split("**").next())
        .map(str::to_string)
}

/// Resolve the oracle, or say why not.
///
/// A DIFFERENT tex is refused rather than used: it does not fail loudly on its
/// own, it reports a different set of divergences, which reads exactly like a
/// regression in texrs.
pub fn oracle(repo: &Path) -> Result<Oracle, String> {
    let program = std::env::var("TEX_ORACLE").unwrap_or_else(|_| "tex".to_string());
    let out = Command::new(&program)
        .arg("--version")
        .output()
        .map_err(|_| format!("no `{program}' on PATH — the harness has no oracle"))?;
    if !out.status.success() {
        return Err(format!("`{program} --version' failed"));
    }
    let banner = String::from_utf8_lossy(&out.stdout);
    let version = banner
        .lines()
        .next()
        .and_then(|l| l.split("TeX ").nth(1))
        .and_then(|v| v.split_whitespace().next())
        .ok_or_else(|| format!("`{program} --version' did not report a TeX version"))?
        .to_string();
    match pinned_version(repo) {
        Some(want) if want != version => Err(format!(
            "oracle is tex {version}, but everything here was measured against {want}.\n\
             A mismatched oracle reports a different divergence set, not an error.\n\
             Set TEX_VERSION_EXPECT={version} to accept this deliberately."
        )),
        Some(_) => Ok(Oracle { program, version }),
        None => Err("no `measured against **tex X.Y**' line in BUGS.md".to_string()),
    }
}

/// The `\message` stream out of tex's `(./name.tex … )` line.
///
/// Continuation lines are joined with nothing between them: tex breaks its
/// terminal output at `max_print_line` mid-token, adding no character of its
/// own. [`reference()`] raises that limit so a wrap should not happen at all;
/// this is what keeps one from being read as an empty stream if it does. The
/// cut is at the LAST paren, not the first, because a message can print one.
pub fn messages_of(out: &str) -> String {
    let Some(at) = out.find("(./") else {
        return String::new();
    };
    let rest = &out[at + 3..];
    let Some((_, after)) = rest.split_once(".tex") else {
        return String::new();
    };
    let body = match after.rfind(')') {
        Some(end) => &after[..end],
        None => after,
    };
    body.replace('\n', "").trim().to_string()
}

/// What real tex prints for the document at `case`.
pub fn reference(oracle: &Oracle, case: &Path) -> String {
    let dir = std::env::temp_dir().join(format!("texrs-oracle-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let dst = dir.join("case.tex");
    if std::fs::copy(case, &dst).is_err() {
        return String::new();
    }
    let out = Command::new(&oracle.program)
        .args(["-interaction=nonstopmode", "case.tex"])
        // Without this tex wraps at 79 columns and the break lands anywhere,
        // including right after the filename — which leaves the whole message
        // stream on a line a one-line reader never sees.
        .env("max_print_line", "8000")
        .current_dir(&dir)
        .output();
    let text = out
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    messages_of(&text)
}

/// What texrs prints for the same document, in process.
pub fn subject(src: &str) -> String {
    match crate::run_messages(src) {
        Ok(m) => m,
        Err(e) => format!("ERROR: {}", e.0),
    }
}

/// One case's verdict.
pub enum Verdict {
    /// Both engines said the same thing.
    Parity,
    /// They differ, and `tests/known_gaps.txt` does not say why.
    Diverges { want: String, got: String },
    /// They differ and the gap is written down.
    Known,
    /// The gap is written down but the case now PASSES: the list is stale, and
    /// removing the entry is part of the fix.
    Stale,
}

/// Compare one case against the oracle, in the light of the known-gap list.
pub fn verdict(oracle: &Oracle, case: &Path, known: &[String]) -> Verdict {
    let want = reference(oracle, case);
    let src = std::fs::read_to_string(case).unwrap_or_default();
    let got = subject(&src);
    let name = case
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let listed = known.contains(&name);
    match (want == got, listed) {
        (true, false) => Verdict::Parity,
        (true, true) => Verdict::Stale,
        (false, true) => Verdict::Known,
        (false, false) => Verdict::Diverges { want, got },
    }
}

/// The separator between blocks in the frozen expectations file.
///
/// A line of its own, followed by the case's name, so the file diffs one case
/// at a time and a reviewer can see which case a changed block belongs to.
pub const FREEZE_SEP: &str = "#==# ";

/// Render the frozen-expectations file for `cases`.
///
/// What this buys: CI has no TeX installation, so `tests/differential.rs` skips
/// there and the corpus goes unverified on every push. Freezing what the oracle
/// said lets `tests/parity.rs` replay the same comparison with no tex at all —
/// the oracle is consulted once, by a person, and its answers are reviewed in
/// the diff like any other expectation.
///
/// It is not a substitute for the live comparison. A frozen file can only say
/// "texrs still prints what tex printed when this was frozen"; only running tex
/// says "and that is still what tex prints". Both run: the live one where there
/// is a tex, this one everywhere.
pub fn freeze(oracle: &Oracle, cases: &[std::path::PathBuf]) -> String {
    let mut out = format!(
        "# Frozen output of tex {}, written by `cargo run --bin parity -- --freeze`.\n\
         # One block per case in tests/cases; tests/parity.rs replays these with\n\
         # no TeX installed. Regenerate when a case changes, and read the diff:\n\
         # a changed block is a changed claim about what the reference engine does.\n",
        oracle.version
    );
    for case in cases {
        let name = case
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        out.push_str(&format!(
            "{FREEZE_SEP}{name}\n{}\n",
            reference(oracle, case)
        ));
    }
    out
}

/// Parse a frozen file into `(case name, expected output)` pairs.
pub fn thawed(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for block in text.split(FREEZE_SEP).skip(1) {
        let Some((name, body)) = block.split_once('\n') else {
            continue;
        };
        out.push((
            name.trim().to_string(),
            body.strip_suffix('\n').unwrap_or(body).to_string(),
        ));
    }
    out
}

/// The cases `tests/known_gaps.txt` names, comments stripped.
pub fn known_gaps(repo: &Path) -> Vec<String> {
    std::fs::read_to_string(repo.join("tests/known_gaps.txt"))
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| l.split_whitespace().next().map(str::to_string))
        .collect()
}

/// Every `.tex` in a directory, sorted so a failure names the same file on
/// every machine.
pub fn cases_in(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut v: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "tex"))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}
