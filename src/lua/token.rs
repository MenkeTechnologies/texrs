//! The `token` library: a chunk reading the input, rather than being handed it.
//!
//! The LuaTeX manual (1.24.0, TeX Live 2026, "The token library") puts the two
//! directions side by side. Either the macro passes what it scanned:
//!
//! ```tex
//! \def\mymacro#1{\directlua{mymacro(\number\dimexpr#1)}}
//! ```
//!
//! or the chunk fetches it:
//!
//! ```tex
//! \def\mymacro{\directlua{mymacro()}}   % with local d = token.scan_dimen()
//! \mymacro 12pt
//! ```
//!
//! > In the first case the input is tokenized and then turned into a string,
//! > then it is passed to Lua where it gets interpreted. In the second case
//! > only a function call gets interpreted but then the input is picked up by
//! > explicitly calling the scanner functions.
//!
//! The second is what this module is. It is the one part of the Lua bridge that
//! cannot work off the snapshot [`super::Bridge`] carries: a scanner has to
//! move the engine's own input pointer, so `\mymacro 12pt` leaves `12pt`
//! consumed and the text after it still there to be typeset.
//!
//! ## How the engine gets into a callback
//!
//! Every other callback in this module's parent is `'static` — built once, when
//! the Lua state is, and living as long as it does. A scanner cannot be: it
//! holds `&mut Engine` and `&mut Lexer`, which are the caller's, and they are
//! valid only while the chunk is running. `mlua::Lua::scope` is the mechanism
//! for exactly that — "the ability to create userdata and callbacks from Rust
//! types that are `!Send` or non-`'static` […] on completion all such created
//! values are automatically dropped and Lua references to them are
//! invalidated" — so `token` is installed for the length of one chunk and taken
//! away after it. A chunk that squirrels `token.scan_int` away in a global and
//! calls it from a LATER chunk gets a Lua error, which becomes a TeX error,
//! which is the right answer: by then there is no input to scan.
//!
//! ## What is not here
//!
//! The parts of the library whose values are TOKEN USERDATA or NODES:
//! `token.create`, `token.get_next`, `token.scan_token`, `token.scan_glue`,
//! `token.scan_toks` and `token.scan_list`. Each of those hands Lua an object
//! with its own accessors over a token's internal command/index pair or over a
//! node list, and texrs has neither. They are present and REFUSE, with a
//! message naming what to use instead where there is something — a chunk that
//! got a number where LuaTeX gave it a userdata would go wrong later, in the
//! output, rather than here.

use super::*;
use crate::expand::Macro;
use crate::token::CsId;

/// The engine and the input a scanner reaches, for the length of one chunk.
struct Ctx<'a> {
    eng: &'a mut Engine,
    lx: &'a mut Lexer,
}

/// The shared handle the callbacks hold. `RefCell` rather than `&mut` passed
/// around because each callback is a separate closure and they all want the
/// same engine; Lua is single-threaded and cannot run two of them at once, so
/// the borrow never actually contends.
type Shared<'a> = Rc<RefCell<Ctx<'a>>>;

/// Run `body` with `token` installed, then take it away again.
///
/// The return is the caller's own result, untouched: a chunk that failed must
/// reach [`super::finish`] as a failure rather than as an error about the
/// library, so the two are kept apart.
pub(super) fn with<T>(
    lua: &Lua,
    eng: &mut Engine,
    lx: &mut Lexer,
    body: impl FnOnce(&Lua) -> T,
) -> mlua::Result<T> {
    // Outside the closure: `mlua::Scope` borrows what its callbacks capture for
    // the whole of the scope, so a handle built INSIDE would be dropped while
    // still borrowed.
    let ctx: Shared = Rc::new(RefCell::new(Ctx { eng, lx }));
    let out = lua.scope(|scope| {
        lua.globals().set("token", table(lua, scope, &ctx)?)?;
        Ok(body(lua))
    });
    // Whatever happened, the table's functions are dead now; leaving it in
    // place would let the next chunk find a `token` whose every member raises
    // "callback destructed" instead of finding nothing.
    lua.globals().raw_remove("token")?;
    out
}

/// A `TexError` from the engine, as the Lua error that will become one again.
fn tex_err(e: TexError) -> mlua::Error {
    mlua::Error::runtime(e.0)
}

/// The `token` table.
fn table<'scope, 'env: 'scope>(
    lua: &Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    ctx: &'env Shared<'env>,
) -> mlua::Result<Table> {
    let t = lua.create_table()?;

    // "scan_int — returns an integer". `scan_number_file` is the engine's own
    // §440 number scanner, so `\count0`, `` `\a ``, `"FF` and `\numexpr` all
    // reach a chunk exactly as they reach any other number position.
    let c = Rc::clone(ctx);
    t.set(
        "scan_int",
        scope.create_function(move |_, ()| {
            let mut c = c.borrow_mut();
            let Ctx { eng, lx } = &mut *c;
            eng.scan_number_file(lx).map_err(tex_err)
        })?,
    )?;

    // "scan_dimen — returns a number representing a dimension". Scaled points,
    // through §448's scanner, so `12pt`, `\dimen0` and `\dimexpr` all work.
    let c = Rc::clone(ctx);
    t.set(
        "scan_dimen",
        scope.create_function(move |_, ()| {
            let mut c = c.borrow_mut();
            let Ctx { eng, lx } = &mut *c;
            eng.scan_dimen_file(lx).map_err(tex_err)
        })?,
    )?;

    // "scan_keyword — returns true if the given keyword is gobbled; as with the
    // regular TeX keyword scanner this is case insensitive". `scan_keyword_cs`
    // is the case-sensitive variant; the engine's scanner is §407's, which is
    // the insensitive one, so the sensitive spelling checks the case itself
    // rather than pretending the two are the same function.
    let c = Rc::clone(ctx);
    t.set(
        "scan_keyword",
        scope.create_function(move |_, word: String| {
            let mut c = c.borrow_mut();
            let Ctx { eng, lx } = &mut *c;
            Ok(eng.scan_keyword(lx, &word, false))
        })?,
    )?;
    let c = Rc::clone(ctx);
    t.set(
        "scan_keyword_cs",
        scope.create_function(move |_, word: String| {
            let mut c = c.borrow_mut();
            let Ctx { eng, lx } = &mut *c;
            let mut seen = Vec::new();
            for want in word.chars() {
                match eng.take_file(lx) {
                    Some(Token::Char(c, _)) if c == want => seen.push(Token::Char(c, Cat::Other)),
                    other => {
                        if let Some(t) = other {
                            lx.push_back(std::slice::from_ref(&t));
                        }
                        // Nothing is consumed when the word is not there.
                        seen.reverse();
                        lx.push_back(&seen);
                        return Ok(false);
                    }
                }
            }
            Ok(true)
        })?,
    )?;

    // "scan_word — returns a sequence of characters with catcode 11 or 12 as
    // string".
    let c = Rc::clone(ctx);
    t.set(
        "scan_word",
        scope.create_function(move |_, ()| {
            let mut c = c.borrow_mut();
            let Ctx { eng, lx } = &mut *c;
            Ok(scan_word(eng, lx))
        })?,
    )?;

    // "scan_csname — returns foo after scanning \foo".
    //
    // The NEXT token, with no leading spaces skipped, which is not what the
    // other scanners do and not what TeX's own scanners do either. Read off
    // `luatex` 1.24.0 rather than assumed: `\directlua{...token.scan_csname()...}
    // \relax` (a space between) answers nil there and `...}\relax` answers
    // `relax`, while `token.scan_word()` skips the same space happily.
    let c = Rc::clone(ctx);
    t.set(
        "scan_csname",
        scope.create_function(move |_, ()| {
            let mut c = c.borrow_mut();
            let Ctx { eng, lx } = &mut *c;
            match eng.take_file(lx) {
                Some(Token::Cs(n)) => Ok(Some(n.name().to_string())),
                Some(other) => {
                    lx.push_back(std::slice::from_ref(&other));
                    Ok(None)
                }
                None => Ok(None),
            }
        })?,
    )?;

    // "The string scanner scans for something between curly braces and expands
    // on the way, or when it sees a control sequence it will return its
    // meaning. Otherwise it will scan characters with catcode letter or other."
    // `scan_argument` is the same thing with the brace case's expansion under
    // the caller's control; nothing here can produce an UNexpanded string, so
    // the flag is read and the braces are expanded either way.
    for name in ["scan_string", "scan_argument"] {
        let c = Rc::clone(ctx);
        t.set(
            name,
            scope.create_function(move |_, _: Variadic<Value>| {
                let mut c = c.borrow_mut();
                let Ctx { eng, lx } = &mut *c;
                scan_string(eng, lx).map_err(tex_err)
            })?,
        )?;
    }

    // "token.get_macro(name)" — the BODY of a macro, and nothing else. What it
    // answers for `\relax`, for a `\chardef` name and for an undefined one is
    // NO VALUE rather than nil: `tostring(token.get_macro('nosuch'))` in
    // `luatex` 1.24.0 is "bad argument #1 to 'tostring' (value expected)", so
    // an empty `Variadic` rather than an `Option`, which would push one nil.
    let c = Rc::clone(ctx);
    t.set(
        "get_macro",
        scope.create_function(move |lua, name: String| {
            let c = c.borrow();
            let Some(Meaning::Macro(m)) = c.eng.meanings.get(&CsId::intern(&name)) else {
                return Ok(Variadic::new());
            };
            let body = lua.create_string(c.eng.tokens_text(&m.body))?;
            Ok(Variadic::from_iter([Value::String(body)]))
        })?,
    )?;

    // "token.get_meaning(name)" — the parameter text, `->`, and the body, which
    // is `\meaning` without its `macro:` prefix. Read off `luatex`:
    // `\def\foo#1{foo-#1}` answers `#1->foo-#1` and `\def\bar{bar}` answers
    // `->bar`. Anything that is not a macro answers no value, as `get_macro`
    // does.
    let c = Rc::clone(ctx);
    t.set(
        "get_meaning",
        scope.create_function(move |lua, name: String| {
            let c = c.borrow();
            let Some(Meaning::Macro(m)) = c.eng.meanings.get(&CsId::intern(&name)) else {
                return Ok(Variadic::new());
            };
            let text = lua.create_string(format!(
                "{}->{}",
                c.eng.tokens_text(&m.params),
                c.eng.tokens_text(&m.body)
            ))?;
            Ok(Variadic::from_iter([Value::String(text)]))
        })?,
    )?;

    // "token.set_macro([catcodetable,] name, content [, 'global'])" — a `\def`
    // made from Lua. The body is read through the MOUTH under the live catcode
    // table, so `token.set_macro('x', '\\relax')` defines a macro whose body is
    // the control sequence rather than six characters; texrs has no
    // `\catcodetable`, so a leading table argument is read and dropped.
    let c = Rc::clone(ctx);
    t.set(
        "set_macro",
        scope.create_function(move |_, args: Variadic<Value>| {
            let args: Vec<Value> = args
                .iter()
                .filter(|v| !matches!(v, Value::Integer(_) | Value::Number(_)))
                .cloned()
                .collect();
            let text = |v: Option<&Value>| match v {
                Some(Value::String(s)) => s.to_str().map(|s| s.to_string()).ok(),
                _ => None,
            };
            let (Some(name), Some(body)) = (text(args.first()), text(args.get(1))) else {
                return Err(mlua::Error::runtime(
                    "token.set_macro wants a name and a body, both strings",
                ));
            };
            let mut c = c.borrow_mut();
            let toks = super::scan(&body, LineMode::Partial, &c.eng.cats);
            c.eng.meanings.insert(
                CsId::intern(&name),
                Meaning::Macro(Macro {
                    params: Vec::new(),
                    body: toks,
                    long: false,
                    protected: false,
                    outer: false,
                }),
            );
            Ok(())
        })?,
    )?;

    // "token.is_defined(name)". The engine's own answer: `\ifdefined` is
    // `meanings.contains_key` (`crate::expand`), so the two agree by
    // construction. What that table does NOT hold is a primitive nothing has
    // redefined — `crate::expand::Engine::new` starts it empty and the lowerer
    // dispatches primitives by name — so `token.is_defined('relax')` is false
    // here where LuaTeX says true, exactly as `\ifdefined\relax` is false here.
    let c = Rc::clone(ctx);
    t.set(
        "is_defined",
        scope.create_function(move |_, name: String| {
            let c = c.borrow();
            Ok(c.eng.meanings.contains_key(&CsId::intern(&name)) || super::claims(&name))
        })?,
    )?;

    // "biggest_char" is a constant, and the constant is Unicode's last code
    // point in both engines.
    t.set("biggest_char", 0x10FFFF)?;

    // The userdata and node halves of the library. See the module docs.
    for (name, instead) in [
        ("create", "token userdata"),
        ("new", "token userdata"),
        ("get_next", "token userdata"),
        ("scan_token", "token userdata"),
        ("expand", "token userdata"),
        (
            "scan_glue",
            "a glue_spec node; token.scan_dimen reads its width",
        ),
        ("scan_toks", "a token table"),
        ("scan_list", "a hlist or vlist node"),
    ] {
        t.set(
            name,
            lua.create_function(move |_, _: Variadic<Value>| {
                Err::<(), _>(mlua::Error::runtime(format!(
                    "token.{name} answers with {instead}, which texrs has no interface for"
                )))
            })?,
        )?;
    }
    Ok(t)
}

/// The next token that is not a space, or nothing.
fn skip_spaces(eng: &mut Engine, lx: &mut Lexer) -> Option<Token> {
    loop {
        match eng.take_file(lx) {
            Some(t) if t.is_space() => continue,
            other => return other,
        }
    }
}

/// A run of letters and others, as a string. Nothing else is consumed.
fn scan_word(eng: &mut Engine, lx: &mut Lexer) -> String {
    let mut out = String::new();
    let Some(first) = skip_spaces(eng, lx) else {
        return out;
    };
    let mut tok = Some(first);
    while let Some(t) = tok {
        match t {
            Token::Char(c, Cat::Letter | Cat::Other) => out.push(c),
            other => {
                lx.push_back(std::slice::from_ref(&other));
                break;
            }
        }
        tok = eng.take_file(lx);
    }
    out
}

/// `token.scan_string`: a braced group, a control sequence's meaning, or a word.
fn scan_string(eng: &mut Engine, lx: &mut Lexer) -> R<String> {
    let Some(tok) = skip_spaces(eng, lx) else {
        return Ok(String::new());
    };
    match tok {
        // Put the brace back and let the engine read the group: `super::
        // read_group` is `\directlua`'s own argument reader, so a group scanned
        // here expands exactly as one scanned there.
        Token::Char(_, Cat::BeginGroup) => {
            lx.push_back(std::slice::from_ref(&tok));
            super::read_group(eng, lx)
        }
        Token::Cs(name) => Ok(match eng.meanings.get(&name) {
            Some(Meaning::Macro(m)) => eng.tokens_text(&m.body),
            _ => eng.meaning_text(&Token::Cs(name)),
        }),
        other => {
            lx.push_back(std::slice::from_ref(&other));
            Ok(scan_word(eng, lx))
        }
    }
}
