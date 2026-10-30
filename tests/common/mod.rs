//! What the differential tests need from the oracle.
//!
//! The logic lives in `texrs::parity` — one implementation, shared by these
//! tests, `src/bin/parity.rs` and `src/bin/parity_fuzz.rs`, because two
//! harnesses that extract the message stream differently are asking the oracle
//! two different questions. This is the thin adapter that turns "no usable
//! oracle" into a loud skip rather than a failure, which is what a test wants
//! and a binary does not.

use std::path::{Path, PathBuf};

#[allow(dead_code)]
pub fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Whether a TeX tree is installed: `kpsewhich` finds plain TeX's macros, the
/// LaTeX article class and LuaTeX's support module, which every run that loads
/// a format reads. CI has no TeX installation, so a test that typesets through
/// one says so and stops, as tests/typeset.rs's metric tests do.
#[allow(dead_code)]
pub fn tex_tree() -> bool {
    let missing = ["plain.tex", "article.cls", "ltluatex.lua"]
        .into_iter()
        .find(|name| {
            std::process::Command::new("kpsewhich")
                .arg(name)
                .output()
                .map_or(true, |o| {
                    String::from_utf8_lossy(&o.stdout).trim().is_empty()
                })
        });
    if let Some(name) = missing {
        eprintln!("skipping: kpsewhich finds no {name} -- no TeX tree installed");
        return false;
    }
    true
}

/// The pinned oracle, or `None` with the reason printed.
///
/// A test skips when tex is missing or is the wrong version — CI has no TeX
/// installation, and a mismatched one reports a different divergence set rather
/// than an error.
#[allow(dead_code)]
pub fn oracle() -> Option<texrs::parity::Oracle> {
    match texrs::parity::oracle(&repo()) {
        Ok(o) => Some(o),
        Err(e) => {
            eprintln!("skipping: {e}");
            None
        }
    }
}

/// The oracle's program name, for a test that shells out itself.
#[allow(dead_code)]
pub fn tex() -> Option<String> {
    oracle().map(|o| o.program)
}

#[allow(dead_code)]
pub fn reference(tex: &str, case: &Path) -> String {
    texrs::parity::reference(
        &texrs::parity::Oracle {
            program: tex.to_string(),
            version: String::new(),
        },
        case,
    )
}

#[allow(dead_code)]
pub fn messages_of(out: &str) -> String {
    texrs::parity::messages_of(out)
}

/// Whether a plain document's list path can run here: it preloads
/// `plain.tex` and sets in `cmr10`. Says so when it cannot.
#[allow(dead_code)]
pub fn list_path_runs() -> bool {
    if texrs::typeset::find_font("cmr10").is_none() || texrs::kpse::locate("plain.tex").is_none() {
        eprintln!("skipping: no cmr10.tfm or plain.tex to run the list path with");
        return false;
    }
    true
}

/// Run `tex` on `src` as `case.tex` in `dir`, leaving what it writes there,
/// and return its message stream ([`texrs::parity::messages_of`]) -- the
/// form `texrs::run_tex_dvi` reports its own in. What follows the file's
/// `)` -- `Output written on case.dvi (1 page, ...)` -- is cut first, since
/// its parentheses would be read as the file's.
#[allow(dead_code)]
pub fn tex_in(tex: &str, dir: &Path, src: &str) -> String {
    std::fs::write(dir.join("case.tex"), src).expect("write the case");
    let out = std::process::Command::new(tex)
        .args(["-interaction=nonstopmode", "case.tex"])
        .env("max_print_line", "8000")
        .env("SOURCE_DATE_EPOCH", "0")
        .current_dir(dir)
        .output()
        .expect("tex runs");
    let text = String::from_utf8_lossy(&out.stdout);
    let text = text
        .split("\nOutput written on ")
        .next()
        .unwrap_or_default();
    texrs::parity::messages_of(text)
}

/// A DVI without its preamble's comment, which is the time of the run
/// (§617).
#[allow(dead_code)]
pub fn without_the_date(dvi: &[u8]) -> Vec<u8> {
    let k = usize::from(dvi[14]);
    [&dvi[..15], &dvi[15 + k..]].concat()
}
