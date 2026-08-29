//! texrs — a TeX engine.
//!
//! Milestone 1 is Knuth's MOUTH and EXPANDER: category codes, the three-state
//! line scanner, `\def` with delimited parameters, `\csname`, `\the`, the count
//! registers and their arithmetic, and `\message`. That is the half of TeX that
//! produces tokens rather than boxes, and it is the half a macro-heavy document
//! spends its time in.
//!
//! It is NOT a typesetter yet: there are no boxes, no glue, no paragraphs and no
//! DVI. `docs/ROADMAP.md` says what that would take. The parity contract for
//! this milestone is the `\message` stream, compared byte-for-byte against real
//! `tex` by `scripts/parity.sh`.

pub mod catcode;
pub mod expand;
pub mod lexer;
pub mod token;

pub use expand::{Engine, TexError};

/// Run `src` and return what `\message` wrote, joined the way TeX's terminal
/// joins it — a space before each message that is not at the start of a line.
pub fn run_messages(src: &str) -> Result<String, TexError> {
    let mut eng = Engine::new();
    eng.run(src)?;
    Ok(eng.messages.join(" "))
}
