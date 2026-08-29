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
pub mod compiler;
pub mod expand;
pub mod ir;
pub mod lexer;
pub mod lower;
pub mod runtime;
pub mod token;

pub use expand::{Engine, TexError};

/// Compile `src` to fusevm bytecode and run it on the VM.
///
/// This is the whole pipeline: mouth -> expander -> command stream -> fusevm
/// bytecode -> fusevm. Nothing here interprets TeX; the VM runs the program.
pub fn run_messages(src: &str) -> Result<String, TexError> {
    let cmds = crate::lower::Lowerer::new().lower(src)?;
    let chunk = crate::compiler::Compiler::new().compile(&cmds);
    let msgs = crate::runtime::run(chunk).map_err(TexError)?;
    Ok(msgs.join(" "))
}

/// The bytecode `src` compiles to, for `--disasm` and for tests that want to
/// see that a construct really lowered rather than being folded away.
pub fn compile(src: &str) -> Result<fusevm::Chunk, TexError> {
    let cmds = crate::lower::Lowerer::new().lower(src)?;
    Ok(crate::compiler::Compiler::new().compile(&cmds))
}
