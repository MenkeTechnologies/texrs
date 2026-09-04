//! Mathematics: `$…$`, `$$…$$`, and what `tex.web` does between them.
//!
//! TeX is two engines in one, and the second of them is this: a formula is not
//! text with symbols in it, it is a TREE (§680) that is converted to boxes by
//! rules stated in a font's own parameters. `x^2` is not an `x` and a raised
//! `2`; it is an Ord noad whose superscript is a script-size mlist, shifted by
//! `sup1` or `sup2` or `sup3` depending on the style, never rising above the
//! x-height's four fifths, and set with `\scriptspace` reserved after it.
//! Every one of those numbers comes out of `cmsy10`'s `fontdimen`s.
//!
//! The port is in four pieces, each named for the sections of `tex.web` it
//! carries:
//!
//! | module | `tex.web` | what it is |
//! |---|---|---|
//! | [`font`] | §699-§701 | the four families and their parameters |
//! | [`noad`] | §680-§698, §764 | the mlist, and the inter-element spacing table |
//! | [`parse`] | §1090-§1206 | reading a formula into an mlist |
//! | [`mlist`] | §704-§767 | `mlist_to_hlist`: the formula, as boxes |
//! | [`set`] | §619-§640 | where each glyph of it lands |
//!
//! ## How a formula reaches the page
//!
//! Everything the lowerer decides that the typesetter has to know travels the
//! document's own character stream as a MARKER: the colour of a run, the face
//! it is set in, where a page breaks. A formula travels the same way, and as
//! the same shape of marker the colour uses -- `\u{1}` spec `\u{2}` text
//! `\u{3}` -- for a reason worth stating: the SPEC carries the setting, which
//! is instructions to a page and no part of the document's words, while the
//! TEXT between the marker and its close is the formula spelled out for a
//! reader. So `--text` prints `x²+1` where the page draws a set formula, and
//! neither path has to know about the other's half.
//!
//! ## What is not here
//!
//! - `\mathaccent` (§738-§742): `\hat`, `\bar`, `\vec` and their siblings.
//! - `\vcenter` (§736), `\mathchoice` (§689), and `\mkern`/`\mskip` written
//!   by the document rather than by the spacing table.
//! - The penalties §761 and §767 insert, because nothing breaks a line inside
//!   a formula here.
//! - The ligature half of `make_ord` (§752-§753). Computer Modern's math
//!   families define no ligature programs, so it has nothing to do; the KERNS
//!   of the same routine are ported.
//! - `\eqno`, equation numbering, and the alignment of `align`: an `&` is
//!   dropped rather than lining a column up, so a two-column display sets as
//!   one row of material.
//! - `\mathcode` and `\delcode` assignments made BY A DOCUMENT. The tables in
//!   [`parse`] are INITEX's and plain.tex's, which is what `$x+y$` means;
//!   `\mathchardef` IS read from the engine, because `src/expand.rs` already
//!   carries it.

pub mod font;
pub mod mlist;
pub mod noad;
pub mod parse;
pub mod set;

use crate::expand::{Engine, TexError};
use crate::ir::Cmd;
use crate::lexer::Lexer;
use crate::token::Token;
use noad::{Noad, Scaled, Style, DISPLAY_STYLE, TEXT_STYLE};
use set::Setting;

/// The first character of a marker spec that carries a formula rather than a
/// colour.
///
/// `styled_runs` in `src/typeset.rs` reads a spec as `r,g,b` and a formula's
/// is not three numbers, so it would have been treated as a colour that could
/// not be parsed and inherited the one around it -- which is the right thing
/// for a path that has not been taught about formulas, and why the tag is
/// needed to tell the two apart in the path that has.
pub const SETTING: char = 'M';

/// The character that marks a RUN as a formula, once `styled_runs` has taken
/// the marker off.
///
/// It exists only between `styled_runs` and the page: it is never written into
/// the document's text, so it can never reach a reader and needs no entry in
/// `typeset::MARKERS`.
pub const RUN: char = '\u{17}';

/// Scaled points as PDF points.
pub fn pt(sp: Scaled) -> f64 {
    sp as f64 / 65536.0
}

/// Whether a marker's spec carries a formula.
pub fn is_setting(spec: &str) -> bool {
    spec.starts_with(SETTING)
}

/// The run text `styled_runs` hands the page for a formula's spec.
pub fn run(spec: &str) -> String {
    format!("{RUN}{spec}")
}

/// The setting a run carries, or `None` when the run is ordinary text.
pub fn run_setting(text: &str) -> Option<Setting> {
    let rest = text.strip_prefix(RUN)?;
    set::decode(rest.strip_prefix(SETTING)?)
}

/// What a formula's run measures, in PDF points, for the line breaker.
pub fn run_width(text: &str) -> Option<f64> {
    run_setting(text).map(|s| pt(s.width))
}

/// The glyphs of a setting, ready to be drawn: the character, its position
/// relative to the formula's reference point, and the size of the font it is
/// set in -- all in PDF points.
pub fn glyphs(s: &Setting) -> Vec<(String, f64, f64, f64)> {
    s.glyphs
        .iter()
        .map(|g| (g.ch.to_string(), pt(g.x), pt(g.y), pt(g.size)))
        .collect()
}

/// The rules of a setting -- fraction bars, overlines, the bar of a radical --
/// as `(x, y, width, height)` in PDF points, `y` being the bottom edge.
pub fn rules(s: &Setting) -> Vec<(f64, f64, f64, f64)> {
    s.rules
        .iter()
        .map(|r| (pt(r.x), pt(r.y), pt(r.width), pt(r.height)))
        .collect()
}

/// Set an mlist and flatten it, at a document type size in points.
///
/// The public seam for a test: it takes the formula already parsed, so it
/// exercises `mlist_to_hlist` and nothing else.
pub fn set_mlist(list: &[Noad], style: Style, size: f64) -> Option<Setting> {
    let fonts = fonts_at(size)?;
    let b = mlist::set(fonts, list, style);
    Some(set::flatten(&b, &|i| set::font_size(fonts, i)))
}

/// Read a formula from a source string, for a test or a caller that has the
/// formula and not a lexer.
///
/// The braces are made grouping characters first. INITEX leaves them ordinary
/// (`src/catcode.rs`), and a document reaches a formula only after something
/// -- plain.tex, or `src/latex/prelude.tex` -- has set them, so a formula read
/// under INITEX's own table would take `\frac{a}{b}` as five symbols.
pub fn parse_formula(source: &str) -> Result<Vec<Noad>, TexError> {
    let mut eng = Engine::new();
    eng.cats.set('{', crate::catcode::Cat::BeginGroup);
    eng.cats.set('}', crate::catcode::Cat::EndGroup);
    let mut lx = Lexer::new(source);
    parse::formula(&mut eng, &mut lx, parse::Stop::MathShift)
}

/// The families, loaded once per type size.
///
/// `find_font` runs `kpsewhich`, which is a process; a book with two thousand
/// formulas in it must not start twenty-four thousand of them. The set is
/// keyed by size and leaked, because a document has one type size and a batch
/// has a handful.
fn fonts_at(size: f64) -> Option<&'static font::MathFonts> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<u64, Option<&'static font::MathFonts>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = size.to_bits();
    let mut held = cache.lock().ok()?;
    if let Some(hit) = held.get(&key) {
        return *hit;
    }
    let loaded = font::MathFonts::load(size);
    let answer = match loaded.usable() {
        true => Some(&*Box::leak(Box::new(loaded))),
        false => None,
    };
    held.insert(key, answer);
    answer
}

/// The math environments a `\begin` can open, and whether they are display.
///
/// LaTeX's own, plus amsmath's, since a document that writes `align` means a
/// displayed formula whether or not the package is loaded here.
const ENVIRONMENTS: &[&str] = &[
    "equation",
    "equation*",
    "displaymath",
    "align",
    "align*",
    "aligned",
    "alignat",
    "alignat*",
    "gather",
    "gather*",
    "gathered",
    "multline",
    "multline*",
    "eqnarray",
    "eqnarray*",
    "math",
    BODY,
];

/// The environment that is not a formula but is where formulas become
/// possible: see the `\begin{document}` arm of [`lower_math`].
const BODY: &str = "document";

/// Whether `tok` opens a formula, and if so set it and write it into `out`.
///
/// Called on every token the lowerer reads, before the macro table is
/// consulted, for the reason colour and the faces are caught in the same
/// place: `src/latex/prelude.tex` defines `\(`, `\)`, `\[` and `\]` to expand
/// to nothing, so by the time a name reaches the macro table the formula's
/// delimiters have gone.
pub fn lower_math(
    eng: &mut Engine,
    size: f64,
    lx: &mut Lexer,
    tok: &Token,
    out: &mut Vec<Cmd>,
) -> Result<bool, TexError> {
    let (style, stop) = match tok {
        // `$` and `$$`. A document reaches here only where the character has
        // category 3: `\$` is an ordinary dollar sign and stays one.
        Token::Char(_, crate::catcode::Cat::MathShift) => match second_shift(eng, lx) {
            true => (DISPLAY_STYLE, parse::Stop::MathShift),
            false => (TEXT_STYLE, parse::Stop::MathShift),
        },
        Token::Cs(name) => match name.name() {
            "(" => (TEXT_STYLE, parse::Stop::Cs(")")),
            "[" => (DISPLAY_STYLE, parse::Stop::Cs("]")),
            "begin" => match environment(lx) {
                Some(BODY) => {
                    // `\begin{document}` is where a LaTeX document's TEXT
                    // starts, and the one point at which `$` can safely be
                    // made a math shift.
                    //
                    // INITEX leaves it Other (`src/catcode.rs`) and
                    // `src/latex/prelude.tex` -- which this may not edit --
                    // never sets it, so without this `$x$` in a LaTeX document
                    // is three characters. It cannot be set any earlier
                    // either: the prelude defines `\$` as the character `$`,
                    // and a definition read while the character is a math
                    // shift would make every `\$` in every book open a
                    // formula. Read here, after the preamble, `\$` still holds
                    // the ordinary dollar sign it was defined with and only
                    // the document's own `$` opens maths.
                    eng.cats.set('$', crate::catcode::Cat::MathShift);
                    return Ok(false);
                }
                Some(_) => {
                    consume_environment_name(eng, lx);
                    let list = parse::formula(eng, lx, parse::Stop::Cs("end"))?;
                    consume_end(eng, lx);
                    emit(out, &list, DISPLAY_STYLE, size, true);
                    return Ok(true);
                }
                None => return Ok(false),
            },
            _ => return Ok(false),
        },
        _ => return Ok(false),
    };
    let list = parse::formula(eng, lx, stop)?;
    // `$$…$$` closes with two math-shift characters, and the scan stopped at
    // the first.
    if style == DISPLAY_STYLE && stop == parse::Stop::MathShift {
        let _ = second_shift(eng, lx);
    }
    emit(out, &list, style, size, style == DISPLAY_STYLE);
    Ok(true)
}

/// Whether the NEXT token is another math shift, which is what makes `$$` a
/// display rather than an empty formula.
fn second_shift(eng: &mut Engine, lx: &mut Lexer) -> bool {
    match lx.next_token(&eng.cats) {
        Some(Token::Char(_, crate::catcode::Cat::MathShift)) => true,
        Some(t) => {
            lx.push_back(&[t]);
            false
        }
        None => false,
    }
}

/// The environment name after a `\begin`, when it names a formula.
///
/// Read as raw characters, the way `Lowerer::peek_environment_name` reads one
/// and for the same reason: the name is needed BEFORE deciding whether the
/// body is a formula. A `\begin` that arrived from a macro expansion has no
/// raw characters to read, so it is declined rather than guessed at.
fn environment(lx: &Lexer) -> Option<&'static str> {
    if !lx.pending.is_empty() {
        return None;
    }
    let chars = lx.chars();
    let mut at = lx.pos();
    while chars.get(at) == Some(&' ') {
        at += 1;
    }
    if chars.get(at) != Some(&'{') {
        return None;
    }
    at += 1;
    let mut name = String::new();
    while let Some(c) = chars.get(at) {
        if *c == '}' {
            return ENVIRONMENTS.iter().find(|e| **e == name).copied();
        }
        name.push(*c);
        at += 1;
        if name.len() > 32 {
            return None;
        }
    }
    None
}

/// Consume the `{name}` that [`environment`] only looked at.
fn consume_environment_name(eng: &mut Engine, lx: &mut Lexer) {
    while let Some(t) = lx.next_token(&eng.cats) {
        if matches!(t, Token::Char(_, crate::catcode::Cat::EndGroup)) {
            break;
        }
    }
}

/// Consume the `\end{name}` the formula scan handed back.
fn consume_end(eng: &mut Engine, lx: &mut Lexer) {
    match lx.next_token(&eng.cats) {
        Some(Token::Cs(c)) if c.name() == "end" => consume_environment_name(eng, lx),
        Some(t) => lx.push_back(&[t]),
        None => {}
    }
}

/// Write a set formula into the command stream.
///
/// A display goes on a line of its own and centred, which is what a display IS
/// -- the paragraph breaks either side are how the text stream says so, and
/// the centring marker is the one `\begin{center}` writes.
fn emit(out: &mut Vec<Cmd>, list: &[Noad], style: Style, size: f64, display: bool) {
    let Some(setting) = set_mlist(list, style, size) else {
        // No math fonts on this machine: the formula still says what it says,
        // so it is written as text rather than dropped.
        push(out, &set::plain(list));
        return;
    };
    let spelled = set::plain(list);
    let marked = format!("\u{1}{SETTING}{}\u{2}{spelled}\u{3}", set::encode(&setting));
    match display {
        true => push(
            out,
            &format!(
                "\n\n{}{marked}{}\n\n",
                crate::typeset::CENTRE,
                crate::typeset::CENTRE_END
            ),
        ),
        false => push(out, &marked),
    }
}

/// Append to the text run in progress, past the line directives that generate
/// no code -- `Lowerer::push_text`'s rule, which cannot be called from here.
fn push(out: &mut Vec<Cmd>, text: &str) {
    let mut at = out.len();
    while at > 0 && matches!(out[at - 1], Cmd::Line(_)) {
        at -= 1;
    }
    match at.checked_sub(1).and_then(|i| out.get_mut(i)) {
        Some(Cmd::Text(t)) => t.push_str(text),
        _ => out.push(Cmd::Text(text.to_string())),
    }
}
