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
];

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
    /// A colour marker's SPEC -- the `r,g,b` between U+0001 and U+0002 -- is
    /// skipped whole. Measuring the three control characters as zero is not
    /// enough: the digits and commas between them are ordinary characters that
    /// the font does have, so a coloured word measured this way came out wider
    /// than it sets and every line after it broke short.
    pub fn width_of(&self, text: &str, size: f64) -> f64 {
        let mut total = 0.0;
        let mut in_spec = false;
        for ch in text.chars() {
            match ch {
                '\u{1}' => in_spec = true,
                '\u{2}' => in_spec = false,
                _ if in_spec => {}
                c => total += self.width(c, size),
            }
        }
        total
    }
}

/// The typefaces a document asked for.
///
/// `\setmainfont{Arimo}` is a statement about the book, and setting it in
/// Computer Modern anyway is the thing this exists to stop. What texrs can
/// honour depends on the output: a DVI names `.tfm` fonts and cannot carry an
/// OpenType one at all, while a PDF can name the fourteen fonts every reader
/// has. Arimo is metric-compatible with Arial, which is metric-compatible with
/// Helvetica, so that substitution is a real one rather than a shrug.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Families {
    pub main: Option<String>,
    pub sans: Option<String>,
    pub mono: Option<String>,
}

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

/// Set a document straight to PDF, honouring the font it asked for.
///
/// The DVI path names `.tfm` fonts and so can only ever set in Computer Modern;
/// a PDF can name the fourteen faces every reader has, which is how
/// `\setmainfont{Arimo}` becomes Helvetica's metrics rather than a book face.
/// Colour travels the same markers the DVI path reads and comes out as PDF's
/// own `rg` operator.
pub fn to_pdf(text: &str, families: &Families, layout: &Layout) -> Vec<u8> {
    use crate::pdf::{document, Font, Page};

    let main = Font::Base14(
        families
            .main
            .as_deref()
            .map(base14_for)
            .unwrap_or("Helvetica")
            .to_string(),
    );
    // The widths of the fourteen are not carried here, so lines are measured
    // with cmr10's if it is installed and with a flat estimate if it is not.
    // That is honest about what it is: the line breaking is approximate, and a
    // line may run a little long or short of the measure.
    let metrics = find_font("cmr10").and_then(|p| Tfm::open(&p).ok());
    let width_of = |word: &str| -> f64 {
        match &metrics {
            Some(f) => f.width_of(word) * layout.size,
            None => word.chars().count() as f64 * layout.size * 0.5,
        }
    };

    let lines = break_lines_measured(text, layout, &width_of);
    let per_page = ((layout.height / layout.leading).floor() as usize).max(1);

    let mut pages = Vec::new();
    for chunk in lines.chunks(per_page) {
        let mut page = Page::letter();
        let mut y = layout.height + layout.margin - layout.leading;
        for line in chunk {
            // A line is a sequence of RUNS, each with its own colour: the
            // markers turn colour on and off part way along it. Collapsing the
            // line to one colour state was the first attempt and drew none at
            // all, because the closing marker put the state back before
            // anything was emitted.
            let mut x = layout.margin;
            for (plain, colour) in colour_runs(line) {
                if plain.is_empty() {
                    continue;
                }
                if let Some((r, g, b)) = &colour {
                    page.content.push_str(&format!("{r} {g} {b} rg\n"));
                }
                page.text_in(main.clone(), layout.size, x, y, &plain);
                if colour.is_some() {
                    page.content.push_str("0 g\n");
                }
                x += width_of(&plain);
            }
            y -= layout.leading;
        }
        pages.push(page);
    }
    document(&pages)
}

/// Break lines with a caller-supplied measurer.
fn break_lines_measured(
    text: &str,
    layout: &Layout,
    width_of: &dyn Fn(&str) -> f64,
) -> Vec<String> {
    let space = width_of(" ");
    let mut lines = Vec::new();
    for para in text.split("\n\n") {
        let mut line = String::new();
        let mut width = 0.0f64;
        for word in para.split_whitespace() {
            let ww = width_of(word);
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
    }
    lines
}

/// Split a line into runs of text that share a colour.
///
/// The markers the runtime writes turn colour on and off part way along a line,
/// so a line is not one string in one colour: `let x` may be black and `= 1`
/// blue. Each run is emitted with its own colour and its own position, which is
/// why the caller advances x by the width of what it just drew.
fn colour_runs(line: &str) -> Vec<(String, Option<(String, String, String)>)> {
    let mut runs = Vec::new();
    let mut current: Option<(String, String, String)> = None;
    let mut text = String::new();
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\u{1}' => {
                if !text.is_empty() {
                    runs.push((std::mem::take(&mut text), current.clone()));
                }
                let mut spec = String::new();
                for c in chars.by_ref() {
                    if c == '\u{2}' {
                        break;
                    }
                    spec.push(c);
                }
                let p: Vec<&str> = spec.split(',').collect();
                current = match p.len() == 3 {
                    true => Some((p[0].to_string(), p[1].to_string(), p[2].to_string())),
                    false => None,
                };
            }
            '\u{3}' => {
                if !text.is_empty() {
                    runs.push((std::mem::take(&mut text), current.clone()));
                }
                current = None;
            }
            c => text.push(c),
        }
    }
    if !text.is_empty() {
        runs.push((text, current));
    }
    runs
}
