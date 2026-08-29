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
pub mod corpus;
pub mod dap;
pub mod docs;
pub mod expand;
pub mod ir;
pub mod lexer;
pub mod lower;
pub mod lsp;
pub mod runtime;
pub mod script_cache;
pub mod tiers;
pub mod token;

pub use expand::{Engine, TexError};

/// Compile `src` to fusevm bytecode and run it on the VM.
///
/// This is the whole pipeline: mouth -> expander -> command stream -> fusevm
/// bytecode -> fusevm. Nothing here interprets TeX; the VM runs the program.
pub fn run_messages(src: &str) -> Result<String, TexError> {
    let chunk = compile(src)?;
    let msgs = crate::runtime::run(chunk).map_err(TexError)?;
    Ok(msgs.join(" "))
}

/// The same, for a document read from `path`: the bytecode comes from the cache
/// when the file has not changed since it was compiled, and is put there when it
/// has. The result is identical either way — the cache is a way of skipping the
/// front of the pipeline, never of changing what it produces.
pub fn run_messages_cached(path: &std::path::Path, src: &str) -> Result<String, TexError> {
    let chunk = compile_cached(path, src)?;
    let msgs = crate::runtime::run(chunk).map_err(TexError)?;
    Ok(msgs.join(" "))
}

/// The bytecode for the document at `path`, from the cache when the file has
/// not changed since it was compiled, and put there when it has.
///
/// Every path that compiles a *file* goes through here, so a document reaches
/// the shard however it was run — a `--disasm` listing compiles the same chunk
/// an ordinary run does, and throwing it away would mean the next run compiles
/// it again.
pub fn compile_cached(path: &std::path::Path, src: &str) -> Result<fusevm::Chunk, TexError> {
    if let Some(chunk) = crate::script_cache::try_load(path) {
        return Ok(chunk);
    }
    let chunk = compile(src)?;
    crate::script_cache::store(path, &chunk);
    Ok(chunk)
}

/// The bytecode `src` compiles to, for `--disasm` and for tests that want to
/// see that a construct really lowered rather than being folded away.
pub fn compile(src: &str) -> Result<fusevm::Chunk, TexError> {
    let cmds = crate::lower::Lowerer::new().lower(src)?;
    Ok(crate::compiler::Compiler::new().compile(&cmds))
}

/// Compile `src`, reporting the line the mouth had reached if it stopped.
///
/// `src/lsp.rs` publishes the diagnostic this returns; every other caller wants
/// [`compile`], which drops the position.
pub fn compile_located(src: &str) -> Result<fusevm::Chunk, (TexError, u32)> {
    let cmds = crate::lower::Lowerer::new().lower_located(src)?;
    Ok(crate::compiler::Compiler::new().compile(&cmds))
}

/// Compile `src` with the `--dap` statement markers in it.
///
/// The markers are extra ops, so this is NOT what an ordinary run compiles:
/// nothing pays for the debugger that is not using it.
pub fn compile_debug(src: &str) -> Result<fusevm::Chunk, TexError> {
    let cmds = crate::lower::Lowerer::new().lower(src)?;
    Ok(crate::compiler::Compiler::new_debug().compile(&cmds))
}

/// Run a document under the debug adapter: markers installed, no tracing JIT.
///
/// Returns the `\message` stream, which the adapter forwards as an output
/// event rather than letting it reach the protocol channel on stdout.
pub fn run_messages_debug(src: &str) -> Result<String, TexError> {
    let chunk = compile_debug(src)?;
    let msgs = crate::runtime::run_debug(chunk, crate::dap::on_debug_line).map_err(TexError)?;
    Ok(msgs.join(" "))
}
