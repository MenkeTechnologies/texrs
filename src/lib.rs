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

pub mod agl;
pub mod aot;
pub mod aot_runtime;
pub mod banner;
pub mod bib;
pub mod bst;
pub mod bstvm;
pub mod bundle;
pub mod catcode;
pub mod cff;
pub mod charcodes;
pub mod cli;
pub mod colour;
pub mod compiler;
pub mod corpus;
pub mod dap;
pub mod dimen;
pub mod docs;
pub mod document;
pub mod dvi;
pub mod dvi_parity;
pub mod expand;
pub mod fontmap;
pub mod format;
pub mod geturl;
pub mod glue;
pub mod image;
pub mod intercepts;
pub mod io;
pub mod ir;
pub mod itar;
pub mod latex;
pub mod lexer;
pub mod linebreak;
pub mod lower;
pub mod lsp;
pub mod parallel;
pub mod parity;
pub mod pdf;
pub mod pdf_parity;
pub mod pk;
pub mod repl;
pub mod runtime;
pub mod rust_ffi;
pub mod script_cache;
pub mod sfnt;
pub mod special;
pub mod status;
pub mod tfm;
pub mod tiers;
pub mod tikz;
pub mod token;
pub mod type1;
pub mod typeset;
pub mod vf;

pub use expand::{Engine, TexError};

/// The messages `src` writes, as a list rather than one joined line.
///
/// [`run_messages`] joins them the way the terminal line does; the REPL needs
/// them apart, because it prints only the ones the newest line added.
///
/// The markers are stripped for the same reason they are stripped from the
/// text: they are how a face and a colour reach a page, not characters of the
/// document. `\message{\texttt{a}}` says `a`, and the prelude's `\texttt` wraps
/// its argument in a face marker, so without this it would say it between two
/// control characters.
pub fn run_messages_list(src: &str) -> Result<Vec<String>, TexError> {
    let chunk = compile(src)?;
    let messages = crate::runtime::run(chunk).map_err(TexError)?;
    Ok(messages.iter().map(|m| without_marks(m)).collect())
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
/// Strip the markers the typesetting path reads, leaving the words.
///
/// The runtime writes the colour ones where `\textcolor` was: U+0001 opens
/// (`r,g,b` follows, closed by U+0002) and U+0003 closes. They are instructions
/// to a DVI driver, not characters of the document, so they are carried on the
/// typesetting path and stripped everywhere else -- a caller asking for the
/// TEXT of a document should not have to know they exist.
///
/// A listing's line break is the one marker that has a plain-text spelling: it
/// says a code line ended, and a newline is what that is in text. Dropping it
/// instead would put a program's statements back on one line, which is the
/// weld this marker exists to undo.
/// A face marker is the same kind of instruction and goes the same way:
/// FACE_PUSH and the one character naming the face, FACE_POP on its own. The
/// code character is a letter, so leaving it would put an `m` in front of every
/// `\texttt`.
fn without_marks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_spec = false;
    let mut face_code = false;
    for ch in text.chars() {
        match ch {
            '\u{1}' => in_spec = true,
            '\u{2}' => in_spec = false,
            '\u{3}' => {}
            // A listing's break IS a newline in the text -- that is what the
            // listing said -- so it comes back as one.
            crate::typeset::LISTING_BREAK => out.push('\n'),
            // The centring and vertical-space markers are the same kind of
            // thing -- where a line sits and how much room is left around it,
            // not characters the document wrote -- so they come out here too.
            // The paragraph breaks a heading's space is written between are
            // what says the same thing in text.
            crate::typeset::CENTRE | crate::typeset::CENTRE_END => {}
            crate::typeset::VERTICAL_SPACE => {}
            // That a line was set to the measure is a fact about the page, not
            // about the words, and the words are all a reader asked for.
            crate::typeset::JUSTIFY => {}
            // A page break is a boundary in the text too, so it comes back as
            // one rather than as a form feed nobody asked to read.
            crate::typeset::PAGE_BREAK => out.push('\n'),
            crate::typeset::FACE_PUSH => face_code = true,
            crate::typeset::FACE_POP => {}
            // A table's structure has a plain-text spelling, the way a
            // listing's line break does: a cell boundary is the space that
            // stands between two cells when they are not set in columns, and a
            // row ends at a newline. A rule is drawn on the page and is not a
            // character, so it and the code naming it come out here.
            crate::typeset::TABLE_CELL => out.push(' '),
            crate::typeset::TABLE_ROW => out.push('\n'),
            crate::typeset::TABLE_MARK => face_code = true,
            // Which part of a longtable a line is -- its head, its foot, a row
            // -- is a fact about the page it is set on, not about the words.
            // Its code character goes the way a rule's does.
            crate::typeset::LONGTABLE => face_code = true,
            // Where a list item's line starts is a fact about the page. The
            // item's own mark -- the bullet, the number, the term -- is text
            // the lowerer wrote and stays; the depth digit after the marker
            // goes the way a face code does.
            crate::typeset::LIST_INDENT => face_code = true,
            // Where a contents belongs, which heading feeds it, and where the
            // page numbering starts are all facts about the pages rather than
            // words the document wrote. The contents itself is built by the
            // typesetter out of pages, which a reader asking for the text has
            // not got; its code character goes the way a face code does.
            crate::typeset::TOC => face_code = true,
            _ if face_code => face_code = false,
            c if in_spec => {
                let _ = c;
            }
            c => out.push(c),
        }
    }
    out
}

/// The document's text with every typesetting marker taken out.
///
/// Public so the marker registry can be walked against it: see
/// `typeset::MARKERS`, which exists because this is the function three
/// implementations in a row forgot.
pub fn text_without_marks(text: &str) -> String {
    without_marks(text)
}

/// The document's text WITH the colour markers, for the typesetter.
pub fn run_text_marked(src: &str) -> Result<String, TexError> {
    let chunk = compile_text(src)?;
    let _ = crate::runtime::run(chunk).map_err(TexError)?;
    Ok(crate::runtime::take_text())
}

pub fn run_text(src: &str) -> Result<String, TexError> {
    Ok(without_marks(&run_text_marked(src)?))
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

/// Typeset a document to DVI, and say where the font came from.
///
/// The text is what `run_text` produces; this adds the part that had been
/// missing entirely -- measuring it in a real font, breaking it into lines and
/// stacking those onto pages. `font_path` is a `.tfm`; `cmr10.tfm` is the one
/// plain TeX sets in, and the name written into the DVI is the file's stem so a
/// driver can find the same metrics.
/// The document's text, with the bytecode taken from the rkyv shard when the
/// file has not changed since it was compiled.
///
/// The compile is 80% of a typesetting run -- the mouth, the expander and the
/// lowerer -- and it is exactly the part that does not change between runs of
/// an unedited file. Reading the chunk back instead of rebuilding it is the
/// difference between setting a book in a second and setting it in a tenth.
pub fn run_text_cached(path: &std::path::Path, src: &str) -> Result<String, TexError> {
    Ok(without_marks(&run_text_marked_cached(path, src)?))
}

/// The same, keeping the colour markers, for the typesetter.
pub fn run_text_marked_cached(path: &std::path::Path, src: &str) -> Result<String, TexError> {
    let chunk = compile_text_cached(path, src)?;
    let _ = crate::runtime::run(chunk).map_err(TexError)?;
    Ok(crate::runtime::take_text())
}

/// The text-carrying bytecode for a file, cached under its own key.
///
/// Keyed on the .tex file itself, in `scripts.rkyv`, guarded by its mtime, with
/// the mode as a suffix: the two chunks differ -- one carries the document's
/// characters and the other does not -- so sharing a key would serve a `--text`
/// run the silent chunk, or the other way about. An earlier revision keyed on a
/// synthetic `foo.tex.text` path, which does not exist, so `canonicalize` failed
/// and the cache never hit at all.
fn compile_text_cached(path: &std::path::Path, src: &str) -> Result<fusevm::Chunk, TexError> {
    if let Some(chunk) = crate::script_cache::try_load_mode(path, "text") {
        return Ok(chunk);
    }
    let src_d = crate::rust_ffi::desugar(src);
    let mut lowerer = crate::lower::Lowerer::new().with_text_output();
    if crate::latex::looks_like_latex(&src_d) {
        lowerer.preload(crate::latex::PRELUDE)?;
    }
    let cmds = lowerer.lower(&src_d)?;
    let chunk = crate::compiler::Compiler::new().compile(&cmds)?;
    crate::script_cache::store_mode(path, "text", &chunk);
    Ok(chunk)
}

/// Typeset a file to DVI, using the bytecode cache.
/// Typeset with a font CHAIN: the primary font, and fallbacks for the glyphs it
/// does not carry.
///
/// This is what `luaotfload.add_fallback` gave a LuaTeX run, and the reason the
/// publication scripts required LuaTeX at all. cmsy10 carries the arrows and the
/// set operators cmr10 has no slot for; what neither has -- box drawing, chiefly
/// -- is set as an ASCII stand-in rather than dropped.
/// Typeset straight to PDF, in the font the document asked for.
///
/// The DVI path can only name `.tfm` fonts, so it sets everything in Computer
/// Modern however loudly a document says `\setmainfont`. A PDF can name the
/// fourteen faces every reader has, so this is where that request is honoured:
/// Arimo asks for Arial's metrics, which are Helvetica's, and a monospace
/// request goes to Courier whatever it was called.
pub fn run_pdf(src: &str) -> Result<Vec<u8>, TexError> {
    run_pdf_at(None, src)
}

/// The same, told which FILE the document was read from, so a path it names
/// can be resolved relative to the directory holding it.
///
/// The argument is the document, not its directory: a font shipped beside it
/// lives in `dir/.fonts/`, and looking for that inside `book.tex/` finds
/// nothing at all -- silently, since a font that cannot be found is a font
/// that gets substituted.
pub fn run_pdf_at(path: Option<&std::path::Path>, src: &str) -> Result<Vec<u8>, TexError> {
    let src_d = crate::rust_ffi::desugar(src);
    let mut lowerer = crate::lower::Lowerer::new().with_text_output();
    if crate::latex::looks_like_latex(&src_d) {
        lowerer.preload(crate::latex::PRELUDE)?;
    }
    let cmds = lowerer.lower(&src_d)?;
    // The families are read while lowering, because that is where the preamble
    // is; they have to be taken from the lowerer before it is dropped.
    let families = lowerer.fonts.clone();
    // `\pagecolor` is read in the same place and for the same reason.
    let page_colour = lowerer.page_colour;
    // So is the page itself: `\documentclass[11pt]` and geometry's margins are
    // preamble, and until they were read every document was set on plain.tex's
    // 10pt-on-12pt, 1in-margin page whatever it asked for.
    let layout = lowerer.layout.clone();
    let chunk = crate::compiler::Compiler::new().compile(&cmds)?;
    let _ = crate::runtime::run(chunk).map_err(TexError)?;
    let text = crate::runtime::take_text();
    Ok(crate::typeset::to_pdf(
        &text,
        &families,
        &layout,
        page_colour,
        path.and_then(|p| p.parent()),
    ))
}

/// How many pages a PDF declares.
///
/// Counted off the page objects themselves: `/Type /Pages` is the page TREE and
/// there is one of those, so it is subtracted rather than matched around.
///
/// `pdf_parity` asks this to decide whether a document produced a page at all,
/// because "no pages, no file" is what the command line does and what tex and
/// luatex do -- a harness that measured the library call underneath would
/// report a divergence the tool does not have.
pub fn pdf_page_count(pdf: &[u8]) -> usize {
    let text = String::from_utf8_lossy(pdf);
    text.matches("/Type /Page").count() - text.matches("/Type /Pages").count()
}

pub fn run_dvi_fallback(
    path: Option<&std::path::Path>,
    src: &str,
    chain: &crate::typeset::FontChain,
    layout: &crate::typeset::Layout,
) -> Result<Vec<u8>, TexError> {
    // The MARKED text: colour is an instruction to the driver, and this is the
    // one path that has a driver to instruct.
    let text = match path {
        Some(p) => run_text_marked_cached(p, src)?,
        None => run_text_marked(src)?,
    };
    Ok(crate::typeset::to_dvi_chain(&text, chain, layout))
}

pub fn run_dvi_cached(
    path: &std::path::Path,
    src: &str,
    font_path: &std::path::Path,
    layout: &crate::typeset::Layout,
) -> Result<Vec<u8>, TexError> {
    let text = run_text_cached(path, src)?;
    let font = crate::tfm::Tfm::open(font_path).map_err(TexError)?;
    let name = font_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("cmr10");
    Ok(crate::typeset::to_dvi(&text, &font, name, layout))
}

pub fn run_dvi(
    src: &str,
    font_path: &std::path::Path,
    layout: &crate::typeset::Layout,
) -> Result<Vec<u8>, TexError> {
    let text = run_text(src)?;
    let font = crate::tfm::Tfm::open(font_path).map_err(TexError)?;
    let name = font_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("cmr10");
    Ok(crate::typeset::to_dvi(&text, &font, name, layout))
}

pub fn run_messages(src: &str) -> Result<String, TexError> {
    let chunk = compile(src)?;
    let msgs = crate::runtime::run(chunk).map_err(TexError)?;
    Ok(without_marks(&msgs.join(" ")))
}

/// The same, for a document read from `path`: the bytecode comes from the cache
/// when the file has not changed since it was compiled, and is put there when it
/// has. The result is identical either way — the cache is a way of skipping the
/// front of the pipeline, never of changing what it produces.
pub fn run_messages_cached(path: &std::path::Path, src: &str) -> Result<String, TexError> {
    let chunk = compile_cached(path, src)?;
    let msgs = crate::runtime::run(chunk).map_err(TexError)?;
    Ok(without_marks(&msgs.join(" ")))
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
