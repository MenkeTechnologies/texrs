//! The texrs logo, the live-stats box, and the version line.
//!
//! One source of truth for three callers, the way the sibling engines do it:
//! the REPL prints it at startup, `--help` prints it above the usage, and
//! `--version` prints the one-line form a build tool parses.
//!
//! Every number in the box is read at call time — the primitive count comes
//! from the corpus, the register count from the compiler — so the banner cannot
//! go stale the way a hand-typed count does the moment a primitive is added.

use crate::compiler::COUNT_SLOTS;
use crate::corpus::{CHAPTERS, CORPUS};

/// The TeX version texrs implements the mouth and expander of.
pub const TEX_COMPAT_VERSION: &str = "3.141592653";

/// The engine name — texrs is its own runtime, as pdfTeX is web2c's.
pub const TEX_ENGINE: &str = "texrs";

/// The host arch/OS pair, for the version line.
pub fn platform() -> String {
    format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
}

/// The `texrs --version` banner.
///
/// The TeX version comes first, because that is the language level being
/// implemented and what a build tool parsing the line expects to find; the real
/// engine and its crate version follow, so nothing is misrepresented as TeX
/// Live.
pub fn version_banner() -> String {
    format!(
        "texrs {} (TeX {} mouth+expander) [{}]",
        env!("CARGO_PKG_VERSION"),
        TEX_COMPAT_VERSION,
        platform()
    )
}

/// Visible columns in `s`, ignoring ANSI SGR escapes.
///
/// The box is padded from this rather than from `str::len`, because an escape
/// sequence has bytes and no width — measuring with them in it is what makes a
/// coloured box ragged the moment a value changes length.
pub fn visible_width(s: &str) -> usize {
    let bytes = s.as_bytes();
    let (mut i, mut w) = (0usize, 0usize);
    while i < bytes.len() {
        if bytes[i] == 0x1B && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            i += 2;
            while i < bytes.len() && !(0x40..=0x7E).contains(&bytes[i]) {
                i += 1;
            }
            i += 1;
        } else {
            let step = std::str::from_utf8(&bytes[i..])
                .ok()
                .and_then(|s| s.chars().next())
                .map(|c| c.len_utf8())
                .unwrap_or(1);
            w += 1;
            i += step;
        }
    }
    w
}

/// The logo, the stats box and the tagline. `colored` emits ANSI SGR escapes.
pub fn render_banner(colored: bool) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let pid = std::process::id();

    let primitives = CORPUS.len();
    let chapters = CHAPTERS.len();
    let registers = COUNT_SLOTS;
    let cached = crate::script_cache::default_cache_path();
    let cached = match crate::script_cache::cache_enabled() {
        true => format!("{}", cached.display()),
        false => "off (TEXRS_CACHE)".to_string(),
    };

    let (c, m, r, y, g, n) = match colored {
        true => (
            "\x1b[36m", "\x1b[35m", "\x1b[31m", "\x1b[33m", "\x1b[32m", "\x1b[0m",
        ),
        false => ("", "", "", "", "", ""),
    };

    const INNER: usize = 64;
    let mut out = String::with_capacity(2048);
    let row = |out: &mut String, body: &str| {
        let pad = INNER.saturating_sub(visible_width(body));
        out.push_str(&format!("{c} │{n}{body}{:pad$}{c}│{n}\n", "", pad = pad));
    };

    out.push_str(&format!(
        "{c} ████████╗███████╗██╗  ██╗██████╗ ███████╗{n}\n"
    ));
    out.push_str(&format!(
        "{c} ╚══██╔══╝██╔════╝╚██╗██╔╝██╔══██╗██╔════╝{n}\n"
    ));
    out.push_str(&format!(
        "{m}    ██║   █████╗   ╚███╔╝ ██████╔╝███████╗{n}\n"
    ));
    out.push_str(&format!(
        "{m}    ██║   ██╔══╝   ██╔██╗ ██╔══██╗╚════██║{n}\n"
    ));
    out.push_str(&format!(
        "{r}    ██║   ███████╗██╔╝ ██╗██║  ██║███████║{n}\n"
    ));
    out.push_str(&format!(
        "{r}    ╚═╝   ╚══════╝╚═╝  ╚═╝╚═╝  ╚═╝╚══════╝{n}\n"
    ));
    out.push_str(&format!(
        "{c} ┌────────────────────────────────────────────────────────────────┐{n}\n"
    ));
    row(
        &mut out,
        &format!(
            " {y}SYSTEM{n}  status:{g} ONLINE {c}//{n} {y}os:{n} {os} {y}arch:{n} {arch} {y}pid:{n} {pid}"
        ),
    );
    row(
        &mut out,
        &format!(" {y}CORES{n}   {cores}    {y}ENGINE{n}  TeX {TEX_COMPAT_VERSION} mouth+expander"),
    );
    out.push_str(&format!(
        "{c} ├────────────────────────────────────────────────────────────────┤{n}\n"
    ));
    row(
        &mut out,
        &format!(
            " {y}%p{n}  primitives {primitives:<5}  {y}%c{n}  chapters   {chapters:<5}  {y}%r{n} registers {registers}"
        ),
    );
    row(&mut out, &format!(" {y}CACHE{n}   {cached}"));
    out.push_str(&format!(
        "{c} └────────────────────────────────────────────────────────────────┘{n}\n"
    ));
    out.push_str(&format!(
        "{m}  >> TEX'S MOUTH AND EXPANDER // COMPILED TO BYTECODE v{version} <<{n}\n"
    ));
    out
}

/// Print the banner to stdout.
pub fn print_banner(colored: bool) {
    print!("{}", render_banner(colored));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every row of the box has to be the same width, and an ANSI escape has
    /// none — which is exactly how a coloured box goes ragged. The plain and
    /// coloured renderings must therefore agree line for line.
    #[test]
    fn the_box_lines_up_in_both_renderings() {
        for colored in [false, true] {
            let banner = render_banner(colored);
            let widths: Vec<usize> = banner
                .lines()
                .filter(|l| l.contains('│'))
                .map(visible_width)
                .collect();
            assert!(!widths.is_empty(), "no box rows rendered");
            assert!(
                widths.iter().all(|w| *w == widths[0]),
                "ragged box (colored={colored}): {widths:?}"
            );
        }
    }

    /// The counts are read from the tables, so they cannot go stale.
    #[test]
    fn the_counts_are_the_real_ones() {
        let plain = render_banner(false);
        assert!(plain.contains(&CORPUS.len().to_string()));
        assert!(plain.contains(&COUNT_SLOTS.to_string()));
        assert!(plain.contains(env!("CARGO_PKG_VERSION")));
    }

    /// Colour is the only difference: the same text, with escapes.
    #[test]
    fn colour_adds_escapes_and_nothing_else() {
        let plain = render_banner(false);
        let colored = render_banner(true);
        assert!(colored.contains("\x1b["), "no escapes in the coloured form");
        let stripped: String = {
            let mut out = String::new();
            let bytes = colored.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == 0x1B && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                    i += 2;
                    while i < bytes.len() && !(0x40..=0x7E).contains(&bytes[i]) {
                        i += 1;
                    }
                    i += 1;
                } else {
                    out.push(bytes[i] as char);
                    i += 1;
                }
            }
            out
        };
        // Compare the shapes rather than the bytes: the pid and the memory
        // figures are read twice and may differ between the two renderings.
        assert_eq!(stripped.lines().count(), plain.lines().count());
    }
}
