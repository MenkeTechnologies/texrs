//! The `texrs --version` banner, and the line the REPL opens with.
//!
//! The TeX version is named first, because that is the language level being
//! implemented and what a build tool parsing the line expects to find; the real
//! engine and its crate version follow, so nothing is misrepresented as TeX
//! Live. `scripts/lib.sh` and `tests/differential.rs` pin the ORACLE's version
//! out of BUGS.md rather than reading this, so the two cannot be confused.

/// The TeX version texrs implements the mouth and expander of.
pub const TEX_COMPAT_VERSION: &str = "3.141592653";

/// The engine name — texrs is its own runtime, as pdfTeX is web2c's.
pub const TEX_ENGINE: &str = "texrs";

/// The host arch/OS pair, for the version line.
pub fn platform() -> String {
    format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
}

/// The `texrs --version` banner.
pub fn version_banner() -> String {
    format!(
        "texrs {} (TeX {} mouth+expander) [{}]",
        env!("CARGO_PKG_VERSION"),
        TEX_COMPAT_VERSION,
        platform()
    )
}

/// The line the interactive prompt opens with. One line: the shell this runs in
/// is a working terminal, not a product tour.
pub fn repl_banner() -> String {
    format!(
        "{} — mouth and expander only; \\end or Ctrl-D to leave",
        version_banner()
    )
}
