//! `\directlua`: the Lua a LuaTeX document contains, actually run.
//!
//! LuaTeX's one real addition to TeX82 is an escape into a scripting language:
//! `\directlua{...}` hands its ⟨general text⟩ to an embedded interpreter, and
//! whatever that chunk *prints* comes back as input. The LuaTeX manual (1.24.0,
//! TeX Live 2026, §2.4.1) states the whole contract in two sentences:
//!
//! > The `\directlua` command is expandable. Since it passes Lua code to the
//! > Lua interpreter its expansion from the TeX viewpoint is usually empty.
//! > However, there are some Lua functions that produce material to be read by
//! > TeX, the so called print functions. The most simple use of these is
//! > `tex.print(<string> s)`. The characters of the string s will be placed on
//! > the TeX input buffer, that is, 'before TeX's eyes' to be read by TeX
//! > immediately.
//!
//! That is what this module is: a real Lua state, and a bridge that turns what
//! it printed into tokens pushed in front of the mouth. The manual's own
//! example is the acceptance criterion —
//!
//! ```tex
//! \count10=20
//! a\directlua{tex.print(tex.count[10]+5)}b
//! ```
//!
//! — which expands to `a25b` and cannot be faked by consuming the chunk.
//!
//! ## Which Lua
//!
//! PUC-Lua 5.3, through `mlua`'s vendored build, because that is the language
//! the documents are written against: "We currently use Lua 5.3 and will follow
//! developments of the language but normally with some delay" (§4.2.1), and
//! `lua.version` in a LuaTeX run answers `Lua 5.3`. fusevm hosts seventeen
//! language frontends and none of them is Lua, so there was nothing to reuse:
//! a Lua frontend for it would be a second Lua implementation to keep bug-for-
//! bug identical with PUC's, which is precisely the kind of near-miss that
//! makes a document's output wrong in a way nobody can see.
//!
//! ## When it runs
//!
//! Expansion is a compile-time act here (see `crate::lower`), so a chunk runs
//! while the document is being lowered, in document order, and its printed
//! tokens are read by the lowerer immediately after. That is the same position
//! `\directlua` occupies in LuaTeX — it is expandable, so it runs when the
//! expander reaches it — and it is why a chunk can define a macro the next line
//! uses.
//!
//! The corollary is what a chunk can SEE. `\count` registers live in fusevm
//! slots at run time and in `Engine::count` while lowering, so `tex.count[10]`
//! reads the lowering-time value, and a write goes to both: the frontend table
//! (so `\the\count10` inside an `\edef` agrees) and a `Cmd::SetCount` (so the
//! same register in the compiled program agrees). `\dimen` and `\skip` are the
//! same slot file at another offset (`crate::compiler`), so they come the same
//! way; `\toks` is a token list rather than a slot, and comes from
//! `Engine::toks`.
//!
//! ## What is NOT here, and why it is absent rather than stubbed
//!
//! - **`node.*`.** Not implemented, at all, and not planned in this module. A
//!   node list is the typesetter's own data structure and texrs's is not
//!   LuaTeX's: there is no `glyph` node with a `char` and a `font` field for
//!   Lua to walk, so `node.new`, `node.traverse` and the callbacks that hand a
//!   list to Lua have nothing to hand it. The global is ABSENT, so a chunk that
//!   reaches for it stops with `attempt to index a nil value (global 'node')`
//!   rather than quietly doing half of what it asked.
//! - **`tex.skip` and the `glue_spec` half of the register interface.** Same
//!   reason: the manual says those "accept and return `glue_spec` userdata node
//!   objects". `tex.glue`, `tex.getglue` and `tex.setglue` are the SAME
//!   registers as plain numbers, they are implemented, and `tex.skip` refuses
//!   with a message saying so.
//! - **The internal parameters** — `tex.hsize`, `tex.parindent`,
//!   `tex.baselineskip` and the rest of the manual's "Internal parameter
//!   values" list. `\hsize` is not a register in texrs and not an assignable
//!   primitive either; the page geometry is `crate::typeset::Families` and its
//!   neighbours, decided elsewhere. Reading `tex.hsize` gives nil.
//! - **`token`'s userdata half** — `token.create`, `token.get_next`,
//!   `token.scan_token`, `token.scan_glue`, `token.scan_toks`,
//!   `token.scan_list`. See `crate::lua::token`; the SCANNERS are implemented
//!   and reach the real input.
//! - **Callbacks.** `luatexbase.add_to_callback` accepts a registration and
//!   never fires it, because every callback LuaTeX defines is about a node list
//!   or a file being opened.

use crate::catcode::Cat;
use crate::expand::{Engine, Meaning, TexError};
use crate::ir::{Cmd, Num};
use crate::lexer::Lexer;
use crate::token::Token;
use mlua::{Lua, Table, Value, Variadic};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

mod token;

type R<T> = Result<T, TexError>;

/// The primitives this module answers, and the whole list of them.
///
/// `\luadirect` is `\directlua` under the name ConTeXt's own macro layer uses;
/// the prelude has always defined both, so both arrive here.
/// A `match` rather than a table scanned linearly: this runs once per control
/// sequence in the document, in front of the lowerer's own dispatch, and eight
/// string comparisons there would be eight per token of a 700,000-token book.
/// The compiler turns this into a length switch and one comparison.
fn is_primitive(name: &str) -> bool {
    matches!(
        name,
        "directlua"
            | "luadirect"
            | "latelua"
            | "luaescapestring"
            | "luafunction"
            | "luafunctioncall"
            | "lateluafunction"
            | "luadef"
    )
}

thread_local! {
    /// The document's Lua state. One per thread because `mlua::Lua` is not
    /// `Send` and because a batch run compiles one document per thread; one per
    /// DOCUMENT because a chunk's globals must survive to the next chunk —
    /// `\directlua{local t = lua.get_functions_table() t[1] = ...}` and the
    /// `\luafunction1` that calls it are two separate chunks.
    static RUNTIME: RefCell<Option<Runtime>> = const { RefCell::new(None) };
    /// Names `\luadef` has bound to a function index.
    static LUADEFS: RefCell<HashMap<String, i64>> = RefCell::new(HashMap::new());
    /// Whether [`LUADEFS`] has anything in it. Read once per control sequence
    /// the lowerer meets, so it must not be a map lookup: a `Cell<bool>` keeps
    /// the common case (no `\luadef` anywhere) to a load and a branch.
    static ANY_LUADEF: Cell<bool> = const { Cell::new(false) };
    /// What `tex.jobname` answers, when something has said.
    static JOBNAME: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// The Lua state, and the buffer its callbacks write through.
struct Runtime {
    lua: Lua,
    bridge: Rc<RefCell<Bridge>>,
    /// The document this state belongs to; see [`document_id`]. A different
    /// document gets a fresh state, so a chunk's globals cannot leak from one
    /// document of a batch into the next.
    owner: usize,
}

/// What a chunk printed, and the register writes it made.
///
/// The engine cannot be borrowed by a Lua callback: the callbacks are `'static`
/// and the engine is a `&mut` the caller holds. So the chunk runs against a
/// SNAPSHOT — which is not a compromise but the exact semantics, because TeX
/// cannot run while a chunk is running, so nothing can change underneath it.
#[derive(Default)]
struct Bridge {
    /// What the print functions produced, in call order.
    prints: Vec<Print>,
    /// `\count` registers as lowering knows them, dimensions included at
    /// `DIMEN_BASE + n` (the same slot file, as `crate::compiler` lays it out).
    counts: HashMap<i64, i64>,
    /// Registers the chunk assigned, so only those are written back.
    counts_written: BTreeMap<i64, i64>,
    /// `\countdef` and `\dimendef` names, so `tex.count.scratchcounter` works.
    count_names: HashMap<String, i64>,
    /// `\toks` registers, as the strings `\the\toks` would produce.
    toks: HashMap<i64, String>,
    toks_written: BTreeMap<i64, String>,
    /// `\toksdef` names.
    toks_names: HashMap<String, i64>,
    /// What `tex.error` was called with, reported once the chunk is done.
    error: Option<String>,
    /// What `texio.write` wrote, which is terminal output rather than input.
    messages: Vec<String>,
    /// The chain `luaotfload.add_fallback` named, when a chunk called it.
    fallbacks: Option<Vec<String>>,
}

/// One print call's payload, with the regime it asked for.
struct Print {
    text: String,
    cats: Regime,
    line: LineMode,
}

/// Which category codes the printed characters get.
#[derive(Clone, Copy, PartialEq)]
enum Regime {
    /// The catcode table in force, which is what `tex.print` and `tex.sprint`
    /// use. The manual: "If n is −1, the currently active catcode regime is
    /// used […] if n is not a valid catcode table, then it is ignored, and the
    /// currently active catcode regime is used instead." texrs has no
    /// `\catcodetable`, so every table id lands in that second sentence.
    Current,
    /// `\the\toks` style, which is `tex.print(-2, ...)` and `tex.write`: "all
    /// category codes are 12 (other) except for the space character, that has
    /// category code 10 (space)".
    Verbatim,
    /// `tex.cprint(n, ...)`: every character gets catcode `n`, spaces included.
    All(Cat),
}

/// Whether the printed text is a whole input line or part of one.
#[derive(Clone, Copy, PartialEq)]
enum LineMode {
    /// `tex.print`: "Each string argument is treated by TEX as a separate input
    /// line", so trailing spaces go and `\endlinechar` is appended — except
    /// after the very last string of the chunk, which the manual singles out.
    Line,
    /// `tex.sprint`/`tex.write`: "TEX does not switch to the 'new line' state,
    /// so that leading spaces are not ignored. No `\endlinechar` is inserted.
    /// Trailing spaces are not removed."
    Partial,
}

/// Whether this control sequence is one this module answers.
///
/// Called for every control sequence the lowerer meets, which is why the
/// `\luadef` map is behind a flag rather than looked up every time.
pub fn claims(name: &str) -> bool {
    if is_primitive(name) {
        return true;
    }
    ANY_LUADEF.with(Cell::get) && LUADEFS.with(|m| m.borrow().contains_key(name))
}

/// The fallback chain the chunk just run named, if it named one.
fn take_fallbacks() -> Option<Vec<String>> {
    RUNTIME.with(|slot| {
        slot.borrow()
            .as_ref()
            .and_then(|rt| rt.bridge.borrow_mut().fallbacks.take())
    })
}

/// Throw away the Lua state. For a caller that knows a document has ended.
pub fn reset() {
    RUNTIME.with(|r| *r.borrow_mut() = None);
    LUADEFS.with(|m| m.borrow_mut().clear());
    ANY_LUADEF.with(|f| f.set(false));
}

/// What `tex.jobname` answers from here on.
pub fn set_jobname(name: &str) {
    JOBNAME.with(|j| *j.borrow_mut() = Some(name.to_string()));
}

/// Lower one of the Lua primitives.
///
/// `out` is the command stream, which a register write is mirrored into, and
/// `fonts` is there for the one thing texrs already read out of a chunk without
/// running it: `luaotfload.add_fallback` names the faces a glyph the document's
/// own face lacks comes from. That reading stays — it is a statement about the
/// PDF backend, made by a chunk that (in a real LuaTeX run) needs luaotfload
/// loaded to do anything at all, and nothing here loads packages.
pub fn lower(
    eng: &mut Engine,
    lx: &mut Lexer,
    name: &str,
    out: &mut Vec<Cmd>,
    fonts: &mut crate::typeset::Families,
) -> R<()> {
    match name {
        "directlua" | "luadirect" => {
            let chunk = read_chunk(eng, lx)?;
            // Read out of the TEXT before anything runs, which is where it was
            // read from when nothing could: a chunk that fails half way through
            // for a reason of its own must not cost the document its fallback
            // chain, and the call is `luaotfload`'s rather than the engine's.
            let chain = crate::typeset::fallback_chain(&chunk);
            if !chain.is_empty() {
                fonts.fallbacks = chain;
            }
            let result = run(eng, lx, out, &chunk, Yield::ToInput);
            // And again from the CALL, which is authoritative: a document that
            // builds its chain rather than writing it out gets the list it
            // actually passed.
            if let Some(chain) = take_fallbacks() {
                fonts.fallbacks = chain;
            }
            result
        }
        // `\latelua` is a whatsit executed at shipout in LuaTeX, and there is no
        // shipout here to hang one on. It runs where it stands, and what it
        // printed is dropped — a whatsit cannot contribute tokens to the input
        // however it is timed, so dropping the print output is the one part of
        // this that is not an approximation.
        "latelua" => {
            let chunk = read_chunk(eng, lx)?;
            run(eng, lx, out, &chunk, Yield::Discard)
        }
        "luaescapestring" => {
            let text = read_group(eng, lx)?;
            let escaped = escape_string(&text);
            let toks = tokenize(&escaped, Regime::Verbatim, LineMode::Partial, &eng.cats);
            lx.push_back(&toks);
            Ok(())
        }
        "luafunction" | "luafunctioncall" => {
            let n = eng.scan_number_file(lx)?;
            call_function(eng, lx, out, n, Yield::ToInput)
        }
        "lateluafunction" => {
            let n = eng.scan_number_file(lx)?;
            call_function(eng, lx, out, n, Yield::Discard)
        }
        "luadef" => {
            let Some(Token::Cs(cs)) = eng.take_file(lx) else {
                return Err(TexError("Missing control sequence after \\luadef".into()));
            };
            let n = eng.scan_number_file(lx)?;
            LUADEFS.with(|m| m.borrow_mut().insert(cs.name().to_string(), n));
            ANY_LUADEF.with(|f| f.set(true));
            Ok(())
        }
        // A name `\luadef` bound to a function index.
        _ => {
            let Some(n) = LUADEFS.with(|m| m.borrow().get(name).copied()) else {
                return Err(TexError(format!("Undefined control sequence \\{name}")));
            };
            call_function(eng, lx, out, n, Yield::ToInput)
        }
    }
}

/// What happens to what the chunk printed.
#[derive(Clone, Copy, PartialEq)]
enum Yield {
    /// Pushed in front of the mouth, to be read next.
    ToInput,
    /// Dropped, because the chunk stands where no input can be contributed.
    Discard,
}

// ── reading the argument ─────────────────────────────────────────────────

/// The ⟨general text⟩ of a Lua primitive, as the string Lua is given.
///
/// The manual: "The ⟨general text⟩ is expanded fully, and then fed into the Lua
/// interpreter. After reading and expansion has been applied to the ⟨general
/// text⟩, the resulting token list is converted to a string as if it was
/// displayed using `\the\toks`." Expansion here is `expand_macros_only`, which
/// is the same pass `\edef` uses: macros expand, `\the` and the conditionals
/// survive as tokens, because those read run-time state that lowering does not
/// have. `tokens_text` is `\the\toks`'s own serialisation, trailing space after
/// a control word included.
///
/// The optional ⟨16-bit number⟩ before the group names a chunk in `lua.name`
/// for error messages. It is read and dropped: nothing here reports a Lua error
/// by chunk name.
fn read_chunk(eng: &mut Engine, lx: &mut Lexer) -> R<String> {
    skip_chunk_name(eng, lx);
    read_group(eng, lx)
}

/// A braced group, expanded and serialised.
fn read_group(eng: &mut Engine, lx: &mut Lexer) -> R<String> {
    let toks = eng.read_balanced_group(lx)?;
    let expanded = eng.expand_macros_only(&toks)?;
    let escaped = apply_escapestring(eng, &expanded);
    Ok(eng.tokens_text(&escaped))
}

/// Apply `\luaescapestring{...}` where it stands inside a chunk.
///
/// It is expandable, and the manual says the ⟨general text⟩ of `\directlua` is
/// expanded fully before it becomes a string — so by the time Lua sees the
/// chunk, `f("\luaescapestring{a"b}")` has already become `f("a\"b")`. That is
/// what the primitive is FOR: it is how a document gets a TeX value that might
/// contain a quote or a backslash into a Lua string literal, and leaving it
/// unexpanded would hand Lua a syntax error instead.
fn apply_escapestring(eng: &Engine, toks: &[Token]) -> Vec<Token> {
    if !toks.iter().any(|t| t.is_cs("luaescapestring")) {
        return toks.to_vec();
    }
    let mut out = Vec::with_capacity(toks.len());
    let mut i = 0;
    while i < toks.len() {
        if !toks[i].is_cs("luaescapestring") {
            out.push(toks[i]);
            i += 1;
            continue;
        }
        let Some((body, next)) = group_after(toks, i + 1) else {
            out.push(toks[i]);
            i += 1;
            continue;
        };
        let inner = apply_escapestring(eng, &body);
        for c in escape_string(&eng.tokens_text(&inner)).chars() {
            out.push(match c == ' ' {
                true => Token::Char(' ', Cat::Space),
                false => Token::Char(c, Cat::Other),
            });
        }
        i = next;
    }
    out
}

/// The balanced group starting at or after `at`, and the index past it.
fn group_after(toks: &[Token], at: usize) -> Option<(Vec<Token>, usize)> {
    let mut i = at;
    while matches!(toks.get(i), Some(t) if t.is_space()) {
        i += 1;
    }
    if !matches!(toks.get(i), Some(Token::Char(_, Cat::BeginGroup))) {
        return None;
    }
    i += 1;
    let mut depth = 1usize;
    let mut body = Vec::new();
    while let Some(t) = toks.get(i) {
        match t {
            Token::Char(_, Cat::BeginGroup) => depth += 1,
            Token::Char(_, Cat::EndGroup) => {
                depth -= 1;
                if depth == 0 {
                    return Some((body, i + 1));
                }
            }
            _ => {}
        }
        body.push(*t);
        i += 1;
    }
    None
}

/// Consume `\directlua 5 {...}`'s `5`, if it is there.
fn skip_chunk_name(eng: &mut Engine, lx: &mut Lexer) {
    let mut held = Vec::new();
    let mut digits = 0usize;
    while let Some(t) = eng.take_file(lx) {
        match t {
            Token::Char(c, _) if c.is_ascii_digit() => {
                digits += 1;
                held.clear();
            }
            _ => {
                held.push(t);
                break;
            }
        }
    }
    // Nothing numeric was there: put back exactly what was taken.
    if digits == 0 {
        for t in held.into_iter().rev() {
            lx.push_back(&[t]);
        }
        return;
    }
    for t in held.into_iter().rev() {
        lx.push_back(&[t]);
    }
}

/// `\luaescapestring`'s escaping (§2.4.3): "embedded backslashes, double and
/// single quotes, and newlines and carriage returns are escaped […] for the
/// line endings, converting them to n and r respectively".
fn escape_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' | '"' | '\'' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

// ── running a chunk ──────────────────────────────────────────────────────

/// Run `chunk`, then apply everything it did.
fn run(
    eng: &mut Engine,
    lx: &mut Lexer,
    out: &mut Vec<Cmd>,
    chunk: &str,
    yield_to: Yield,
) -> R<()> {
    let (lua, bridge) = runtime(eng)?;
    snapshot(eng, out, &bridge);
    let result = token::with(&lua, eng, lx, |lua| {
        lua.load(chunk).set_name("\\directlua").exec()
    });
    finish(eng, lx, out, &bridge, yield_to, flatten(result))
}

/// A failure to install the `token` library reads as the chunk's own failure.
///
/// There is nothing else it could be: the library is built fresh for every
/// chunk, so a state that cannot take it is a state the chunk cannot run in.
fn flatten(result: mlua::Result<mlua::Result<()>>) -> mlua::Result<()> {
    result.and_then(|inner| inner)
}

/// Call `lua.get_functions_table()[n]`, which is what `\luafunction n` is.
///
/// The manual: "The function, when called in fact gets one argument, being the
/// index", which is why `n` is passed.
fn call_function(
    eng: &mut Engine,
    lx: &mut Lexer,
    out: &mut Vec<Cmd>,
    n: i64,
    yield_to: Yield,
) -> R<()> {
    let (lua, bridge) = runtime(eng)?;
    snapshot(eng, out, &bridge);
    // The `token` library is installed here too, and that is the whole point of
    // the manual's second worked example: `\def\mymacro{\directlua{mymacro()}}`
    // with `local d = token.scan_dimen()` inside `mymacro` reads `\mymacro 12pt`
    // by fetching from the input rather than by being handed a string.
    let result = token::with(&lua, eng, lx, |lua| {
        functions_table(lua)
            .and_then(|t| t.get::<mlua::Function>(n))
            .and_then(|f| f.call::<()>(n))
    });
    finish(eng, lx, out, &bridge, yield_to, flatten(result))
}

/// Apply a finished chunk: its register writes, its output, its error.
fn finish(
    eng: &mut Engine,
    lx: &mut Lexer,
    out: &mut Vec<Cmd>,
    bridge: &Rc<RefCell<Bridge>>,
    yield_to: Yield,
    result: mlua::Result<()>,
) -> R<()> {
    let mut b = bridge.borrow_mut();
    // Register writes stand even when the chunk then failed, exactly as an
    // assignment made before an error stands in TeX.
    for (&reg, &v) in &b.counts_written {
        eng.count.insert(reg, v);
        out.push(Cmd::SetCount(reg, Num::Literal(v)));
    }
    let toks_written = std::mem::take(&mut b.toks_written);
    for (reg, text) in toks_written {
        let toks = tokenize(&text, Regime::Verbatim, LineMode::Partial, &eng.cats);
        eng.toks.insert(reg, toks);
    }
    // What the chunk logged is terminal output, and the terminal output of a
    // compiled document is its `Cmd::Message` stream — `Engine::messages` is
    // the interpreting path's, which a lowered document never reaches.
    for text in std::mem::take(&mut b.messages) {
        out.push(Cmd::Message(vec![crate::ir::MsgOp::Text(text)]));
    }
    if let Some(e) = b.error.take() {
        return Err(TexError(e));
    }
    if let Err(e) = result {
        return Err(TexError(lua_error_text(&e)));
    }
    if yield_to == Yield::Discard {
        b.prints.clear();
        return Ok(());
    }
    let prints = std::mem::take(&mut b.prints);
    let mut toks = Vec::new();
    let last = prints.len().saturating_sub(1);
    for (i, p) in prints.iter().enumerate() {
        // "The very last string of the very last tex.print command in a
        // \directlua will not have the \endlinechar appended, all others do."
        let line = match p.line == LineMode::Line && i == last {
            true => LineMode::Partial,
            false => p.line,
        };
        toks.extend(tokenize(&p.text, p.cats, line, &eng.cats));
    }
    lx.push_back(&toks);
    Ok(())
}

/// A Lua error as a TeX one.
///
/// mlua wraps a callback's error in a `CallbackError` carrying a traceback; the
/// traceback is Lua's own stack and says nothing a TeX user can act on, so what
/// is kept is the message. What must NOT happen is a panic or a silent skip:
/// a chunk that failed is a document that is wrong, and the run stops.
fn lua_error_text(e: &mlua::Error) -> String {
    let text = match e {
        mlua::Error::CallbackError { cause, .. } => return lua_error_text(cause),
        mlua::Error::SyntaxError { message, .. } => format!("Lua syntax error: {message}"),
        other => format!("Lua error: {other}"),
    };
    // A TeX error is one line. Lua's stack traceback is the interpreter's own
    // frames and says nothing about the document, so it is cut here — the
    // message that precedes it carries the chunk name and the line.
    let text = match text.split_once("\nstack traceback:") {
        Some((head, _)) => head.to_string(),
        None => text,
    };
    text.replace('\n', " ").trim_end().to_string()
}

/// The control sequence a document's Lua state is remembered by.
///
/// The NUL keeps it out of reach of any document — a control sequence's name
/// comes from the mouth and the mouth cannot produce one — which is the same
/// trick `crate::expand`'s advice markers use, and for the same reason.
const OWNER_CS: &str = "\u{0}lua-document";

/// Which document this is, marking it if it has not been marked.
///
/// A batch run compiles many documents on one thread and a chunk's globals must
/// not leak from one into the next, so the state has to know when the document
/// changed. The engine's ADDRESS will not do it: a `Lowerer` is usually a local,
/// so the next document's engine lands on the same bytes as the last one's and
/// two documents look like one. A mark carried in the engine's own meaning
/// table is per-engine by construction. It is written directly rather than
/// through an assignment, so no save record is made for it and a `\directlua`
/// inside a group does not lose the mark when the group closes.
fn document_id(eng: &mut Engine) -> i64 {
    let cs = crate::token::CsId::intern(OWNER_CS);
    if let Some(Meaning::CharDef(id)) = eng.meanings.get(&cs) {
        return *id;
    }
    // Seeded rather than started at 1: `\dump` writes the meaning table into a
    // format file, so an id can come back in another process, and a counter
    // that always starts at 1 would let a restored id collide with a live one.
    static NEXT: once_cell::sync::Lazy<std::sync::atomic::AtomicI64> =
        once_cell::sync::Lazy::new(|| {
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| i64::from(d.subsec_nanos()))
                .unwrap_or(1);
            std::sync::atomic::AtomicI64::new(seed * 1_000_000 + 1)
        });
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    eng.meanings.insert(cs, Meaning::CharDef(id));
    id
}

/// The state for this document, building it on first use.
fn runtime(eng: &mut Engine) -> R<(Lua, Rc<RefCell<Bridge>>)> {
    let owner = document_id(eng) as usize;
    RUNTIME.with(|slot| {
        let mut slot = slot.borrow_mut();
        let fresh = match slot.as_ref() {
            Some(rt) => rt.owner != owner,
            None => true,
        };
        if fresh {
            *slot = Some(build(owner).map_err(|e| TexError(lua_error_text(&e)))?);
            LUADEFS.with(|m| m.borrow_mut().clear());
            ANY_LUADEF.with(|f| f.set(false));
        }
        let rt = slot.as_ref().expect("just built");
        Ok((rt.lua.clone(), Rc::clone(&rt.bridge)))
    })
}

/// What the frontend reads out of the engine before handing control to Lua.
fn snapshot(eng: &Engine, out: &[Cmd], bridge: &Rc<RefCell<Bridge>>) {
    let mut b = bridge.borrow_mut();
    b.prints.clear();
    b.error = None;
    b.counts.clone_from(&eng.count);
    fold_registers(out, &mut b.counts);
    b.counts_written.clear();
    b.toks_written.clear();
    b.count_names.clear();
    b.toks_names.clear();
    b.toks.clear();
    for &reg in eng.toks.keys() {
        b.toks.insert(reg, eng.toks_text(reg));
    }
    // `tex.count.scratchcounter` — "It is possible to use the names of relevant
    // \attributedef, \countdef, \dimendef, \skipdef, or \toksdef control
    // sequences as indices to these tables".
    for (name, meaning) in eng.meanings.iter() {
        match meaning {
            Meaning::CountDef(r) => {
                b.count_names.insert(name.name().to_string(), *r);
            }
            Meaning::ToksDef(r) => {
                b.toks_names.insert(name.name().to_string(), *r);
            }
            _ => {}
        }
    }
}

/// The registers the program will hold when it reaches this point.
///
/// `\count10=20` at the top of a document is not a frontend fact: a count lives
/// in a fusevm slot, so lowering emits `Cmd::SetCount` and `Engine::count`
/// stays empty. A chunk asking `tex.count[10]` would then read 0 where LuaTeX
/// reads 20 — the manual's own worked example. So the straight-line prefix of
/// the program that has been emitted so far is evaluated here, which is exactly
/// the part whose value is decided before the chunk runs.
///
/// What is deliberately NOT folded, because its value is not decided yet:
/// either arm of a run-time conditional, a loop body, and a group — a group
/// saves and restores every register it assigns, so its writes do not reach
/// here in the first place. A register whose only assignment is inside one of
/// those reads as whatever it was before it, and that is the one place a chunk
/// sees less than LuaTeX would.
fn fold_registers(cmds: &[Cmd], into: &mut HashMap<i64, i64>) {
    let value = |n: &Num, seen: &HashMap<i64, i64>| match n {
        Num::Literal(v) => Some(*v),
        Num::Count(r) => seen.get(r).copied(),
        Num::Rust { .. } => None,
    };
    for cmd in cmds {
        match cmd {
            Cmd::SetCount(reg, n) => match value(n, into) {
                Some(v) => {
                    into.insert(*reg, v);
                }
                // An unknown value must not leave a stale one behind.
                None => {
                    into.remove(reg);
                }
            },
            Cmd::Arith(op, reg, n) => {
                let (cur, by) = (into.get(reg).copied(), value(n, into));
                match (cur, by) {
                    (Some(a), Some(b)) => {
                        let v = match op {
                            crate::ir::Arith::Add => a + b,
                            crate::ir::Arith::Mul => a * b,
                            crate::ir::Arith::Div if b != 0 => a / b,
                            crate::ir::Arith::Div => a,
                        };
                        into.insert(*reg, v);
                    }
                    _ => {
                        into.remove(reg);
                    }
                }
            }
            // Colour wraps text, and its body runs unconditionally.
            Cmd::Color { body, .. } => fold_registers(body, into),
            _ => {}
        }
    }
}

/// The job name, which nothing threads into the lowerer.
///
/// `-jobname` and the input file are on the command line, and that is where
/// this reads them when nothing has called [`set_jobname`]. tex's own default
/// for input that came from the terminal rather than a file is `texput`.
fn jobname() -> String {
    if let Some(j) = JOBNAME.with(|j| j.borrow().clone()) {
        return j;
    }
    let mut args = std::env::args().skip(1);
    let mut file = None;
    while let Some(a) = args.next() {
        if let Some(v) = a
            .strip_prefix("-jobname=")
            .or_else(|| a.strip_prefix("--jobname="))
        {
            return v.to_string();
        }
        if a == "-jobname" || a == "--jobname" {
            if let Some(v) = args.next() {
                return v;
            }
        }
        if file.is_none() && !a.starts_with('-') && !a.starts_with('\\') && !a.starts_with('&') {
            file = Some(a);
        }
    }
    file.map(|f| {
        std::path::Path::new(&f)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| f.clone())
    })
    .unwrap_or_else(|| "texput".to_string())
}

// ── the tokens a print produces ──────────────────────────────────────────

/// Turn printed text into tokens, under the regime the print asked for.
///
/// The `Current` regime runs the text through the MOUTH — a `Lexer` over the
/// string, reading with the live catcode table — which is what makes this the
/// manual's "in-memory virtual file that is fed to the TEX scanner". Anything
/// else would be a second tokeniser to keep in step with the first.
fn tokenize(
    text: &str,
    regime: Regime,
    line: LineMode,
    cats: &crate::catcode::CatTable,
) -> Vec<Token> {
    match regime {
        Regime::Verbatim => text
            .chars()
            .map(|c| match c == ' ' {
                true => Token::Char(' ', Cat::Space),
                false => Token::Char(c, Cat::Other),
            })
            .collect(),
        // Through the mouth as well, under a table that says `cat` for every
        // character the text contains. Mapping the characters straight to
        // tokens would be wrong for exactly the catcodes the manual's own
        // example is about: `tex.cprint(9, ...)` is "all get ignored" and
        // `tex.cprint(14, ...)` is "comment triggers", and neither happens to a
        // token that never passed a scanner. `tex.cprint(10, ...)` collapses a
        // run of spaces into one for the same reason.
        Regime::All(cat) => {
            let mut table = crate::catcode::CatTable::new();
            for c in text.chars() {
                table.set(c, cat);
            }
            scan(text, LineMode::Partial, &table)
        }
        Regime::Current => scan(text, line, cats),
    }
}

/// The `Current` regime: the mouth, over a string.
fn scan(text: &str, line: LineMode, cats: &crate::catcode::CatTable) -> Vec<Token> {
    let mut out = Vec::new();
    let body = match line {
        // A whole input line: trailing spaces are dropped by TeX's line reader
        // (`tex.web` §31), and `\endlinechar` follows. Feeding the newline to
        // the mouth rather than inventing a token is what gets the three-state
        // rule right for free: a line end is a space in state M and a `\par` in
        // state N, so a printed empty string is a `\par` exactly as it is in
        // LuaTeX.
        LineMode::Line => format!("{}\n", text.trim_end_matches(' ')),
        LineMode::Partial => {
            // "TEX does not switch to the 'new line' state, so that leading
            // spaces are not ignored." A fresh `Lexer` starts in state N, which
            // would eat them, so the leading run is emitted as the one space
            // token state M would have produced and the rest is scanned.
            let rest = text.trim_start_matches(' ');
            if rest.len() != text.len() {
                out.push(Token::Char(' ', Cat::Space));
            }
            rest.to_string()
        }
    };
    let mut lx = Lexer::new(&body);
    while let Some(t) = lx.next_token(cats) {
        out.push(t);
    }
    out
}

// ── the Lua side ─────────────────────────────────────────────────────────

/// Where `lua.get_functions_table()`'s table is kept.
const FUNCTIONS_KEY: &str = "__texrs_lua_functions";

fn functions_table(lua: &Lua) -> mlua::Result<Table> {
    lua.globals().get::<Table>(FUNCTIONS_KEY)
}

/// Build the state: PUC-Lua 5.3 with its standard libraries, plus the `tex`
/// table a document reaches TeX through.
///
/// `Lua::new()` loads mlua's `StdLib::ALL_SAFE`, which is every standard
/// library except `debug` — so `string`, `table`, `math`, `os`, `io`, `utf8`,
/// `coroutine` and `package` are the ones PUC-Lua ships, not re-implementations.
fn build(owner: usize) -> mlua::Result<Runtime> {
    let lua = Lua::new();
    let bridge = Rc::new(RefCell::new(Bridge::default()));
    let globals = lua.globals();
    globals.set(FUNCTIONS_KEY, lua.create_table()?)?;
    globals.set("tex", tex_table(&lua, &bridge)?)?;
    globals.set("lua", lua_table(&lua)?)?;
    globals.set("texio", texio_table(&lua, &bridge)?)?;
    globals.set("status", status_table(&lua)?)?;
    globals.set("luatexbase", luatexbase_table(&lua)?)?;
    globals.set("luaotfload", luaotfload_table(&lua, &bridge)?)?;
    Ok(Runtime { lua, bridge, owner })
}

/// Lua's own `tostring`, so a number reaches TeX formatted as Lua formats it.
///
/// This is not decoration: Lua 5.3 prints a float with `%.14g`, so
/// `tex.print(math.pi)` is `3.1415926535898` — the manual prints exactly that —
/// while Rust's `f64` Display would make it `3.141592653589793` and every
/// document that prints a computed dimension would disagree with LuaTeX in the
/// last three digits.
fn lua_tostring(lua: &Lua, v: &Value) -> mlua::Result<String> {
    match v {
        Value::String(s) => Ok(s.to_str()?.to_string()),
        other => lua
            .globals()
            .get::<mlua::Function>("tostring")?
            .call::<String>(other.clone()),
    }
}

/// A print call's arguments: the optional catcode-table selector, then the
/// strings.
///
/// "tex.print(<number> n, <string> s, ...)" — a leading number is the selector
/// only when something follows it, because `tex.print(tex.count[10]+5)` is a
/// number being printed.
fn print_args(lua: &Lua, args: &Variadic<Value>) -> mlua::Result<(Option<i64>, Vec<String>)> {
    let mut rest = &args[..];
    let mut selector = None;
    if rest.len() > 1 {
        if let Some(n) = rest[0].as_integer() {
            selector = Some(n);
            rest = &rest[1..];
        }
    }
    let mut texts = Vec::new();
    // "If there is a table argument instead of a list of strings, this has to
    // be a consecutive array of strings to print (the first non-string value
    // will stop the printing process)."
    if let [Value::Table(t)] = rest {
        let len = t.raw_len();
        for i in 1..=len {
            match t.raw_get::<Value>(i)? {
                Value::String(s) => texts.push(s.to_str()?.to_string()),
                Value::Integer(n) => texts.push(n.to_string()),
                Value::Number(_) => texts.push(lua_tostring(lua, &t.raw_get::<Value>(i)?)?),
                _ => break,
            }
        }
        return Ok((selector, texts));
    }
    for v in rest {
        match v {
            Value::Nil | Value::Boolean(_) | Value::Table(_) | Value::Function(_) => break,
            other => texts.push(lua_tostring(lua, other)?),
        }
    }
    Ok((selector, texts))
}

/// Drop a leading `"global"` from a setter's arguments.
///
/// "In the function-based interface, it is possible to define values globally
/// by using the string `global` as the first function argument." The prefix is
/// accepted and ignored, because a Lua write here is not scoped by a group in
/// the first place — it goes straight into the register table and into a
/// `Cmd::SetCount`, and neither is undone by an `\endgroup`.
fn drop_global(args: &Variadic<Value>) -> Vec<Value> {
    match args.first() {
        Some(Value::String(s)) if s.to_str().map(|s| s == "global").unwrap_or(false) => {
            args.iter().skip(1).cloned().collect()
        }
        _ => args.iter().cloned().collect(),
    }
}

/// The regime a selector asks for. −2 is `\the\toks`; every other value falls
/// through to the live table, because texrs has no `\catcodetable` and the
/// manual says an invalid one is ignored.
fn regime_of(selector: Option<i64>) -> Regime {
    match selector {
        Some(-2) => Regime::Verbatim,
        _ => Regime::Current,
    }
}

/// The `tex` table: the print functions, the registers, the helpers.
fn tex_table(lua: &Lua, bridge: &Rc<RefCell<Bridge>>) -> mlua::Result<Table> {
    let tex = lua.create_table()?;

    for (name, line) in [("print", LineMode::Line), ("sprint", LineMode::Partial)] {
        let b = Rc::clone(bridge);
        tex.set(
            name,
            lua.create_function(move |lua, args: Variadic<Value>| {
                let (selector, texts) = print_args(lua, &args)?;
                let cats = regime_of(selector);
                let mut b = b.borrow_mut();
                for text in texts {
                    b.prints.push(Print { text, cats, line });
                }
                Ok(())
            })?,
        )?;
    }

    // "Each string argument is treated by TEX as a special kind of input line
    // that makes it suitable for use as a quick way to dump information: all
    // catcodes on that line are either 'space' (for ' ') or 'character' (for
    // all others). There is no \endlinechar appended."
    let b = Rc::clone(bridge);
    tex.set(
        "write",
        lua.create_function(move |lua, args: Variadic<Value>| {
            let (_, texts) = print_args(lua, &args)?;
            let mut b = b.borrow_mut();
            for text in texts {
                b.prints.push(Print {
                    text,
                    cats: Regime::Verbatim,
                    line: LineMode::Partial,
                });
            }
            Ok(())
        })?,
    )?;

    // `tex.cprint(n, ...)`: "takes a number indicating the to be used catcode".
    let b = Rc::clone(bridge);
    tex.set(
        "cprint",
        lua.create_function(move |lua, args: Variadic<Value>| {
            let n = args
                .first()
                .and_then(|v| v.as_integer())
                .ok_or_else(|| mlua::Error::runtime("tex.cprint wants a catcode"))?;
            let cat = crate::catcode::cat_from_i64(n)
                .ok_or_else(|| mlua::Error::runtime(format!("bad catcode {n}")))?;
            let rest = Variadic::from_iter(args.iter().skip(1).cloned());
            let (_, texts) = print_args(lua, &rest)?;
            let mut b = b.borrow_mut();
            for text in texts {
                b.prints.push(Print {
                    text,
                    cats: Regime::All(cat),
                    line: LineMode::Partial,
                });
            }
            Ok(())
        })?,
    )?;

    // "This function is basically a shortcut for repeated calls to
    // tex.sprint(<number> n, <string> s, ...), once for each of the supplied
    // argument tables."
    let b = Rc::clone(bridge);
    tex.set(
        "tprint",
        lua.create_function(move |lua, args: Variadic<Value>| {
            for arg in args.iter() {
                let Value::Table(t) = arg else { continue };
                let inner = Variadic::from_iter(t.clone().sequence_values::<Value>().flatten());
                let (selector, texts) = print_args(lua, &inner)?;
                let cats = regime_of(selector);
                let mut b = b.borrow_mut();
                for text in texts {
                    b.prints.push(Print {
                        text,
                        cats,
                        line: LineMode::Partial,
                    });
                }
            }
            Ok(())
        })?,
    )?;

    tex.set("count", register_table(lua, bridge, Kind::Count)?)?;
    tex.set("dimen", register_table(lua, bridge, Kind::Dimen)?)?;
    tex.set("toks", register_table(lua, bridge, Kind::Toks)?)?;
    // An attribute register is a count register in every way that matters here:
    // "Like the counts, the attribute registers accept and return Lua numbers."
    // Nothing in texrs reads attributes, so this is storage a chunk can use.
    tex.set("attribute", register_table(lua, bridge, Kind::Attribute)?)?;

    for (name, kind) in [
        ("getcount", Kind::Count),
        ("getdimen", Kind::Dimen),
        ("getattribute", Kind::Attribute),
    ] {
        let b = Rc::clone(bridge);
        tex.set(
            name,
            lua.create_function(move |_, key: Value| {
                let b = b.borrow();
                let reg = resolve(&b, kind, &key)?;
                Ok(b.counts.get(&reg).copied().unwrap_or(0))
            })?,
        )?;
    }
    for (name, kind) in [
        ("setcount", Kind::Count),
        ("setdimen", Kind::Dimen),
        ("setattribute", Kind::Attribute),
    ] {
        let b = Rc::clone(bridge);
        tex.set(
            name,
            lua.create_function(move |_, args: Variadic<Value>| {
                // `tex.setcount(["global",] n, v)`.
                let args = drop_global(&args);
                let [key, value] = &args[..] else {
                    return Err(mlua::Error::runtime(
                        "tex.set* wants a register and a value",
                    ));
                };
                let mut b = b.borrow_mut();
                let reg = resolve(&b, kind, key)?;
                let v = scaled_value(kind, value)?;
                b.counts.insert(reg, v);
                b.counts_written.insert(reg, v);
                Ok(())
            })?,
        )?;
    }
    tex.set("glue", register_table(lua, bridge, Kind::Glue)?)?;

    // "width, stretch, shrink, stretch_order, shrink_order = tex.getglue(n)
    // […] when you pass false as second argument to getglue you only get the
    // width returned."
    let b = Rc::clone(bridge);
    tex.set(
        "getglue",
        lua.create_function(move |_, args: Variadic<Value>| {
            let key = args.first().cloned().unwrap_or(Value::Nil);
            let b = b.borrow();
            let base = resolve(&b, Kind::Glue, &key)?;
            let (w, st, sh, sto, sho) = glue_get(&b, base);
            match args.get(1) {
                Some(Value::Boolean(false)) => Ok(Variadic::from_iter([Value::Integer(w)])),
                _ => Ok(Variadic::from_iter(
                    [w, st, sh, sto, sho].map(Value::Integer),
                )),
            }
        })?,
    )?;
    // "tex.setglue (['global'], <number> n, width, stretch, shrink,
    // stretch_order, shrink_order) […] If you pass no values or if a value is
    // not a number the corresponding property will become a zero."
    let b = Rc::clone(bridge);
    tex.set(
        "setglue",
        lua.create_function(move |_, args: Variadic<Value>| {
            let args = drop_global(&args);
            let Some(key) = args.first() else {
                return Err(mlua::Error::runtime("tex.setglue wants a register"));
            };
            let n = |i: usize| args.get(i).and_then(|v| v.as_integer()).unwrap_or(0);
            let mut b = b.borrow_mut();
            let base = resolve(&b, Kind::Glue, key)?;
            glue_set(&mut b, base, (n(1), n(2), n(3), n(4), n(5)))?;
            Ok(())
        })?,
    )?;
    // "The skip registers accept and return glue_spec userdata node objects."
    // There are no nodes here (see the module docs), so `tex.skip` refuses
    // rather than answering with something that is not a node: a chunk that
    // indexed a number where LuaTeX gave it a node would go wrong later, in the
    // output, instead of here. `tex.getglue` is the same register in numbers,
    // and the message says so.
    for name in ["getskip", "setskip", "getmuskip", "setmuskip", "getmuglue"] {
        tex.set(
            name,
            lua.create_function(move |_, _: Variadic<Value>| {
                Err::<(), _>(mlua::Error::runtime(format!(
                    "tex.{name} needs a glue_spec node, which texrs has no node interface for; \
                     use tex.getglue/tex.setglue, which are the same registers as numbers"
                )))
            })?,
        )?;
    }
    // A box register holds a NODE LIST, and there is nothing here that could
    // stand in for one: `\box0` is a `crate::box_` built by the packer, not a
    // number and not a string. These say so instead of being absent, because
    // "attempt to call a nil value" would not tell a document's author which of
    // the two engines' worlds it had walked into.
    for name in [
        "getbox", "setbox", "splitbox", "getlist", "setlist", "getnest", "getmath", "setmath",
    ] {
        tex.set(
            name,
            lua.create_function(move |_, _: Variadic<Value>| {
                Err::<(), _>(mlua::Error::runtime(format!(
                    "tex.{name} works on node lists, which texrs has no node interface for"
                )))
            })?,
        )?;
    }
    let skip = lua.create_table()?;
    let mt = lua.create_table()?;
    let refuse = lua.create_function(|_, _: Variadic<Value>| {
        Err::<(), _>(mlua::Error::runtime(
            "tex.skip is a glue_spec node, which texrs has no node interface for; \
             use tex.glue (the width) or tex.getglue (all five numbers)",
        ))
    })?;
    mt.set("__index", refuse.clone())?;
    mt.set("__newindex", refuse)?;
    skip.set_metatable(Some(mt))?;
    tex.set("skip", skip)?;

    // "local d = tex.getdimen('foo') if tex.isdimen('bar') then …" — the test
    // half of the threesome. It does not answer a BOOLEAN: `luatex` 1.24.0
    // answers the register NUMBER the argument names — `tex.iscount(0)` is `0`
    // and `tex.iscount('zz')` is `42` for a `\countdef\zz=42` — and `false`
    // only when the name is not a register. Read off the engine, because `0` is
    // truthy in Lua and a boolean here would still have passed every `if`.
    for (name, kind) in [
        ("iscount", Kind::Count),
        ("isdimen", Kind::Dimen),
        ("isskip", Kind::Glue),
        ("isglue", Kind::Glue),
        ("istoks", Kind::Toks),
        ("isattribute", Kind::Attribute),
    ] {
        let b = Rc::clone(bridge);
        tex.set(
            name,
            lua.create_function(move |_, key: Value| {
                let b = b.borrow();
                let Ok(slot) = resolve(&b, kind, &key) else {
                    return Ok(Value::Boolean(false));
                };
                let base = match kind {
                    Kind::Count | Kind::Toks => 0,
                    Kind::Dimen => crate::compiler::DIMEN_BASE,
                    Kind::Glue => crate::compiler::SKIP_BASE,
                    Kind::Attribute => ATTRIBUTE_BASE,
                };
                let stride = match kind {
                    Kind::Glue => crate::compiler::SKIP_STRIDE,
                    _ => 1,
                };
                Ok(Value::Integer((slot - base) / stride))
            })?,
        )?;
    }

    let b = Rc::clone(bridge);
    tex.set(
        "gettoks",
        lua.create_function(move |_, key: Value| {
            let b = b.borrow();
            let reg = resolve(&b, Kind::Toks, &key)?;
            Ok(b.toks.get(&reg).cloned().unwrap_or_default())
        })?,
    )?;
    let b = Rc::clone(bridge);
    tex.set(
        "settoks",
        lua.create_function(move |_, args: Variadic<Value>| {
            let args = drop_global(&args);
            let [key, value] = &args[..] else {
                return Err(mlua::Error::runtime(
                    "tex.settoks wants a register and a string",
                ));
            };
            let mut b = b.borrow_mut();
            let reg = resolve(&b, Kind::Toks, key)?;
            let text = value.to_string()?;
            b.toks.insert(reg, text.clone());
            b.toks_written.insert(reg, text);
            Ok(())
        })?,
    )?;

    // A field rather than a function, as the manual lists it. The job name is
    // fixed for a run, so it is read once here rather than per chunk.
    tex.set("jobname", jobname())?;
    let b = Rc::clone(bridge);
    tex.set(
        "error",
        lua.create_function(move |_, (msg, _help): (String, Option<Table>)| {
            b.borrow_mut().error = Some(msg);
            Ok(())
        })?,
    )?;

    // "Converts the number o or a string s that represents an explicit
    // dimension into an integer number of scaled points."
    tex.set(
        "sp",
        lua.create_function(|_, v: Value| {
            scaled_value(Kind::Dimen, &v).map_err(|e| mlua::Error::runtime(e.to_string()))
        })?,
    )?;
    tex.set(
        "round",
        lua.create_function(|_, v: f64| Ok(clamp_register(v.round() as i64)))?,
    )?;
    tex.set(
        "scale",
        lua.create_function(|_, (v, delta): (f64, f64)| {
            Ok(clamp_register((v * delta).round() as i64))
        })?,
    )?;
    tex.set(
        "number",
        lua.create_function(|_, n: i64| Ok(n.to_string()))?,
    )?;
    tex.set(
        "romannumeral",
        lua.create_function(|_, n: i64| Ok(roman(n)))?,
    )?;
    // `\directlua{tex.enableprimitives('', tex.extraprimitives())}` is the first
    // line of every `luatex -ini` document, because LuaTeX hides its extra
    // primitives until asked. texrs never hid them, so this is the no-op that
    // lets such a document run unchanged rather than stopping on a nil call.
    tex.set(
        "enableprimitives",
        lua.create_function(|_, _: Variadic<Value>| Ok(()))?,
    )?;
    tex.set(
        "extraprimitives",
        lua.create_function(|lua, _: Variadic<Value>| lua.create_table())?,
    )?;
    Ok(tex)
}

/// Which register file a name or number is an index into.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Count,
    Dimen,
    /// `\skip`, read and written as five numbers rather than as a node — the
    /// manual's `tex.glue`/`tex.getglue`/`tex.setglue` family. `tex.skip` and
    /// `tex.getskip` are the same registers through a `glue_spec` NODE, and
    /// there are no nodes here, so those refuse rather than answer.
    Glue,
    Toks,
    Attribute,
}

impl Kind {
    /// How the register is spelt in an error message.
    fn primitive(self) -> &'static str {
        match self {
            Kind::Count => "count",
            Kind::Dimen => "dimen",
            Kind::Glue => "skip",
            Kind::Toks => "toks",
            Kind::Attribute => "attribute",
        }
    }
}

/// Attribute registers, kept clear of the count and dimension slots.
const ATTRIBUTE_BASE: i64 = 1 << 20;

/// How many registers of each kind there are.
///
/// TeX82's 256 (`tex.web` §236), which is what `crate::compiler` lays out and
/// what `\countdef`, `\dimendef` and `\skipdef` accept — not LuaTeX's 65,536.
/// The check matters rather than being decoration: the count, dimension and
/// glue files are consecutive ranges of ONE slot file, so `tex.count[300]`
/// unchecked is a write to `\dimen44` and `tex.dimen[300]` unchecked is a write
/// into the middle of `\skip11`. An out-of-range index is refused, because a
/// document that silently wrote to a register it did not name is the failure
/// mode this whole module exists to end.
const REGISTERS: i64 = 256;

/// A register index from a Lua key: a number, or a `\countdef`-style name.
fn resolve(b: &Bridge, kind: Kind, key: &Value) -> mlua::Result<i64> {
    let base = match kind {
        Kind::Count | Kind::Toks => 0,
        Kind::Dimen => crate::compiler::DIMEN_BASE,
        Kind::Glue => crate::compiler::SKIP_BASE,
        Kind::Attribute => ATTRIBUTE_BASE,
    };
    // A glue is four slots — natural, stretch, shrink, and the packed orders —
    // so register n starts at four times n. `crate::lower`'s own arithmetic.
    let stride = match kind {
        Kind::Glue => crate::compiler::SKIP_STRIDE,
        _ => 1,
    };
    if let Some(n) = key.as_integer() {
        // An attribute is storage this module keeps for the chunk and a token
        // register is a map rather than a slot range, so neither can collide
        // with a neighbour and neither is bounded by the slot file.
        let bounded = matches!(kind, Kind::Count | Kind::Dimen | Kind::Glue);
        if bounded && !(0..REGISTERS).contains(&n) {
            return Err(mlua::Error::runtime(format!(
                "\\{}{n} is out of range: texrs has {REGISTERS} {} registers",
                kind.primitive(),
                kind.primitive(),
            )));
        }
        return Ok(base + n * stride);
    }
    let Value::String(s) = key else {
        return Err(mlua::Error::runtime(
            "register index must be a number or a name",
        ));
    };
    let name = s.to_str()?.to_string();
    let found = match kind {
        Kind::Toks => b.toks_names.get(&name).copied(),
        // A `\dimendef` or `\skipdef` name already carries its base and its
        // stride, which is how `crate::expand` records one, so neither is
        // applied twice.
        _ => b.count_names.get(&name).copied(),
    };
    found.ok_or_else(|| mlua::Error::runtime(format!("no register named {name}")))
}

/// A glue register's five numbers, as `tex.getglue` reports them.
///
/// The two ORDERS are translated. texrs numbers them as TeX82 does — 1, 2, 3
/// are `fil`, `fill`, `filll` (`crate::glue::order_unit`) — while LuaTeX has a
/// fourth, finer infinity called `fi` and numbers from it, so its `fil` is 2.
/// Read off `luatex` 1.24.0 rather than assumed: `\skip0=1pt plus 2fill minus
/// 3filll` answers `3` and `4` from `tex.getglue(0)`, and `plus 2fi` answers
/// `1`. A document that branches on the order it got is written against THOSE
/// numbers, so that is what it is given.
fn glue_get(b: &Bridge, base: i64) -> (i64, i64, i64, i64, i64) {
    let at = |i: i64| b.counts.get(&(base + i)).copied().unwrap_or(0);
    let packed = at(3);
    (
        at(0),
        at(1),
        at(2),
        to_luatex_order(packed / 4),
        to_luatex_order(packed % 4),
    )
}

/// Write a glue register's four slots from LuaTeX's five numbers.
fn glue_set(b: &mut Bridge, base: i64, glue: (i64, i64, i64, i64, i64)) -> mlua::Result<()> {
    let (w, st, sh, sto, sho) = glue;
    let packed = from_luatex_order(sto)? * 4 + from_luatex_order(sho)?;
    for (i, v) in [w, st, sh, packed].into_iter().enumerate() {
        let v = clamp_register(v);
        b.counts.insert(base + i as i64, v);
        b.counts_written.insert(base + i as i64, v);
    }
    Ok(())
}

/// texrs's infinity order as LuaTeX numbers it.
fn to_luatex_order(order: i64) -> i64 {
    match order {
        0 => 0,
        n => n + 1,
    }
}

/// LuaTeX's infinity order as texrs numbers it.
fn from_luatex_order(order: i64) -> mlua::Result<i64> {
    match order {
        0 => Ok(0),
        // `fi` is LuaTeX's own extra infinity, finer than `fil`. texrs's glue
        // has TeX82's three orders and no slot to put a fourth in, so a chunk
        // asking for one is told rather than quietly given `fil`.
        1 => Err(mlua::Error::runtime(
            "stretch order 1 is LuaTeX's `fi`, which texrs has no unit for",
        )),
        n if (2..=4).contains(&n) => Ok(n - 1),
        n => Err(mlua::Error::runtime(format!("bad glue order {n}"))),
    }
}

/// What a register write stores: a count takes a number, a dimension takes a
/// number of scaled points OR a string with a unit.
///
/// "The dimension registers accept Lua numbers (in scaled points) or strings
/// (with an included absolute dimension; em and ex and px are forbidden). The
/// result is always a number in scaled points."
fn scaled_value(kind: Kind, v: &Value) -> mlua::Result<i64> {
    if let Some(n) = v.as_integer() {
        return Ok(clamp_register(n));
    }
    if let Some(n) = v.as_number() {
        return Ok(clamp_register(n.round() as i64));
    }
    let Value::String(s) = v else {
        return Err(mlua::Error::runtime(
            "register value must be a number or a dimension",
        ));
    };
    if kind != Kind::Dimen {
        return Err(mlua::Error::runtime("register value must be a number"));
    }
    parse_dimen(s.to_str()?.trim()).ok_or_else(|| mlua::Error::runtime("bad dimension"))
}

/// A dimension as `tex.sp` reads one: a sign, a number, a unit.
///
/// "For parsing the string, the same scanning and conversion rules are used
/// that LuaTEX would use if it was scanning a dimension specifier […] except
/// […] infinite dimension units (fil...) are forbidden." The conversion is
/// `crate::dimen`'s, which is `tex.web` §458's exact integer ratio rather than
/// a float multiply — the reason `1in` is `72.26999pt`.
fn parse_dimen(text: &str) -> Option<i64> {
    let mut chars = text.chars().peekable();
    let mut sign = 1i64;
    while let Some(&c) = chars.peek() {
        match c {
            '-' => {
                sign = -sign;
                chars.next();
            }
            '+' | ' ' => {
                chars.next();
            }
            _ => break,
        }
    }
    let mut int = String::new();
    while let Some(&c) = chars.peek() {
        if !c.is_ascii_digit() {
            break;
        }
        int.push(c);
        chars.next();
    }
    let mut frac = String::new();
    if matches!(chars.peek(), Some('.') | Some(',')) {
        chars.next();
        while let Some(&c) = chars.peek() {
            if !c.is_ascii_digit() {
                break;
            }
            frac.push(c);
            chars.next();
        }
    }
    if int.is_empty() && frac.is_empty() {
        return None;
    }
    let unit: String = chars.collect::<String>().trim().to_lowercase();
    if unit.starts_with("fil") || unit == "em" || unit == "ex" || unit == "px" {
        return None;
    }
    let sp = crate::dimen::to_scaled(
        int.parse::<i64>().unwrap_or(0),
        crate::dimen::round_decimals(&frac),
        &unit,
    )?;
    Some(clamp_register(sign * sp))
}

/// "returns a number that is in the range of a valid TEX register value".
fn clamp_register(n: i64) -> i64 {
    n.clamp(-crate::dimen::MAX_DIMEN, crate::dimen::MAX_DIMEN)
}

/// `\romannumeral`'s digits, for `tex.romannumeral`.
fn roman(mut n: i64) -> String {
    if n <= 0 {
        return String::new();
    }
    let table = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut out = String::new();
    for (v, s) in table {
        while n >= v {
            out.push_str(s);
            n -= v;
        }
    }
    out
}

/// One of the register sub-tables: `tex.count[10]` and `tex.count[10] = 5`.
///
/// A metatable rather than a snapshot copied into a plain table, because a
/// write has to be recorded: the frontend applies only the registers the chunk
/// actually assigned, so a chunk that reads a hundred and writes one does not
/// emit a hundred `Cmd::SetCount`s.
fn register_table(lua: &Lua, bridge: &Rc<RefCell<Bridge>>, kind: Kind) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    let mt = lua.create_table()?;
    let b = Rc::clone(bridge);
    mt.set(
        "__index",
        lua.create_function(move |lua, (_t, key): (Table, Value)| {
            let b = b.borrow();
            let reg = resolve(&b, kind, &key)?;
            match kind {
                Kind::Toks => lua
                    .create_string(b.toks.get(&reg).cloned().unwrap_or_default())
                    .map(Value::String),
                // "The glue registers are just skip registers but instead of
                // userdata are verbose." Indexing one answers a single value in
                // Lua whatever the register holds, and `luatex` answers the
                // WIDTH: `\skip0=1pt plus 2fil minus 3pt` then `tex.glue[0]` is
                // `65536`. The other four are `tex.getglue`'s.
                Kind::Glue => Ok(Value::Integer(glue_get(&b, reg).0)),
                _ => Ok(Value::Integer(b.counts.get(&reg).copied().unwrap_or(0))),
            }
        })?,
    )?;
    let b = Rc::clone(bridge);
    mt.set(
        "__newindex",
        lua.create_function(move |_, (_t, key, value): (Table, Value, Value)| {
            let mut b = b.borrow_mut();
            let reg = resolve(&b, kind, &key)?;
            match kind {
                Kind::Toks => {
                    let text = value.to_string()?;
                    b.toks.insert(reg, text.clone());
                    b.toks_written.insert(reg, text);
                }
                // A number is a rigid glue of that width; a table is the five
                // numbers `tex.setglue` takes, short tables padded with zeros.
                // `luatex` refuses both spellings — `tex.glue[2] = 65536` is
                // "argument of 'setglue' must be a string or a number" there —
                // so nothing that works in LuaTeX changes meaning here.
                Kind::Glue => {
                    let g = match &value {
                        Value::Table(t) => {
                            let n = |i| t.raw_get::<Option<i64>>(i).unwrap_or(None).unwrap_or(0);
                            (n(1), n(2), n(3), n(4), n(5))
                        }
                        other => (scaled_value(Kind::Dimen, other)?, 0, 0, 0, 0),
                    };
                    glue_set(&mut b, reg, g)?;
                }
                _ => {
                    let v = scaled_value(kind, &value)?;
                    b.counts.insert(reg, v);
                    b.counts_written.insert(reg, v);
                }
            }
            Ok(())
        })?,
    )?;
    t.set_metatable(Some(mt))?;
    Ok(t)
}

/// The `lua` table: the version, and the function registry `\luafunction` calls.
fn lua_table(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    // "<string> s = lua.version — This returns the Lua version identifier
    // string." `_VERSION` is the interpreter's own answer, so this cannot drift
    // from the interpreter that is actually running.
    t.set("version", lua.globals().get::<Value>("_VERSION")?)?;
    t.set(
        "get_functions_table",
        lua.create_function(|lua, ()| functions_table(lua))?,
    )?;
    Ok(t)
}

/// `texio.write` and `texio.write_nl`: terminal output, not input.
///
/// texrs's terminal output IS its `\message` stream, so that is where these go.
/// The selector argument (`"term"`, `"log"`, `"term and log"`) is accepted and
/// ignored — there is one stream here.
fn texio_table(lua: &Lua, bridge: &Rc<RefCell<Bridge>>) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    for name in ["write", "write_nl"] {
        let b = Rc::clone(bridge);
        t.set(
            name,
            lua.create_function(move |lua, args: Variadic<Value>| {
                let mut parts = Vec::new();
                for v in args.iter() {
                    let s = lua_tostring(lua, v)?;
                    if matches!(s.as_str(), "term" | "log" | "term and log") && parts.is_empty() {
                        continue;
                    }
                    parts.push(s);
                }
                b.borrow_mut().messages.push(parts.join(""));
                Ok(())
            })?,
        )?;
    }
    Ok(t)
}

/// `status`: what the run is.
///
/// Deliberately texrs's own identity rather than a forged LuaTeX version. A
/// document that branches on `status.luatex_version` is asking which engine it
/// is in, and answering "LuaTeX" would be a lie that only shows up later, in
/// the output — so `status.luatex_version` is ABSENT here, and a document that
/// reads it gets nil rather than `124`. (`luatex` 1.24.0 answers `124`, its
/// `status.banner` is `This is LuaTeX, Version 1.24.0 (TeX Live 2026)` and its
/// `status.engine` is `luatex`; none of those is what this engine is.)
///
/// `status.list()` is the one function of the library: it hands back every
/// field as a table, which is how a document that wants to log what it is
/// running under does it without naming a field that may not exist.
fn status_table(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("banner", format!("texrs {}", env!("CARGO_PKG_VERSION")))?;
    t.set("engine", "texrs")?;
    t.set("jobname", jobname())?;
    // "-ini" is what LuaTeX calls a run that is building a format rather than
    // reading one. texrs has no format files to build, so no run is one.
    t.set("ini_version", false)?;
    let fields = t.clone();
    t.set(
        "list",
        lua.create_function(move |lua, ()| {
            let out = lua.create_table()?;
            for pair in fields.clone().pairs::<Value, Value>() {
                let (k, v) = pair?;
                if !matches!(v, Value::Function(_)) {
                    out.set(k, v)?;
                }
            }
            Ok(out)
        })?,
    )?;
    Ok(t)
}

/// `luaotfload`: the one call from a loaded package that the backend answers.
///
/// Every book in the corpus opens with
/// `\directlua{luaotfload.add_fallback("symfb", {...})}`, and that list is the
/// only statement a document makes of which faces a glyph its own face lacks
/// comes from — the box drawing, the arrows and the Greek that made those books
/// need LuaTeX. `crate::typeset` implements exactly that, so the function is
/// real here rather than a stub, and it is present rather than absent for a
/// second reason: nothing loads packages, so a chunk calling into a nil global
/// would stop a document that used to run.
///
/// `luaotfload.add_fallback(name, list)` and `luaotfload.add_fallback(list)` are
/// both spellings the package takes; the name is the chain's own and nothing
/// here refers to it.
fn luaotfload_table(lua: &Lua, bridge: &Rc<RefCell<Bridge>>) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    let b = Rc::clone(bridge);
    t.set(
        "add_fallback",
        lua.create_function(move |_, args: Variadic<Value>| {
            let list = args.iter().find_map(|v| match v {
                Value::Table(t) => Some(t.clone()),
                _ => None,
            });
            let Some(list) = list else {
                return Err(mlua::Error::runtime(
                    "luaotfload.add_fallback wants a table of font specifications",
                ));
            };
            let chain: Vec<String> = list
                .sequence_values::<String>()
                .flatten()
                // A specification is `Family:option=value;`, and the family is
                // what can be resolved; `crate::typeset::fallback_chain` cuts
                // it the same way.
                .map(|s| s.split(':').next().unwrap_or_default().trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !chain.is_empty() {
                b.borrow_mut().fallbacks = Some(chain);
            }
            Ok(())
        })?,
    )?;
    Ok(t)
}

/// `luatexbase`: the registration calls a package makes at load time.
///
/// Enough for a document that says `luatexbase.provides_module{...}` or
/// registers a callback to run rather than stop on a nil index. A callback
/// NEVER fires — there is no node list to fire it on — so `add_to_callback`
/// records nothing and says so by returning the id it was given.
fn luatexbase_table(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set(
        "provides_module",
        lua.create_function(|_, _: Variadic<Value>| Ok(()))?,
    )?;
    t.set(
        "add_to_callback",
        lua.create_function(|_, args: Variadic<Value>| {
            Ok(args.first().and_then(|v| v.as_integer()).unwrap_or(0))
        })?,
    )?;
    t.set(
        "remove_from_callback",
        lua.create_function(|_, _: Variadic<Value>| Ok(()))?,
    )?;
    t.set(
        "callback_descriptions",
        lua.create_function(|lua, _: Variadic<Value>| lua.create_table())?,
    )?;
    t.set(
        "registernumber",
        lua.create_function(|_, _: Variadic<Value>| Ok(Value::Nil))?,
    )?;
    t.set(
        "module_warning",
        lua.create_function(|_, _: Variadic<Value>| Ok(()))?,
    )?;
    t.set(
        "module_error",
        lua.create_function(|_, (module, msg): (String, String)| {
            Err::<(), _>(mlua::Error::runtime(format!("{module}: {msg}")))
        })?,
    )?;
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dimension_string_reaches_the_scaled_points_tex_computes() {
        // The same figures `crate::dimen`'s own tests read off `tex -ini`.
        assert_eq!(parse_dimen("1pt"), Some(65536));
        assert_eq!(parse_dimen("1in"), Some(4736286));
        assert_eq!(parse_dimen("-0.5pt"), Some(-32768));
        assert_eq!(parse_dimen("5sp"), Some(5));
        // "infinite dimension units (fil...) are forbidden"
        assert_eq!(parse_dimen("1fil"), None);
        assert_eq!(parse_dimen("1em"), None);
    }

    #[test]
    fn escaping_is_what_luaescapestring_does() {
        assert_eq!(escape_string(r#"a\b"c'd"#), r#"a\\b\"c\'d"#);
        assert_eq!(escape_string("a\nb"), "a\\nb");
    }

    #[test]
    fn romannumeral_matches_tex() {
        assert_eq!(roman(1987), "mcmlxxxvii");
        assert_eq!(roman(4), "iv");
        assert_eq!(roman(0), "");
    }
}
