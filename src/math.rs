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
//! The port is in six pieces, each named for the sections of `tex.web` it
//! carries:
//!
//! | module | `tex.web` | what it is |
//! |---|---|---|
//! | [`font`] | §699-§701 | the four families and their parameters |
//! | [`noad`] | §680-§698, §764 | the mlist, and the inter-element spacing table |
//! | [`parse`] | §1090-§1206 | reading a formula into an mlist |
//! | [`mlist`] | §704-§767 | `mlist_to_hlist`: the formula, as boxes |
//! | [`set`] | §619-§640 | where each glyph of it lands |
//! | [`align`] | §810, §1204-§1206 | a display's columns, and its equation number |
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
//! - The general alignment of §768-§800. `&` now lines a display's columns up
//!   by §810's rule and `\\` starts a row, but `\halign`'s preamble, `\span`,
//!   `\omit`, `\noalign` and `\tabskip` are not ported and cannot be: no path
//!   in this engine reaches `\halign` from a formula, and the preambles the
//!   environments use are written in package macros this port does not run.
//!   [`align`] says exactly what that costs.
//! - LaTeX's `equation` counter as a REGISTER. A counter is a `\count`
//!   (`src/latex/counters.rs`), a register is run-time state, and a formula
//!   is SET from what lowering read, before the document runs. So the
//!   typesetting run numbers a display from [`number::Numbering`], which
//!   follows the same counters while lowering from the macros that define
//!   them (`\the<counter>`, the `\cl@` reset lists); a `\setcounter` whose
//!   value is not a literal number, or a step made inside a macro this does
//!   not see, is not followed.
//! - The penalties of §761 and §767 are inserted -- `\binoppenalty` after a
//!   Bin and `\relpenalty` after a Rel -- but nothing reads them: a set
//!   formula reaches the paragraph breaker as ONE word, so no line breaks
//!   inside one.
//! - The ligature half of `make_ord` (§752-§753). Computer Modern's math
//!   families define no ligature programs, so it has nothing to do; the KERNS
//!   of the same routine are ported.
//! - `\mathcode` and `\delcode` assignments made BY A DOCUMENT. The tables in
//!   [`parse`] are INITEX's and plain.tex's, which is what `$x+y$` means;
//!   `\mathchardef` IS read from the engine, because `src/expand.rs` already
//!   carries it.

pub mod align;
pub mod font;
pub mod mlist;
pub mod noad;
pub mod number;
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

/// The same, with §767's penalties in a formula that is set inside a
/// paragraph.
///
/// §1194: `mlist_penalties := (mode > 0)`, which is true for a `$…$` in
/// running text and false for a display (§1199).
fn set_formula(fonts: &font::MathFonts, list: &[Noad], style: Style, display: bool) -> Setting {
    let b = match display {
        true => mlist::set(fonts, list, style),
        false => mlist::set_with_penalties(fonts, list, style),
    };
    set::flatten(&b, &|i| set::font_size(fonts, i))
}

/// Set a whole display: its rows lined up at shared column widths, and the
/// equation number `\eqno` put beside it.
///
/// One `Setting` per row, because a row is set as a display line of its own --
/// see [`align`] for why. `measure` is `\displaywidth` in points, which only
/// an equation number needs.
pub fn set_display(
    f: &parse::Formula,
    style: Style,
    size: f64,
    environment: &str,
    measure: f64,
) -> Option<Vec<Setting>> {
    Some(display_rows(
        fonts_at(size)?,
        f,
        &eqno(f),
        f.number_left,
        style,
        environment,
        measure,
    ))
}

/// The number a `\eqno` or `\leqno` put on `f`, beside its last row: §1204
/// places ONE display, so `$$…\eqno(3)$$` numbers the row it ends.
fn eqno(f: &parse::Formula) -> Vec<(usize, Vec<Noad>)> {
    f.number
        .iter()
        .map(|n| (f.rows.len().saturating_sub(1), n.clone()))
        .collect()
}

/// [`set_display`] in families already loaded, with each of `numbers` -- a
/// row and its number's mlist -- set beside the row it names, at the left
/// when `left`.
#[allow(clippy::too_many_arguments)]
fn display_rows(
    fonts: &font::MathFonts,
    f: &parse::Formula,
    numbers: &[(usize, Vec<Noad>)],
    left: bool,
    style: Style,
    environment: &str,
    measure: f64,
) -> Vec<Setting> {
    let widest = f.rows.iter().map(|r| r.len()).max().unwrap_or(1);
    let columns = align::columns(environment, widest);
    let mut boxes = align::rows(fonts, &f.rows, style, &columns);
    let z = (measure * 65536.0).round() as noad::Scaled;
    for (row, number) in numbers {
        let Some(at) = boxes.get_mut(*row) else {
            continue;
        };
        // §1199: the equation number is an mlist of its own, set at TEXT
        // style whatever style the display is in, and packed at its natural
        // width before §1204 places it -- on every row it numbers, since a
        // row of an amsmath `align` is a display line of its own.
        let a = mlist::set(fonts, number, TEXT_STYLE);
        let body = std::mem::take(at);
        *at = align::numbered(fonts, body, a, z, left);
    }
    boxes
        .iter()
        .map(|b| set::flatten(b, &|i| set::font_size(fonts, i)))
        .collect()
}

/// Read a formula from a source string, for a test or a caller that has the
/// formula and not a lexer.
///
/// The braces are made grouping characters first. INITEX leaves them ordinary
/// (`src/catcode.rs`), and a document reaches a formula only after something
/// -- plain.tex, or `src/latex/prelude.tex` -- has set them, so a formula read
/// under INITEX's own table would take `\frac{a}{b}` as five symbols.
pub fn parse_formula(source: &str) -> Result<Vec<Noad>, TexError> {
    Ok(read_formula(source)?.flat())
}

/// The same, keeping the rows, the columns and the equation number a display
/// can carry.
pub fn read_formula(source: &str) -> Result<parse::Formula, TexError> {
    let mut eng = Engine::new();
    eng.cats.set('{', crate::catcode::Cat::BeginGroup);
    eng.cats.set('}', crate::catcode::Cat::EndGroup);
    eng.cats.set('&', crate::catcode::Cat::AlignTab);
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
///
/// The formula is READ here, because reading it expands macros against the
/// engine and consumes the input. It is not SET here: see [`Deferred`].
pub fn lower_math(
    eng: &mut Engine,
    size: f64,
    measure: f64,
    lx: &mut Lexer,
    tok: &Token,
    out: &mut Vec<Cmd>,
    deferred: &mut Formulas,
) -> Result<bool, TexError> {
    // A sectioning command or a `\setcounter` moves the counters an equation
    // number reads; it is followed here and left for the lowerer to act on.
    deferred.numbering.watch(eng, lx, tok);
    let Some((f, style, display, environment)) = read(eng, lx, tok)? else {
        return Ok(false);
    };
    let numbers = numbered(eng, out, &mut deferred.numbering, &f, environment);
    let left = number::left(eng);
    emit(
        out,
        deferred,
        f,
        numbers,
        left,
        style,
        size,
        display,
        environment,
        measure,
    );
    Ok(true)
}

/// Number `f`'s places as latex.ltx does -- [`number::places`], the question
/// the label pass asks too -- and write into `out` what each `\label` in it
/// names. Returns each numbered row and the number set beside it.
///
/// The labels go in as `typeset::REF_VALUE` spans: the number, then the
/// label, which `typeset::unit_numbers` reads as `\@currentlabel` followed by
/// a `\label`. A place with no number writes its labels alone and they name
/// whatever was current, as LaTeX's do.
fn numbered(
    eng: &Engine,
    out: &mut Vec<Cmd>,
    numbering: &mut number::Numbering,
    f: &parse::Formula,
    environment: &str,
) -> Vec<(usize, String)> {
    use crate::typeset::{ref_mark, REF_LABEL, REF_VALUE};
    let mut numbers = Vec::new();
    let mut marks = String::new();
    for place in number::places(f, environment) {
        let shown = match place.number {
            number::Number::Step => {
                numbering.step(eng, "equation");
                let n = numbering.the(eng, "equation");
                marks.push_str(&ref_mark(REF_VALUE, &n));
                Some(format!("({n})"))
            }
            number::Number::Tag(tag, starred) => {
                let t = number::spelled(tag);
                marks.push_str(&ref_mark(REF_VALUE, &t));
                Some(match starred {
                    true => t,
                    false => format!("({t})"),
                })
            }
            number::Number::Unnumbered => None,
        };
        for key in place.labels {
            marks.push_str(&ref_mark(REF_LABEL, &number::spelled(key)));
        }
        // A `\eqno` the document wrote is the number already.
        if let (Some(shown), None) = (shown, &f.number) {
            numbers.push((place.row, shown));
        }
    }
    if !marks.is_empty() {
        push(out, &marks);
    }
    numbers
}

/// Whether `tok` opens a formula, and if so read it and set nothing -- the
/// label pass's [`lower_math`].
///
/// The pass (`src/latex/aux.rs`) collects the `.aux` and has no page, but it
/// must still read a formula exactly as the run that sets it does: `\frac`,
/// `\sqrt`, `\left` and the rest are the math parser's, not macros, and a pass
/// that met them as text stopped at `! Undefined control sequence \frac.` and
/// wrote no `.aux` at all. Latex's `.aux` is unaffected by what a formula
/// draws; what it does take from one is the `\label`s inside it, and the
/// equation number a numbered display steps (ltmath.dtx's `equation`,
/// amsmath's `align`, `gather`, `multline`, and `eqnarray`). Those are put
/// back in front of the input as `\refstepcounter{equation}` and `\label{..}`,
/// in the order latex.ltx meets them, for the pass to run as it runs any other.
/// Which places step is [`number::places`]'s answer, the one the typesetting
/// run numbers its displays by.
pub fn pass_over_math(eng: &mut Engine, lx: &mut Lexer, tok: &Token) -> Result<bool, TexError> {
    let Some((f, _, _, environment)) = read(eng, lx, tok)? else {
        return Ok(false);
    };
    let equation: Vec<Token> = "equation"
        .chars()
        .map(|c| Token::Char(c, crate::catcode::Cat::Letter))
        .collect();
    let mut back = Vec::new();
    for place in number::places(&f, environment) {
        match place.number {
            number::Number::Step => call(&mut back, "refstepcounter", &equation),
            // amsmath's `\tag` makes the tag what a `\label` writes, and
            // steps nothing.
            number::Number::Tag(tag, _) => {
                back.push(Token::cs("def"));
                back.push(Token::cs("@currentlabel"));
                back.push(Token::Char('{', crate::catcode::Cat::BeginGroup));
                back.extend_from_slice(tag);
                back.push(Token::Char('}', crate::catcode::Cat::EndGroup));
            }
            number::Number::Unnumbered => {}
        }
        for key in place.labels {
            call(&mut back, "label", key);
        }
    }
    lx.push_back(&back);
    Ok(true)
}

/// `\name{arg}`, as tokens, onto `back`.
fn call(back: &mut Vec<Token>, name: &str, arg: &[Token]) {
    back.push(Token::cs(name));
    back.push(Token::Char('{', crate::catcode::Cat::BeginGroup));
    back.extend_from_slice(arg);
    back.push(Token::Char('}', crate::catcode::Cat::EndGroup));
}

/// The formula `tok` opens, read: what [`lower_math`] sets and
/// [`pass_over_math`] does not. `None` when `tok` opens none.
fn read(
    eng: &mut Engine,
    lx: &mut Lexer,
    tok: &Token,
) -> Result<Option<(parse::Formula, Style, bool, &'static str)>, TexError> {
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
                    return Ok(None);
                }
                Some(name) => {
                    consume_environment_name(eng, lx);
                    let f = parse::formula(eng, lx, parse::Stop::Cs("end"))?;
                    consume_end(eng, lx);
                    return Ok(Some((f, DISPLAY_STYLE, true, name)));
                }
                None => return Ok(None),
            },
            _ => return Ok(None),
        },
        _ => return Ok(None),
    };
    let f = parse::formula(eng, lx, stop)?;
    // `$$…$$` closes with two math-shift characters, and the scan stopped at
    // the first.
    if style == DISPLAY_STYLE && stop == parse::Stop::MathShift {
        let _ = second_shift(eng, lx);
    }
    Ok(Some((f, style, style == DISPLAY_STYLE, "")))
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

/// Write a formula into the command stream, its setting left as a slot.
///
/// A display goes on a line of its own and centred, which is what a display IS
/// -- the paragraph breaks either side are how the text stream says so, and
/// the centring marker is the one `\begin{center}` writes. A display with
/// COLUMNS in it -- an `align`, an `eqnarray` -- writes one such line per row,
/// every one of them packed to the same width, which is what lines the columns
/// up: see [`align`] for why the rows are not one vertical box.
///
/// Everything but each row's setting is written NOW, so the text the lowerer
/// reads back -- `push_text_char` looks at the last character of the run for
/// a ligature's first half and a face marker -- is what it was when formulas
/// were set in place: a marker ends in `\u{3}` (or a display's `\n`) whatever
/// its spec holds, and whether there is a marker at all depends only on the
/// families loading, which is answered here.
#[allow(clippy::too_many_arguments)]
fn emit(
    out: &mut Vec<Cmd>,
    deferred: &mut Formulas,
    f: parse::Formula,
    numbers: Vec<(usize, String)>,
    left: bool,
    style: Style,
    size: f64,
    display: bool,
    environment: &'static str,
    measure: f64,
) {
    let Some(fonts) = fonts_at(size) else {
        // No math fonts on this machine: the formula still says what it says,
        // so it is written as text rather than dropped.
        push(out, &set::plain(&f.flat()));
        return;
    };
    let index = deferred.next_index();
    // LaTeX's numbers when the display has any, else the `\eqno` it wrote:
    // each with its mlist and as a reader of the text gets it.
    let (numbers, left): (Vec<(usize, Vec<Noad>, String)>, bool) = match numbers.is_empty() {
        true => (
            eqno(&f)
                .into_iter()
                .map(|(row, n)| {
                    let spelled = set::plain(&n);
                    (row, n, spelled)
                })
                .collect(),
            f.number_left,
        ),
        false => (
            numbers
                .into_iter()
                // Joined to the row with no space, as a `\eqno` is: a space
                // would split the marker into two words on the way to the
                // page.
                .map(|(row, n)| (row, number::mlist(&n), n))
                .collect(),
            left,
        ),
    };
    // The ordinary case, and the only one before `&` and `\eqno` meant
    // anything: one row, one column, no number.
    if f.is_single() && numbers.is_empty() {
        push_setting(out, &slot(index, 0), &set::plain(&f.flat()), display);
    } else {
        // `display_rows` sets one line per row.
        for (at, row) in f.rows.iter().enumerate() {
            let mut spelled: String = row.iter().map(|cell| set::plain(cell)).collect();
            // The equation number goes beside the row it was placed beside,
            // so a reader gets it where the page draws it.
            for (_, _, number) in numbers.iter().filter(|(r, _, _)| *r == at) {
                spelled.push_str(number);
            }
            push_setting(out, &slot(index, at), &spelled, display);
        }
    }
    deferred.push(Deferred {
        formula: f,
        numbers: numbers.into_iter().map(|(row, n, _)| (row, n)).collect(),
        left,
        fonts,
        style,
        display,
        environment,
        measure,
    });
}

/// A formula read while the document was lowered and not yet set.
///
/// Setting a formula is `mlist_to_hlist` and a walk of the boxes it makes,
/// and on a document dense with mathematics it was the larger part of
/// lowering. It reads nothing the lowerer goes on to change: the mlist is
/// read once and finished, and the rest of what layout depends on is what
/// this captures -- the style, the families at the type size in force (its
/// fontdimens are every number §700-§767 uses), whether it is a display
/// (§1194's penalties), the environment (§810's columns) and the measure
/// (§1204's equation number). Nothing the lowerer does reads a spec back, so
/// the settings are made after lowering, on the pool when there are enough
/// of them, and written into their slots by [`Formulas::fill`].
pub struct Deferred {
    formula: parse::Formula,
    /// The numbered rows, each with the mlist set beside it, and whether at
    /// the left: fixed where the formula was read, so the setting is the
    /// same whichever thread makes it.
    numbers: Vec<(usize, Vec<Noad>)>,
    left: bool,
    fonts: &'static font::MathFonts,
    style: Style,
    display: bool,
    environment: &'static str,
    measure: f64,
}

impl Deferred {
    /// The encoded spec of each row, in the order [`emit`] wrote the slots.
    fn specs(&self) -> Vec<String> {
        let f = &self.formula;
        let settings = match f.is_single() && self.numbers.is_empty() {
            true => vec![set_formula(self.fonts, &f.flat(), self.style, self.display)],
            false => display_rows(
                self.fonts,
                f,
                &self.numbers,
                self.left,
                self.style,
                self.environment,
                self.measure,
            ),
        };
        settings.iter().map(set::encode).collect()
    }
}

/// How many formulas go to a worker at a time. A document with fewer is set
/// on the calling thread, at the join, as is the part-batch at the end of
/// any document: handing out a handful costs more than setting them.
const BATCH: usize = 32;

/// One batch's settings: the encoded spec of each row of each formula in it.
/// Shared, because a [`Formulas::tap`] reads them as they come back while
/// the join still has to write them into the commands.
pub type Settings = std::sync::Arc<Vec<Vec<String>>>;

/// One batch's settings, the index of its first formula, or the panic that
/// stopped it, to be raised where the formulas are joined.
type Batch = (usize, std::thread::Result<Settings>);

/// Told of each batch as it comes back: the index of its first formula and its
/// settings. See [`Formulas::tap`].
pub type Tap = std::sync::Arc<dyn Fn(usize, Settings) + Send + Sync>;

/// The settings of the batches back so far, by formula index.
///
/// A batch handed out holds [`BATCH`] formulas and starts at a multiple of
/// it: `Formulas::push` sends exactly when that many are pending, and nothing
/// else empties the pending list before the join.
#[derive(Default)]
pub struct SettingsTable {
    batches: Vec<Option<Settings>>,
}

impl SettingsTable {
    pub fn insert(&mut self, first: usize, settings: Settings) {
        let at = first / BATCH;
        if self.batches.len() <= at {
            self.batches.resize(at + 1, None);
        }
        self.batches[at] = Some(settings);
    }

    /// The spec of row `row` of formula `index`, if its batch is back.
    pub fn get(&self, index: usize, row: usize) -> Option<&str> {
        let batch = self.batches.get(index / BATCH)?.as_ref()?;
        batch.get(index % BATCH)?.get(row).map(String::as_str)
    }
}

/// The formulas the lowerer has read and not yet seen set.
///
/// A formula's setting depends only on what its [`Deferred`] holds, which is
/// fixed when the formula is read, so it can be set while the lowerer goes
/// on reading the rest of the document. Formulas are collected into batches
/// of [`BATCH`] and each full batch is handed to the rayon pool as soon as it
/// fills; [`Formulas::fill`] sets what is left on the calling thread, waits
/// for the batches still out and writes every setting into the slot its
/// index names. With one thread in the pool -- `TEXRS_THREADS=1` -- nothing
/// is handed out and every formula is set at the join, on the caller.
///
/// Setting reports nothing: it is arithmetic on the mlist and the families'
/// fontdimens, so there are no diagnostics whose order the overlap could
/// change. A panic in a worker is carried back and raised at the join, as a
/// parallel iterator would raise it.
#[derive(Default)]
pub struct Formulas {
    /// Read and not yet handed to a worker; the first is formula
    /// `read - pending.len()`.
    pending: Vec<Deferred>,
    /// How many have been read: the index the next one's slot names.
    read: usize,
    /// How many batches are out.
    sent: usize,
    /// Where the batches report back, opened with the first one sent.
    results: Option<(
        std::sync::mpsc::Sender<Batch>,
        std::sync::mpsc::Receiver<Batch>,
    )>,
    /// Where each batch is ALSO reported as it comes back, for a reader that
    /// fills slots before the join does: see [`Formulas::tap`].
    tap: Option<Tap>,
    /// LaTeX's counters, followed in reading order: the equation numbers are
    /// decided as the formulas are read, never on a worker.
    numbering: number::Numbering,
}

impl Formulas {
    /// The index the next formula's slot names.
    fn next_index(&self) -> usize {
        self.read
    }

    fn push(&mut self, d: Deferred) {
        self.pending.push(d);
        self.read += 1;
        if self.pending.len() == BATCH && rayon::current_num_threads() > 1 {
            self.send();
        }
    }

    /// Tell `tap` of every batch handed out from now on, with the index of
    /// its first formula, as soon as it is set. The join is not
    /// affected: it still receives every batch and fills every slot.
    pub fn tap(&mut self, tap: Tap) {
        self.tap = Some(tap);
    }

    /// Hand the pending batch to the pool.
    fn send(&mut self) {
        let batch = std::mem::replace(&mut self.pending, Vec::with_capacity(BATCH));
        let first = self.read - batch.len();
        let (tx, _) = self.results.get_or_insert_with(std::sync::mpsc::channel);
        let tx = tx.clone();
        let tap = self.tap.clone();
        self.sent += 1;
        rayon::spawn(move || {
            let specs = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Settings::new(batch.iter().map(Deferred::specs).collect())
            }));
            // The join fills every slot whatever the tap made of it.
            if let (Some(tap), Ok(specs)) = (&tap, &specs) {
                tap(first, Settings::clone(specs));
            }
            // The receiver is gone only when lowering failed and its formulas
            // were abandoned.
            let _ = tx.send((first, specs));
        });
    }

    /// Set every formula not yet set and write each setting into its slot.
    ///
    /// The settings are independent of one another and of the order they
    /// are made in, and each lands in the slot its index names, so the
    /// commands come out the same whatever the thread count.
    pub fn fill(mut self, cmds: &mut [Cmd]) {
        if self.read == 0 {
            return;
        }
        let tail = std::mem::take(&mut self.pending);
        let tail_first = self.read - tail.len();
        let tail_specs: Vec<Vec<String>> = tail.iter().map(Deferred::specs).collect();
        let mut batches = SettingsTable::default();
        if let Some((tx, rx)) = self.results.take() {
            drop(tx);
            for _ in 0..self.sent {
                let (first, batch) = receive(&rx);
                let batch = batch.unwrap_or_else(|p| std::panic::resume_unwind(p));
                batches.insert(first, batch);
            }
        }
        let spec = |index: usize, row: usize| match index.checked_sub(tail_first) {
            Some(at) => tail_specs.get(at)?.get(row).map(String::as_str),
            None => batches.get(index, row),
        };
        fill_cmds(cmds, &spec);
        // A formula-heavy document's settings are tens of thousands of small
        // allocations, and freeing them is work nothing downstream waits on.
        match self.sent > 0 {
            true => rayon::spawn(move || drop((batches, tail_specs, tail))),
            false => drop((batches, tail_specs, tail)),
        }
    }
}

/// The next report back from a job handed to the pool. On a pool thread -- a
/// caller inside `install`, as the tests are -- the wait runs other jobs, which
/// may be the one it is waiting for; elsewhere it blocks.
pub(crate) fn receive<T>(rx: &std::sync::mpsc::Receiver<T>) -> T {
    loop {
        if let Ok(b) = rx.try_recv() {
            return b;
        }
        if rayon::yield_now() != Some(rayon::Yield::Executed) {
            return rx.recv().expect("every job handed out reports back");
        }
    }
}

/// What opens a slot inside a formula's marker spec. An encoded spec starts
/// with the width, a digit or a minus sign, so `#` there is never a setting.
const SLOT: &str = "\u{1}M#";

/// The slot for row `row` of the `index`-th deferred formula.
fn slot(index: usize, row: usize) -> String {
    format!("#{index}.{row}")
}

/// Where a formula's spec is looked up: by its index and the row.
type SpecOf<'s> = dyn Fn(usize, usize) -> Option<&'s str> + 's;

fn fill_cmds<'s>(cmds: &mut [Cmd], specs: &SpecOf<'s>) {
    for cmd in cmds {
        match cmd {
            Cmd::Text(t) => fill_text(t, specs),
            Cmd::Color { body, .. } | Cmd::Group { body, .. } | Cmd::Loop { body, .. } => {
                fill_cmds(body, specs)
            }
            Cmd::IfNum {
                then_branch,
                else_branch,
                ..
            }
            | Cmd::IfOdd {
                then_branch,
                else_branch,
                ..
            } => {
                fill_cmds(then_branch, specs);
                fill_cmds(else_branch, specs);
            }
            _ => {}
        }
    }
}

/// Replace each slot in `t` with the spec it names. Text that only looks like
/// a slot -- an index no formula has -- is left as it was.
fn fill_text<'s>(t: &mut String, specs: &SpecOf<'s>) {
    if t.contains(SLOT) {
        *t = filled(t, specs);
    }
}

/// `t` with each slot replaced by the spec it names, as [`Formulas::fill`]
/// writes it into a command.
pub fn filled<'s>(t: &str, specs: &SpecOf<'s>) -> String {
    let mut out = String::with_capacity(t.len() * 2);
    let mut rest = t;
    while let Some(at) = rest.find(SLOT) {
        // Keep `\u{1}M`, drop the `#`.
        out.push_str(&rest[..at + SLOT.len() - 1]);
        let after = &rest[at + SLOT.len()..];
        let (named, end) = slot_named(after);
        match named.and_then(|(i, r)| specs(i, r)) {
            Some(spec) => {
                out.push_str(spec);
                rest = &after[end..];
            }
            None => {
                out.push('#');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Where in `t` the first slot opens whose spec `specs` does not have: how
/// far [`filled`] writes what the join will. `None` when it has every one.
pub fn first_unsettled<'s>(t: &str, specs: &SpecOf<'s>) -> Option<usize> {
    let mut from = 0;
    while let Some(at) = t[from..].find(SLOT) {
        let opens = from + at;
        let (named, end) = slot_named(&t[opens + SLOT.len()..]);
        if named.and_then(|(i, r)| specs(i, r)).is_none() {
            return Some(opens);
        }
        from = opens + SLOT.len() + end;
    }
    None
}

/// The formula and row a slot names, read from just after its opening, and
/// where the name ends.
fn slot_named(after: &str) -> (Option<(usize, usize)>, usize) {
    let end = after.find('\u{2}').unwrap_or(after.len());
    let named = after[..end]
        .split_once('.')
        .and_then(|(i, r)| Some((i.parse::<usize>().ok()?, r.parse::<usize>().ok()?)));
    (named, end)
}

/// One formula, as the marker the typesetter reads.
fn push_setting(out: &mut Vec<Cmd>, spec: &str, spelled: &str, display: bool) {
    let marked = format!("\u{1}{SETTING}{spec}\u{2}{spelled}\u{3}");

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
