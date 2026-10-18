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
//! slots at run time and in `Engine::known` while lowering, so `tex.count[10]`
//! reads the lowering-time value, and a write goes to both: the frontend table
//! (so `\the\count10` inside an `\edef` agrees) and a `Cmd::SetCount` (so the
//! same register in the compiled program agrees). `\dimen` and `\skip` are the
//! same slot file at another offset (`crate::compiler`), so they come the same
//! way; `\toks` is a token list rather than a slot, and comes from
//! `Engine::toks`.
//!
//! ## The node library, and the lists the stomach builds
//!
//! `node` (`crate::lua::node`) is LuaTeX's, both faces of it: every type, every
//! field at luatex's index, the direct face as bare integers, attribute lists,
//! the properties table, and `node.hpack`/`node.vpack`/`node.dimensions` over
//! `crate::pack`'s port of tex.web §649-§679. The lists TeX builds are the
//! stomach's (`crate::stomach`), and they reach Lua through the callbacks: a
//! `callback.register` of a list callback fills the matching
//! `crate::node::hooks` point, so the stomach hands its list over while the
//! program runs and packs what comes back.
//!
//! ## What is NOT here, and why it is absent rather than stubbed
//!
//! - **The box registers and the current list from a chunk.** `tex.box`,
//!   `tex.getbox`, `tex.setbox`, `tex.getlist`, `tex.getnest`, `node.write`:
//!   the stomach holds them while the program runs, and a chunk runs while
//!   the program is lowered, before any of them exists. These say so rather
//!   than being absent, because "attempt to call a nil value" would not tell a
//!   document's author which of the two engines' worlds it had walked into.
//! - **The internal parameters texrs does not hold** -- `tex.prevdepth`,
//!   `tex.pagegoal`, LuaTeX's and e-TeX's additions to §236 -- refuse, naming
//!   where the value would live. The ones it does hold, `crate::intpar` and
//!   `crate::params`, are read and assigned (`crate::lua::tex`).
//!
//! ## The rest of LuaTeX's state
//!
//! [`build`] follows luastuff.c's `luainterpreter()`: `crate::lua::libs`
//! opens Lua's libraries and LuaTeX's C ones (LPeg, slnunicode, LuaFileSystem,
//! bit32, zlib, gzip -- compiled from the sources LuaTeX ships), `md5`, `sha2`
//! and the `string`/`os` extensions; then come `tex`, `token`, `node`, `texio`,
//! `kpse` (`crate::lua::kpse`, with the `require` searcher), `callback`, `lua`
//! (`crate::lua::lualib`) and `status`, and last `luatex-core.lua`'s
//! gap-filling. The engine a library function needs is lent to it for the
//! length of each call into Lua (`crate::lua::ctx`), so a function a package
//! keeps from one chunk works in the next, as a C function over TeX's globals
//! does in LuaTeX.

use crate::catcode::Cat;
use crate::expand::{Engine, Meaning, TexError};
use crate::ir::{Cmd, Num};
use crate::lexer::Lexer;
use crate::token::Token;
use mlua::{Lua, Table, Value, Variadic};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

pub mod callback;
mod ctx;
mod extraprimitives;
mod fio;
pub mod font;
mod kpse;
mod libs;
mod lualib;
mod node;
pub mod pdf;
mod tex;
mod token;

pub use font::{LuaChar, LuaFont};
pub use kpse::set_progname;

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
            | "luabytecode"
            | "luabytecodecall"
            | "luatexrevision"
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
    /// What `texio.write` and `texio.write_nl` wrote, which is terminal output
    /// rather than input.
    messages: Vec<Message>,
    /// The chain `luaotfload.add_fallback` named, when a chunk called it.
    fallbacks: Option<Vec<String>>,
}

/// One line of terminal output from `texio`.
struct Message {
    text: String,
    /// `write_nl`: on a fresh line, as tex.web's `print_nl`.
    nl: bool,
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

/// A warning from inside a library function, which goes where `texio.write_nl`
/// goes: the run's terminal output.
fn warn(text: &str) {
    let bridge = RUNTIME.with(|slot| slot.borrow().as_ref().map(|rt| Rc::clone(&rt.bridge)));
    if let Some(b) = bridge {
        say(
            &b,
            Message {
                text: text.to_string(),
                nl: true,
            },
        );
    }
}

/// Terminal output from Lua. Inside a chunk it is kept with the chunk's other
/// effects and written where the chunk stands in the program; from a callback
/// the program calls while it runs, it goes straight into the run's terminal
/// stream.
fn say(bridge: &Rc<RefCell<Bridge>>, m: Message) {
    if ctx::depth() > 0 {
        bridge.borrow_mut().messages.push(m);
        return;
    }
    // The run's stream keeps no column, so both are a message of their own,
    // which the stream separates from the next by the space `\message` puts.
    crate::runtime::tell(vec![crate::stomach::Said::Terminal(m.text)], false);
}

/// What Lua printed while a hook the lowerer calls ran with the engine lent
/// (`define_font`), handed to the engine's terminal stream where the lowerer
/// stands, as a chunk's messages are where the chunk stands.
fn notes_to_engine(eng: &mut Engine) {
    let bridge = RUNTIME.with(|slot| slot.borrow().as_ref().map(|rt| Rc::clone(&rt.bridge)));
    let Some(b) = bridge else { return };
    let said = std::mem::take(&mut b.borrow_mut().messages);
    for m in said {
        eng.terminal_note(&m.text);
    }
}

/// Run `f` with the document's Lua state, when there is one.
fn with_lua<T>(f: impl FnOnce(&Lua) -> T) -> Option<T> {
    let lua = RUNTIME.with(|slot| slot.borrow().as_ref().map(|rt| rt.lua.clone()))?;
    Some(f(&lua))
}

/// The attribute registers that are set, as (number, value) in number order:
/// the list `node.current_attr()` answers and a new node is given.
fn current_attributes() -> Vec<(i64, i64)> {
    RUNTIME.with(|slot| {
        let slot = slot.borrow();
        let Some(rt) = slot.as_ref() else {
            return Vec::new();
        };
        let b = rt.bridge.borrow();
        let mut out: Vec<(i64, i64)> = b
            .counts
            .iter()
            .filter(|(k, v)| **k >= ATTRIBUTE_BASE && **v != node::UNUSED_ATTRIBUTE)
            .map(|(k, v)| (*k - ATTRIBUTE_BASE, *v))
            .collect();
        out.sort();
        out
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
            let (name, chunk) = read_chunk(eng, lx)?;
            // Read out of the TEXT before anything runs, which is where it was
            // read from when nothing could: a chunk that fails half way through
            // for a reason of its own must not cost the document its fallback
            // chain, and the call is `luaotfload`'s rather than the engine's.
            let chain = crate::typeset::fallback_chain(&chunk);
            if !chain.is_empty() {
                fonts.fallbacks = chain;
            }
            let result = run(eng, lx, out, name.as_deref(), &chunk, Yield::ToInput);
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
            let (name, chunk) = read_chunk(eng, lx)?;
            run(eng, lx, out, name.as_deref(), &chunk, Yield::Discard)
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
        // `\luabytecode n` and `\luabytecodecall n` run bytecode register `n`
        // (llualib.c `luabytecodecall`), the first as an expandable command
        // and the second as an unexpandable one -- which here is the same thing.
        "luabytecode" | "luabytecodecall" => {
            let n = eng.scan_number_file(lx)?;
            let (lua, bridge) = runtime(eng)?;
            snapshot(eng, &bridge);
            let result = ctx::enter(eng, lx, || lualib::call(&lua, n));
            finish(eng, lx, out, &bridge, Yield::ToInput, result)
        }
        // `\luatexrevision` (a convert command): "0" for 1.24.0, the release
        // whose Lua interface this module implements.
        "luatexrevision" => {
            lx.push_back(&[Token::Char('0', Cat::Other)]);
            Ok(())
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
/// Before the group, textoken.c's `scan_lua_state` reads what names the chunk
/// in Lua's error messages: `name {...}`, an expanded name of its own, or a
/// register number whose `lua.name[n]` is set. Without either, the chunk is
/// `=[\directlua]`, which Lua prints as `[\directlua]:1:`.
fn read_chunk(eng: &mut Engine, lx: &mut Lexer) -> R<(Option<String>, String)> {
    let name = scan_lua_state(eng, lx)?;
    Ok((name, read_group(eng, lx)?))
}

/// A braced group, expanded and serialised.
fn read_group(eng: &mut Engine, lx: &mut Lexer) -> R<String> {
    let toks = eng.read_balanced_group(lx)?;
    // The ⟨general text⟩ is expanded as an `\edef` body is (`scan_toks(false,
    // true)`): macros, conditionals, `\the`, and the expandable primitives a
    // name stands for, such as iftex.sty's `\IFTEX@detokenize`.
    let expanded = eng.expand_edef_body(&toks)?;
    let escaped = apply_escapestring(eng, &expanded);
    // The chunk is the token list printed as `token_show` prints it, which
    // doubles a macro parameter character: `#` of catcode 6 reaches Lua as
    // `##`, in luatex as here. And luatex writes it with
    // `tokenlist_to_cstring(..., inhibit_par)`: the `\par` a blank line in the
    // argument makes is left out, so a chunk may have blank lines in it --
    // luatexbase.sty's do (measured: `\directlua{f([==[a<blank line>b]==])}`
    // passes Lua `a b`).
    let shown: Vec<Token> = escaped
        .iter()
        .filter(|t| !t.is_cs("par"))
        .flat_map(|t| match t {
            Token::Char(_, Cat::Param) => vec![*t, *t],
            _ => vec![*t],
        })
        .collect();
    Ok(eng.tokens_text(&shown))
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

/// `scan_lua_state` (textoken.c): spaces and `\relax` skipped, then nothing if a
/// brace follows, `name` and an expanded group, or a register number, which
/// names the chunk only when `lua.name` has a name for it.
fn scan_lua_state(eng: &mut Engine, lx: &mut Lexer) -> R<Option<String>> {
    let next = loop {
        match eng.take_file(lx) {
            Some(t) if t.is_space() || t.is_cs("relax") => continue,
            other => break other,
        }
    };
    let Some(tok) = next else {
        return Ok(None);
    };
    lx.push_back(std::slice::from_ref(&tok));
    if matches!(tok, Token::Char(_, Cat::BeginGroup)) {
        return Ok(None);
    }
    if eng.scan_keyword(lx, "name", false)? {
        return read_group(eng, lx).map(Some);
    }
    let n = eng.scan_number_file(lx)?;
    Ok(lualib::chunk_name(n))
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
    name: Option<&str>,
    chunk: &str,
    yield_to: Yield,
) -> R<()> {
    let (lua, bridge) = runtime(eng)?;
    snapshot(eng, &bridge);
    let default = match yield_to {
        Yield::ToInput => "=[\\directlua]",
        Yield::Discard => "=[\\latelua]",
    };
    let name = name.unwrap_or(default);
    let result = ctx::enter(eng, lx, || lua.load(chunk).set_name(name).exec());
    finish(eng, lx, out, &bridge, yield_to, result)
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
    snapshot(eng, &bridge);
    // The engine is lent here too, and that is the whole point of the manual's
    // second worked example: `\def\mymacro{\directlua{mymacro()}}` with
    // `local d = token.scan_dimen()` inside `mymacro` reads `\mymacro 12pt` by
    // fetching from the input rather than by being handed a string.
    let result = ctx::enter(eng, lx, || {
        functions_table(&lua)
            .and_then(|t| t.get::<mlua::Function>(n))
            .and_then(|f| f.call::<()>(n))
    });
    finish(eng, lx, out, &bridge, yield_to, result)
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
        eng.known.set(reg, Some(v));
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
    for m in std::mem::take(&mut b.messages) {
        let op = match m.nl {
            // `print_nl` is a line end and then the text. The message stream
            // does not track the terminal's column, so the line end is always
            // written; a report is glued to what precedes it, as `print_nl` is, and
            // marked as a DIAGNOSTIC so it is not counted as an error.
            true => crate::ir::MsgOp::Report(format!("{}\n{}", crate::expand::DIAGNOSTIC, m.text)),
            false => crate::ir::MsgOp::Text(m.text),
        };
        out.push(Cmd::Message(vec![op]));
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
fn snapshot(eng: &Engine, bridge: &Rc<RefCell<Bridge>>) {
    let mut b = bridge.borrow_mut();
    b.prints.clear();
    b.error = None;
    // Everything the lowerer has emitted so far has been followed into
    // `Engine::known`, so the chunk reads the value the program will hold
    // when it reaches this point -- `\count10=20` then `tex.count[10]` is 20,
    // the manual's own worked example.
    b.counts = eng.known.values();
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

/// A job name given outright -- by [`set_jobname`] or `-jobname` -- rather than
/// taken from the input file's name.
///
/// `\jobname` is expanded while LOWERING, so it is baked into the bytecode, and
/// the bytecode cache is keyed on the file. A name that is not the file's own
/// has to be part of that key, or a run with `-jobname=b` is served the chunk a
/// run with `-jobname=a` compiled.
pub fn explicit_jobname() -> Option<String> {
    if let Some(j) = JOBNAME.with(|j| j.borrow().clone()) {
        return Some(j);
    }
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if let Some(v) = a
            .strip_prefix("-jobname=")
            .or_else(|| a.strip_prefix("--jobname="))
        {
            return Some(v.to_string());
        }
        if a == "-jobname" || a == "--jobname" {
            return args.next();
        }
    }
    None
}

/// The job name, which nothing threads into the lowerer.
///
/// `-jobname` and the input file are on the command line, and that is where
/// this reads them when nothing has called [`set_jobname`]. tex's own default
/// for input that came from the terminal rather than a file is `texput`.
pub(crate) fn jobname() -> String {
    if let Some(j) = explicit_jobname() {
        return j;
    }
    let mut file = None;
    for a in std::env::args().skip(1) {
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

/// `lua.get_functions_table()`'s table, which `\luafunction n` indexes; it
/// lives in the registry, as llualib.c keeps it.
fn functions_table(lua: &Lua) -> mlua::Result<Table> {
    lua.named_registry_value::<Table>(lualib::FUNCTIONS)
}

/// Build the state the way `luainterpreter()` does (luatexdir/lua/luastuff.c):
/// Lua's libraries and LuaTeX's extra C libraries ([`libs`]), then the TeX
/// libraries in its order -- `tex`, `token`, `node`, `texio`, `kpse`,
/// `callback`, `lua`, `status` -- then the empty `texconfig` table, the kpse
/// `require` searcher (luainit.c's `setup_lua_path`) and last the gap-filling
/// half of `luatex-core.lua`.
fn build(owner: usize) -> mlua::Result<Runtime> {
    let lua = libs::new_state()?;
    callback::reset();
    let bridge = Rc::new(RefCell::new(Bridge::default()));
    // One arena per document, alongside the bridge and for the same reason: a
    // node a chunk made must still be there for the next chunk, and must be
    // gone by the next document.
    let arena = Rc::new(RefCell::new(node::Arena::default()));
    let globals = lua.globals();
    let register = |name: &str, t: Table| -> mlua::Result<()> {
        let loaded: Table = globals.get::<Table>("package")?.get("loaded")?;
        loaded.set(name, t.clone())?;
        globals.set(name, t)
    };
    register("tex", tex_table(&lua, &bridge, &arena)?)?;
    register("token", token::table(&lua, &arena)?)?;
    register("node", node::table(&lua, &arena)?)?;
    register("texio", texio_table(&lua, &bridge)?)?;
    register("kpse", kpse::table(&lua)?)?;
    register("callback", callback::table(&lua, &arena)?)?;
    register("lua", lualib::table(&lua)?)?;
    register("status", status_table(&lua)?)?;
    register("font", font::table(&lua)?)?;
    register("pdf", pdf::table(&lua, &bridge)?)?;
    register(
        "pdfe",
        unported(
            &lua,
            "pdfe",
            PDFE,
            "reads PDF files through pplib, which is not ported",
        )?,
    )?;
    globals.set("texconfig", lua.create_table()?)?;
    kpse::install_searcher(&lua)?;
    libs::core(&lua)?;
    globals.set("luatexbase", luatexbase_table(&lua)?)?;
    globals.set("luaotfload", luaotfload_table(&lua, &bridge)?)?;
    Ok(Runtime { lua, bridge, owner })
}

/// `pdfe`'s functions, as luatex 1.24.0 lists them.
const PDFE: &[&str] = &[
    "arraytotable",
    "close",
    "closestream",
    "dictionarytotable",
    "getarray",
    "getboolean",
    "getbox",
    "getcatalog",
    "getdictionary",
    "getfromarray",
    "getfromdictionary",
    "getfromreference",
    "getfromstream",
    "getinfo",
    "getinteger",
    "getmemoryusage",
    "getname",
    "getnofobjects",
    "getnofpages",
    "getnumber",
    "getpage",
    "getpages",
    "getsize",
    "getstatus",
    "getstream",
    "getstring",
    "gettrailer",
    "getversion",
    "new",
    "open",
    "openstream",
    "pagestotable",
    "readfromstream",
    "readwholestream",
    "type",
    "unencrypt",
];

/// A library whose every function says it is not ported. For one a package
/// indexes unconditionally when it loads -- the luaotfload font loader takes
/// `local openpdf = pdfe.new` at load time, for the colour fonts that carry
/// PDF glyphs -- so that the load goes through and a document that actually
/// uses it is told why it stopped, rather than an index of nil at load.
fn unported(
    lua: &Lua,
    lib: &'static str,
    names: &[&'static str],
    why: &'static str,
) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    for name in names {
        let name: &'static str = name;
        t.set(
            name,
            lua.create_function(move |_, _: Variadic<Value>| {
                Err::<(), _>(mlua::Error::runtime(format!("{lib}.{name}: {lib} {why}")))
            })?,
        )?;
    }
    Ok(t)
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
fn tex_table(
    lua: &Lua,
    bridge: &Rc<RefCell<Bridge>>,
    arena: &Rc<RefCell<node::Arena>>,
) -> mlua::Result<Table> {
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
                Ok(register_value(&b, kind, reg))
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
    // That is the whole difference between `tex.skip` and `tex.getglue`, which
    // is the same registers as bare numbers. It refused while there was no node
    // interface; `crate::lua::node` carries a `glue_spec` faithfully -- five
    // integers, no subtype and no `prev`, exactly as `node.fields(39)` reports
    // in luatex 1.24.0 -- so the register half of it answers now.
    let b = Rc::clone(bridge);
    let a = Rc::clone(arena);
    tex.set(
        "getskip",
        lua.create_function(move |lua, key: Value| {
            let base = {
                let b = b.borrow();
                resolve(&b, Kind::Glue, &key)?
            };
            let glue = glue_get(&b.borrow(), base);
            node::spec_node(lua, &a, glue)
        })?,
    )?;
    let b = Rc::clone(bridge);
    let a = Rc::clone(arena);
    tex.set(
        "setskip",
        lua.create_function(move |_, args: Variadic<Value>| {
            let args = drop_global(&args);
            let [key, value] = &args[..] else {
                return Err(mlua::Error::runtime(
                    "tex.setskip wants a register and a glue_spec node",
                ));
            };
            let glue = node::spec_values(&a, value)?;
            let mut b = b.borrow_mut();
            let base = resolve(&b, Kind::Glue, key)?;
            glue_set(&mut b, base, glue)
        })?,
    )?;
    // The `\muskip` half of the same interface. `\muskip` is a register file of
    // its own now (`crate::compiler::MUSKIP_BASE`, scanned in math units by
    // `Engine::scan_muglue`), so these read and write it exactly as the `\skip`
    // pair above do -- a `glue_spec` node for `tex.muskip`/`getmuskip`, five
    // bare numbers for `tex.getmuglue`.
    let b = Rc::clone(bridge);
    let a = Rc::clone(arena);
    tex.set(
        "getmuskip",
        lua.create_function(move |lua, key: Value| {
            let base = {
                let b = b.borrow();
                resolve(&b, Kind::MuGlue, &key)?
            };
            let glue = glue_get(&b.borrow(), base);
            node::spec_node(lua, &a, glue)
        })?,
    )?;
    let b = Rc::clone(bridge);
    let a = Rc::clone(arena);
    tex.set(
        "setmuskip",
        lua.create_function(move |_, args: Variadic<Value>| {
            let args = drop_global(&args);
            let [key, value] = &args[..] else {
                return Err(mlua::Error::runtime(
                    "tex.setmuskip wants a register and a glue_spec node",
                ));
            };
            let glue = node::spec_values(&a, value)?;
            let mut b = b.borrow_mut();
            let base = resolve(&b, Kind::MuGlue, key)?;
            glue_set(&mut b, base, glue)
        })?,
    )?;
    let b = Rc::clone(bridge);
    tex.set(
        "getmuglue",
        lua.create_function(move |_, args: Variadic<Value>| {
            let key = args.first().cloned().unwrap_or(Value::Nil);
            let b = b.borrow();
            let base = resolve(&b, Kind::MuGlue, &key)?;
            let (w, st, sh, sto, sho) = glue_get(&b, base);
            match args.get(1) {
                Some(Value::Boolean(false)) => Ok(Variadic::from_iter([Value::Integer(w)])),
                _ => Ok(Variadic::from_iter(
                    [w, st, sh, sto, sho].map(Value::Integer),
                )),
            }
        })?,
    )?;
    let b = Rc::clone(bridge);
    tex.set(
        "setmuglue",
        lua.create_function(move |_, args: Variadic<Value>| {
            let args = drop_global(&args);
            let Some(key) = args.first() else {
                return Err(mlua::Error::runtime("tex.setmuglue wants a register"));
            };
            let n = |i: usize| args.get(i).and_then(|v| v.as_integer()).unwrap_or(0);
            let mut b = b.borrow_mut();
            let base = resolve(&b, Kind::MuGlue, key)?;
            glue_set(&mut b, base, (n(1), n(2), n(3), n(4), n(5)))?;
            Ok(())
        })?,
    )?;
    tex.set("muglue", register_table(lua, bridge, Kind::MuGlue)?)?;
    // `tex.muskip[n]`, the table spelling of the pair above.
    let muskip = lua.create_table()?;
    let mt = lua.create_table()?;
    let b = Rc::clone(bridge);
    let a = Rc::clone(arena);
    mt.set(
        "__index",
        lua.create_function(move |lua, (_, key): (Table, Value)| {
            let base = {
                let b = b.borrow();
                resolve(&b, Kind::MuGlue, &key)?
            };
            let glue = glue_get(&b.borrow(), base);
            node::spec_node(lua, &a, glue)
        })?,
    )?;
    let b = Rc::clone(bridge);
    let a = Rc::clone(arena);
    mt.set(
        "__newindex",
        lua.create_function(move |_, (_, key, value): (Table, Value, Value)| {
            let glue = node::spec_values(&a, &value)?;
            let mut b = b.borrow_mut();
            let base = resolve(&b, Kind::MuGlue, &key)?;
            glue_set(&mut b, base, glue)
        })?,
    )?;
    muskip.set_metatable(Some(mt))?;
    tex.set("muskip", muskip)?;
    // A box register holds a NODE LIST, and the lists are the stomach's: they
    // exist while the program runs, and a chunk runs while it is lowered, before
    // any box has been built. These say so instead of being absent, because
    // "attempt to call a nil value" would not tell a document's author which of
    // the two engines' worlds it had walked into.
    for name in [
        "getbox", "setbox", "splitbox", "getlist", "setlist", "getnest", "getmath", "setmath",
    ] {
        tex.set(
            name,
            lua.create_function(move |_, _: Variadic<Value>| {
                Err::<(), _>(mlua::Error::runtime(format!(
                    "tex.{name} works on the stomach's node lists, which exist while the program runs and not while a chunk is lowered"
                )))
            })?,
        )?;
    }
    // `tex.skip[n]` is `tex.getskip(n)` through a table, as every register
    // family in the manual is offered both ways.
    let skip = lua.create_table()?;
    let mt = lua.create_table()?;
    let b = Rc::clone(bridge);
    let a = Rc::clone(arena);
    mt.set(
        "__index",
        lua.create_function(move |lua, (_, key): (Table, Value)| {
            let base = {
                let b = b.borrow();
                resolve(&b, Kind::Glue, &key)?
            };
            let glue = glue_get(&b.borrow(), base);
            node::spec_node(lua, &a, glue)
        })?,
    )?;
    let b = Rc::clone(bridge);
    let a = Rc::clone(arena);
    mt.set(
        "__newindex",
        lua.create_function(move |_, (_, key, value): (Table, Value, Value)| {
            let glue = node::spec_values(&a, &value)?;
            let mut b = b.borrow_mut();
            let base = resolve(&b, Kind::Glue, &key)?;
            glue_set(&mut b, base, glue)
        })?,
    )?;
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
        ("ismuskip", Kind::MuGlue),
        ("ismuglue", Kind::MuGlue),
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
                if let Ok(Some((_, n))) =
                    ctx::with("tex.is", |eng, _| Ok(eng.pooled_register(slot)))
                {
                    return Ok(Value::Integer(n));
                }
                let base = match kind {
                    Kind::Count | Kind::Toks => 0,
                    Kind::Dimen => crate::compiler::DIMEN_BASE,
                    Kind::Glue => crate::compiler::SKIP_BASE,
                    Kind::MuGlue => crate::compiler::MUSKIP_BASE,
                    Kind::Attribute => ATTRIBUTE_BASE,
                };
                let stride = match kind {
                    Kind::Glue | Kind::MuGlue => crate::compiler::SKIP_STRIDE,
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
    // The version of the Lua interface this is (see `status_table`).
    tex.set("luatexversion", 124)?;
    tex.set("luatexrevision", "0")?;
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
    // `tex.runtoks(f)` and `tex.runtoks(register)`: the tokens the function
    // puts back (`token.put_next`, `tex.print`), or the register's list, run
    // NOW by a nested main control (ltexlib.c `tex_run_toks`,
    // `local_control`), and Lua resumes when they are used up. The nested
    // control is the engine's (`Engine::local_control`): definitions, `\let`,
    // the code tables and internal integers take effect, and what only the
    // program could do refuses by name.
    let b = Rc::clone(bridge);
    tex.set(
        "runtoks",
        lua.create_function(
            move |_, (what, _force, _grouped): (Value, Option<bool>, Option<bool>)| {
                let stop = ctx::with("tex.runtoks", |_, lx| Ok(lx.pending.len()))?;
                match what {
                    Value::Function(f) => {
                        let printed_before = b.borrow().prints.len();
                        f.call::<()>(())?;
                        let prints: Vec<Print> =
                            b.borrow_mut().prints.drain(printed_before..).collect();
                        ctx::with("tex.runtoks", |eng, lx| {
                            let mut toks = Vec::new();
                            for p in &prints {
                                toks.extend(tokenize(&p.text, p.cats, p.line, &eng.cats));
                            }
                            lx.push_back(&toks);
                            Ok(())
                        })?;
                    }
                    Value::Integer(_) | Value::String(_) => {
                        let reg = match &what {
                            Value::Integer(n) => *n,
                            Value::String(s) => {
                                let name = s.to_str()?.to_string();
                                b.borrow().toks_names.get(&name).copied().ok_or_else(|| {
                                    mlua::Error::runtime(format!(
                                        "tex.runtoks: no token register named {name}"
                                    ))
                                })?
                            }
                            _ => unreachable!(),
                        };
                        ctx::with("tex.runtoks", |eng, lx| {
                            let list = eng.toks.get(&reg).cloned().unwrap_or_default();
                            lx.push_back(&list);
                            Ok(())
                        })?;
                    }
                    _ => return Err(mlua::Error::runtime("invalid argument to tex.runtoks")),
                }
                ctx::with("tex.runtoks", |eng, lx| {
                    eng.local_control(lx, stop)
                        .map_err(|e| mlua::Error::runtime(e.0))
                })
            },
        )?,
    )?;
    tex.set(
        "romannumeral",
        lua.create_function(|_, n: i64| Ok(roman(n)))?,
    )?;
    // `tex.extraprimitives(origin...)`: the names of the primitives made by
    // those origins (all of luatex's but TeX82's and the core's when none is
    // given), in primitive-table order.
    tex.set(
        "extraprimitives",
        lua.create_function(|lua, args: Variadic<Value>| {
            use extraprimitives::{Origin, PRIMITIVES};
            let mut mask = Vec::new();
            if args.is_empty() {
                mask = vec![Origin::Etex, Origin::Luatex];
            }
            for a in args.iter() {
                if let Value::String(s) = a {
                    match &*s.to_str()? {
                        "etex" => mask.push(Origin::Etex),
                        "tex" => mask.push(Origin::Tex),
                        "core" => mask.push(Origin::Core),
                        "luatex" => mask.push(Origin::Luatex),
                        _ => {}
                    }
                }
            }
            lua.create_sequence_from(
                PRIMITIVES
                    .iter()
                    .filter(|(origin, _)| mask.contains(origin))
                    .map(|(_, name)| *name),
            )
        })?,
    )?;
    // `tex.enableprimitives(prefix, names)`: each name that is a primitive is
    // defined as `prefix` and the name -- or as the name itself when `prefix`
    // already begins it -- unless that control sequence means something
    // already (ltexlib.c's `tex_enableprimitives`, `primitive_def`, which is
    // global). Every primitive texrs has is defined from the start, so with an
    // empty prefix this changes only a name a document undefined.
    tex.set(
        "enableprimitives",
        lua.create_function(|_, args: Variadic<Value>| {
            let [Value::String(prefix), names] = &args[..] else {
                return Err(mlua::Error::runtime("wrong number of arguments"));
            };
            let Value::Table(names) = names else {
                return Err(mlua::Error::runtime(
                    "Expected an array of names as second argument",
                ));
            };
            let prefix = prefix.to_str()?.to_string();
            let mut prims = Vec::new();
            for v in names.sequence_values::<Value>() {
                let Value::String(s) = v? else { break };
                prims.push(s.to_str()?.to_string());
            }
            ctx::with("tex.enableprimitives", |eng, _| {
                for prim in prims {
                    if !token::is_primitive_name(&prim) {
                        continue;
                    }
                    let name = match prim.starts_with(&prefix) {
                        true => prim.clone(),
                        false => format!("{prefix}{prim}"),
                    };
                    if token::means_something(eng, &name) {
                        continue;
                    }
                    eng.meanings.insert(
                        crate::token::CsId::intern(&name),
                        Meaning::Primitive(crate::token::CsId::intern(&prim)),
                    );
                }
                Ok(())
            })
        })?,
    )?;

    // The manual's "Internal parameter values" (`tex.tolerance`, `tex.get`,
    // `tex.set`), the character-code tables and the hash: `crate::lua::tex`.
    tex::install(lua, &tex, bridge, arena)?;
    Ok(tex)
}

/// Why an internal parameter cannot be answered, or `None` if the name is not
/// one of the manual's parameters at all.
///
/// The lists are the manual's own (1.24.0, "Internal parameter values"), split
/// by where the value really lives in texrs. Being specific is the point: "not
/// implemented" would leave a chunk's author guessing whether the parameter is
/// coming, and each of these three answers is a different fact about the engine.
fn parameter_reason(key: &Value) -> mlua::Result<Option<String>> {
    let Value::String(s) = key else {
        return Ok(None);
    };
    Ok(parameter_reason_text(&s.to_str()?))
}

/// [`parameter_reason`], by name.
fn parameter_reason_text(name: &str) -> Option<String> {
    let name = name.to_string();
    // The page geometry. `\hsize` is a register a document can assign
    // (`crate::params`), but `crate::typeset::Layout` holds the measure, the leading, the
    // indent and the paragraph spacing, decided from the class options and
    // `geometry`'s. A chunk cannot read it because it is not decided where the
    // chunk stands -- lowering runs before the page is laid out at all.
    const GEOMETRY: &[&str] = &[
        "hsize",
        "vsize",
        "parindent",
        "hoffset",
        "voffset",
        "pagewidth",
        "pageheight",
        "pagetopoffset",
        "pagebottomoffset",
        "pageleftoffset",
        "pagerightoffset",
        "baselineskip",
        "lineskip",
        "lineskiplimit",
        "parskip",
        "leftskip",
        "rightskip",
        "parfillskip",
        "topskip",
        "splittopskip",
        "spaceskip",
        "xspaceskip",
        "hangindent",
        "hangafter",
    ];
    // The line breaker's. `\tolerance`, `\pretolerance` and the demerit weights
    // are constants in `crate::linebreak` rather than registers a document can
    // set -- written down in BUGS.md, and this is the same fact seen from Lua.
    const BREAKER: &[&str] = &[
        "tolerance",
        "pretolerance",
        "hbadness",
        "vbadness",
        "hfuzz",
        "vfuzz",
        "adjdemerits",
        "linepenalty",
        "doublehyphendemerits",
        "finalhyphendemerits",
        "exhyphenpenalty",
        "hyphenpenalty",
        "interlinepenalty",
        "clubpenalty",
        "widowpenalty",
        "displaywidowpenalty",
        "brokenpenalty",
        "looseness",
        "emergencystretch",
        "lefthyphenmin",
        "righthyphenmin",
        "uchyph",
        "lastlinefit",
        "language",
        "overfullrule",
    ];
    // Everything else on the manual's list: the stomach's state, the output
    // routine's, the token-list parameters, the maths parameters, the tracing
    // switches and the date. The ones that are §236 integer parameters are
    // held in `crate::intpar` for the TeX side; none is in the chunk's bridge.
    const REST: &[&str] = &[
        "abovedisplayshortskip",
        "abovedisplayskip",
        "belowdisplayshortskip",
        "belowdisplayskip",
        "binoppenalty",
        "bodydir",
        "boxmaxdepth",
        "catcodetable",
        "clubpenalties",
        "day",
        "deadcycles",
        "defaulthyphenchar",
        "defaultskewchar",
        "delimiterfactor",
        "delimitershortfall",
        "displayindent",
        "displaywidowpenalties",
        "displaywidth",
        "endlinechar",
        "errhelp",
        "errorcontextlines",
        "escapechar",
        "everycr",
        "everydisplay",
        "everyeof",
        "everyhbox",
        "everyjob",
        "everymath",
        "everypar",
        "everyvbox",
        "fam",
        "floatingpenalty",
        "globaldefs",
        "holdinginserts",
        "insertpenalties",
        "interlinepenalties",
        "lastkern",
        "lastnodetype",
        "lastpenalty",
        "lastskip",
        "localbrokenpenalty",
        "localinterlinepenalty",
        "mag",
        "mathdir",
        "mathsurround",
        "maxdeadcycles",
        "maxdepth",
        "medmuskip",
        "month",
        "nest",
        "newlinechar",
        "nulldelimiterspace",
        "output",
        "outputpenalty",
        "pagebottomoffset",
        "pagedepth",
        "pagedir",
        "pagefilllstretch",
        "pagefillstretch",
        "pagefilstretch",
        "pagegoal",
        "pageshrink",
        "pagestretch",
        "pagetotal",
        "pardir",
        "parshape",
        "pausing",
        "postdisplaypenalty",
        "predisplaydirection",
        "predisplaypenalty",
        "predisplaysize",
        "prevdepth",
        "prevgraf",
        "relpenalty",
        "savinghyphcodes",
        "savingvdiscards",
        "scriptspace",
        "showboxbreadth",
        "showboxdepth",
        "spacefactor",
        "splitmaxdepth",
        "tabskip",
        "textdir",
        "thickmuskip",
        "thinmuskip",
        "time",
        "tracingassigns",
        "tracingcommands",
        "tracinggroups",
        "tracingifs",
        "tracinglostchars",
        "tracingmacros",
        "tracingnesting",
        "tracingonline",
        "tracingoutput",
        "tracingpages",
        "tracingparagraphs",
        "tracingrestores",
        "tracingscantokens",
        "tracingstats",
        "widowpenalties",
        "year",
    ];
    let name = name.as_str();
    // A §236 integer parameter, a §247 dimension, §224 glue or §230 token
    // parameter IS held (`crate::intpar`, `crate::params`) and `crate::lua::tex`
    // answers it; what is refused here is the rest of the manual's list.
    if crate::intpar::index(name).is_some()
        || crate::params::register_slot(name).is_some()
        || crate::params::toks_register(name).is_some()
    {
        return None;
    }
    if GEOMETRY.contains(&name) {
        return Some(format!(
            "tex.{name} is one of LuaTeX's internal parameters, and the page \
             geometry lives in crate::typeset::Layout, decided from the class and \
             geometry options rather than read from what the document assigned, \
             and it is not decided yet where a chunk stands"
        ));
    }
    if BREAKER.contains(&name) {
        return Some(format!(
            "tex.{name} is one of LuaTeX's internal parameters, and the line \
             breaker uses a constant in crate::linebreak rather than reading it"
        ));
    }
    if REST.contains(&name) {
        return Some(format!(
            "tex.{name} is one of LuaTeX's internal parameters, and texrs does not \
             hold it at all: it belongs to the stomach, the output routine or the \
             maths, none of which is on the path a run takes"
        ));
    }
    None
}

/// Which register file a name or number is an index into.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Count,
    Dimen,
    /// `\skip`. Two spellings reach it: `tex.glue`/`tex.getglue`/`tex.setglue`
    /// as five bare numbers, and `tex.skip`/`tex.getskip`/`tex.setskip` through
    /// the `glue_spec` NODE the manual describes — the same registers either
    /// way, and both are implemented.
    Glue,
    /// `\muskip`, which is a glue register in the second glue file: the same
    /// four slots, scanned in MATH units (`tex.web` §455, and `Engine::scan_muglue`).
    MuGlue,
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
            Kind::MuGlue => "muskip",
            Kind::Toks => "toks",
            Kind::Attribute => "attribute",
        }
    }
}

/// A count, dimension or attribute register's value. An attribute that was
/// never set is LuaTeX's `UNUSED_ATTRIBUTE`, -0x7FFFFFFF, which
/// `tex.attribute[0]` answers in luatex 1.24.0 before anything sets it.
fn register_value(b: &Bridge, kind: Kind, reg: i64) -> i64 {
    match kind {
        Kind::Attribute => b
            .counts
            .get(&reg)
            .copied()
            .unwrap_or(node::UNUSED_ATTRIBUTE),
        _ => slot_value(b, reg),
    }
}

/// A slot's value as the chunk sees it: what the chunk wrote, else what the
/// engine knows (the snapshot leaves out the zeros, and the slots never
/// assigned, whose initial value is `\pxdimen`'s 65781sp or a page offset's
/// inch for some), else the initial value when no chunk is running.
fn slot_value(b: &Bridge, slot: i64) -> i64 {
    if let Some(v) = b.counts.get(&slot) {
        return *v;
    }
    if ctx::depth() > 0 {
        if let Ok(Some(v)) = ctx::with("tex", |eng, _| Ok(eng.known.get(slot))) {
            return v;
        }
    }
    crate::params::initial_value(slot)
}

/// Attribute registers, kept clear of the count and dimension slots.
const ATTRIBUTE_BASE: i64 = 1 << 20;

/// How many registers of each kind there are.
///
/// LuaTeX's 65,536 of each kind (the manual, "Registers"), as the engine has
/// them: 0..255 at fixed slots and the rest from a pool per file. An index past
/// the end is refused rather than turned into a slot, because the files are
/// ranges of ONE slot file and a slot computed for a register that is not
/// there would be a write to whichever neighbour it landed in.
const REGISTERS: i64 = 65536;

/// A register index from a Lua key: a number, or a `\countdef`-style name.
fn resolve(b: &Bridge, kind: Kind, key: &Value) -> mlua::Result<i64> {
    let base = match kind {
        Kind::Count | Kind::Toks => 0,
        Kind::Dimen => crate::compiler::DIMEN_BASE,
        Kind::Glue => crate::compiler::SKIP_BASE,
        Kind::MuGlue => crate::compiler::MUSKIP_BASE,
        Kind::Attribute => ATTRIBUTE_BASE,
    };
    // A glue is four slots — natural, stretch, shrink, and the packed orders —
    // so register n starts at four times n. `crate::lower`'s own arithmetic.
    let stride = match kind {
        Kind::Glue | Kind::MuGlue => crate::compiler::SKIP_STRIDE,
        _ => 1,
    };
    if let Some(n) = key.as_integer() {
        // An attribute is storage this module keeps for the chunk and a token
        // register is a map rather than a slot range, so neither can collide
        // with a neighbour and neither is bounded by the slot file.
        let file = match kind {
            Kind::Count => Some(crate::compiler::RegFile::Count),
            Kind::Dimen => Some(crate::compiler::RegFile::Dimen),
            Kind::Glue => Some(crate::compiler::RegFile::Skip),
            Kind::MuGlue => Some(crate::compiler::RegFile::MuSkip),
            _ => None,
        };
        let Some(file) = file else {
            return Ok(base + n * stride);
        };
        if !(0..REGISTERS).contains(&n) {
            return Err(mlua::Error::runtime(format!(
                "\\{}{n} is out of range: there are {REGISTERS} {} registers",
                kind.primitive(),
                kind.primitive(),
            )));
        }
        if n < 256 {
            return Ok(base + n * stride);
        }
        // A register past 255 has a slot from its file's pool, given the
        // first time it is named (`Engine::register_address`).
        return ctx::with("tex", |eng, _| {
            eng.register_address(file, n)
                .map_err(|e| mlua::Error::runtime(e.0))
        });
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
    let at = |i: i64| slot_value(b, base + i);
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
                Kind::Glue | Kind::MuGlue => Ok(Value::Integer(glue_get(&b, reg).0)),
                _ => Ok(Value::Integer(register_value(&b, kind, reg))),
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
                Kind::Glue | Kind::MuGlue => {
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

/// `texio`: terminal output, not input (luatexdir/lua/ltexiolib.c).
///
/// texrs's terminal output IS its message stream, so that is where these go:
/// `write_nl` as tex.web's `print_nl` -- a fresh line unless the terminal is at
/// the start of one -- and `write` as a message after whatever came before it.
///
/// The selector is ltexiolib.c's `get_selector_value`, quirks included. It is
/// looked for only when there is more than one argument, and then the FIRST
/// argument is always taken as one: `"term"`, `"log"`, `"term and log"`, and
/// any other string counts as `"term and log"` -- so `texio.write_nl("a", "b")`
/// prints only `b`, in luatex as here. A number names a `\write` stream.
/// What goes to the log ALONE is not printed: texrs writes no transcript, and a
/// `"log"` line on the terminal is exactly what luatex never shows there.
fn texio_table(lua: &Lua, bridge: &Rc<RefCell<Bridge>>) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    for (name, nl) in [("write", false), ("write_nl", true)] {
        let b = Rc::clone(bridge);
        t.set(
            name,
            lua.create_function(move |lua, args: Variadic<Value>| {
                let printable = |v: &Value| {
                    matches!(v, Value::String(_) | Value::Integer(_) | Value::Number(_))
                };
                if args.is_empty() || !printable(&args[args.len() - 1]) {
                    return Err(mlua::Error::runtime("no string to print"));
                }
                let mut rest = &args[..];
                let mut to_terminal = true;
                if args.len() > 1 {
                    match &args[0] {
                        Value::String(s) => to_terminal = &*s.to_str()? != "log",
                        Value::Integer(_) | Value::Number(_) => {}
                        _ => {
                            return Err(mlua::Error::runtime(
                                "first argument is not 'term and log', 'term', 'log' or a number",
                            ))
                        }
                    }
                    rest = &args[1..];
                }
                let mut text = String::new();
                for v in rest {
                    if !printable(v) {
                        return Err(mlua::Error::runtime("argument is not a string"));
                    }
                    text.push_str(&lua_tostring(lua, v)?);
                }
                if to_terminal {
                    say(&b, Message { text, nl });
                }
                Ok(())
            })?,
        )?;
    }
    // `setescape` switches how control characters are shown on the terminal,
    // which texrs prints as they are; `closeinput` ends the current input
    // file, which a chunk running at lowering time cannot reach.
    t.set("setescape", lua.create_function(|_, _: Value| Ok(()))?)?;
    t.set(
        "closeinput",
        lua.create_function(|_, ()| {
            Err::<(), _>(mlua::Error::runtime(
                "texio.closeinput ends the input file TeX is reading, which texrs's mouth \
                 does not let a chunk close",
            ))
        })?,
    )?;
    Ok(t)
}

/// `status`: what the run is (luatexdir/lua/lstatslib.c).
///
/// LuaTeX's `status` holds three functions -- `list`, `resetmessages`,
/// `setexitcode` -- and answers every field through `__index`, computing it
/// when asked. The fields are answered here in three groups:
///
/// - **The interface version.** `luatex_version` 124, `luatex_revision` "0",
///   `luatex_engine` "luatex", `development_id` 7724: the LuaTeX release whose
///   Lua interface this module implements, measured off `luatex` 1.24.0 (TeX
///   Live 2026). They are what a package tests -- ltluatex.lua takes
///   `token.create` only when `tex.luatexversion > 81`, luaotfload refuses to
///   load below its minimum -- so a different answer would refuse the very
///   packages this interface exists to run. `banner` is texrs's own, because
///   it is the one field that says which program is running.
/// - **The run's options.** `ini_version` false (texrs has no format to
///   build), `shell_escape` 0 (no shell commands are run, see
///   `crate::lua::libs`), `safer_option` 0, `luadebug_option` 0,
///   `kpse_used` 1, `output_active` false.
/// - **The run's position**, live: `linenumber`, when a chunk is running;
///   `log_name` from the job name. `filename` is absent: the lexer holds the
///   text it reads, not the name of the file it came from.
///
/// TeX's memory statistics -- `str_ptr`, `pool_size`, `var_used`, `cs_count`
/// and the rest of lstatslib.c's table -- describe luatex's arrays, which texrs
/// does not have; they answer nil rather than a number nothing measured.
fn status_table(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    let fields = |lua: &Lua| -> mlua::Result<Table> {
        let f = lua.create_table()?;
        f.set(
            "banner",
            format!("This is texrs, Version {}", env!("CARGO_PKG_VERSION")),
        )?;
        f.set("luatex_version", 124)?;
        f.set("luatex_revision", "0")?;
        f.set("luatex_engine", "luatex")?;
        f.set("development_id", 7724)?;
        f.set("luatex_hashtype", "lua")?;
        f.set("luatex_hashchars", 6)?;
        f.set("ini_version", false)?;
        f.set("shell_escape", libs::SHELL_ESCAPE)?;
        f.set("safer_option", 0)?;
        f.set("luadebug_option", 0)?;
        f.set("kpse_used", 1)?;
        f.set("output_active", false)?;
        f.set("jobname", jobname())?;
        f.set("log_name", format!("{}.log", jobname()))?;
        f.set("callbacks", 0)?;
        for (name, var) in [
            ("lc_ctype", "LC_CTYPE"),
            ("lc_collate", "LC_COLLATE"),
            ("lc_numeric", "LC_NUMERIC"),
        ] {
            let v = std::env::var(var)
                .or_else(|_| std::env::var("LC_ALL"))
                .or_else(|_| std::env::var("LANG"))
                .unwrap_or_else(|_| "C".to_string());
            f.set(name, v)?;
        }
        if let Ok(line) = ctx::with("status", |_, lx| Ok(lx.line())) {
            f.set("linenumber", line as i64)?;
        }
        Ok(f)
    };
    let mt = lua.create_table()?;
    mt.set(
        "__index",
        lua.create_function(move |lua, (_, key): (Table, Value)| fields(lua)?.get::<Value>(key))?,
    )?;
    t.set_metatable(Some(mt))?;
    // `statslist`: every field, as a table.
    t.set("list", lua.create_function(move |lua, ()| fields(lua))?)?;
    // `resetmessages` clears the last error and warning strings, which are
    // not kept here; `setexitcode` sets the code the run exits with, which
    // the driver decides.
    t.set("resetmessages", lua.create_function(|_, ()| Ok(()))?)?;
    t.set("setexitcode", lua.create_function(|_, _: i64| Ok(()))?)?;
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
