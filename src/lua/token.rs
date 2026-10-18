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
//! The second is what this module is: a port of luatexdir/lua/lnewtokenlib.c.
//! A scanner moves the engine's own input pointer, so `\mymacro 12pt` leaves
//! `12pt` consumed and the text after it still there to be typeset. The engine
//! is reached through [`super::ctx`], so a function a package keeps in a local
//! -- ltluatex.lua's `createtoken = token.create`, luaotfload's scanners --
//! works whenever it is called while TeX is running a chunk.
//!
//! ## Token userdata
//!
//! A LuaTeX token is a number: `cmd * 2^21 + chr` for a character, and
//! `cs_token_flag + p` for a control sequence, `p` being its hash slot. Its
//! fields (`command`, `cmdname`, `mode`, `index`, `csname`, `expandable`,
//! `protected`, `active`) are read through that number at the moment they are
//! asked for, so a control sequence's token answers with its CURRENT meaning.
//! Here a token carries texrs's own `Token` and is read through the engine's
//! meaning table in the same way. What a field answers is luatex's:
//!
//! - `command`/`cmdname` are luatex's command codes (`token.commands()`),
//!   with a primitive's command taken from `crate::luatex_primitives`, the
//!   census of luatex's own table, so `token.create("hbox").cmdname` is
//!   `make_box` as it is there.
//! - `mode` of a register name is the register's position in luatex's eqtb,
//!   measured: `\countdef\a=0` gives `333450`, `\dimendef` `464561`, `\skipdef`
//!   `71107`, `\muskipdef` `136643`, `\toksdef` `202200`, `\attributedef`
//!   `398986` -- the bases ltluatex.lua's `registernumber` subtracts. `index`
//!   is `mode` less that base, as lnewtokenlib.c's `get_index` computes it.
//!
//! What does NOT agree, because it is an address in luatex's memory: `id`, the
//! `tok` of a control sequence, and the `mode` of a macro and of a primitive
//! (a pointer to the macro's token list, and a primitive's internal `chr`).
//! Each is a number consistent within a run, which is the only property a
//! package can rely on in luatex either.

use super::*;
use crate::expand::Macro;
use crate::token::CsId;
use mlua::{AnyUserData, MetaMethod, UserData, UserDataFields, UserDataMethods};

/// A `TexError` from the engine, as the Lua error that will become one again.
fn tex_err(e: TexError) -> mlua::Error {
    mlua::Error::runtime(e.0)
}

/// luatex's command names, by command code: `token.commands()` in luatex
/// 1.24.0, read off the binary (luatoken.c's `command_names[]`).
pub(super) const COMMANDS: &[&str] = &[
    "relax",
    "left_brace",
    "right_brace",
    "math_shift",
    "tab_mark",
    "car_ret",
    "mac_param",
    "sup_mark",
    "sub_mark",
    "endv",
    "spacer",
    "letter",
    "other_char",
    "par_end",
    "stop",
    "delim_num",
    "char_num",
    "math_char_num",
    "mark",
    "node",
    "xray",
    "make_box",
    "hmove",
    "vmove",
    "un_hbox",
    "un_vbox",
    "remove_item",
    "hskip",
    "vskip",
    "mskip",
    "kern",
    "mkern",
    "leader_ship",
    "halign",
    "valign",
    "no_align",
    "vrule",
    "hrule",
    "novrule",
    "nohrule",
    "insert",
    "vadjust",
    "ignore_spaces",
    "after_assignment",
    "after_group",
    "partoken_name",
    "break_penalty",
    "start_par",
    "ital_corr",
    "accent",
    "math_accent",
    "discretionary",
    "eq_no",
    "left_right",
    "math_comp",
    "limit_switch",
    "above",
    "math_style",
    "math_choice",
    "non_script",
    "vcenter",
    "case_shift",
    "message",
    "normal",
    "extension",
    "option",
    "lua_function_call",
    "lua_bytecode_call",
    "lua_call",
    "in_stream",
    "begin_group",
    "end_group",
    "omit",
    "ex_space",
    "boundary",
    "radical",
    "super_sub_script",
    "no_super_sub_script",
    "math_shift_cs",
    "end_cs_name",
    "char_ghost",
    "assign_local_box",
    "char_given",
    "math_given",
    "xmath_given",
    "last_item",
    "toks_register",
    "assign_toks",
    "assign_int",
    "assign_attr",
    "assign_dimen",
    "assign_glue",
    "assign_mu_glue",
    "assign_font_dimen",
    "assign_font_int",
    "assign_hang_indent",
    "set_aux",
    "set_prev_graf",
    "set_page_dimen",
    "set_page_int",
    "set_box_dimen",
    "set_tex_shape",
    "set_etex_shape",
    "def_char_code",
    "def_del_code",
    "extdef_math_code",
    "extdef_del_code",
    "def_family",
    "set_math_param",
    "set_font",
    "def_font",
    "register",
    "assign_box_direction",
    "assign_box_dir",
    "assign_direction",
    "assign_dir",
    "combinetoks",
    "advance",
    "multiply",
    "divide",
    "prefix",
    "let",
    "shorthand_def",
    "def_lua_call",
    "read_to_cs",
    "def",
    "set_box",
    "hyph_data",
    "set_interaction",
    "letterspace_font",
    "expand_font",
    "copy_font",
    "set_font_id",
    "undefined_cs",
    "expand_after",
    "no_expand",
    "input",
    "lua_expandable_call",
    "lua_local_call",
    "if_test",
    "fi_or_else",
    "cs_name",
    "convert",
    "variable",
    "feedback",
    "the",
    "top_bot_mark",
    "call",
    "long_call",
    "outer_call",
    "long_outer_call",
    "end_template",
    "dont_expand",
    "glue_ref",
    "shape_ref",
    "box_ref",
    "data",
];

/// A command's code, by name.
fn command(name: &str) -> i64 {
    COMMANDS
        .iter()
        .position(|c| *c == name)
        .map(|i| i as i64)
        .unwrap_or(0)
}

/// `max_command_cmd`: every command above it expands. It is `set_font_id`'s
/// code, the last of the non-expandable ones (luatex's `commands.h` puts
/// `undefined_cs` immediately after it).
fn max_command() -> i64 {
    command("set_font_id")
}

/// luatex's eqtb bases for the register families, measured (module docs).
const COUNT_BASE: i64 = 333_450;
const SCALED_BASE: i64 = 464_561;
const SKIP_BASE: i64 = 71_107;
const MU_SKIP_BASE: i64 = 136_643;
const TOKS_BASE: i64 = 202_200;
const ATTRIBUTE_BASE: i64 = 398_986;

/// `too_big_char`: the `chr` of `\relax` and of `\par` in luatex, so their
/// `index` is nil.
const TOO_BIG_CHAR: i64 = 0x11_0000;

/// `cs_token_flag`, luatex's `0x1FFFFFFF`.
const CS_TOKEN_FLAG: i64 = 0x1FFF_FFFF;

/// What a token means now: a command code and a `chr`.
fn cmd_chr(eng: &Engine, tok: &Token) -> (i64, i64) {
    let name = match tok {
        Token::Char(c, Cat::Active) => Engine::active_cs(*c),
        Token::Char(c, cat) => return (*cat as i64, *c as i64),
        Token::Cs(name) => *name,
    };
    match eng.meanings.get(&name) {
        Some(Meaning::Macro(m)) => {
            let cmd = match (m.long, m.outer) {
                (false, false) => "call",
                (true, false) => "long_call",
                (false, true) => "outer_call",
                (true, true) => "long_outer_call",
            };
            (command(cmd), macro_mode(m))
        }
        Some(Meaning::Primitive(p)) => primitive_cmd(p.name()),
        Some(Meaning::Char(c, cat)) => (*cat as i64, *c as i64),
        Some(Meaning::CharDef(n)) => (command("char_given"), *n),
        Some(Meaning::MathCharDef(n)) => (command("math_given"), *n),
        Some(Meaning::ToksDef(r)) => (command("assign_toks"), TOKS_BASE + r),
        Some(Meaning::CountDef(slot)) => register_cmd(eng, *slot),
        // A font identifier is `set_font` with the font's number (§1257).
        Some(Meaning::Font(f)) => (command("set_font"), i64::from(*f)),
        None => primitive_cmd(name.name()),
    }
}

/// A `\countdef`-family name's command and mode. `crate::expand` records one
/// as a SLOT in the register file `crate::compiler` lays out -- counts from
/// 0, dimensions from `DIMEN_BASE`, glue four slots wide from `SKIP_BASE`,
/// muglue after it -- and the slot says which family it is.
fn register_cmd(eng: &Engine, slot: i64) -> (i64, i64) {
    use crate::compiler::{RegFile, DIMEN_BASE, MUSKIP_BASE, SKIP_BASE as TX_SKIP, SKIP_STRIDE};
    // A register past 255 was given a pooled slot; the engine knows which.
    if let Some((file, n)) = eng.pooled_register(slot) {
        return match file {
            RegFile::Count => (command("assign_int"), COUNT_BASE + n),
            RegFile::Dimen => (command("assign_dimen"), SCALED_BASE + n),
            RegFile::Skip => (command("assign_glue"), SKIP_BASE + n),
            RegFile::MuSkip => (command("assign_mu_glue"), MU_SKIP_BASE + n),
        };
    }
    if slot >= super::ATTRIBUTE_BASE {
        (
            command("assign_attr"),
            ATTRIBUTE_BASE + slot - super::ATTRIBUTE_BASE,
        )
    } else if slot >= MUSKIP_BASE {
        (
            command("assign_mu_glue"),
            MU_SKIP_BASE + (slot - MUSKIP_BASE) / SKIP_STRIDE,
        )
    } else if slot >= TX_SKIP {
        (
            command("assign_glue"),
            SKIP_BASE + (slot - TX_SKIP) / SKIP_STRIDE,
        )
    } else if slot >= DIMEN_BASE {
        (command("assign_dimen"), SCALED_BASE + slot - DIMEN_BASE)
    } else {
        (command("assign_int"), COUNT_BASE + slot)
    }
}

/// A primitive's command by its luatex name, `undefined_cs` for a name that
/// is not one. `\relax` and `\par` carry luatex's `too_big_char` so their
/// `index` is nil as it is there; any other primitive's internal `chr` is not
/// known here and reads as 0.
fn primitive_cmd(name: &str) -> (i64, i64) {
    match crate::luatex_primitives::TABLE.binary_search_by(|(n, _, _)| (*n).cmp(name)) {
        Ok(i) => {
            let cmd = command(crate::luatex_primitives::TABLE[i].1);
            let chr = match name {
                "relax" | "par" => TOO_BIG_CHAR,
                _ => 0,
            };
            (cmd, chr)
        }
        Err(_) => (command("undefined_cs"), 0),
    }
}

/// A stable number for a macro's definition, standing for luatex's pointer to
/// its token list: the `Rc`'s address, which is the same for every name
/// `\let` to the same definition, as the pointer is in luatex.
fn macro_mode(m: &Rc<Macro>) -> i64 {
    ((Rc::as_ptr(m) as usize >> 4) & 0x00FF_FFFF) as i64
}

thread_local! {
    /// A number per control-sequence name, standing for its hash slot in
    /// `tok`, and the counter `id` comes from.
    static CS_SLOTS: RefCell<HashMap<&'static str, i64>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<i64> = const { Cell::new(1) };
}

fn cs_slot(name: CsId) -> i64 {
    CS_SLOTS.with(|m| {
        let mut m = m.borrow_mut();
        let n = m.len() as i64 + 1;
        *m.entry(name.name()).or_insert(n)
    })
}

/// A token, as Lua holds it.
#[derive(Clone)]
pub(super) struct LuaToken {
    pub(super) tok: Token,
    /// luatex's `id`, the token's memory address there; a serial here.
    id: i64,
}

impl LuaToken {
    pub(super) fn new(tok: Token) -> LuaToken {
        let id = NEXT_ID.with(|n| {
            let v = n.get();
            n.set(v + 1);
            v
        });
        LuaToken { tok, id }
    }

    /// The control sequence this token is, if it is one. An active character
    /// is one, under its own name (luatex's `\xEF\xBF\xBF` prefix).
    fn cs(&self) -> Option<CsId> {
        match self.tok {
            Token::Cs(n) => Some(n),
            Token::Char(c, Cat::Active) => Some(Engine::active_cs(c)),
            _ => None,
        }
    }

    /// `tok`: `cmd * 2^21 + chr` for a character, `cs_token_flag + p` for a
    /// control sequence.
    fn tok(&self) -> i64 {
        match (self.cs(), self.tok) {
            (Some(n), _) => CS_TOKEN_FLAG + cs_slot(n),
            (None, Token::Char(c, cat)) => ((cat as i64) << 21) + c as i64,
            (None, Token::Cs(_)) => unreachable!("a Cs token is a control sequence"),
        }
    }

    /// `csname`: the name, without the active prefix; nil for a character.
    fn csname(&self) -> Option<String> {
        match self.tok {
            Token::Cs(n) => Some(n.name().to_string()),
            Token::Char(c, Cat::Active) => Some(c.to_string()),
            _ => None,
        }
    }

    /// The fields that need the engine, read under it.
    fn field(&self, lua: &Lua, key: &str) -> mlua::Result<Value> {
        let need = |what: &str| format!("token.{what}");
        Ok(match key {
            "command" => Value::Integer(ctx::with(&need(key), |eng, _| {
                Ok(cmd_chr(eng, &self.tok).0)
            })?),
            "cmdname" => {
                let cmd = ctx::with(&need(key), |eng, _| Ok(cmd_chr(eng, &self.tok).0))?;
                Value::String(lua.create_string(COMMANDS[cmd as usize])?)
            }
            "mode" => Value::Integer(ctx::with(&need(key), |eng, _| {
                Ok(cmd_chr(eng, &self.tok).1)
            })?),
            "index" => {
                let (cmd, chr) = ctx::with(&need(key), |eng, _| Ok(cmd_chr(eng, &self.tok)))?;
                let base = match COMMANDS[cmd as usize] {
                    "assign_int" => COUNT_BASE,
                    "assign_attr" => ATTRIBUTE_BASE,
                    "assign_dimen" => SCALED_BASE,
                    "assign_glue" => SKIP_BASE,
                    "assign_mu_glue" => MU_SKIP_BASE,
                    "assign_toks" => TOKS_BASE,
                    _ => 0,
                };
                let i = chr - base;
                match (0..=65535).contains(&i) {
                    true => Value::Integer(i),
                    false => Value::Nil,
                }
            }
            "csname" => match self.csname() {
                Some(s) => Value::String(lua.create_string(s)?),
                None => Value::Nil,
            },
            "id" => Value::Integer(self.id),
            "tok" => Value::Integer(self.tok()),
            "active" => Value::Boolean(matches!(self.tok, Token::Char(_, Cat::Active))),
            "expandable" => {
                let cmd = ctx::with(&need(key), |eng, _| Ok(cmd_chr(eng, &self.tok).0))?;
                Value::Boolean(cmd > max_command())
            }
            "protected" => Value::Boolean(ctx::with(&need(key), |eng, _| {
                Ok(match self.cs().and_then(|n| eng.meanings.get(&n)) {
                    Some(Meaning::Macro(m)) => m.protected,
                    _ => false,
                })
            })?),
            _ => Value::Nil,
        })
    }
}

impl UserData for LuaToken {
    fn add_fields<F: UserDataFields<Self>>(_: &mut F) {}

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // `lua_tokenlib_getfield`: one `__index` that dispatches on the key.
        methods.add_meta_method(MetaMethod::Index, |lua, this, key: Value| match key {
            Value::String(s) => this.field(lua, &s.to_str()?),
            _ => Ok(Value::Nil),
        });
        // luatex's `__tostring`: `<luatex.token ...>`, shown with its tok.
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(format!("<luatex.token: {}>", this.tok()))
        });
        methods.add_meta_method(MetaMethod::Eq, |_, a, b: AnyUserData| {
            Ok(b.borrow::<LuaToken>().is_ok_and(|b| b.tok == a.tok))
        });
    }
}

/// `check_istoken`: the token argument, or luatex's error.
fn as_token(v: &Value) -> mlua::Result<LuaToken> {
    if let Value::UserData(u) = v {
        if let Ok(t) = u.borrow::<LuaToken>() {
            return Ok(t.clone());
        }
    }
    Err(mlua::Error::runtime(format!(
        "token lib: lua <token> expected, not an object with type {}",
        v.type_name()
    )))
}

/// The tokens of a `put_next` argument list: a table of tokens, or the tokens
/// themselves (`run_put_next`).
fn tokens_of(args: &Variadic<Value>) -> mlua::Result<Vec<Token>> {
    match args.first() {
        Some(Value::Table(t)) if args.len() == 1 => {
            let mut out = Vec::new();
            for v in t.clone().sequence_values::<Value>() {
                out.push(as_token(&v?)?.tok);
            }
            Ok(out)
        }
        Some(Value::Table(_)) => Err(mlua::Error::runtime(
            "token lib: only one table permitted in put_next",
        )),
        _ => args.iter().map(|v| as_token(v).map(|t| t.tok)).collect(),
    }
}

/// The `token` table.
pub(super) fn table(lua: &Lua, arena: &Rc<RefCell<node::Arena>>) -> mlua::Result<Table> {
    let t = lua.create_table()?;

    // "scan_int — returns an integer". `scan_number_file` is the engine's own
    // §440 number scanner, so `\count0`, `` `\a ``, `"FF` and `\numexpr` all
    // reach a chunk exactly as they reach any other number position.
    t.set(
        "scan_int",
        lua.create_function(|_, ()| {
            ctx::with("token.scan_int", |eng, lx| {
                eng.scan_number_file(lx).map_err(tex_err)
            })
        })?,
    )?;

    // "scan_dimen([inf [, mu]])": scaled points through §448's scanner; with
    // `inf` the infinite units are allowed and the order is a second result,
    // numbered as luatex numbers it (`fil` is 2). `mu` asks for math units,
    // which only the muglue scanner reads here.
    t.set(
        "scan_dimen",
        lua.create_function(|_, (inf, mu): (Option<bool>, Option<bool>)| {
            ctx::with("token.scan_dimen", |eng, lx| {
                if mu.unwrap_or(false) {
                    let (w, ..) = eng.scan_muglue(lx).map_err(tex_err)?;
                    return Ok(Variadic::from_iter([Value::Integer(w)]));
                }
                match inf.unwrap_or(false) {
                    true => {
                        let (v, order) = eng.scan_dimen_or_fil(lx, false).map_err(tex_err)?;
                        Ok(Variadic::from_iter([
                            Value::Integer(v),
                            Value::Integer(super::to_luatex_order(order)),
                        ]))
                    }
                    false => Ok(Variadic::from_iter([Value::Integer(
                        eng.scan_dimen_file(lx).map_err(tex_err)?,
                    )])),
                }
            })
        })?,
    )?;

    // "scan_glue([mu])": a glue_spec node.
    let a = Rc::clone(arena);
    t.set(
        "scan_glue",
        lua.create_function(move |lua, mu: Option<bool>| {
            let glue = ctx::with("token.scan_glue", |eng, lx| {
                let g = match mu.unwrap_or(false) {
                    true => eng.scan_muglue(lx),
                    false => eng.scan_glue(lx),
                };
                g.map_err(tex_err)
            })?;
            let (w, st, sh, sto, sho) = glue;
            node::spec_node(
                lua,
                &a,
                (
                    w,
                    st,
                    sh,
                    super::to_luatex_order(sto),
                    super::to_luatex_order(sho),
                ),
            )
        })?,
    )?;

    // "scan_keyword — returns true if the given keyword is gobbled; as with the
    // regular TeX keyword scanner this is case insensitive".
    t.set(
        "scan_keyword",
        lua.create_function(|_, word: String| {
            ctx::with("token.scan_keyword", |eng, lx| {
                eng.scan_keyword(lx, &word, false).map_err(tex_err)
            })
        })?,
    )?;
    // `scan_keyword_cs` is the case-sensitive variant; the engine's scanner is
    // §407's, the insensitive one, so the sensitive spelling checks the case
    // itself rather than pretending the two are the same function.
    t.set(
        "scan_keyword_cs",
        lua.create_function(|_, word: String| {
            ctx::with("token.scan_keyword_cs", |eng, lx| {
                let mut seen = Vec::new();
                for want in word.chars() {
                    match eng.take_file(lx) {
                        Some(Token::Char(c, _)) if c == want => {
                            seen.push(Token::Char(c, Cat::Other))
                        }
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
            })
        })?,
    )?;

    // "scan_word — returns a sequence of characters with catcode 11 or 12 as
    // string".
    t.set(
        "scan_word",
        lua.create_function(|_, ()| {
            ctx::with("token.scan_word", |eng, lx| Ok(scan_word(eng, lx)))
        })?,
    )?;

    // "scan_csname — returns foo after scanning \foo".
    //
    // The NEXT token, with no leading spaces skipped, which is not what the
    // other scanners do and not what TeX's own scanners do either. Read off
    // `luatex` 1.24.0 rather than assumed: `\directlua{...token.scan_csname()...}
    // \relax` (a space between) answers nil there and `...}\relax` answers
    // `relax`, while `token.scan_word()` skips the same space happily. A
    // non-control-sequence is consumed, as `get_next` consumes it there.
    t.set(
        "scan_csname",
        lua.create_function(|_, ()| {
            ctx::with("token.scan_csname", |eng, lx| match eng.take_file(lx) {
                Some(t @ (Token::Cs(_) | Token::Char(_, Cat::Active))) => {
                    Ok(LuaToken::new(t).csname())
                }
                Some(other) => {
                    lx.push_back(std::slice::from_ref(&other));
                    Ok(None)
                }
                None => Ok(None),
            })
        })?,
    )?;

    // "The string scanner scans for something between curly braces and expands
    // on the way, or when it sees a control sequence it will return its
    // meaning. Otherwise it will scan characters with catcode letter or other."
    // `scan_argument` is the same thing with the brace case's expansion under
    // the caller's control; nothing here can produce an UNexpanded string, so
    // the flag is read and the braces are expanded either way.
    for name in ["scan_string", "scan_argument"] {
        t.set(
            name,
            lua.create_function(move |_, _: Variadic<Value>| {
                ctx::with(name, |eng, lx| scan_string(eng, lx).map_err(tex_err))
            })?,
        )?;
    }

    // "scan_float / scan_real": a decimal number with `.` or `,` as the point,
    // signs collapsed as `scan_dimen` collapses them; `scan_float` also takes an
    // exponent. Nil when what was read is not a number Lua can convert.
    for (name, exponent) in [("scan_float", true), ("scan_real", false)] {
        t.set(
            name,
            lua.create_function(move |_, ()| {
                ctx::with(name, |eng, lx| Ok(scan_float(eng, lx, exponent)))
            })?,
        )?;
    }

    // "scan_code([bitset])": the character code of the next character when its
    // catcode is in the set (letters and others by default), nil otherwise.
    t.set(
        "scan_code",
        lua.create_function(|_, set: Option<i64>| {
            let set = match set {
                None | Some(0) => (1 << 11) | (1 << 12),
                Some(s) => s,
            };
            ctx::with("token.scan_code", |eng, lx| match eng.take_file(lx) {
                Some(Token::Char(c, cat)) if (cat as i64) < 16 && set & (1 << cat as i64) != 0 => {
                    Ok(Some(c as i64))
                }
                Some(other) => {
                    lx.push_back(std::slice::from_ref(&other));
                    Ok(None)
                }
                None => Ok(None),
            })
        })?,
    )?;

    // "scan_toks([macro [, expand]])": a braced group as a table of tokens;
    // `expand` expands it the way `\edef` does. The macro-definition form reads
    // a parameter text before the brace, which nothing here scans.
    t.set(
        "scan_toks",
        lua.create_function(|lua, (macro_def, expand): (Option<bool>, Option<bool>)| {
            if macro_def.unwrap_or(false) {
                return Err(mlua::Error::runtime(
                    "token.scan_toks(true) reads a macro's parameter text, which texrs's \
                     scanner does not hand out",
                ));
            }
            let toks = ctx::with("token.scan_toks", |eng, lx| {
                let body = eng.read_group_tokens(lx).map_err(tex_err)?;
                match expand.unwrap_or(false) {
                    true => eng.expand_edef_body(&body).map_err(tex_err),
                    false => Ok(body),
                }
            })?;
            let out = lua.create_table_with_capacity(toks.len(), 0)?;
            for (i, t) in toks.into_iter().enumerate() {
                out.raw_set(i + 1, LuaToken::new(t))?;
            }
            Ok(out)
        })?,
    )?;

    // "get_next — returns the next token in the input, as a token userdata,
    // without expanding it".
    t.set(
        "get_next",
        lua.create_function(|_, ()| {
            ctx::with("token.get_next", |eng, lx| {
                Ok(eng.take_file(lx).map(LuaToken::new))
            })
        })?,
    )?;
    // "scan_token — expands the input and returns the next unexpandable token":
    // expansion here is the engine's macro expansion, one macro at a time
    // until the token at the front is not one.
    t.set(
        "scan_token",
        lua.create_function(|_, ()| {
            ctx::with("token.scan_token", |eng, lx| loop {
                let Some(tok) = eng.take_file(lx) else {
                    return Ok(None);
                };
                let name = match tok {
                    Token::Cs(n) => n,
                    Token::Char(c, Cat::Active) => Engine::active_cs(c),
                    _ => return Ok(Some(LuaToken::new(tok))),
                };
                match eng.meanings.get(&name) {
                    Some(Meaning::Macro(_)) => eng.expand_macro_file(lx, name).map_err(tex_err)?,
                    _ => return Ok(Some(LuaToken::new(tok))),
                }
            })
        })?,
    )?;
    // "put_next(token [, ...]) / put_next{...}": the tokens go back in front
    // of the input, read next in the order given.
    for name in ["put_next", "unchecked_put_next"] {
        t.set(
            name,
            lua.create_function(move |_, args: Variadic<Value>| {
                if args.is_empty() {
                    return Ok(());
                }
                let toks = tokens_of(&args)?;
                ctx::with(name, |_, lx| {
                    lx.push_back(&toks);
                    Ok(())
                })
            })?,
        )?;
    }

    // "create(name | char [, catcode])" (`run_build`): a character with the
    // catcode given or the current one (0, 9, 14 and 15 cannot be tokens and
    // become 12, with luatex's warning), or a control sequence by name -- which
    // is entered into the hash if it was not there, as `string_lookup` does.
    t.set(
        "create",
        lua.create_function(|lua, (v, cat): (Value, Option<i64>)| match v {
            Value::Integer(_) | Value::Number(_) => {
                let code = v.as_integer().unwrap_or_else(|| v.as_number().unwrap_or(0.0) as i64);
                let c = char::from_u32(code as u32).unwrap_or('\u{FFFD}');
                let cat = match cat {
                    Some(n) => n,
                    None => ctx::with("token.create", |eng, _| Ok(eng.cats.get(c) as i64))?,
                };
                let cat = match cat {
                    0 | 9 | 14 | 15 => {
                        let msg = format!(
                            "token lib: not a good token, catcode {cat} can not be returned, so 12 will be used"
                        );
                        super::warn(&msg);
                        12
                    }
                    n => n,
                };
                let cat = crate::catcode::cat_from_i64(cat)
                    .ok_or_else(|| mlua::Error::runtime(format!("bad catcode {cat}")))?;
                Ok(Value::UserData(lua.create_userdata(LuaToken::new(Token::Char(c, cat)))?))
            }
            Value::String(s) => {
                let name = s.to_str()?.to_string();
                if name.is_empty() {
                    return Ok(Value::Nil);
                }
                Ok(Value::UserData(lua.create_userdata(LuaToken::new(Token::cs(&name)))?))
            }
            _ => Ok(Value::Nil),
        })?,
    )?;
    // "new(chr, cmd)": a token from a raw command code and a character; only
    // the character commands are tokens texrs can hold.
    t.set(
        "new",
        lua.create_function(|lua, (chr, cmd): (i64, i64)| {
            let c = char::from_u32(chr as u32).unwrap_or('\u{FFFD}');
            let cat = crate::catcode::cat_from_i64(cmd).filter(|_| (1..=13).contains(&cmd));
            match cat {
                Some(cat) => Ok(lua.create_userdata(LuaToken::new(Token::Char(c, cat)))?),
                None => Err(mlua::Error::runtime(format!(
                    "token.new({chr}, {cmd}): command {cmd} is not a character command, \
                     and texrs holds a token as a character or a control sequence"
                ))),
            }
        })?,
    )?;
    t.set(
        "is_token",
        lua.create_function(|_, v: Value| {
            Ok(matches!(&v, Value::UserData(u) if u.borrow::<LuaToken>().is_ok()))
        })?,
    )?;
    // "token.commands()": command code to name.
    t.set(
        "commands",
        lua.create_function(|lua, ()| {
            let out = lua.create_table()?;
            for (i, c) in COMMANDS.iter().enumerate() {
                out.raw_set(i as i64, *c)?;
            }
            Ok(out)
        })?,
    )?;
    t.set(
        "command_id",
        lua.create_function(|_, name: Value| {
            Ok(match name {
                Value::String(s) => {
                    let s = s.to_str()?.to_string();
                    COMMANDS.iter().position(|c| *c == s).map(|i| i as i64)
                }
                _ => None,
            })
        })?,
    )?;
    // The function spellings of the fields: `token.get_cmdname(t)` and so on.
    for field in [
        "command",
        "cmdname",
        "index",
        "mode",
        "csname",
        "id",
        "tok",
        "active",
        "expandable",
        "protected",
    ] {
        t.set(
            format!("get_{field}"),
            lua.create_function(move |lua, v: Value| as_token(&v)?.field(lua, field))?,
        )?;
    }

    // "token.get_macro(name)" — the BODY of a macro, and nothing else. What it
    // answers for `\relax`, for a `\chardef` name and for an undefined one is
    // NO VALUE rather than nil: `tostring(token.get_macro('nosuch'))` in
    // `luatex` 1.24.0 is "bad argument #1 to 'tostring' (value expected)", so
    // an empty `Variadic` rather than an `Option`, which would push one nil.
    t.set(
        "get_macro",
        lua.create_function(|lua, name: String| {
            ctx::with("token.get_macro", |eng, _| {
                let Some(Meaning::Macro(m)) = eng.meanings.get(&CsId::intern(&name)) else {
                    return Ok(Variadic::new());
                };
                let body = lua.create_string(eng.tokens_text(&m.body))?;
                Ok(Variadic::from_iter([Value::String(body)]))
            })
        })?,
    )?;

    // "token.get_meaning(name)" — the parameter text, `->`, and the body, which
    // is `\meaning` without its `macro:` prefix. Read off `luatex`:
    // `\def\foo#1{foo-#1}` answers `#1->foo-#1` and `\def\bar{bar}` answers
    // `->bar`. Anything that is not a macro answers no value, as `get_macro`
    // does.
    t.set(
        "get_meaning",
        lua.create_function(|lua, name: String| {
            ctx::with("token.get_meaning", |eng, _| {
                let Some(Meaning::Macro(m)) = eng.meanings.get(&CsId::intern(&name)) else {
                    return Ok(Variadic::new());
                };
                let text = lua.create_string(format!(
                    "{}->{}",
                    eng.tokens_text(&m.params),
                    eng.tokens_text(&m.body)
                ))?;
                Ok(Variadic::from_iter([Value::String(text)]))
            })
        })?,
    )?;

    // "token.set_macro([catcodetable,] name, content [, 'global'])" — a `\def`
    // made from Lua. The body is read through the MOUTH under the live catcode
    // table, so `token.set_macro('x', '\\relax')` defines a macro whose body is
    // the control sequence rather than six characters; texrs has no
    // `\catcodetable`, so a leading table argument is read and dropped.
    t.set(
        "set_macro",
        lua.create_function(|_, args: Variadic<Value>| {
            let args: Vec<Value> = args
                .iter()
                .filter(|v| !matches!(v, Value::Integer(_) | Value::Number(_)))
                .cloned()
                .collect();
            let text = |v: Option<&Value>| match v {
                Some(Value::String(s)) => s.to_str().map(|s| s.to_string()).ok(),
                _ => None,
            };
            let Some(name) = text(args.first()) else {
                return Err(mlua::Error::runtime(
                    "token.set_macro wants a name and a body, both strings",
                ));
            };
            // A missing body is an empty macro, as `set_macro(name)` is there.
            let body = text(args.get(1)).unwrap_or_default();
            ctx::with("token.set_macro", |eng, _| {
                let toks = super::scan(&body, LineMode::Partial, &eng.cats);
                eng.meanings.insert(
                    CsId::intern(&name),
                    Meaning::Macro(std::rc::Rc::new(Macro {
                        params: Vec::new(),
                        body: toks,
                        long: false,
                        protected: false,
                        outer: false,
                    })),
                );
                Ok(())
            })
        })?,
    )?;

    // "token.set_char(name, number [, 'global'])": a `\chardef` made from Lua.
    t.set(
        "set_char",
        lua.create_function(|_, args: Variadic<Value>| {
            let name = match args.first() {
                Some(Value::String(s)) => s.to_str()?.to_string(),
                _ => {
                    return Err(mlua::Error::runtime(
                        "token.set_char wants a name and a number",
                    ))
                }
            };
            let n = args.get(1).and_then(|v| v.as_integer()).unwrap_or(0);
            ctx::with("token.set_char", |eng, _| {
                eng.meanings
                    .insert(CsId::intern(&name), Meaning::CharDef(n));
                Ok(())
            })
        })?,
    )?;

    // "token.is_defined(name [, onlycs])". luatex answers whether the name's
    // `eq_type` is something other than `undefined_cs`, or with the second
    // argument true whether it is in the hash at all. A primitive nothing has
    // redefined is not in texrs's meaning table -- `crate::expand::Engine::new`
    // starts it empty and the lowerer dispatches primitives by name -- so the
    // luatex primitive census answers for those.
    t.set(
        "is_defined",
        lua.create_function(|_, (name, onlycs): (Value, Option<bool>)| {
            let Value::String(name) = name else {
                return Ok(false);
            };
            let name = name.to_str()?.to_string();
            if name.is_empty() {
                return Ok(false);
            }
            ctx::with("token.is_defined", |eng, _| {
                let cs = CsId::intern(&name);
                let defined = means_something(eng, &name);
                Ok(match onlycs.unwrap_or(false) {
                    true => defined || CS_SLOTS.with(|m| m.borrow().contains_key(cs.name())),
                    false => defined,
                })
            })
        })?,
    )?;

    // "expand": luatex runs one expansion step of `cur_cmd`, the command
    // the last scanner left behind -- a register of TeX's scanner that texrs's
    // scanners do not keep.
    t.set(
        "expand",
        lua.create_function(|_, ()| {
            Err::<(), _>(mlua::Error::runtime(
                "token.expand expands the token TeX's scanner last read (cur_cmd), which \
                 texrs's scanners do not keep",
            ))
        })?,
    )?;

    // "biggest_char" is a constant, and the constant is Unicode's last code
    // point in both engines.
    t.set("biggest_char", lua.create_function(|_, ()| Ok(0x10FFFF))?)?;

    // The node half: a list scanned as a box.
    t.set(
        "scan_list",
        lua.create_function(|_, _: Variadic<Value>| {
            Err::<(), _>(mlua::Error::runtime(
                "token.scan_list scans a box into a node list, and the stomach that \
                 would build it is not on texrs's path yet",
            ))
        })?,
    )?;
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

/// `run_scan_float_indeed`: signs, digits with `.` or `,` as the point, and
/// (with `exponent`) an `e` or `E` with an optional sign and digits.
fn scan_float(eng: &mut Engine, lx: &mut Lexer, exponent: bool) -> Option<f64> {
    let mut buf = String::new();
    let mut negative = false;
    let mut tok = loop {
        match skip_spaces(eng, lx) {
            Some(Token::Char('-', Cat::Other)) => negative = !negative,
            Some(Token::Char('+', Cat::Other)) => {}
            other => break other,
        }
    };
    if negative {
        buf.push('-');
    }
    let digit = |t: &Option<Token>| match t {
        Some(Token::Char(c, Cat::Other)) if c.is_ascii_digit() => Some(*c),
        _ => None,
    };
    let point = |t: &Option<Token>| matches!(t, Some(Token::Char('.' | ',', Cat::Other)));
    let mut seen_point = false;
    loop {
        if let Some(c) = digit(&tok) {
            buf.push(c);
        } else if point(&tok) && !seen_point {
            seen_point = true;
            buf.push('.');
        } else {
            break;
        }
        tok = eng.take_file(lx);
    }
    if exponent && matches!(tok, Some(Token::Char('e' | 'E', _))) {
        buf.push('e');
        tok = eng.take_file(lx);
        if let Some(Token::Char(c @ ('+' | '-'), Cat::Other)) = tok {
            buf.push(c);
            tok = eng.take_file(lx);
        }
        while let Some(c) = digit(&tok) {
            buf.push(c);
            tok = eng.take_file(lx);
        }
    }
    if let Some(t) = tok {
        lx.push_back(std::slice::from_ref(&t));
    }
    buf.parse::<f64>().ok()
}

/// `token.scan_string`: a braced group, a macro's body, or a word; nil (and
/// the token put back) for anything else.
fn scan_string(eng: &mut Engine, lx: &mut Lexer) -> R<Option<String>> {
    let Some(tok) = skip_spaces(eng, lx) else {
        return Ok(None);
    };
    match tok {
        // Put the brace back and let the engine read the group: `super::
        // read_group` is `\directlua`'s own argument reader, so a group scanned
        // here expands exactly as one scanned there.
        Token::Char(_, Cat::BeginGroup) => {
            lx.push_back(std::slice::from_ref(&tok));
            super::read_group(eng, lx).map(Some)
        }
        Token::Cs(name) => Ok(Some(match eng.meanings.get(&name) {
            Some(Meaning::Macro(m)) => eng.tokens_text(&m.body),
            _ => eng.meaning_text(&Token::Cs(name)),
        })),
        Token::Char(_, Cat::Letter | Cat::Other) => {
            lx.push_back(std::slice::from_ref(&tok));
            Ok(Some(scan_word(eng, lx)))
        }
        other => {
            lx.push_back(std::slice::from_ref(&other));
            Ok(None)
        }
    }
}

/// Whether `name` is one of luatex's primitives.
pub(super) fn is_primitive_name(name: &str) -> bool {
    primitive_cmd(name).0 != command("undefined_cs")
}

/// Whether the control sequence `name` means anything: its `eq_type` is not
/// `undefined_cs`. A primitive nothing has redefined is not in texrs's
/// meaning table -- `crate::expand::Engine::new` starts it empty and the
/// lowerer dispatches primitives by name -- so luatex's primitive census
/// answers for those.
pub(super) fn means_something(eng: &Engine, name: &str) -> bool {
    eng.meanings.contains_key(&CsId::intern(name)) || super::claims(name) || is_primitive_name(name)
}
