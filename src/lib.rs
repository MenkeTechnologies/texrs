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
//! `tex` by `cargo run --bin parity`.

pub mod aot;
pub mod aot_runtime;
pub mod banner;
pub mod bib;
pub mod bst;
pub mod bstvm;
pub mod bundle;
pub mod catcode;
pub mod cli;
pub mod compiler;
pub mod corpus;
pub mod dap;
pub mod docs;
pub mod document;
pub mod dvi;
pub mod expand;
pub mod format;
pub mod geturl;
pub mod intercepts;
pub mod io;
pub mod ir;
pub mod latex;
pub mod lexer;
pub mod lower;
pub mod lsp;
pub mod parallel;
pub mod parity;
pub mod pk;
pub mod repl;
pub mod runtime;
pub mod rust_ffi;
pub mod script_cache;
pub mod status;
pub mod tfm;
pub mod tiers;
pub mod token;
pub mod vf;

pub use expand::{Engine, TexError};

/// The messages `src` writes, as a list rather than one joined line.
///
/// [`run_messages`] joins them the way the terminal line does; the REPL needs
/// them apart, because it prints only the ones the newest line added.
pub fn run_messages_list(src: &str) -> Result<Vec<String>, TexError> {
    let chunk = compile(src)?;
    crate::runtime::run(chunk).map_err(TexError)
}

/// Compile `src` to fusevm bytecode and run it on the VM.
///
/// This is the whole pipeline: mouth -> expander -> command stream -> fusevm
/// bytecode -> fusevm. Nothing here interprets TeX; the VM runs the program.
/// Run a document and return the TEXT it produced, not its messages.
///
/// This is what `--text` prints. An engine with a mouth and an expander and no
/// stomach cannot typeset -- there is no line breaking, no page, no font -- but
/// it can say what the document's words are after every macro has been
/// expanded, which is the difference between a book compiling to a program that
/// prints nothing and one that prints the book.
pub fn run_text(src: &str) -> Result<String, TexError> {
    let chunk = compile_text(src)?;
    let _ = crate::runtime::run(chunk).map_err(TexError)?;
    Ok(crate::runtime::take_text())
}

/// The bytecode [`run_text`] runs: the same pipeline, with the document's own
/// words lowered as well as its messages.
pub fn compile_text(src: &str) -> Result<fusevm::Chunk, TexError> {
    let src = crate::rust_ffi::desugar(src);
    let mut lowerer = crate::lower::Lowerer::new().with_text_output();
    if crate::latex::looks_like_latex(&src) {
        lowerer.preload(crate::latex::PRELUDE)?;
    }
    let cmds = lowerer.lower(&src)?;
    crate::compiler::Compiler::new().compile(&cmds)
}

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
    let cmds = commands(src)?;
    crate::compiler::Compiler::new().compile(&cmds)
}

/// The command stream `src` lowers to, for `--dump-ast` and for tests that want
/// to read the frontend's output before the code generator touches it.
///
/// This is the stage `compile` hands to the code generator, produced the same
/// way — desugared, with the LaTeX prelude preloaded when the document asks for
/// it — so what `--dump-ast` prints is what `--disasm` was generated from and
/// not a second, more agreeable pipeline.
pub fn commands(src: &str) -> Result<Vec<crate::ir::Cmd>, TexError> {
    let src = crate::rust_ffi::desugar(src);
    let mut lowerer = crate::lower::Lowerer::new();
    if crate::latex::looks_like_latex(&src) {
        lowerer.preload(crate::latex::PRELUDE)?;
    }
    lowerer.lower(&src)
}

/// Whether an `\end` in `src` stops the run, rather than the source merely
/// running out.
///
/// tex closes the file's paren differently for the two — `(./doc.tex MSGS )`
/// when `\end` stopped it, `(./doc.tex MSGS)` when the file ran out — and only
/// the second keeps reading, from the command line if there is more there. The
/// driver asks this; nothing else needs to.
pub fn source_ends_run(src: &str) -> bool {
    let src = crate::rust_ffi::desugar(src);
    let mut lowerer = crate::lower::Lowerer::new();
    let _ = lowerer.lower(&src);
    lowerer.ended
}

/// Compile `src`, reporting the line the mouth had reached if it stopped.
///
/// `src/lsp.rs` publishes the diagnostic this returns; every other caller wants
/// [`compile`], which drops the position.
pub fn compile_located(src: &str) -> Result<fusevm::Chunk, (TexError, u32)> {
    // The desugar pads its replacement with newlines, so a diagnostic after a
    // `\rust{ … }` block still lands on the line the author wrote it on.
    let src = crate::rust_ffi::desugar(src);
    let cmds = crate::lower::Lowerer::new().lower_located(&src)?;
    // A pool the chunk cannot address is not a position in the source, so the
    // diagnostic carries no line rather than a wrong one.
    crate::compiler::Compiler::new()
        .compile(&cmds)
        .map_err(|e| (e, 0))
}

/// Compile `src` with the `--dap` statement markers in it.
///
/// The markers are extra ops, so this is NOT what an ordinary run compiles:
/// nothing pays for the debugger that is not using it.
pub fn compile_debug(src: &str) -> Result<fusevm::Chunk, TexError> {
    let src = crate::rust_ffi::desugar(src);
    let cmds = crate::lower::Lowerer::new().lower(&src)?;
    crate::compiler::Compiler::new_debug().compile(&cmds)
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
