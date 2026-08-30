//! What the differential tests need from the oracle.
//!
//! The logic lives in `texrs::parity` — one implementation, shared by these
//! tests, `src/bin/parity.rs` and `src/bin/parity_fuzz.rs`, because two
//! harnesses that extract the message stream differently are asking the oracle
//! two different questions. This is the thin adapter that turns "no usable
//! oracle" into a loud skip rather than a failure, which is what a test wants
//! and a binary does not.

use std::path::{Path, PathBuf};

pub fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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
