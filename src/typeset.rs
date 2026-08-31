//! The smallest honest stomach: text and font metrics into DVI pages.
//!
//! texrs could read a document and say what its words were, and could read and
//! write the DVI format, and had nothing joining the two -- `dvi::Writer` was
//! never called outside its own tests. So a book "ran" and produced no page.
//! This is the join: measure each character in a real font, break the text into
//! lines at a measure, stack the lines down a page, and ship the result.
//!
//! What this is NOT is `tex.web`'s stomach. TeX breaks a paragraph by looking at
//! every feasible sequence of breakpoints and minimising total badness
//! (§813-§890); this takes the first break that fits, which is what every
//! word processor before TeX did and what TeX was written to improve on. There
//! is no hyphenation, no glue stretching or shrinking, no page-breaking by
//! penalties, no maths, no boxes a document can nest. A paragraph set here and
//! the same paragraph set by tex will not agree line for line.
//!
//! It is a page you can open, which is the difference between a document that
//! produces nothing and one that produces something imperfect.

use crate::dvi::Writer;
use crate::tfm::Tfm;

/// DVI's unit: 1 sp, of which there are 65536 to the printer's point.
const SP: f64 = 65536.0;

/// The page and paragraph shape, in points.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// Text width, TeX's `\hsize`.
    pub measure: f64,
    /// Text height, TeX's `\vsize`.
    pub height: f64,
    /// Distance between baselines, TeX's `\baselineskip`.
    pub leading: f64,
    /// Where the text starts, from the top-left of the page.
    pub margin: f64,
    /// The font's design size, at which its metrics are stated.
    pub size: f64,
}

impl Default for Layout {
    fn default() -> Self {
        // plain.tex's own page: 6.5in by 8.9in of text, 12pt leading, 1in
        // margins. A reader comparing against a tex run should not first have
        // to account for a different page.
        Self {
            measure: 469.75,
            height: 643.20,
            leading: 12.0,
            margin: 72.0,
            size: 10.0,
        }
    }
}

/// The paper the corpus is set on, and the only paper `pdf::Page` makes: 8.5
/// by 11 inches, in PDF's points.
const PAPER_WIDTH: f64 = 612.0;
const PAPER_HEIGHT: f64 = 792.0;

/// PDF's point is 1/72in; TeX's is 1/72.27in, and every dimension a LaTeX
/// preamble states is in TeX's. The page they land on is 612 by 792 of PDF's,
/// so the two cannot be used interchangeably. Measured, in the content stream
/// of the lualatex-built scifi2/docs/book.pdf: every body line begins at
/// x=68.4, which is 0.95in of PDF points and not of TeX's 68.66.
const BP_PER_PT: f64 = 72.0 / 72.27;

impl Layout {
    /// Take the type size from `\documentclass[11pt]{extreport}`, and with it
    /// the leading LaTeX pairs with that size.
    ///
    /// Every book in the corpus states a size and texrs set all of them at
    /// plain TeX's 10pt on 12pt leading regardless, so an 11pt book got 53
    /// lines on a page where lualatex gives it 48 -- and came out short by
    /// that ratio.
    pub fn absorb_class_options(&mut self, options: &str) {
        for option in options.split(',') {
            let Some(size) = option
                .trim()
                .strip_suffix("pt")
                .and_then(|n| n.parse::<f64>().ok())
                .filter(|size| *size > 0.0)
            else {
                continue;
            };
            self.size = size * BP_PER_PT;
            self.leading = normal_leading(size) * BP_PER_PT;
        }
    }

    /// Take the margins from `\usepackage[margin=0.95in]{geometry}`, and with
    /// them the measure and the text height they leave on the paper.
    ///
    /// Only `margin`, which sets all four sides at once: that is what pandoc
    /// writes and what every book in the corpus asks for. `left`, `top` and
    /// their siblings can each differ, and a `Layout` has ONE margin for all
    /// four sides, so honouring them here would put the text somewhere the
    /// document did not ask for rather than leave it where it was.
    pub fn absorb_geometry_options(&mut self, options: &str) {
        for option in options.split(',') {
            let Some((key, value)) = option.split_once('=') else {
                continue;
            };
            if key.trim() != "margin" {
                continue;
            }
            let Some(margin) = dimen_bp(value.trim()) else {
                continue;
            };
            self.margin = margin;
            self.measure = PAPER_WIDTH - 2.0 * margin;
            self.height = PAPER_HEIGHT - 2.0 * margin;
        }
    }
}

/// The leading LaTeX sets a type size on, taken from the class option files
/// that decide it: `size10.clo:48` sets 10pt on 12, `size11.clo:48` sets 11pt
/// on 13.6, `size12.clo:48` sets 12pt on 14.5, and extsizes' own
/// `size8/9/14/17/20.clo:12` carry the sizes `extreport` adds. A size no file
/// names is set on 1.2 of itself, which is what those files come to and what
/// TeX's `\baselineskip` assumes.
fn normal_leading(size: f64) -> f64 {
    let named = [
        (8.0, 9.5),
        (9.0, 11.0),
        (10.0, 12.0),
        (11.0, 13.6),
        (12.0, 14.5),
        (14.0, 17.0),
        (17.0, 22.0),
        (20.0, 25.0),
    ];
    match named.iter().find(|(pt, _)| *pt == size) {
        Some((_, on)) => *on,
        None => size * 1.2,
    }
}

/// A LaTeX dimension -- `0.95in`, `25mm`, `72bp` -- in PDF points.
///
/// Every TeX unit is two letters, so the suffix that matches is the only one
/// that can.
fn dimen_bp(text: &str) -> Option<f64> {
    let units = [
        ("in", 72.0),
        ("bp", 1.0),
        ("pt", BP_PER_PT),
        ("pc", 12.0 * BP_PER_PT),
        ("mm", 72.0 / 25.4),
        ("cm", 72.0 / 2.54),
    ];
    let (unit, per_unit) = units.iter().find(|(unit, _)| text.ends_with(unit))?;
    let number: f64 = text.strip_suffix(unit)?.trim().parse().ok()?;
    Some(number * per_unit)
}

/// Set `text` in `font`, and return a DVI file.
///
/// The font is named in the DVI by `font_name` so a driver can find it; the
/// metrics come from the `.tfm` the caller opened, which is the same file the
/// driver will use to place the characters.
pub fn to_dvi(text: &str, font: &Tfm, font_name: &str, layout: &Layout) -> Vec<u8> {
    // The single-font path, kept for callers that have one .tfm and want it
    // used literally. Everything else goes through the chain.
    let chain = FontChain {
        fonts: vec![Loaded {
            tfm: font.clone(),
            name: font_name.to_string(),
        }],
        map: Vec::new(),
    };
    to_dvi_chain(text, &chain, layout)
}

/// Set `text` through a font chain, falling back per glyph.
pub fn to_dvi_chain(text: &str, chain: &FontChain, layout: &Layout) -> Vec<u8> {
    let mut w = Writer::new("texrs");
    let at = (layout.size * SP) as i32;
    for (i, f) in chain.fonts.iter().enumerate() {
        // The design size and checksum have to match the .tfm or a driver
        // refuses the file: they are how it checks it has the font the document
        // was set in.
        w.define_font(i as u32, &f.name, at, f.tfm.checksum, at);
    }

    let lines = break_lines_chain(text, chain, layout);
    let per_page = ((layout.height / layout.leading).floor() as usize).max(1);

    for (page, chunk) in lines.chunks(per_page).enumerate() {
        let counts = [page as i32 + 1, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        w.begin_page(counts);
        let mut current = usize::MAX;
        let mut baseline = layout.margin + layout.leading;
        for line in chunk {
            // Each line starts at the left margin, on its own baseline. DVI is
            // relative, so the position is pushed and popped rather than
            // tracked: a driver reading it never has to know where the last
            // line ended.
            w.push();
            w.down((baseline * SP) as i32);
            w.right((layout.margin * SP) as i32);
            set_line(&mut w, line, chain, layout, &mut current);
            w.pop();
            baseline += layout.leading;
        }
        w.end_page();
    }
    w.finish()
}

/// Put one line's characters down, switching fonts as the chain requires.
fn set_line(w: &mut Writer, line: &str, chain: &FontChain, layout: &Layout, current: &mut usize) {
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        // Colour arrives in the text stream as U+0001 r,g,b U+0002 ... U+0003,
        // which the runtime writes where `\textcolor` was. DVI has no colour of
        // its own; a driver is told through a `\special`, and this pair is the
        // one dvipdfmx and dvips both read.
        if ch == '\u{1}' {
            let mut spec = String::new();
            for c in chars.by_ref() {
                if c == '\u{2}' {
                    break;
                }
                spec.push(c);
            }
            let parts: Vec<&str> = spec.split(',').collect();
            if parts.len() == 3 {
                w.special(&format!(
                    "color push rgb {} {} {}",
                    parts[0], parts[1], parts[2]
                ));
            }
            continue;
        }
        if ch == '\u{3}' {
            w.special("color pop");
            continue;
        }
        // A face marker names a font this path does not have: the chain is
        // Computer Modern and its fallbacks, chosen by which glyph is where,
        // not by what the document asked for. Skipping it is not just a
        // refusal to honour it -- the code character after U+000E is a LETTER,
        // so leaving the pair in the stream sets an `m` in the middle of every
        // \texttt in the book.
        if ch == FACE_PUSH {
            let _ = chars.next();
            continue;
        }
        if ch == FACE_POP {
            continue;
        }
        if let Some((f, slot)) = chain.resolve(ch) {
            // A font switch is an op in the file, so it is emitted only when the
            // font actually changes -- one per run of characters, not one per
            // character.
            if *current != f {
                w.font(f as u32);
                *current = f;
            }
            let width = chain.fonts[f]
                .tfm
                .char(slot)
                .map(|m| m.width)
                .unwrap_or(0.0);
            w.set_char(u32::from(slot), (width * layout.size * SP) as i32);
            continue;
        }
        // No font in the chain has it. The stand-in is set in the primary font
        // rather than dropped: a glyph that vanishes takes the meaning of the
        // line with it.
        let Some(text) = FontChain::approximate(ch) else {
            continue;
        };
        if *current != 0 {
            w.font(0);
            *current = 0;
        }
        for b in text.bytes() {
            let width = chain.fonts[0].tfm.char(b).map(|m| m.width).unwrap_or(0.0);
            w.set_char(u32::from(b), (width * layout.size * SP) as i32);
        }
    }
}

/// Break `text` into lines that fit the measure.
///
/// First fit, not best fit: a word is added while it still fits and starts a new
/// line when it does not. `tex.web` §813 does far better -- it considers every
/// feasible set of breakpoints for the whole paragraph at once -- and the
/// difference is visible as a raggeder right edge here.
pub fn break_lines(text: &str, font: &Tfm, layout: &Layout) -> Vec<String> {
    let chain = FontChain {
        fonts: vec![Loaded {
            tfm: font.clone(),
            name: String::from("cmr10"),
        }],
        map: Vec::new(),
    };
    break_lines_chain(text, &chain, layout)
}

/// The same, measuring through a font chain.
pub fn break_lines_chain(text: &str, chain: &FontChain, layout: &Layout) -> Vec<String> {
    use rayon::prelude::*;
    // Paragraph-parallel. A paragraph is broken independently of its
    // neighbours: the measure is the same for all of them, and no state crosses
    // the blank line between two, so this fans out with nothing to
    // synchronise. `map` keeps them in order, which matters -- a book whose
    // paragraphs arrived in completion order would be a different book.
    let paras: Vec<&str> = text.split("\n\n").collect();
    paras
        .par_iter()
        .map(|para| break_paragraph(para, chain, layout))
        .reduce(Vec::new, |mut acc, mut more| {
            acc.append(&mut more);
            acc
        })
}

/// One paragraph, first-fit.
fn break_paragraph(para: &str, chain: &FontChain, layout: &Layout) -> Vec<String> {
    // A code listing is already broken, by the author. Its lines are what the
    // program is; the measure has no say in where they end.
    if let Some(code) = listing_lines(para) {
        return code.map(str::to_string).collect();
    }
    let space = chain.width(' ', layout.size);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut width = 0.0f64;
    for word in para.split_whitespace() {
        let ww = chain.width_of(word, layout.size);
        let need = match line.is_empty() {
            true => ww,
            false => width + space + ww,
        };
        if !line.is_empty() && need > layout.measure {
            lines.push(std::mem::take(&mut line));
            width = ww;
            line.push_str(word);
            continue;
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
        width = need;
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Find a `.tfm` by name, asking the TeX installation first.
///
/// `kpsewhich` is how every TeX program answers this question, so asking it
/// gets the same answer the driver will get when it goes looking for the same
/// font. The fixed paths are the fallback for a machine without it.
pub fn find_font(name: &str) -> Option<std::path::PathBuf> {
    let file = format!("{name}.tfm");
    if let Ok(out) = std::process::Command::new("kpsewhich").arg(&file).output() {
        if out.status.success() {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !p.is_empty() && std::path::Path::new(&p).exists() {
                return Some(std::path::PathBuf::from(p));
            }
        }
    }
    for root in [
        "/usr/local/texlive",
        "/Library/TeX/texbin/../../texmf-dist",
        "/usr/share/texmf-dist",
    ] {
        let hit = std::process::Command::new("find")
            .args([root, "-name", &file, "-maxdepth", "8"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string()
            })
            .filter(|p| !p.is_empty());
        if let Some(p) = hit {
            return Some(std::path::PathBuf::from(p));
        }
    }
    None
}

/// A font and the name a DVI file calls it by.
pub struct Loaded {
    pub tfm: Tfm,
    pub name: String,
}

/// A chain of fonts, tried in order for each character.
///
/// This is `luaotfload.add_fallback` in a TFM world, and it is the reason the
/// publication scripts required LuaTeX: a per-glyph fallback fixes missing
/// arrows, box drawing and symbols in EVERY context, verbatim code included,
/// and no other engine offered one. A `.tfm` addresses 256 slots, so a chain
/// here is not "another Unicode font" but another 256-slot font plus the map
/// saying which character lives in which of its slots.
///
/// Resolution order per character: the primary font's own map, then each
/// fallback in turn, then an ASCII stand-in for the shapes Computer Modern
/// simply does not have. What it never does is drop the character silently --
/// a glyph that vanishes takes the meaning of a line with it, and a line that
/// reads `a -- b` where the source said `a → b` is worse than one that says so.
pub struct FontChain {
    pub fonts: Vec<Loaded>,
    /// Where a character lives: which font of the chain, and which slot.
    map: Vec<(char, usize, u8)>,
}

/// Computer Modern's own positions for the characters these documents use.
///
/// `cmr10` is the text font; `cmsy10` is the symbol font and carries the
/// arrows, the set operators AND the section and paragraph marks -- those last
/// two are not in the text font, which is the mistake this table was written
/// with the first time: `§` was pointed at cmr10 slot 120 and printed an `x`.
///
/// The slots are read off what `tex` itself emits for `\S`, `\P`,
/// `\rightarrow`, `\leftarrow`, `\cup` and `\times` -- `dvitype` on its
/// output names the character each one set. A wrong slot here prints a wrong
/// glyph in silence, so guessing at them is not good enough.
const CM_SYMBOLS: &[(char, &str, u8)] = &[
    ('→', "cmsy10", 33),
    ('←', "cmsy10", 32),
    ('↔', "cmsy10", 36),
    ('−', "cmsy10", 0),
    ('×', "cmsy10", 2),
    ('÷', "cmsy10", 4),
    ('±', "cmsy10", 6),
    ('∪', "cmsy10", 91),
    ('∩', "cmsy10", 92),
    ('≤', "cmsy10", 20),
    ('≥', "cmsy10", 21),
    ('≠', "cmsy10", 54),
    ('·', "cmsy10", 1),
    ('∞', "cmsy10", 49),
    ('§', "cmsy10", 120),
    ('¶', "cmsy10", 123),
    ('†', "cmsy10", 121),
    ('‡', "cmsy10", 122),
];

/// Shapes Computer Modern has no glyph for, and the nearest honest stand-in.
///
/// Box drawing is the case that matters here: the documents draw trees with it,
/// and cm has no box-drawing at all. A rule could be drawn for each, but a
/// character grid built out of rules is a different picture; the ASCII shapes
/// keep the tree readable and keep the columns lined up, which is what the
/// drawing is for.
const APPROXIMATIONS: &[(char, &str)] = &[
    // Computer Modern has no copyright sign; `(c)` is what a reader of the text
    // would have written anyway.
    ('©', "(c)"),
    ('®', "(R)"),
    ('™', "(TM)"),
    ('—', "---"),
    ('–', "--"),
    ('…', "..."),
    ('─', "-"),
    ('│', "|"),
    ('├', "|-"),
    ('└', "`-"),
    ('┌', ",-"),
    ('┐', "-."),
    ('┘', "-'"),
    ('┬', "-,-"),
    ('┴', "-'-"),
    ('┼', "-+-"),
    ('•', "*"),
    ('“', "``"),
    ('”', "''"),
    ('‘', "`"),
    ('’', "'"),
    // The double rules, the blocks and the marks the corpus draws with, none of
    // which any of the three sources above carries: not Computer Modern, not
    // WinAnsi, not the Symbol font. Counted over the 41 books, U+2500 appears
    // 9,134 times, U+2550 700 and U+2588 473, so without a stand-in the tables
    // and the bars in them are holes on the page.
    ('═', "="),
    ('║', "|"),
    ('╔', ",="),
    ('╗', "=."),
    ('╚', "`="),
    ('╝', "='"),
    ('╠', "|="),
    ('╣', "=|"),
    ('╦', "=,="),
    ('╩', "='="),
    ('╬', "=+="),
    ('█', "#"),
    ('▉', "#"),
    ('▊', "#"),
    ('▋', "#"),
    ('▌', "|"),
    ('▍', "|"),
    ('▎', "|"),
    ('▏', "|"),
    ('▄', "_"),
    ('▅', "_"),
    ('▂', "_"),
    ('●', "*"),
    ('○', "o"),
    ('◆', "*"),
    ('◇', "o"),
    ('★', "*"),
    ('☆', "*"),
    ('▲', "^"),
    ('▼', "v"),
    ('►', ">"),
    ('▶', ">"),
    ('◀', "<"),
    ('❯', ">"),
    // A tick and a cross, as a terminal writes them when it has no glyph
    // either. A checklist whose ticks vanished reads as a list of things that
    // were NOT done, which is the opposite of what it says.
    ('✓', "+"),
    ('✔', "+"),
    ('✗', "x"),
    ('✘', "x"),
    // The arrows no font on either path carries. The length is part of what a
    // long arrow says, so a long one is not set as a short one.
    ('↕', "|"),
    ('⟶', "-->"),
    ('⟵', "<--"),
    ('⟷', "<->"),
    ('⟹', "==>"),
    ('⟸', "<=="),
    ('↦', "|->"),
    ('↪', "->"),
    ('↩', "<-"),
    ('↗', "/"),
    ('↘', "\\"),
    ('↙', "/"),
    ('↖', "\\"),
    ('∓', "-+"),
    ('∘', "o"),
    ('↯', "!"),
    ('⚠', "!"),
    ('ℹ', "i"),
    ('┤', "-|"),
    ('╭', ",-"),
    ('╮', "-."),
    ('╰', "`-"),
    ('╯', "-'"),
    ('▁', "_"),
    ('▃', "_"),
    ('▆', "#"),
    ('▇', "#"),
    ('░', "."),
    ('▒', ":"),
    ('▓', "#"),
    ('▸', ">"),
    ('▹', ">"),
    ('◂', "<"),
    // The superscript and subscript digits. WinAnsi has ¹ ² ³ and stops, so
    // the rest of a footnote's numbering came out as nothing at all.
    ('⁰', "0"),
    ('⁴', "4"),
    ('⁵', "5"),
    ('⁶', "6"),
    ('⁷', "7"),
    ('⁸', "8"),
    ('⁹', "9"),
    ('ⁿ', "n"),
    ('₀', "0"),
    ('₁', "1"),
    ('₂', "2"),
    ('₃', "3"),
    ('₄', "4"),
    // The Mac modifier keys, which these books name in every keybinding table.
    // Spelling them is what a reader without the glyph needs; dropping them
    // leaves a shortcut with no key in it.
    ('⌘', "Cmd"),
    ('⌥', "Opt"),
    ('⌃', "Ctrl"),
    ('⇧', "Shift"),
];

/// The characters a document spells as a control sequence.
///
/// `\rightarrow` is not decoration: a document that writes one and meets an
/// engine that has never heard of it stops dead -- `! Undefined control
/// sequence \rightarrow.` -- and produces no page at all. Each name yields the
/// CHARACTER it means and nothing else, so every table that decides how a
/// character is DRAWN -- `CM_SYMBOLS` for the DVI path, `SYMBOL_FONT` and
/// `APPROXIMATIONS` for the PDF one -- stays the only place that decides it. A
/// name mapped straight to a font slot would be a second such table, and it
/// would drift from the first the moment either was touched.
///
/// The characters are the ones LaTeX's own `fontmath.ltx` and `latex.ltx` give
/// these names, read as Unicode: `\to` and `\rightarrow` are one macro in TeX
/// and are one entry here.
pub const SYMBOL_MACROS: &[(&str, char)] = &[
    // Arrows. `\to` and `\gets` are plain TeX's names for the first two.
    ("rightarrow", '→'),
    ("to", '→'),
    ("leftarrow", '←'),
    ("gets", '←'),
    ("uparrow", '↑'),
    ("downarrow", '↓'),
    ("leftrightarrow", '↔'),
    ("updownarrow", '↕'),
    ("Rightarrow", '⇒'),
    ("Leftarrow", '⇐'),
    ("Leftrightarrow", '⇔'),
    ("Uparrow", '⇑'),
    ("Downarrow", '⇓'),
    ("longrightarrow", '⟶'),
    ("longleftarrow", '⟵'),
    ("longleftrightarrow", '⟷'),
    ("Longrightarrow", '⟹'),
    ("Longleftarrow", '⟸'),
    ("mapsto", '↦'),
    ("hookrightarrow", '↪'),
    ("hookleftarrow", '↩'),
    ("nearrow", '↗'),
    ("searrow", '↘'),
    ("swarrow", '↙'),
    ("nwarrow", '↖'),
    // Greek, lower case. TeX has no `\omicron`: it is the letter `o`, and a
    // document writes it as one.
    ("alpha", 'α'),
    ("beta", 'β'),
    ("gamma", 'γ'),
    ("delta", 'δ'),
    // `\epsilon` is the lunate U+03F5 in TeX and `\varepsilon` the open one;
    // neither the Symbol font nor a text face carries the lunate, so both are
    // read as U+03B5, which is the letter either way.
    ("epsilon", 'ε'),
    ("varepsilon", 'ε'),
    ("zeta", 'ζ'),
    ("eta", 'η'),
    ("theta", 'θ'),
    ("vartheta", 'ϑ'),
    ("iota", 'ι'),
    ("kappa", 'κ'),
    ("lambda", 'λ'),
    ("mu", 'μ'),
    ("nu", 'ν'),
    ("xi", 'ξ'),
    ("pi", 'π'),
    ("varpi", 'ϖ'),
    ("rho", 'ρ'),
    ("sigma", 'σ'),
    ("varsigma", 'ς'),
    ("tau", 'τ'),
    ("upsilon", 'υ'),
    ("phi", 'φ'),
    ("varphi", 'ϕ'),
    ("chi", 'χ'),
    ("psi", 'ψ'),
    ("omega", 'ω'),
    // Greek, upper case. Only the letters that differ from a Latin capital
    // have a name in TeX, which is why there is no `\Alpha`.
    ("Gamma", 'Γ'),
    ("Delta", 'Δ'),
    ("Theta", 'Θ'),
    ("Lambda", 'Λ'),
    ("Xi", 'Ξ'),
    ("Pi", 'Π'),
    ("Sigma", 'Σ'),
    ("Upsilon", 'Υ'),
    ("Phi", 'Φ'),
    ("Psi", 'Ψ'),
    ("Omega", 'Ω'),
    // Relations.
    ("le", '≤'),
    ("leq", '≤'),
    ("ge", '≥'),
    ("geq", '≥'),
    ("ne", '≠'),
    ("neq", '≠'),
    ("approx", '≈'),
    ("equiv", '≡'),
    ("sim", '∼'),
    ("cong", '≅'),
    ("propto", '∝'),
    ("subset", '⊂'),
    ("supset", '⊃'),
    ("subseteq", '⊆'),
    ("supseteq", '⊇'),
    ("in", '∈'),
    ("notin", '∉'),
    ("ni", '∋'),
    ("perp", '⊥'),
    // Operators and the rest of the mathematics these books write.
    ("times", '×'),
    ("div", '÷'),
    ("pm", '±'),
    ("mp", '∓'),
    ("cdot", '⋅'),
    ("bullet", '•'),
    ("ast", '∗'),
    ("circ", '∘'),
    ("oplus", '⊕'),
    ("otimes", '⊗'),
    ("cup", '∪'),
    ("cap", '∩'),
    ("emptyset", '∅'),
    ("varnothing", '∅'),
    ("forall", '∀'),
    ("exists", '∃'),
    ("neg", '¬'),
    ("lnot", '¬'),
    ("land", '∧'),
    ("wedge", '∧'),
    ("lor", '∨'),
    ("vee", '∨'),
    ("sum", '∑'),
    ("prod", '∏'),
    ("int", '∫'),
    ("partial", '∂'),
    ("nabla", '∇'),
    ("infty", '∞'),
    ("surd", '√'),
    ("angle", '∠'),
    ("therefore", '∴'),
    ("aleph", 'ℵ'),
    ("prime", '′'),
    ("diamond", '◆'),
    ("star", '★'),
    ("dagger", '†'),
    ("ddagger", '‡'),
    // The text symbols that are spelled as a control sequence and are not
    // characters a keyboard has. `\S` and `\P` are Computer Modern's section
    // and paragraph marks, which `CM_SYMBOLS` already places.
    ("S", '§'),
    ("P", '¶'),
    ("dag", '†'),
    ("ddag", '‡'),
    ("copyright", '©'),
    ("textcopyright", '©'),
    ("textregistered", '®'),
    ("texttrademark", '™'),
    ("pounds", '£'),
    ("textsterling", '£'),
    ("texteuro", '€'),
    ("euro", '€'),
    ("textdegree", '°'),
    ("textbullet", '•'),
    ("textperiodcentered", '·'),
    ("textemdash", '—'),
    ("textendash", '–'),
    ("textellipsis", '…'),
    ("textmu", 'µ'),
    ("textpm", '±'),
    ("texttimes", '×'),
    ("textdiv", '÷'),
];

/// The character `\name` stands for, if it is one of the symbols above.
///
/// Asked only where a control sequence would otherwise be undefined, so a
/// document that defines its own `\star` keeps it.
pub fn symbol_char(name: &str) -> Option<char> {
    SYMBOL_MACROS
        .iter()
        .find(|(macro_name, _)| *macro_name == name)
        .map(|(_, ch)| *ch)
}

/// The characters of `text` that will actually be SET, with the colour and face
/// markers dropped.
///
/// A marker's SPEC -- the `r,g,b` between U+0001 and U+0002 -- has to be
/// skipped whole. Charging the three control characters nothing is not enough:
/// the digits and commas between them are ordinary characters the font does
/// have, so a coloured word measured that way comes out wider than it sets and
/// every line after it breaks short. The DVI side learned that first and the
/// PDF side kept charging for them, breaking its lines at a quarter of the
/// measure -- so the skip lives here, where both measuring paths reach it,
/// rather than being written out twice and drifting apart again.
///
/// A face marker is the same shape and skipped the same way: U+000E and the
/// ONE character naming the face, U+000F on its own.
pub fn printing_chars(text: &str) -> impl Iterator<Item = char> + '_ {
    let mut in_spec = false;
    let mut face_code = false;
    text.chars().filter(move |&ch| match ch {
        '\u{1}' => {
            in_spec = true;
            false
        }
        '\u{2}' => {
            in_spec = false;
            false
        }
        '\u{3}' => false,
        // The centring and vertical-space markers are instructions to the
        // page, not glyphs: a centred line carries one at its head, and
        // measuring it would push the line off centre by whatever the font
        // happens to have in that slot.
        CENTRE | CENTRE_END | VERTICAL_SPACE | JUSTIFY => false,
        FACE_PUSH => {
            face_code = true;
            false
        }
        FACE_POP => false,
        _ if face_code => {
            face_code = false;
            false
        }
        _ => !in_spec,
    })
}

impl FontChain {
    /// Load `primary` and each fallback by name, skipping any that is missing.
    ///
    /// A missing fallback is not an error: the chain degrades to what is
    /// installed, and the approximations catch what no loaded font carries.
    pub fn load(primary: &str, fallbacks: &[&str]) -> Result<FontChain, String> {
        let mut fonts = Vec::new();
        let path = find_font(primary).ok_or_else(|| format!("{primary}.tfm not found"))?;
        fonts.push(Loaded {
            tfm: Tfm::open(&path)?,
            name: primary.to_string(),
        });
        for f in fallbacks {
            if let Some(p) = find_font(f) {
                if let Ok(tfm) = Tfm::open(&p) {
                    fonts.push(Loaded {
                        tfm,
                        name: (*f).to_string(),
                    });
                }
            }
        }
        let mut map = Vec::new();
        for (ch, font_name, slot) in CM_SYMBOLS {
            if let Some(i) = fonts.iter().position(|f| f.name == *font_name) {
                // Only claim the slot if the font really defines it.
                if fonts[i].tfm.char(*slot).is_some() {
                    map.push((*ch, i, *slot));
                }
            }
        }
        Ok(FontChain { fonts, map })
    }

    /// Which font and slot carries `ch`, if any font in the chain does.
    pub fn resolve(&self, ch: char) -> Option<(usize, u8)> {
        // ASCII is the primary font's own, which is the common case and worth
        // answering before any search.
        if ch.is_ascii() && self.fonts[0].tfm.char(ch as u8).is_some() {
            return Some((0, ch as u8));
        }
        self.map
            .iter()
            .find(|(c, _, _)| *c == ch)
            .map(|(_, f, s)| (*f, *s))
    }

    /// The ASCII stand-in for a character no font in the chain carries.
    pub fn approximate(ch: char) -> Option<&'static str> {
        APPROXIMATIONS
            .iter()
            .find(|(c, _)| *c == ch)
            .map(|(_, s)| *s)
    }

    /// The width of `ch` in points at `size`, however it is being rendered.
    pub fn width(&self, ch: char, size: f64) -> f64 {
        // The colour markers are instructions to the driver, not glyphs: they
        // occupy no space on the page, and measuring them would push words onto
        // the next line for text that is not there.
        if matches!(ch, '\u{1}' | '\u{2}' | '\u{3}') || ch.is_control() {
            return 0.0;
        }
        if let Some((f, slot)) = self.resolve(ch) {
            return self.fonts[f].tfm.char(slot).map(|m| m.width).unwrap_or(0.0) * size;
        }
        if let Some(text) = Self::approximate(ch) {
            return self.fonts[0].tfm.width_of(text) * size;
        }
        0.0
    }

    /// The width of a whole string, resolving each character through the chain.
    ///
    /// The colour markers and their spec are not measured -- see
    /// `printing_chars`, which the PDF path measures through as well.
    pub fn width_of(&self, text: &str, size: f64) -> f64 {
        printing_chars(text).map(|c| self.width(c, size)).sum()
    }
}

/// The name every PDF reader knows the fallback font by.
///
/// Symbol is one of the fourteen a reader is required to have, so naming it
/// costs nothing in the file and works everywhere -- which is the only reason a
/// per-glyph fallback can be had here at all: an arbitrary system font would
/// have to be embedded AND addressed, and a simple font's encoding has 256
/// slots to address it with.
pub const SYMBOL_FONT_NAME: &str = "Symbol";

/// The Symbol font's own encoding: the code that draws a character, and what
/// that code advances, in 1/1000 em.
///
/// This IS the per-glyph fallback. `luaotfload.add_fallback` is what the
/// publication scripts needed LuaTeX for -- a character the chosen face has no
/// glyph for is fetched from another font, in every context including verbatim
/// -- and this is the same thing with the other font fixed at the one every
/// reader already has. It carries the arrows, the Greek and the relations,
/// which is most of what a text face is missing.
///
/// Read off `psyr.afm`, Adobe's own metrics for the font, joined to Unicode
/// through `glyphlist.txt`: `C 174 ; WX 987 ; N arrowright` and
/// `arrowright;2192`. Both files ship with every TeX installation. A wrong code
/// here draws a wrong glyph in silence, so they are not guessed.
const SYMBOL_FONT: &[(char, u8, i64)] = &[
    ('\u{AC}', 216, 713),    // logicalnot
    ('\u{B0}', 176, 400),    // degree
    ('\u{B1}', 177, 549),    // plusminus
    ('\u{B5}', 109, 576),    // mu
    ('\u{D7}', 180, 549),    // multiply
    ('\u{F7}', 184, 549),    // divide
    ('\u{192}', 166, 500),   // florin
    ('\u{391}', 65, 722),    // Alpha
    ('\u{392}', 66, 667),    // Beta
    ('\u{393}', 71, 603),    // Gamma
    ('\u{395}', 69, 611),    // Epsilon
    ('\u{396}', 90, 611),    // Zeta
    ('\u{397}', 72, 722),    // Eta
    ('\u{398}', 81, 741),    // Theta
    ('\u{399}', 73, 333),    // Iota
    ('\u{39A}', 75, 722),    // Kappa
    ('\u{39B}', 76, 686),    // Lambda
    ('\u{39C}', 77, 889),    // Mu
    ('\u{39D}', 78, 722),    // Nu
    ('\u{39E}', 88, 645),    // Xi
    ('\u{39F}', 79, 722),    // Omicron
    ('\u{3A0}', 80, 768),    // Pi
    ('\u{3A1}', 82, 556),    // Rho
    ('\u{3A3}', 83, 592),    // Sigma
    ('\u{3A4}', 84, 611),    // Tau
    ('\u{3A5}', 85, 690),    // Upsilon
    ('\u{3A6}', 70, 763),    // Phi
    ('\u{3A7}', 67, 722),    // Chi
    ('\u{3A8}', 89, 795),    // Psi
    ('\u{3B1}', 97, 631),    // alpha
    ('\u{3B2}', 98, 549),    // beta
    ('\u{3B3}', 103, 411),   // gamma
    ('\u{3B4}', 100, 494),   // delta
    ('\u{3B5}', 101, 439),   // epsilon
    ('\u{3B6}', 122, 494),   // zeta
    ('\u{3B7}', 104, 603),   // eta
    ('\u{3B8}', 113, 521),   // theta
    ('\u{3B9}', 105, 329),   // iota
    ('\u{3BA}', 107, 549),   // kappa
    ('\u{3BB}', 108, 549),   // lambda
    ('\u{3BD}', 110, 521),   // nu
    ('\u{3BE}', 120, 493),   // xi
    ('\u{3BF}', 111, 549),   // omicron
    ('\u{3C0}', 112, 549),   // pi
    ('\u{3C1}', 114, 549),   // rho
    ('\u{3C2}', 86, 439),    // sigma1
    ('\u{3C3}', 115, 603),   // sigma
    ('\u{3C4}', 116, 439),   // tau
    ('\u{3C5}', 117, 576),   // upsilon
    ('\u{3C6}', 102, 521),   // phi
    ('\u{3C7}', 99, 549),    // chi
    ('\u{3C8}', 121, 686),   // psi
    ('\u{3C9}', 119, 686),   // omega
    ('\u{3D1}', 74, 631),    // theta1
    ('\u{3D2}', 161, 620),   // Upsilon1
    ('\u{3D5}', 106, 603),   // phi1
    ('\u{3D6}', 118, 713),   // omega1
    ('\u{2022}', 183, 460),  // bullet
    ('\u{2026}', 188, 1000), // ellipsis
    ('\u{2032}', 162, 247),  // minute
    ('\u{2033}', 178, 411),  // second
    ('\u{2044}', 164, 167),  // fraction
    ('\u{2111}', 193, 686),  // Ifraktur
    ('\u{2118}', 195, 987),  // weierstrass
    ('\u{211C}', 194, 795),  // Rfraktur
    ('\u{2126}', 87, 768),   // Omega
    ('\u{2135}', 192, 823),  // aleph
    ('\u{2190}', 172, 987),  // arrowleft
    ('\u{2191}', 173, 603),  // arrowup
    ('\u{2192}', 174, 987),  // arrowright
    ('\u{2193}', 175, 603),  // arrowdown
    ('\u{2194}', 171, 1042), // arrowboth
    ('\u{21B5}', 191, 658),  // carriagereturn
    ('\u{21D0}', 220, 987),  // arrowdblleft
    ('\u{21D1}', 221, 603),  // arrowdblup
    ('\u{21D2}', 222, 987),  // arrowdblright
    ('\u{21D3}', 223, 603),  // arrowdbldown
    ('\u{21D4}', 219, 1042), // arrowdblboth
    ('\u{2200}', 34, 713),   // universal
    ('\u{2202}', 182, 494),  // partialdiff
    ('\u{2203}', 36, 549),   // existential
    ('\u{2205}', 198, 823),  // emptyset
    ('\u{2206}', 68, 612),   // Delta
    ('\u{2207}', 209, 713),  // gradient
    ('\u{2208}', 206, 713),  // element
    ('\u{2209}', 207, 713),  // notelement
    ('\u{220B}', 39, 439),   // suchthat
    ('\u{220F}', 213, 823),  // product
    ('\u{2211}', 229, 713),  // summation
    ('\u{2212}', 45, 549),   // minus
    ('\u{2217}', 42, 500),   // asteriskmath
    ('\u{221A}', 214, 549),  // radical
    ('\u{221D}', 181, 713),  // proportional
    ('\u{221E}', 165, 713),  // infinity
    ('\u{2220}', 208, 768),  // angle
    ('\u{2227}', 217, 603),  // logicaland
    ('\u{2228}', 218, 603),  // logicalor
    ('\u{2229}', 199, 768),  // intersection
    ('\u{222A}', 200, 768),  // union
    ('\u{222B}', 242, 274),  // integral
    ('\u{2234}', 92, 863),   // therefore
    ('\u{223C}', 126, 549),  // similar
    ('\u{2245}', 64, 549),   // congruent
    ('\u{2248}', 187, 549),  // approxequal
    ('\u{2260}', 185, 549),  // notequal
    ('\u{2261}', 186, 549),  // equivalence
    ('\u{2264}', 163, 549),  // lessequal
    ('\u{2265}', 179, 549),  // greaterequal
    ('\u{2282}', 204, 713),  // propersubset
    ('\u{2283}', 201, 713),  // propersuperset
    ('\u{2284}', 203, 713),  // notsubset
    ('\u{2286}', 205, 713),  // reflexsubset
    ('\u{2287}', 202, 713),  // reflexsuperset
    ('\u{2295}', 197, 768),  // circleplus
    ('\u{2297}', 196, 768),  // circlemultiply
    ('\u{22A5}', 94, 658),   // perpendicular
    ('\u{22C5}', 215, 250),  // dotmath
    ('\u{2320}', 243, 686),  // integraltp
    ('\u{2321}', 245, 686),  // integralbt
    ('\u{2329}', 225, 329),  // angleleft
    ('\u{232A}', 241, 329),  // angleright
    ('\u{25CA}', 224, 494),  // lozenge
    ('\u{2660}', 170, 753),  // spade
    ('\u{2663}', 167, 753),  // club
    ('\u{2665}', 169, 753),  // heart
    ('\u{2666}', 168, 753),  // diamond
    // Three glyphs the Adobe list names for their MATHEMATICAL character when
    // the Greek letter is the same shape and the same slot: `mu` is U+00B5 the
    // micro sign, `Delta` U+2206 the increment, `Omega` U+2126 the ohm. A
    // document writing Greek writes U+03BC, U+0394 and U+03A9, and with only
    // the names above those three fell past every table and were dropped --
    // measured on zshrs/docs/book.tex, whose two U+03BC reached no page.
    ('\u{3BC}', 109, 576), // mu, as the letter
    ('\u{394}', 68, 612),  // Delta, as the letter
    ('\u{3A9}', 87, 768),  // Omega, as the letter
];

/// WinAnsi's own codes, for the characters that are not where Unicode puts
/// them.
///
/// A PDF font here is written with `/Encoding /WinAnsiEncoding`, so a code IS a
/// WinAnsi code. From U+00A0 up WinAnsi is Latin-1 and the code is the
/// codepoint; between 0x80 and 0x9F it is Windows' own set of punctuation, and
/// that is where the em dash, the ellipsis and the curly quotes live -- 2,063
/// em dashes and 3,486 ellipses in the corpus, every one of them written into
/// the file as its UTF-8 bytes and drawn as two or three wrong letters.
const WINANSI_PUNCTUATION: &[(char, u8)] = &[
    ('€', 0x80),
    ('‚', 0x82),
    ('ƒ', 0x83),
    ('„', 0x84),
    ('…', 0x85),
    ('†', 0x86),
    ('‡', 0x87),
    ('ˆ', 0x88),
    ('‰', 0x89),
    ('Š', 0x8A),
    ('‹', 0x8B),
    ('Œ', 0x8C),
    ('Ž', 0x8E),
    ('‘', 0x91),
    ('’', 0x92),
    ('“', 0x93),
    ('”', 0x94),
    ('•', 0x95),
    ('–', 0x96),
    ('—', 0x97),
    ('˜', 0x98),
    ('™', 0x99),
    ('š', 0x9A),
    ('›', 0x9B),
    ('œ', 0x9C),
    ('ž', 0x9E),
    ('Ÿ', 0x9F),
];

/// The WinAnsi code for a character, if WinAnsi has one.
pub fn winansi_code(ch: char) -> Option<u8> {
    match ch {
        ' '..='~' => Some(ch as u8),
        '\u{A0}'..='\u{FF}' => Some(ch as u8),
        _ => WINANSI_PUNCTUATION
            .iter()
            .find(|(c, _)| *c == ch)
            .map(|(_, code)| *code),
    }
}

/// What a WinAnsi code means, which is what a font's `cmap` has to be asked
/// for.
///
/// A reader resolves a code in a non-symbolic TrueType font by its WinAnsi
/// GLYPH NAME and that name's Unicode value, so the width written beside the
/// code has to be looked up the same way. Reading code 0x97 as U+0097 finds
/// nothing and writes `.notdef`'s width where the em dash's belongs.
pub fn winansi_unicode(code: u8) -> Option<char> {
    match code {
        0x20..=0x7E => Some(code as char),
        0xA0..=0xFF => Some(code as char),
        _ => WINANSI_PUNCTUATION
            .iter()
            .find(|(_, c)| *c == code)
            .map(|(ch, _)| *ch),
    }
}

/// Where one character comes from when it is drawn.
///
/// The order the answer is looked for in is the chain: the face the document
/// asked for, then the fallback font, then a stand-in built out of characters
/// every face has. What it never does is drop the character -- a glyph that
/// vanishes takes the meaning of the line with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Glyph {
    /// A code in the face in force, which HAS the character.
    Own(u8),
    /// A code in the Symbol font, which has it where the face does not.
    Fallback(u8),
    /// ASCII the face draws itself, standing in for a shape nothing has.
    StandIn(&'static str),
}

/// How `ch` reaches the page, given what the face in force can draw.
///
/// `covers` answers from the font file's own `cmap`, which `embed_file` already
/// reads: whether a face HAS a codepoint is a question the file answers, and
/// asking it is the difference between a glyph and a blank.
pub fn glyph_for(ch: char, covers: &dyn Fn(char) -> bool) -> Option<Glyph> {
    if let Some(code) = winansi_code(ch) {
        if ch.is_ascii() || covers(ch) {
            return Some(Glyph::Own(code));
        }
    }
    if let Some((_, code, _)) = SYMBOL_FONT.iter().find(|(c, _, _)| *c == ch) {
        return Some(Glyph::Fallback(*code));
    }
    FontChain::approximate(ch).map(Glyph::StandIn)
}

/// What a code in the fallback font MEANS, as against which glyph it draws.
///
/// A PDF says only the second, so a page that draws an arrow from the fallback
/// is a page nobody can search for one: `pdftotext` on the first run of these
/// came back with the arrow simply missing from the text. The driver writes
/// this out as the font's `/ToUnicode`.
pub fn symbol_unicode(code: u8) -> Option<char> {
    SYMBOL_FONT
        .iter()
        .find(|(_, c, _)| *c == code)
        .map(|(ch, _, _)| *ch)
}

/// What a Symbol code advances, in 1/1000 em.
fn symbol_width(code: u8) -> i64 {
    SYMBOL_FONT
        .iter()
        .find(|(_, c, _)| *c == code)
        .map(|(_, _, w)| *w)
        .unwrap_or(500)
}

/// The typefaces a document asked for.
///
/// `\setmainfont{Arimo}` is a statement about the book, and setting it in
/// Computer Modern anyway is the thing this exists to stop. What texrs can
/// honour depends on the output: a DVI names `.tfm` fonts and cannot carry an
/// OpenType one at all, while a PDF can name the fourteen fonts every reader
/// has. Arimo is metric-compatible with Arial, which is metric-compatible with
/// Helvetica, so that substitution is a real one rather than a shrug.
/// Where a document keeps the font it supplied with itself.
///
/// fontspec's `\setmainfont{Arimo}[Path=..., Extension=.ttf,
/// UprightFont=Arimo-VF]` does not name an INSTALLED family: it names a file
/// that ships beside the document. Looking that name up among the installed
/// families finds nothing, and `fc-match` answers anyway, with its default --
/// so the book was set in whatever that default happened to be, which is the
/// "everything comes out in the wrong face" complaint exactly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontFile {
    /// `Path=` -- a directory, written with a trailing separator.
    pub path: Option<String>,
    /// `UprightFont=` -- the file's name, without its extension.
    pub upright: Option<String>,
    /// `BoldFont=` -- the file `\textbf` is set from, when there is one.
    pub bold: Option<String>,
    /// `ItalicFont=` -- the file `\emph` and `\textit` are set from.
    pub italic: Option<String>,
    /// `Extension=` -- `.ttf`, including the dot.
    pub extension: Option<String>,
}

impl FontFile {
    /// Read the options fontspec was given: `Key=Value` separated by commas,
    /// with `{...}` groups around values that contain commas of their own.
    pub fn parse(options: &str) -> FontFile {
        let mut out = FontFile::default();
        for (key, value) in top_level_pairs(options) {
            let value = value.trim().trim_matches(|c| c == '{' || c == '}');
            match key.trim() {
                "Path" => out.path = Some(value.to_string()),
                "UprightFont" => out.upright = Some(value.to_string()),
                // The faces `\textbf` and `\emph` ask for. A book names them in
                // the same option list as the upright file and every one of the
                // corpus books ships the files, so the only thing that ever
                // stopped `\emph` from being italic was that these two keys
                // were read past.
                "BoldFont" => out.bold = Some(value.to_string()),
                "ItalicFont" => out.italic = Some(value.to_string()),
                "Extension" => out.extension = Some(value.to_string()),
                _ => {}
            }
        }
        out
    }

    /// The file this names, looked for where it might actually be.
    ///
    /// `Path=` is written by whatever produced the document and is regularly an
    /// absolute path into a directory that no longer exists -- a build
    /// scratchpad, another machine. The fonts themselves ship beside the
    /// document, so a path that has gone stale is retried by its last
    /// component against the document's own directory.
    pub fn resolve(&self, near: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
        self.resolve_face(Face::Main, near)
    }

    /// The file for one face, looked for in the same two places.
    ///
    /// `BoldFont=` and `ItalicFont=` name a file the way `UprightFont=` does
    /// and live in the same directory, so they are resolved by the same rules
    /// rather than by a second copy of them. A face the document named no file
    /// for resolves to nothing, and the caller falls back to the main face.
    pub fn resolve_face(
        &self,
        face: Face,
        near: Option<&std::path::Path>,
    ) -> Option<std::path::PathBuf> {
        let named = match face {
            Face::Bold => self.bold.as_deref(),
            Face::Italic => self.italic.as_deref(),
            _ => self.upright.as_deref(),
        }?;
        let extension = self.extension.as_deref().unwrap_or(".ttf");
        let file = format!("{named}{extension}");
        if let Some(path) = &self.path {
            let full = std::path::Path::new(path).join(&file);
            if full.is_file() {
                return Some(full);
            }
            // The stale-path case: keep the directory the fonts are IN and look
            // for it beside the document instead.
            if let (Some(dir), Some(near)) = (last_component(path), near) {
                let beside = near.join(dir).join(&file);
                if beside.is_file() {
                    return Some(beside);
                }
            }
        }
        let beside = near?.join(&file);
        beside.is_file().then_some(beside)
    }
}

/// The last directory of a path written with a trailing separator.
fn last_component(path: &str) -> Option<&str> {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|c| !c.is_empty())
}

/// `Key=Value` pairs, splitting only on the commas that are not inside braces.
///
/// fontspec values nest: `UprightFeatures={RawFeature={axis={wght=400}}}` holds
/// commas in some documents, and splitting on every comma tears it apart and
/// makes the keys after it unreadable.
fn top_level_pairs(options: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let bytes = options.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                if let Some(pair) = split_pair(&options[start..i]) {
                    out.push(pair);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    if let Some(pair) = split_pair(&options[start..]) {
        out.push(pair);
    }
    out
}

fn split_pair(text: &str) -> Option<(&str, &str)> {
    let at = text.find('=')?;
    Some((&text[..at], &text[at + 1..]))
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Families {
    pub main: Option<String>,
    /// The file `\setmainfont`s options named, when the document ships its
    /// own font rather than naming an installed one.
    pub main_file: FontFile,
    pub sans: Option<String>,
    pub mono: Option<String>,
    /// The same for `\setmonofont`: every corpus book ships its monospace face
    /// beside itself, so the family name alone resolves to nothing and
    /// `\texttt` would be set in the body font after all.
    pub mono_file: FontFile,
}

/// The face a stretch of text is set in.
///
/// `\texttt` appears 683,577 times in the corpus, `\emph` 35,369 and `\textbf`
/// 34,159, and all three were set in the body face: nothing between the mouth
/// and the page carried WHICH face was asked for, so a PDF came out with one
/// font resource and one `Tf` operator, and every code identifier in every book
/// was set in the prose font.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Face {
    /// What `\setmainfont` named: the document's prose.
    #[default]
    Main,
    /// `\ttfamily`, which is what `\texttt` is.
    Mono,
    /// `\bfseries`, which is what `\textbf` is.
    Bold,
    /// `\itshape`, which is what `\emph` and `\textit` are.
    Italic,
}

impl Face {
    /// The one character that names this face inside a marker.
    pub fn code(self) -> char {
        match self {
            Face::Main => 'r',
            Face::Mono => 'm',
            Face::Bold => 'b',
            Face::Italic => 'i',
        }
    }

    /// The face a marker names. Anything else is the main face: a marker that
    /// arrives damaged must not take the rest of the document with it.
    pub fn from_code(code: char) -> Face {
        match code {
            'm' => Face::Mono,
            'b' => Face::Bold,
            'i' => Face::Italic,
            _ => Face::Main,
        }
    }

    /// Where this face's font sits in the four the page is set from.
    fn index(self) -> usize {
        match self {
            Face::Main => 0,
            Face::Mono => 1,
            Face::Bold => 2,
            Face::Italic => 3,
        }
    }
}

/// A face marker opens: U+000E and one [`Face::code`] character.
///
/// Colour travels the text as a marker because it wraps a run of characters
/// rather than being one; a face is the same shape of thing and travels the
/// same way. Two characters rather than colour's variable-length spec, because
/// there are four faces and no spec to carry.
pub const FACE_PUSH: char = '\u{11}';

/// A face marker closes. The stack under it is what `\ttfamily` inside a
/// `\textbf` needs: the outer face comes back when the inner one ends.
pub const FACE_POP: char = '\u{12}';

/// What a requested family maps to among the fourteen fonts a PDF reader has.
///
/// The mapping is by what the face IS, not by its name: Arimo is Arial's
/// metrics, Liberation Sans is Arial's metrics, and Arial is Helvetica's, so
/// all three set at the same widths as Helvetica. A monospace request goes to
/// Courier whatever it was called. A name nothing is known about falls to
/// Helvetica rather than to Computer Modern, because a document that asked for
/// a font at all was asking not to be set in a book face.
pub fn base14_for(family: &str) -> &'static str {
    let f = family.to_ascii_lowercase();
    let mono = [
        "mono",
        "courier",
        "consolas",
        "menlo",
        "inconsolata",
        "sharetechmono",
        "jetbrains",
        "fira code",
        "source code",
    ];
    if mono.iter().any(|m| f.contains(m)) {
        return "Courier";
    }
    let serif = [
        "times",
        "serif",
        "georgia",
        "garamond",
        "minion",
        "palatino",
        "book",
        "charter",
        "libertine",
        "stix",
    ];
    if serif.iter().any(|m| f.contains(m)) {
        return "Times-Roman";
    }
    "Helvetica"
}

/// The member of the fourteen that carries one face of `base`.
///
/// The fourteen are four faces of three families plus two symbol fonts, so a
/// document that named a family but shipped no file for its bold or italic
/// still gets a face that is bold or italic -- which is the whole request. The
/// names are not built the same way for all three: Helvetica and Courier take
/// `-Bold` and `-Oblique`, and Times-Roman's siblings drop the `Roman` for
/// `Times-Bold` and `Times-Italic`. Getting that wrong names a font no reader
/// has and the substitution is silent.
pub fn base14_face(base: &str, face: Face) -> String {
    match face {
        Face::Main => base.to_string(),
        Face::Mono => "Courier".to_string(),
        Face::Bold if base == "Times-Roman" => "Times-Bold".to_string(),
        Face::Italic if base == "Times-Roman" => "Times-Italic".to_string(),
        Face::Bold => format!("{base}-Bold"),
        Face::Italic => format!("{base}-Oblique"),
    }
}

/// Set a document straight to PDF, honouring the font it asked for.
///
/// The DVI path names `.tfm` fonts and so can only ever set in Computer Modern;
/// a PDF can name the fourteen faces every reader has, which is how
/// `\setmainfont{Arimo}` becomes Helvetica's metrics rather than a book face.
/// Colour travels the same markers the DVI path reads and comes out as PDF's
/// own `rg` operator.
/// `page` is what `\pagecolor` asked for, painted under everything else.
/// `near` is the directory the document was read from, which is where a font
/// it ships with itself is looked for.
pub fn to_pdf(
    text: &str,
    families: &Families,
    layout: &Layout,
    page_colour: Option<crate::colour::Rgb>,
    near: Option<&std::path::Path>,
) -> Vec<u8> {
    use crate::pdf::{document, Font, Page, Set};

    // The font the document asked for, EMBEDDED if it can be found: that is
    // the difference between a page set in Arimo's metrics and one set in
    // Arimo. Failing that, one of the fourteen, which gets the widths right
    // and the shapes wrong -- better than refusing to typeset.
    let requested = families.main.as_deref();
    // A font the document SHIPS is tried first: it is the one the document
    // actually meant, and its family name is regularly not installed at all.
    let embedded = families
        .main_file
        .resolve(near)
        .and_then(|file| embed_file(&file))
        .or_else(|| requested.and_then(embed_family));
    let main = embedded.clone().unwrap_or_else(|| {
        Font::Base14(requested.map(base14_for).unwrap_or("Helvetica").to_string())
    });

    // The other three faces, in the order `Face::index` puts them: main, mono,
    // bold, italic. Each is the file the document named for it, then the family
    // it named, then the member of the fourteen that carries that face -- and
    // the main font when the document asked for none of those, so a `\texttt`
    // in a document with no monospace family still sets rather than vanishing.
    let base = match &main {
        Font::Base14(name) => name.clone(),
        _ => requested.map(base14_for).unwrap_or("Helvetica").to_string(),
    };
    let mono = families
        .mono_file
        .resolve(near)
        .and_then(|file| embed_file(&file))
        .or_else(|| families.mono.as_deref().and_then(embed_family))
        .or_else(|| {
            families
                .mono
                .as_deref()
                .map(|f| Font::Base14(base14_for(f).to_string()))
        })
        .unwrap_or_else(|| main.clone());
    // A bold or italic FILE is what the corpus books ship: `BoldFont=Arimo-VF`
    // beside `UprightFont=Arimo-VF`. Where the two name the same file the font
    // is the same font, `Page::text_in` recognises it as one and the page is
    // set in the main face -- which is the honest outcome, since a variable
    // font's weight axis is not something this can instantiate.
    let face_file = |face: Face| {
        families
            .main_file
            .resolve_face(face, near)
            .and_then(|file| embed_file(&file))
    };
    let bold =
        face_file(Face::Bold).unwrap_or_else(|| Font::Base14(base14_face(&base, Face::Bold)));
    let italic =
        face_file(Face::Italic).unwrap_or_else(|| Font::Base14(base14_face(&base, Face::Italic)));
    let fonts = [main.clone(), mono, bold, italic];

    // Measure in the face that will be printed. An embedded font carries its
    // own widths; without one, cmr10's are the closest thing installed, and a
    // line may then run a little long or short of the measure.
    let embedded_widths: Vec<Option<Vec<i64>>> = fonts
        .iter()
        .map(|f| match f {
            Font::TrueType { widths, .. } => Some(widths.clone()),
            _ => None,
        })
        .collect();
    // Which characters each face can actually draw, from the font file's own
    // cmap. This is the question `luaotfload.add_fallback` asks per glyph, and
    // it is the reason the publication scripts required LuaTeX at all: a
    // character the face has no glyph for has to come from somewhere else, and
    // it cannot be fetched without first knowing that it is missing.
    let coverage: Vec<Option<std::collections::BTreeSet<u32>>> =
        fonts.iter().map(face_coverage).collect();
    // A face with no file to read is one of the fourteen, and carrying all of
    // WinAnsi is what being one of them means.
    let covers = |ch: char, face: Face| match &coverage[face.index()] {
        Some(has) => has.contains(&(ch as u32)),
        None => true,
    };
    let metrics = find_font("cmr10").and_then(|p| Tfm::open(&p).ok());
    // What a stretch the face draws ITSELF costs. The codes are what the file
    // will hold and the characters are what the document wrote; the two differ
    // wherever WinAnsi puts a character somewhere other than its codepoint, and
    // each table is indexed by the one it is stated in.
    let own_width = |codes: &str, source: &str, face: Face| -> f64 {
        if let Some(Some(w)) = embedded_widths.get(face.index()) {
            // PDF widths are 1/1000 em, and codes below 32 are not in the table.
            return codes
                .chars()
                .map(|c| {
                    let at = (c as usize).saturating_sub(32);
                    let mille = w.get(at).copied().unwrap_or(500);
                    mille as f64 / 1000.0 * layout.size
                })
                .sum();
        }
        match &metrics {
            // The .tfm reader kerns and ligatures across neighbouring
            // characters, so it needs the string whole rather than an iterator.
            Some(f) => f.width_of(source) * layout.size,
            None => source.chars().count() as f64 * layout.size * 0.5,
        }
    };
    let piece_width = |piece: &Piece, face: Face| -> f64 {
        match piece.fallback {
            // The fallback font's metrics are its own, and they are nothing
            // like the face's: `arrowright` is 987/1000 em where a letter is
            // about 500. Charging the face's widths for it would push every
            // line holding an arrow off the measure.
            true => piece
                .codes
                .chars()
                .map(|c| symbol_width(c as u8) as f64 / 1000.0 * layout.size)
                .sum(),
            false => own_width(&piece.codes, &piece.source, face),
        }
    };
    // Every branch measures through `printing_chars`. A marker's spec is digits
    // and commas, which every one of these tables has a real width for, so a
    // word inside one \textcolor was charged for its five letters plus fourteen
    // spec characters plus three markers -- and a line of them broke after four
    // words where the same line uncoloured holds seventeen. It cost whole
    // pages: rubyrs/docs/book.tex set in 340 and sets in 186 with them skipped.
    let width_of = |word: &str, face: Face| -> f64 {
        // Plain ASCII is the case that runs a million times a book: every face
        // draws it, and the code and the character are the same number, so
        // there is nothing to decide per character. Answering it here keeps an
        // ordinary word measured exactly as it was before any of this.
        if word.is_ascii() {
            let plain: String = printing_chars(word).collect();
            return own_width(&plain, &plain, face);
        }
        drawn(word, &|c| covers(c, face))
            .iter()
            .map(|piece| piece_width(piece, face))
            .sum()
    };

    let lines = break_lines_measured(text, layout, &width_of);
    let per_page = ((layout.height / layout.leading).floor() as usize).max(1);

    // The colour stack, seeded with what the document is set in where it has
    // not said otherwise. Seeding it means a run with nothing pushed still
    // carries a colour, so every run is emitted with an explicit `rg` and none
    // inherits whatever the run before it left behind.
    //
    // It lives out here, across lines and across pages, because a `\color` is
    // in force until its group closes and a book says `\color{textPrim}` ONCE,
    // above everything. Restarting it per line put that colour back to black
    // on the second line of the document.
    let mut colours: Vec<Spec> = vec![(
        DEFAULT_COLOUR.0.to_string(),
        DEFAULT_COLOUR.1.to_string(),
        DEFAULT_COLOUR.2.to_string(),
    )];
    // The face stack, for the same reason: `\ttfamily` holds until its group
    // closes, and a group can hold a paragraph.
    let mut faces: Vec<Face> = vec![Face::Main];

    let mut pages = Vec::new();
    for chunk in paginate(&lines, per_page) {
        let mut page = Page::letter();
        // The page is painted first, and only first: a fill drawn after the
        // text covers it, and a document that sets a dark page sets light
        // text to go on it -- so getting the order wrong loses the words.
        if let Some((r, g, b)) = page_colour {
            page.content.push_str(&format!(
                "{r} {g} {b} rg\n0 0 {} {} re\nf\n0 0 0 rg\n",
                page.width, page.height
            ));
        }
        let mut y = layout.height + layout.margin - layout.leading;
        for line in chunk {
            // A line of vertical space is space: the baseline moves down and
            // nothing is drawn on it. Falling through to the run loop below
            // would draw the marker itself as a character.
            if !line.is_empty() && line.chars().all(|c| c == VERTICAL_SPACE) {
                y -= layout.leading;
                continue;
            }
            // A centred line is positioned by its measured width rather than
            // at the margin -- that is the whole of what centring is here.
            // The marker is a PREFIX the breaker put on, so it comes off
            // before the line is measured or drawn.
            let (centred, line) = match line.strip_prefix(CENTRE) {
                Some(rest) => (true, rest),
                None => (false, line),
            };
            // A full line is set TO the measure; a ragged one is set at
            // whatever it comes to. The breaker decides which -- only a line
            // the next word would not fit on is full -- and says so with the
            // same kind of prefix. A centred line never carries it.
            let (justified, line) = match line.strip_prefix(JUSTIFY) {
                Some(rest) => (true, rest),
                None => (false, line),
            };
            // A line is a sequence of RUNS, each with its own colour: the
            // markers turn colour on and off part way along it. Collapsing the
            // line to one colour state was the first attempt and drew none at
            // all, because the closing marker put the state back before
            // anything was emitted.
            // A line is a sequence of RUNS, each with its own colour and face:
            // the markers turn both on and off part way along it. Collapsing the
            // line to one colour state was the first attempt and drew none at
            // all, because the closing marker put the state back before
            // anything was emitted.
            //
            // A centred line is measured the way it will be DRAWN, each run in
            // its own face, because a line of code centred on the prose font's
            // widths sits off centre by the difference. The splitter is asked
            // on copies of the stacks so the drawing pass below still walks
            // them for real.
            let width: f64 = match centred {
                true => {
                    let (mut c, mut f) = (colours.clone(), faces.clone());
                    styled_runs(line, &mut c, &mut f)
                        .iter()
                        .map(|(plain, _, face)| width_of(plain, *face))
                        .sum()
                }
                false => 0.0,
            };
            // What each space on a justified line is widened by: the room left
            // over at the measure, shared out between them. Measured the same
            // way a centred line is, on copies of the stacks, because it is the
            // same question -- what does this line come to as it will be drawn.
            // A line with no space in it has nothing to share the room between
            // and is set where it stands.
            let extra: f64 = match justified {
                true => {
                    let (mut c, mut f) = (colours.clone(), faces.clone());
                    let runs = styled_runs(line, &mut c, &mut f);
                    let natural: f64 = runs
                        .iter()
                        .map(|(plain, _, face)| width_of(plain, *face))
                        .sum();
                    let spaces: usize = runs
                        .iter()
                        .map(|(plain, _, _)| plain.chars().filter(|c| *c == ' ').count())
                        .sum();
                    match spaces {
                        0 => 0.0,
                        n => (layout.measure - natural) / n as f64,
                    }
                }
                false => 0.0,
            };
            let mut x = match centred {
                true => layout.margin + (layout.measure - width).max(0.0) / 2.0,
                false => layout.margin,
            };
            for (plain, (r, g, b), face) in styled_runs(line, &mut colours, &mut faces) {
                if plain.is_empty() {
                    continue;
                }
                // Every run says its colour, so the run after a `\textcolor`
                // gets the colour it was NESTED IN back. Resetting to `0 g`
                // instead is what drew the rest of a dark-paged book black.
                page.content.push_str(&format!("{r} {g} {b} rg\n"));
                // A run is not necessarily one font either: a character the
                // face cannot draw is drawn from the fallback, which is a
                // different font resource and so a `Tf` of its own. The pieces
                // are positioned exactly as the runs are, each one advancing x
                // by what it just drew.
                for piece in drawn(&plain, &|c| covers(c, face)) {
                    let font = match piece.fallback {
                        true => Font::Base14(SYMBOL_FONT_NAME.to_string()),
                        false => fonts[face.index()].clone(),
                    };
                    // The piece takes its share of the room: what it measures,
                    // plus the widening for each space that falls inside it.
                    // x advances by what was actually SET and not by what the
                    // glyphs come to, or the piece after it would be drawn back
                    // over the space just widened.
                    let natural = piece_width(&piece, face);
                    let spaces = piece.codes.chars().filter(|c| *c == ' ').count() as f64;
                    let set = Set {
                        natural,
                        width: natural + extra * spaces,
                    };
                    page.text_set(font, layout.size, x, y, &piece.codes, set);
                    x += set.width;
                }
            }
            y -= layout.leading;
        }
        pages.push(page);
    }
    document(&pages)
}

/// One stretch of a run that a single font draws.
struct Piece {
    /// What goes in the content stream: one byte a glyph, in that font's own
    /// encoding, held as `char`s that are all under 256.
    codes: String,
    /// The document's own characters, for a face whose widths are not in the
    /// file and are measured out of `cmr10` instead.
    source: String,
    /// Whether the codes are the fallback font's rather than the face's.
    fallback: bool,
}

/// Resolve a run through the chain, into the stretches each font draws.
///
/// Neighbouring characters that land in the same font are one piece, so a line
/// of prose is still one `Tj` and only the arrow in the middle of it is its
/// own. A character that no face, no fallback and no stand-in has is left out,
/// which is what the DVI path does with the same character.
fn drawn(text: &str, covers: &dyn Fn(char) -> bool) -> Vec<Piece> {
    let mut pieces: Vec<Piece> = Vec::new();
    for ch in printing_chars(text) {
        let Some(glyph) = glyph_for(ch, covers) else {
            continue;
        };
        let (fallback, codes, source) = match glyph {
            Glyph::Own(code) => (false, (code as char).to_string(), ch.to_string()),
            Glyph::Fallback(code) => (true, (code as char).to_string(), ch.to_string()),
            // A stand-in is measured as what it SETS and not as what it stands
            // for: `Cmd` takes three letters' room wherever it lands.
            Glyph::StandIn(text) => (false, text.to_string(), text.to_string()),
        };
        match pieces.last_mut() {
            Some(last) if last.fallback == fallback => {
                last.codes.push_str(&codes);
                last.source.push_str(&source);
            }
            _ => pieces.push(Piece {
                codes,
                source,
                fallback,
            }),
        }
    }
    pieces
}

/// Which characters a face can draw, out of the font file's own `cmap`.
///
/// `None` for one of the fourteen: it has no file to read, and carrying all of
/// WinAnsi is what being one of them means.
fn face_coverage(font: &crate::pdf::Font) -> Option<std::collections::BTreeSet<u32>> {
    let crate::pdf::Font::TrueType { bytes, .. } = font else {
        return None;
    };
    let sfnt = crate::sfnt::Sfnt::parse(bytes.clone()).ok()?;
    Some(sfnt.cmap().ok()?.into_keys().collect())
}

/// Break lines with a caller-supplied measurer.
///
/// The measurer is asked for a width IN A FACE, and the face is state the text
/// carries: a `\ttfamily` holds until its group closes, so the stack has to
/// live across words, lines and paragraphs here exactly as it does where the
/// page is drawn. Measuring a monospace word in the prose font is how a line
/// of code came out narrower than it sets.
fn break_lines_measured(
    text: &str,
    layout: &Layout,
    width_of: &dyn Fn(&str, Face) -> f64,
) -> Vec<String> {
    // The breaker cares about the face and not about the colour, but the two
    // markers are interleaved in one stream, so the same splitter reads both
    // and the colour half goes on a stack nothing here looks at.
    let mut colours: Vec<Spec> = Vec::new();
    let mut faces: Vec<Face> = vec![Face::Main];
    let mut lines = Vec::new();
    // Centring is a REGION and outlives the paragraph its marker landed in: a
    // title page is one `\begin{center}` holding half a dozen `\par`-separated
    // pieces, so the flag is carried across the loops rather than reset in
    // them.
    let mut centred = false;
    for para in text.split("\n\n") {
        // A code listing is already broken, by the author -- see
        // `listing_lines`, which the DVI breaker reads the same way.
        if let Some(code) = listing_lines(para) {
            lines.extend(code.map(str::to_string));
            continue;
        }
        // A forced break is its own line, so the paginator can see one. It has
        // to come out here because `split_whitespace` below counts a form feed
        // as whitespace and would silently drop it.
        for (part_number, part) in para.split(PAGE_BREAK).enumerate() {
            if part_number > 0 {
                lines.push(PAGE_BREAK.to_string());
            }
            // Vertical space is its own line for the same reason: a vertical
            // tab is whitespace to Rust, so the space a heading asked for
            // would vanish into the gap between two words.
            for (space_number, part) in part.split(VERTICAL_SPACE).enumerate() {
                if space_number > 0 {
                    lines.push(VERTICAL_SPACE.to_string());
                }
                fill(
                    part,
                    &mut centred,
                    layout,
                    &mut colours,
                    &mut faces,
                    width_of,
                    &mut lines,
                );
            }
        }
    }
    lines
}

/// Fill one stretch of text into lines at the measure, first-fit, marking each
/// line the centring markers put it inside.
///
/// The markers are cut out BEFORE words are counted because a region opens
/// against the text it centres -- `\centering` is followed straight by the
/// title -- so a word carrying one would be measured with it and set with it.
/// Cutting there also ends the line in hand, which is what a change of
/// alignment means: LaTeX's own `\centering` applies to whole paragraphs.
#[allow(clippy::too_many_arguments)]
fn fill(
    text: &str,
    centred: &mut bool,
    layout: &Layout,
    colours: &mut Vec<Spec>,
    faces: &mut Vec<Face>,
    width_of: &dyn Fn(&str, Face) -> f64,
    lines: &mut Vec<String>,
) {
    let mut rest = text;
    loop {
        let (stretch, marker) = match rest.find([CENTRE, CENTRE_END]) {
            Some(at) => (&rest[..at], rest[at..].chars().next()),
            None => (rest, None),
        };
        let mut line = String::new();
        let mut width = 0.0f64;
        // A centred line says so in its first character, so the page can
        // position it by what it measures. The alternative -- a parallel list
        // of which lines are centred -- would have to survive pagination, and
        // the form feed a forced break travels as is the pattern already here.
        let start = |centred: bool| match centred {
            true => CENTRE.to_string(),
            false => String::new(),
        };
        for word in stretch.split_whitespace() {
            // The space between two words is set in the face in force where it
            // falls, which is the one the word BEFORE it left; and the word
            // itself costs what it costs in the faces its own markers select.
            // Measuring a monospace word in the prose font is how a line of
            // code came out narrower than it sets.
            let space = width_of(" ", current_face(faces));
            let ww = word_width(word, colours, faces, width_of);
            let need = match line.is_empty() {
                true => ww,
                false => width + space + ww,
            };
            if !line.is_empty() && need > layout.measure {
                // A line pushed HERE is one the next word would not fit on, so
                // it is a FULL line and is the one set to the measure. The last
                // line of a paragraph falls out of this loop below and stays
                // ragged, which is what TeX does with it; and a centred line is
                // positioned by its own width, so it is left alone.
                let full = std::mem::take(&mut line);
                lines.push(match *centred {
                    true => full,
                    false => format!("{JUSTIFY}{full}"),
                });
                width = ww;
                line = start(*centred);
                line.push_str(word);
                continue;
            }
            match line.is_empty() {
                true => line = start(*centred),
                false => line.push(' '),
            }
            line.push_str(word);
            width = need;
        }
        if !line.is_empty() {
            lines.push(line);
        }
        match marker {
            Some(m) => {
                *centred = m == CENTRE;
                rest = &rest[stretch.len() + m.len_utf8()..];
            }
            None => return,
        }
    }
}

/// Split broken lines into pages, at a forced break or when the page is full.
///
/// `chunks(per_page)` alone cannot do this: it fills every page to the brim,
/// so a `\newpage` has nowhere to say anything and a chapter starts wherever
/// the previous one happened to end.
fn paginate(lines: &[String], per_page: usize) -> Vec<Vec<&str>> {
    let mut pages: Vec<Vec<&str>> = Vec::new();
    let mut page: Vec<&str> = Vec::new();
    for line in lines {
        if line.chars().all(|c| c == PAGE_BREAK) && !line.is_empty() {
            // Consecutive breaks do not make blank pages: \clearpage after
            // \newpage is one break, which is what both mean together.
            if !page.is_empty() {
                pages.push(std::mem::take(&mut page));
            }
            continue;
        }
        if page.len() >= per_page {
            pages.push(std::mem::take(&mut page));
        }
        page.push(line);
    }
    if !page.is_empty() {
        pages.push(page);
    }
    pages
}

/// A forced page break, carried through the text the way colour is.
///
/// `\newpage` and `\clearpage` were defined by the prelude to expand to
/// nothing, so a book's title page, copyright page and first chapter ran
/// together into one stream of prose and the page count came out at half what
/// the document asks for. A form feed is what the character means, it survives
/// the run because it is not a word, and it is split out of the text BEFORE
/// words are, since Rust counts it as whitespace and would otherwise drop it.
pub const PAGE_BREAK: char = '\u{c}';

/// Every character the typesetting path reads as an INSTRUCTION rather than as
/// text, and whether one character of argument follows it.
///
/// A marker says where a line sits, what face it is in, what colour it takes --
/// none of which the document wrote. So none may ever reach a reader, and
/// `without_marks` in lib.rs is the one place that decides what a reader gets.
///
/// The list exists because forgetting that place is the mistake this port keeps
/// making. Three separate parallel implementations added a marker, taught
/// `to_pdf` and the line breaker about it, and left `without_marks` alone: one
/// of them put 122 raw control characters into a book's `--text` output, and
/// the file's own comment already recorded the same fault as fixed once before.
/// `every_marker_is_stripped_from_the_text_a_reader_gets` walks this list, so a
/// constant added without an entry here fails a test rather than shipping.
pub const MARKERS: &[(char, bool)] = &[
    // The colour trio: a spec opens, runs to its close, and a pop ends the run.
    ('\u{1}', false),
    ('\u{2}', false),
    ('\u{3}', false),
    (LISTING_BREAK, false),
    (PAGE_BREAK, false),
    (CENTRE, false),
    (CENTRE_END, false),
    (VERTICAL_SPACE, false),
    (JUSTIFY, false),
    // The one that carries an argument: the character naming the face.
    (FACE_PUSH, true),
    (FACE_POP, false),
];

/// The end of a line INSIDE a code listing, carried through the text the way a
/// page break is.
///
/// Pandoc wraps every code block in `Highlighting`, and that body is real TeX --
/// `\NormalTok{…}` and its siblings have to expand -- so it cannot be read as
/// verbatim and its newlines reached the breaker as ordinary spaces.
/// `split_whitespace` then reflowed the program into the prose around it.
/// Measured: rubyrs/docs/book.tex set a nine-line `dup_value` as three, one of
/// them `... => v.clone(), Some(obj) => { let copy = obj.clone();
/// self.alloc(copy) } None =>`, and the book came out in 208 pages where
/// lualatex sets it in 332. A vertical tab is a line separator by definition,
/// it survives the run because it is not a word, and like the page break it is
/// split out BEFORE words are.
pub const LISTING_BREAK: char = '\u{b}';

/// The lines of a code listing, or `None` when the paragraph is prose.
///
/// A listing arrives as `LISTING_BREAK`-TERMINATED lines, so an empty segment
/// is an empty code line and is kept: a blank line in a program is part of the
/// program, and a paragraph break -- which is what the lexer would have made of
/// it -- is not. Prose holds no break and is broken to the measure as before.
///
/// Both breakers ask this, so a listing is recognised the same way on the DVI
/// and PDF paths rather than in two places that could drift apart.
fn listing_lines(para: &str) -> Option<impl Iterator<Item = &str>> {
    para.contains(LISTING_BREAK)
        .then(|| para.split_terminator(LISTING_BREAK))
}

/// The start of a centred region, and, on a broken line, that the line is
/// centred.
///
/// `\begin{center}` and `\centering` were defined by the prelude to expand to
/// nothing, so "centred line" and "left line" came out as one flowing line and
/// a title page -- which is built entirely out of centred pieces -- collapsed
/// into the prose after it. Centring is carried the way a forced break is,
/// in the text itself: a region marker the breaker consumes, and then one
/// character at the head of each line it produced, which is what survives
/// pagination without a second structure beside the lines saying which of them
/// were centred.
pub const CENTRE: char = '\u{e}';

/// The end of a centred region, back to the margin.
pub const CENTRE_END: char = '\u{f}';

/// That a broken line is FULL, and so is set to the measure rather than at the
/// width its glyphs come to.
///
/// Justification is a property of the line and not of the text, so it cannot be
/// worked out again where the page is drawn: by then the paragraphs have been
/// flattened into one list of lines and nothing says which of them the author
/// ended and which the breaker did. The breaker knows -- it pushed the line
/// because the next word did not fit -- and says so at the head of the line,
/// the way a centred line is marked, which is what survives pagination without
/// a second structure beside the lines.
///
/// A device control character rather than a letter, for the same reason the
/// others are: it is not a character any document writes.
pub const JUSTIFY: char = '\u{13}';

/// A line's worth of vertical space with nothing drawn on it.
///
/// `\chapter` and `\section` set their heading hard against the paragraph
/// after it, so a heading was indistinguishable from the text it introduces
/// and the page held lines that lualatex spends on white space. A vertical tab
/// is what the character means; like the form feed it is split out of the text
/// before words are, because Rust counts it as whitespace too.
pub const VERTICAL_SPACE: char = '\u{10}';

/// The r, g and b a marker carried, kept verbatim as written rather than
/// parsed, because they are handed straight back to PDF's `rg` operator.
type Spec = (String, String, String);

/// A stretch of a line, the colour it is drawn in and the face it is set in.
///
/// Always a colour, never "none": the stack it comes off is seeded with the
/// document's default, so every run says what it is drawn in. The face is the
/// same -- the bottom of its stack is the main face.
type StyledRun = (String, Spec, Face);

/// The colour every LaTeX document starts in, as PDF wants it written.
///
/// A document that wants another says `\color`, which arrives here as a marker
/// and pushes on top of this one.
const DEFAULT_COLOUR: (&str, &str, &str) = ("0", "0", "0");

/// The markers the runtime writes turn colour on and off part way along a line,
/// so a line is not one string in one colour: `let x` may be black and `= 1`
/// blue. Each run is emitted with its own colour and its own position, which is
/// why the caller advances x by the width of what it just drew.
///
/// Colour is a STACK, not one current colour, because the markers nest:
/// `\color{textPrim}` at the top of a book is still in force inside the
/// `\texttt` that pushes neonCyan for one word and pops again. Popping to "no
/// colour" -- what this did -- meant that after the first closing marker
/// everything was drawn in black, on a page `\pagecolor` had painted #05050A:
/// 573,723 of the 715,546 drawn characters of `rubyrs/docs/book.tex`, a whole
/// book black on black. The DVI path has had this right all along, as `color
/// push rgb` / `color pop` at typeset.rs:117-137; this is the same thing.
///
/// The stack belongs to the CALLER and carries from line to line, because a
/// `\color` set once above the whole document is in force on every line under
/// it, not just the one the marker landed on. Its bottom entry is the
/// document's default and is never popped, so an unbalanced closing marker
/// leaves that in force rather than nothing at all.
///
/// The face markers are read in the SAME pass and split the line the same way,
/// because the two nest inside each other: a book's `\texttt` is a mono face
/// wrapped around a `\color`, and two passes would have to agree about where
/// the other one's markers were.
fn styled_runs(line: &str, stack: &mut Vec<Spec>, faces: &mut Vec<Face>) -> Vec<StyledRun> {
    let mut runs = Vec::new();
    let mut text = String::new();
    let mut chars = line.chars().peekable();
    let default: Spec = (
        DEFAULT_COLOUR.0.to_string(),
        DEFAULT_COLOUR.1.to_string(),
        DEFAULT_COLOUR.2.to_string(),
    );
    let top = |stack: &[Spec]| stack.last().cloned().unwrap_or_else(|| default.clone());
    while let Some(ch) = chars.next() {
        match ch {
            FACE_PUSH => {
                if !text.is_empty() {
                    runs.push((std::mem::take(&mut text), top(stack), current_face(faces)));
                }
                // The code character belongs to the marker whatever it is: a
                // marker read as one character would set the other as a glyph.
                let code = chars.next().unwrap_or_else(|| Face::Main.code());
                faces.push(Face::from_code(code));
            }
            FACE_POP => {
                if !text.is_empty() {
                    runs.push((std::mem::take(&mut text), top(stack), current_face(faces)));
                }
                // The bottom entry is the main face and is never popped, so an
                // unbalanced close leaves the document in its own face.
                if faces.len() > 1 {
                    faces.pop();
                }
            }
            '\u{1}' => {
                if !text.is_empty() {
                    runs.push((std::mem::take(&mut text), top(stack), current_face(faces)));
                }
                let mut spec = String::new();
                for c in chars.by_ref() {
                    if c == '\u{2}' {
                        break;
                    }
                    spec.push(c);
                }
                let p: Vec<&str> = spec.split(',').collect();
                // A spec that is not three components still pushes -- the
                // colour it inherits -- because the `\u{3}` that closes it is
                // coming either way and has to pop what this pushed, not the
                // entry underneath.
                let pushed = match p.len() == 3 {
                    true => (p[0].to_string(), p[1].to_string(), p[2].to_string()),
                    false => top(stack),
                };
                stack.push(pushed);
            }
            '\u{3}' => {
                if !text.is_empty() {
                    runs.push((std::mem::take(&mut text), top(stack), current_face(faces)));
                }
                if stack.len() > 1 {
                    stack.pop();
                }
            }
            c => text.push(c),
        }
    }
    if !text.is_empty() {
        runs.push((text, top(stack), current_face(faces)));
    }
    runs
}

/// The face in force, which is the main one when nothing has pushed.
fn current_face(faces: &[Face]) -> Face {
    faces.last().copied().unwrap_or_default()
}

/// What one word costs, in the faces it is set in.
///
/// A word can carry markers -- `\texttt{x}` is a marker, a letter and a marker
/// -- so it is not necessarily one width in one face. The plain case is the
/// one that runs a million times a book, so it is answered without splitting
/// anything.
fn word_width(
    word: &str,
    colours: &mut Vec<Spec>,
    faces: &mut Vec<Face>,
    width_of: &dyn Fn(&str, Face) -> f64,
) -> f64 {
    if !word.contains(['\u{1}', '\u{3}', FACE_PUSH, FACE_POP]) {
        return width_of(word, current_face(faces));
    }
    styled_runs(word, colours, faces)
        .iter()
        .map(|(text, _, face)| width_of(text, *face))
        .sum()
}

/// Find the file for a font family the document named.
///
/// `\setmainfont{Arimo}` names a FAMILY, not a path, and resolving one is what
/// fontconfig exists for. `fc-match` is asked first because it answers the way
/// every other program on the machine would; the directory walk is the fallback
/// for a machine without it. A family nothing matches returns `None` and the
/// caller falls back to naming one of the fourteen, which gets the widths right
/// and the shapes wrong -- better than refusing to typeset.
pub fn find_family(family: &str) -> Option<std::path::PathBuf> {
    if let Ok(out) = std::process::Command::new("fc-match")
        .args(["-f", "%{file}", family])
        .output()
    {
        if out.status.success() {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let path = std::path::Path::new(&p);
            // fc-match ALWAYS answers, with a default when it has no match, so
            // the answer is only useful if it looks like the family asked for.
            if path.exists() && file_matches(path, family) {
                return Some(path.to_path_buf());
            }
        }
    }
    let wanted = normalise(family);
    for dir in [
        "/System/Library/Fonts",
        "/System/Library/Fonts/Supplemental",
        "/Library/Fonts",
        "/usr/share/fonts",
        "/usr/local/share/fonts",
    ] {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext != "ttf" && ext != "otf" {
                continue;
            }
            if normalise(&path.file_stem()?.to_string_lossy()).starts_with(&wanted) {
                return Some(path);
            }
        }
    }
    None
}

/// A font file's stem, compared the way a family name should be: case and
/// spacing are not part of the identity.
fn file_matches(path: &std::path::Path, family: &str) -> bool {
    path.file_stem()
        .map(|s| normalise(&s.to_string_lossy()).starts_with(&normalise(family)))
        .unwrap_or(false)
}

fn normalise(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Load a family as an embeddable PDF font: its outlines, and its own widths.
///
/// This is the difference between a page set in Arimo's METRICS and one set in
/// Arimo. The widths come from the font's `hmtx` through its `cmap`, so a line
/// is measured in the face it will be printed in rather than in Computer
/// Modern's.
pub fn embed_family(family: &str) -> Option<crate::pdf::Font> {
    embed_file(&find_family(family)?)
}

/// Embed the font FILE a document supplied itself.
///
/// fontspec `Path=` names a directory of `.ttf` files that ship WITH the
/// document, which is how a book uses a face nobody has installed. Looking
/// such a name up among the installed families finds nothing, and fc-match
/// answers with a default -- so the book was set in whatever that default was.
pub fn embed_file(path: &std::path::Path) -> Option<crate::pdf::Font> {
    let bytes = std::fs::read(path).ok()?;
    let sfnt = crate::sfnt::Sfnt::parse(bytes.clone()).ok()?;
    // A CFF-flavoured OpenType carries PostScript outlines, which a
    // /FontFile2 must not: that entry means a TrueType font program.
    if sfnt.is_cff() {
        return None;
    }
    let head = sfnt.head().ok()?;
    let hhea = sfnt.hhea().ok()?;
    let cmap = sfnt.cmap().ok()?;
    let advances = sfnt.advance_widths().ok()?;
    let upem = f64::from(head.units_per_em.max(1));
    let scale = |v: f64| (v * 1000.0 / upem).round() as i64;

    // PDF wants a width for every code in the range, in 1/1000 em, so a code
    // the font has no glyph for still needs an entry.
    //
    // A code is a WinAnsi code -- that is the encoding the font is written with
    // -- and a `cmap` is indexed by Unicode, so the two are joined by what the
    // code MEANS. They agree from 0xA0 up, where WinAnsi is Latin-1, and part
    // company between 0x80 and 0x9F: reading code 0x97 as U+0097 finds nothing
    // and writes `.notdef`'s width where the em dash's belongs.
    let mut widths = Vec::with_capacity(224);
    for code in 32u32..=255 {
        let character = u8::try_from(code).ok().and_then(winansi_unicode);
        let gid = character
            .and_then(|c| cmap.get(&(c as u32)))
            .copied()
            .unwrap_or(0) as usize;
        let adv = advances
            .get(gid)
            .or_else(|| advances.last())
            .copied()
            .unwrap_or(0);
        widths.push(scale(f64::from(adv)));
    }
    Some(crate::pdf::Font::TrueType {
        name: path.file_stem()?.to_string_lossy().replace(' ', ""),
        bytes,
        widths,
        bbox: [
            scale(f64::from(head.x_min)),
            scale(f64::from(head.y_min)),
            scale(f64::from(head.x_max)),
            scale(f64::from(head.y_max)),
        ],
        ascent: scale(f64::from(hhea.ascender)),
        descent: scale(f64::from(hhea.descender)),
    })
}

#[cfg(test)]
mod font_file_tests {
    use super::FontFile;

    /// The options a real book writes, verbatim.
    const REAL: &str = "\n    Path=/private/tmp/build/scifi2/docs/.fonts/,\n    Extension=.ttf,\n    RawFeature={fallback=symfb},\n    UprightFont=Arimo-VF,\n    UprightFeatures={RawFeature={axis={wght=400}}},\n    BoldFont=Arimo-VF,\n    BoldFeatures={RawFeature={axis={wght=700}}},\n";

    #[test]
    fn the_options_a_document_ships_its_font_with() {
        let spec = FontFile::parse(REAL);
        assert_eq!(spec.upright.as_deref(), Some("Arimo-VF"));
        assert_eq!(spec.extension.as_deref(), Some(".ttf"));
        assert_eq!(
            spec.path.as_deref(),
            Some("/private/tmp/build/scifi2/docs/.fonts/")
        );
    }

    #[test]
    fn the_face_files_are_read_from_the_same_options() {
        // `BoldFont=` and `ItalicFont=` sit in the option list `UprightFont=`
        // does, and reading past them is why `\textbf` and `\emph` were set in
        // the upright face. The corpus's bold IS the upright file at another
        // weight, which is honest to report as such: what is asserted is that
        // the key was READ, not that the file differs.
        let spec = FontFile::parse(REAL);
        assert_eq!(spec.bold.as_deref(), Some("Arimo-VF"));
        assert_eq!(spec.italic.as_deref(), None, "this book names no italic");
        let both = FontFile::parse("Extension=.ttf,UprightFont=A,ItalicFont=A-Italic");
        assert_eq!(both.italic.as_deref(), Some("A-Italic"));
        // A face the document named no file for resolves to nothing, whatever
        // is beside the document, so the caller falls back to the main face
        // rather than setting the italic in some file that happens to be there.
        assert_eq!(both.resolve_face(super::Face::Bold, None), None);
    }

    #[test]
    fn a_value_holding_commas_does_not_tear_the_keys_after_it() {
        // `UprightFeatures={RawFeature={axis={wght=400}}}` sits BEFORE the keys
        // that matter in some documents. Splitting on every comma leaves
        // `wght=400}}}` as a key and loses everything past it.
        let spec = FontFile::parse("Extension=.ttf,Numbers={Proportional,Lining},UprightFont=X-VF");
        assert_eq!(spec.upright.as_deref(), Some("X-VF"));
        assert_eq!(spec.extension.as_deref(), Some(".ttf"));
    }

    #[test]
    fn a_stale_path_is_retried_beside_the_document() {
        // `Path=` is written when the document is BUILT and regularly points
        // into a scratch directory that is gone by the time it is read. The
        // fonts ship with the document, so the last component is tried there.
        let dir = std::env::temp_dir().join(format!("texrs_ff_{}", std::process::id()));
        let fonts = dir.join(".fonts");
        std::fs::create_dir_all(&fonts).expect("mkdir");
        let file = fonts.join("Arimo-VF.ttf");
        std::fs::write(&file, b"not really a font").expect("write");

        let spec =
            FontFile::parse("Path=/gone/for/good/.fonts/,Extension=.ttf,UprightFont=Arimo-VF");
        assert_eq!(spec.resolve(Some(&dir)), Some(file.clone()));
        // With nowhere to look, it resolves to nothing rather than to a guess.
        assert_eq!(spec.resolve(None), None);

        // A path that IS there wins outright.
        let direct = format!(
            "Path={}/,Extension=.ttf,UprightFont=Arimo-VF",
            fonts.display()
        );
        assert_eq!(FontFile::parse(&direct).resolve(None), Some(file));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn options_that_name_no_file_resolve_to_nothing() {
        // `\setmainfont{Georgia}` with no options at all must fall through to
        // the installed families rather than resolving to some file nearby.
        assert_eq!(
            FontFile::parse("").resolve(Some(std::path::Path::new("/tmp"))),
            None
        );
    }
}
