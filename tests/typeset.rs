//! Typesetting: text and font metrics into DVI pages.
//!
//! texrs could read a document and say what its words were, and could read and
//! write DVI, and nothing joined the two -- `dvi::Writer` was never called
//! outside its own tests, so a book "ran" and produced no page.
//!
//! What is asserted here is that a real DVI comes out and that the arithmetic
//! is the font's rather than invented. What is NOT claimed is `tex.web`'s
//! typesetting: lines are broken first-fit, where tex minimises total badness
//! over a whole paragraph (§813), and there is no hyphenation, no glue, one
//! font and no maths.

use texrs::tfm::Tfm;
use texrs::typeset::{
    break_lines, find_font, to_dvi, to_dvi_chain, FontChain, Layout, LISTING_BREAK,
};

/// The text font these tests measure in, or `None` where TeX is not installed.
///
/// The metrics belong to an INSTALLATION, not to this crate: `find_font` asks
/// `kpsewhich` and then walks the texmf trees, and a machine with neither has
/// no cmr10.tfm to read. tests/fontmap.rs guards the same way and returns
/// early, for the same reason. Installing them part-way is worse than not at
/// all -- `texlive-base` carries cmr10 and cmsy10 and makes these thirteen
/// pass, and then fontmap.rs starts asserting against an installation that
/// names 438 fonts where it wants more than a thousand.
///
/// So: on any machine with TeX -- every developer machine -- all of these run
/// and assert in full. Where there is none they say so and stop.
fn font() -> Option<Tfm> {
    let path = find_font("cmr10")?;
    // Found but unreadable is a real fault and still fails: only the ABSENCE
    // of an installation is a reason not to run.
    Some(Tfm::open(&path).expect("cmr10.tfm was found but could not be read"))
}

/// The font, or leave the test unrun and say why.
macro_rules! font_or_skip {
    () => {
        match font() {
            Some(font) => font,
            None => {
                eprintln!("skipping: no TeX installation, so there are no metrics to measure in");
                return;
            }
        }
    };
}

/// The same, for a test that needs the fallback chain rather than one font.
macro_rules! chain_or_skip {
    () => {
        match FontChain::load("cmr10", &["cmsy10"]) {
            Ok(chain) => chain,
            Err(_) => {
                eprintln!("skipping: no TeX installation, so there are no metrics to measure in");
                return;
            }
        }
    };
}

#[test]
fn a_dvi_file_is_produced_and_parses_as_one() {
    let f = font_or_skip!();
    let dvi = to_dvi("hello world", &f, "cmr10", &Layout::default());
    let parsed = texrs::dvi::Dvi::parse(&dvi).expect("texrs must read back what it wrote");
    assert_eq!(parsed.pages(), 1, "one line of text is one page");
}

#[test]
fn every_line_fits_the_measure() {
    // The whole job of line breaking. A line wider than the measure would run
    // off the page, and nothing downstream would catch it.
    let f = font_or_skip!();
    let layout = Layout::default();
    let text = "the quick brown fox jumps over the lazy dog ".repeat(80);
    for line in break_lines(&text, &f, &layout) {
        let w = f.width_of(&line) * layout.size;
        assert!(
            w <= layout.measure + 0.01,
            "line is {w:.1}pt against a {:.1}pt measure: {line:?}",
            layout.measure
        );
    }
}

#[test]
fn a_word_too_wide_to_fit_still_gets_a_line() {
    // First-fit has to make progress even when a single word exceeds the
    // measure, or it loops forever or drops the word.
    let f = font_or_skip!();
    let long = "x".repeat(400);
    let lines = break_lines(&long, &f, &Layout::default());
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0], long, "the word is kept whole, not truncated");
}

#[test]
fn paragraphs_stay_in_order_though_they_are_broken_in_parallel() {
    // Broken with rayon across paragraphs. They are independent, but their
    // ORDER is the document: a book whose paragraphs arrived in completion
    // order would be a different book.
    let f = font_or_skip!();
    let text: String = (0..200)
        .map(|i| format!("para{i} some filler words here\n\n"))
        .collect();
    let lines = break_lines(&text, &f, &Layout::default());
    let firsts: Vec<usize> = lines
        .iter()
        .filter_map(|l| l.strip_prefix("para"))
        .filter_map(|r| r.split_whitespace().next())
        .filter_map(|n| n.parse().ok())
        .collect();
    assert_eq!(firsts.len(), 200, "every paragraph is there");
    assert!(firsts.windows(2).all(|w| w[0] < w[1]), "and in order");
}

#[test]
fn a_long_document_becomes_more_than_one_page() {
    let f = font_or_skip!();
    let text = "word ".repeat(20_000);
    let dvi = to_dvi(&text, &f, "cmr10", &Layout::default());
    let parsed = texrs::dvi::Dvi::parse(&dvi).expect("parse");
    assert!(parsed.pages() > 1, "got {} pages", parsed.pages());
}

#[test]
fn the_text_survives_the_round_trip_through_dvi() {
    // The characters that went in are the characters a reader gets back, which
    // is what makes the page the document rather than a plausible shape.
    let f = font_or_skip!();
    let dvi = to_dvi("typesetting works", &f, "cmr10", &Layout::default());
    let parsed = texrs::dvi::Dvi::parse(&dvi).expect("parse");
    let got = parsed.text();
    assert!(got.contains("typesetting"), "got {got:?}");
    assert!(got.contains("works"), "got {got:?}");
}

#[test]
fn a_glyph_the_text_font_lacks_comes_from_a_fallback() {
    // `luaotfload.add_fallback` in a TFM world, and the reason the publication
    // scripts required LuaTeX: cmr10 has no arrow, cmsy10 does.
    let chain = chain_or_skip!();
    let (font, slot) = chain.resolve('→').expect("an arrow must resolve");
    assert_eq!(chain.fonts[font].name, "cmsy10");
    assert_eq!(slot, 33, "the slot tex itself sets for \\rightarrow");
}

#[test]
fn the_section_mark_comes_from_the_symbol_font_not_the_text_font() {
    // This table was written wrong the first time: `§` pointed at cmr10 slot
    // 120, which is an `x`, and the page said "x" where the document said "§"
    // without anything reporting a problem.
    let chain = chain_or_skip!();
    let (font, slot) = chain.resolve('§').expect("a section mark must resolve");
    assert_eq!(chain.fonts[font].name, "cmsy10");
    assert_eq!(slot, 120);
}

#[test]
fn ascii_still_comes_from_the_text_font() {
    let chain = chain_or_skip!();
    for ch in ['a', 'Z', '7', '.', ' '] {
        let (font, slot) = chain.resolve(ch).expect("ascii resolves");
        assert_eq!(chain.fonts[font].name, "cmr10", "for {ch:?}");
        assert_eq!(slot, ch as u8);
    }
}

#[test]
fn a_shape_no_font_carries_is_approximated_rather_than_dropped() {
    // Computer Modern has no box drawing at all, and these documents draw trees
    // with it. A glyph that vanishes takes the meaning of the line with it.
    assert_eq!(FontChain::approximate('├'), Some("|-"));
    assert_eq!(FontChain::approximate('─'), Some("-"));
    assert_eq!(FontChain::approximate('—'), Some("---"));
    assert_eq!(FontChain::approximate('©'), Some("(c)"));
    assert_eq!(FontChain::approximate('a'), None, "ascii needs no stand-in");
}

#[test]
fn a_fallback_glyph_reaches_the_dvi_and_switches_font() {
    // The end of the chain: the arrow must appear in the file as cmsy10's
    // slot 33, with a font switch before it, or the page shows whatever cmr10
    // happens to have at that position.
    let chain = chain_or_skip!();
    let dvi = to_dvi_chain("a → b", &chain, &Layout::default());
    let parsed = texrs::dvi::Dvi::parse(&dvi).expect("parse");
    let summary = parsed.summary();
    assert!(
        summary.contains("cmsy10"),
        "the fallback font is used: {summary}"
    );
    assert_eq!(parsed.pages(), 1);
}

#[test]
fn a_missing_fallback_is_not_an_error() {
    // The chain degrades to what is installed; the approximations catch what no
    // loaded font carries.
    //
    // The claim is that the MISSING fallback is tolerated, which only means
    // anything where the base font is there to load: with no installation at
    // all neither loads, and the test would be asserting nothing.
    let _base = font_or_skip!();
    let chain = FontChain::load("cmr10", &["definitely-not-a-font"]).expect("still loads");
    assert_eq!(chain.fonts.len(), 1);
    assert!(chain.resolve('a').is_some());
}

#[test]
fn colour_reaches_the_dvi_as_a_special() {
    // DVI has no colour of its own: a driver is told through a `\special`, and
    // `color push rgb R G B` / `color pop` is the pair dvipdfmx and dvips both
    // read. dvipdfmx turns them into `1 0 0 rg` around the words.
    let src = "\\documentclass{article}\n\\begin{document}\n\
               plain \\textcolor[rgb]{1.00,0.00,0.00}{RED} plain\n\\end{document}\n";
    let text = texrs::run_text_marked(src).expect("run");
    let chain = chain_or_skip!();
    let dvi = to_dvi_chain(&text, &chain, &Layout::default());
    let s = String::from_utf8_lossy(&dvi);
    assert!(
        s.contains("color push rgb 1 0 0"),
        "no colour push in the file"
    );
    assert!(
        s.contains("color pop"),
        "a push without a pop leaves the page red"
    );
}

#[test]
fn the_colour_markers_are_not_part_of_the_document_text() {
    // They are instructions to a driver. A caller asking for the TEXT should
    // not have to know they exist, and a `--text` run that emitted control
    // characters would be wrong for every consumer of it.
    let src = "\\documentclass{article}\n\\begin{document}\n\
               \\textcolor[rgb]{0.25,0.44,0.63}{code}\n\\end{document}\n";
    let plain = texrs::run_text(src).expect("run");
    assert_eq!(plain.trim(), "code");
    assert!(!plain
        .chars()
        .any(|c| matches!(c, '\u{1}' | '\u{2}' | '\u{3}')));
}

#[test]
fn a_colour_marker_takes_no_space_on_the_line() {
    // Measured as glyphs they would push words onto the next line for text that
    // is not there, and every line after would be wrong.
    let chain = chain_or_skip!();
    let layout = Layout::default();
    let plain = "word word word";
    let marked = "\u{1}1,0,0\u{2}word word word\u{3}";
    assert_eq!(
        chain.width_of(plain, layout.size),
        chain.width_of(marked, layout.size),
        "the markers must measure zero"
    );
}

#[test]
fn the_font_a_document_asks_for_is_the_font_it_gets() {
    // The complaint this answers: everything set in Computer Modern however
    // loudly the document said \setmainfont. The mapping is by what the face
    // IS -- Arimo carries Arial's metrics and Arial carries Helvetica's -- not
    // by matching the name.
    use texrs::typeset::base14_for;
    assert_eq!(base14_for("Arimo"), "Helvetica");
    assert_eq!(base14_for("Liberation Sans"), "Helvetica");
    assert_eq!(base14_for("ShareTechMono"), "Courier");
    assert_eq!(base14_for("JetBrains Mono"), "Courier");
    assert_eq!(base14_for("Times New Roman"), "Times-Roman");
    assert_eq!(base14_for("STIX Two Text"), "Times-Roman");
    // A face nothing is known about still is not a book font: a document that
    // named one at all was asking not to be set in Computer Modern.
    assert_eq!(base14_for("Orbitron"), "Helvetica");
}

#[test]
fn setmainfont_reaches_the_pdf() {
    let src = |f: &str| {
        format!(
            "\\documentclass{{article}}\n\\usepackage{{fontspec}}\n\\setmainfont{{{f}}}\n\
             \\begin{{document}}\nThe quick brown fox.\n\\end{{document}}\n"
        )
    };
    // The contract is that the REQUEST reaches the file, by one of two routes:
    // the font itself when the machine has it, and a face carrying the same
    // widths when it does not. Which route a given family takes depends on
    // what is installed, so the test asserts the outcome rather than the route.
    let mono = texrs::run_pdf(&src("ShareTechMono")).expect("pdf");
    let mono = String::from_utf8_lossy(&mono);
    assert!(
        mono.contains("/Courier") || mono.contains("/FontFile2"),
        "a monospace request reaches the file as Courier or as itself"
    );
    // A face nothing is known about must still not fall back to a book font:
    // a document that named one at all was asking not to be set in Computer
    // Modern.
    let unknown = texrs::run_pdf(&src("NoSuchFontExistsAnywhere")).expect("pdf");
    let unknown = String::from_utf8_lossy(&unknown);
    assert!(unknown.contains("/Helvetica"), "got {unknown:?}");
    assert!(!unknown.contains("/FontFile2"), "nothing to embed");
}

#[test]
fn colour_survives_the_pdf_path_as_pdfs_own_operator() {
    let src = "\\documentclass{article}\n\\begin{document}\n\
               plain \\textcolor[rgb]{1.00,0.00,0.00}{RED} plain\n\\end{document}\n";
    let pdf = texrs::run_pdf(src).expect("pdf");
    let s = String::from_utf8_lossy(&pdf);
    assert!(s.contains("1 0 0 rg"), "the colour is set");
    // And put back after -- as the colour underneath, said in full. It used to
    // be the literal `0 g`, which is only right when the colour underneath
    // happens to be black; a book that sets a body colour is then drawn in
    // black for everything after its first `\textcolor`.
    assert!(s.contains("0 0 0 rg"), "and put back after: {s:?}");
}

#[test]
fn a_line_is_split_into_runs_where_the_colour_changes() {
    // A colour marker turns colour on part way ALONG a line, so a line is not
    // one string in one colour. Treating it as one drew no colour at all: the
    // closing marker put the state back before anything was emitted.
    let src = "\\documentclass{article}\n\\begin{document}\n\
               before \\textcolor[rgb]{0,0,1}{middle} after\n\\end{document}\n";
    let pdf = texrs::run_pdf(src).expect("pdf");
    let s = String::from_utf8_lossy(&pdf);
    assert!(s.contains("(before )"), "the run before the colour: {s:?}");
    assert!(s.contains("(middle)"), "the coloured run");
    assert!(s.contains("0 0 1 rg"), "with the colour set for it");
}

/// Every run of text a PDF draws, with the fill colour in force when it is
/// drawn.
///
/// The content streams texrs writes are not compressed, so the operators can be
/// read straight out of the file: `R G B rg` sets the colour, `(text) Tj` draws
/// under it. This is what a reader does with the page, which is the only way to
/// ask what colour a word actually came out in.
fn drawn(pdf: &[u8]) -> Vec<(String, String)> {
    let s = String::from_utf8_lossy(pdf).into_owned();
    let mut runs = Vec::new();
    let mut colour = String::from("none");
    for (at, _) in s.match_indices(['g', 'j']) {
        let head = &s[..at + 1];
        if let Some(spec) = head.strip_suffix(" rg") {
            // The three components before the operator, taken from the back:
            // the one before them is a newline, or `stream`, or whatever the
            // object header ended with.
            let mut back = spec.split_ascii_whitespace().rev();
            if let (Some(b), Some(g), Some(r)) = (back.next(), back.next(), back.next()) {
                if [r, g, b].iter().all(|n| n.parse::<f64>().is_ok()) {
                    colour = format!("{r} {g} {b}");
                }
            }
            continue;
        }
        if !head.ends_with(") Tj") {
            continue;
        }
        // Back over the string to its opening bracket. The documents below draw
        // no brackets of their own, so the nearest one is where the run starts.
        let body = &head[..head.len() - 4];
        if let Some(open) = body.rfind('(') {
            runs.push((colour.clone(), body[open + 1..].to_string()));
        }
    }
    runs
}

#[test]
fn a_colour_switch_survives_the_textcolor_nested_inside_it() {
    // `\color` is in force until its group closes, so the `\textcolor` inside
    // it is a colour ON TOP of it, not instead of it: what follows goes back to
    // the switch, not to black. Popping to black is what drew every book in the
    // corpus black on its own #05050A page -- they all say `\color{textPrim}`
    // once and then use `\textcolor` for code, 683,577 times.
    let src = "\\documentclass{article}\n\\begin{document}\n\
               \\color[rgb]{0.87,0.94,1}before \
               \\textcolor[rgb]{0,0,1}{middle} after\n\\end{document}\n";
    let pdf = texrs::run_pdf(src).expect("pdf");
    let runs = drawn(&pdf);
    let colour_of = |want: &str| {
        runs.iter()
            .find(|(_, text)| text.contains(want))
            .map(|(colour, _)| colour.clone())
            .unwrap_or_else(|| panic!("nothing drawn containing {want:?}: {runs:?}"))
    };
    assert_eq!(colour_of("before"), "0.87 0.94 1", "the switch is in force");
    assert_eq!(
        colour_of("middle"),
        "0 0 1",
        "the nested colour on top of it"
    );
    assert_eq!(
        colour_of("after"),
        "0.87 0.94 1",
        "and the switch is back after it: {runs:?}"
    );
}

#[test]
fn a_colour_switch_holds_for_every_line_under_it() {
    // The switch is set once, above a whole document, and the text under it is
    // broken into lines afterwards. A colour that only lasted the line its
    // marker landed on coloured the first line of a book and left the other
    // 339 pages black.
    let body = "sentence ".repeat(120);
    let src = format!(
        "\\documentclass{{article}}\n\\begin{{document}}\n\
         \\color[rgb]{{0.87,0.94,1}}{body}\n\\end{{document}}\n"
    );
    let pdf = texrs::run_pdf(&src).expect("pdf");
    let runs = drawn(&pdf);
    let drawn_lines = runs.iter().filter(|(_, t)| t.contains("sentence")).count();
    assert!(drawn_lines > 3, "the text wrapped: {drawn_lines} lines");
    for (colour, text) in &runs {
        assert_eq!(
            colour, "0.87 0.94 1",
            "line drawn in the wrong colour: {text:?}"
        );
    }
}

/// A family that exists on the machine running the tests, or `None`.
///
/// Font availability is not a property of the code, so a test that needs a real
/// font says so and skips rather than failing on a machine without one.
fn some_installed_family() -> Option<&'static str> {
    [
        "Georgia",
        "Verdana",
        "Arial",
        "DejaVu Sans",
        "Liberation Sans",
    ]
    .into_iter()
    .find(|f| texrs::typeset::find_family(f).is_some())
}

#[test]
fn the_font_file_the_document_named_is_carried_in_the_pdf() {
    // Naming one of the fourteen gets Arimo's WIDTHS via Helvetica and the
    // wrong shapes. This is the font itself: `/FontFile2` is the font program,
    // and `pdffonts` reports such a file as embedded.
    let Some(family) = some_installed_family() else {
        eprintln!("skipping: no known font installed");
        return;
    };
    let src = format!(
        "\\documentclass{{article}}\n\\usepackage{{fontspec}}\n\\setmainfont{{{family}}}\n\
         \\begin{{document}}\nThe quick brown fox.\n\\end{{document}}\n"
    );
    let pdf = texrs::run_pdf(&src).expect("pdf");
    let s = String::from_utf8_lossy(&pdf);
    assert!(
        s.contains("/FontFile2"),
        "the font program must be in the file"
    );
    assert!(s.contains("/TrueType"), "and declared as one");
    assert!(
        s.contains("/Widths"),
        "with its own widths, not a substitute's"
    );
}

#[test]
fn a_family_nothing_matches_still_typesets() {
    // Refusing to set a document because a font is missing would be worse than
    // setting it in a face that carries the same widths.
    let src = "\\documentclass{article}\n\\usepackage{fontspec}\n\
               \\setmainfont{NoSuchFontExistsAnywhere}\n\
               \\begin{document}\nwords\n\\end{document}\n";
    let pdf = texrs::run_pdf(src).expect("pdf");
    let s = String::from_utf8_lossy(&pdf);
    assert!(s.contains("/BaseFont"), "something was named");
    assert!(!s.contains("/FontFile2"), "but nothing was embedded");
}

#[test]
fn an_embedded_font_is_measured_in_its_own_widths() {
    // A line broken with Computer Modern's widths and set in another face runs
    // long or short. The widths come from the font's hmtx through its cmap.
    let Some(family) = some_installed_family() else {
        return;
    };
    let font = texrs::typeset::embed_family(family).expect("embeddable");
    let texrs::pdf::Font::TrueType { widths, .. } = &font else {
        panic!("expected a TrueType font");
    };
    assert_eq!(widths.len(), 224, "codes 32..=255 inclusive");
    // A space is narrower than an M in every text face there is.
    let space = widths[0];
    let em = widths[('M' as usize) - 32];
    assert!(space < em, "space {space} should be narrower than M {em}");
}

/// `\setmainfont{Arimo}[Path=..., UprightFont=Arimo-VF]` names a FILE that
/// ships with the document rather than an installed family. Reading the family
/// and dropping the options is how a book whose font nobody has installed came
/// out in whatever fc-match answered with.
#[test]
fn the_lowerer_keeps_the_font_file_the_preamble_named() {
    let src = concat!(
        "\\documentclass{article}\n\\usepackage{fontspec}\n",
        "\\setmainfont{Arimo}[\n",
        "    Path=/somewhere/.fonts/,\n",
        "    Extension=.ttf,\n",
        "    RawFeature={fallback=symfb},\n",
        "    UprightFont=Arimo-VF,\n",
        "]\n",
        "\\begin{document}\nwords\n\\end{document}\n"
    );
    let mut lowerer = texrs::lower::Lowerer::new().with_text_output();
    lowerer.preload(texrs::latex::PRELUDE).expect("prelude");
    lowerer.lower(src).expect("lower");
    assert_eq!(lowerer.fonts.main.as_deref(), Some("Arimo"));
    assert_eq!(
        lowerer.fonts.main_file.upright.as_deref(),
        Some("Arimo-VF"),
        "the options were read and thrown away"
    );
    assert_eq!(
        lowerer.fonts.main_file.path.as_deref(),
        Some("/somewhere/.fonts/")
    );
}

/// `\newpage` and its siblings were defined by the prelude to expand to
/// nothing, so a book's title page, copyright page and first chapter ran
/// together into one stream of prose: the scifi2 novel came out at 144 pages
/// where lualatex sets it at 270, and page one held the title, the copyright
/// notice and the opening of the next section drawn over each other.
#[test]
fn a_forced_break_starts_a_new_page() {
    let src = concat!(
        "\\documentclass{article}\n\\begin{document}\n",
        "first page text\n\\newpage\nsecond page text\n",
        "\\clearpage\nthird page text\n\\end{document}\n"
    );
    let pdf = texrs::run_pdf(src).expect("pdf");
    assert_eq!(
        count_pages(&pdf),
        3,
        "one page per break, not one page total"
    );
}

#[test]
fn consecutive_breaks_do_not_make_a_blank_page() {
    // `\clearpage` right after `\newpage` is one break between two pages.
    // Emitting a page per marker would leave an empty sheet between chapters.
    let src = concat!(
        "\\documentclass{article}\n\\begin{document}\n",
        "one\n\\newpage\n\\clearpage\ntwo\n\\end{document}\n"
    );
    assert_eq!(count_pages(&texrs::run_pdf(src).expect("pdf")), 2);
}

#[test]
fn a_chapter_begins_a_page_in_both_its_forms() {
    // `\chapter` was `#1` in the prelude -- the heading text and nothing else
    // -- so no chapter began a page. The starred form is one token different
    // and, unread, that `*` becomes the first character of the heading.
    let src = concat!(
        "\\documentclass{report}\n\\begin{document}\n",
        "front matter\n\\chapter{First}\nbody one\n",
        "\\chapter*{Unnumbered}\nbody two\n\\end{document}\n"
    );
    let pdf = texrs::run_pdf(src).expect("pdf");
    assert_eq!(count_pages(&pdf), 3);
    let text = String::from_utf8_lossy(&pdf);
    assert!(
        !text.contains("*Unnumbered"),
        "the star leaked into the heading"
    );
}

/// Pages in a PDF, counted from the page objects themselves.
fn count_pages(pdf: &[u8]) -> usize {
    String::from_utf8_lossy(pdf)
        .matches("/Type /Page\n")
        .count()
        .max(
            String::from_utf8_lossy(pdf).matches("/Type /Page").count()
                - String::from_utf8_lossy(pdf).matches("/Type /Pages").count(),
        )
}
/// The same words, once plain and once with every one of them coloured.
fn coloured_and_plain(preamble: &str, repeats: usize) -> (usize, usize) {
    let doc = |body: String| {
        format!("\\documentclass{{article}}\n{preamble}\\begin{{document}}\n{body}\n\\end{{document}}\n")
    };
    let plain = texrs::run_pdf(&doc("alpha ".repeat(repeats))).expect("pdf");
    let marked = doc("\\textcolor[rgb]{0.25,0.44,0.63}{alpha} ".repeat(repeats));
    let marked = texrs::run_pdf(&marked).expect("pdf");
    (count_pages(&plain), count_pages(&marked))
}

#[test]
fn colouring_a_word_does_not_change_where_the_pdf_breaks_the_line() {
    // The marker's SPEC -- `0.25,0.44,0.63` between U+0001 and U+0002 -- is
    // digits and commas, and the widths tables the PDF path measures with have
    // real widths for those, so a five-letter word inside one \textcolor was
    // charged for twenty-two characters. Measured: a line held four coloured
    // words where the same line uncoloured holds seventeen, and
    // rubyrs/docs/book.tex set in 340 pages where it sets in 186 with the
    // markers skipped. The DVI path has skipped them since it was written;
    // this is the same skip, reached through the same helper.
    let (plain, coloured) = coloured_and_plain("", 600);
    assert_eq!(
        coloured, plain,
        "colour is not text and must not push words onto later pages"
    );
}

#[test]
fn a_coloured_word_costs_nothing_in_an_embedded_fonts_own_widths() {
    // The branch above measures in cmr10's metrics; a document that ships or
    // names a real font is measured in that font's `/Widths`, which is the
    // branch every book in the corpus takes. Both had to be given the skip.
    let Some(family) = some_installed_family() else {
        eprintln!("skipping: no known font installed");
        return;
    };
    let preamble = format!("\\usepackage{{fontspec}}\n\\setmainfont{{{family}}}\n");
    let (plain, coloured) = coloured_and_plain(&preamble, 600);
    assert_eq!(coloured, plain, "the spec digits were charged as glyphs");
}

/// Every `... Tf 1 0 0 1 x y Tm` in a PDF this crate wrote, as
/// `(size, x, baseline)`.
///
/// The content streams texrs writes are uncompressed, so the operators can be
/// read straight out of the bytes; that is how the page it actually set is
/// checked rather than how many pages came out.
fn set_text(pdf: &[u8]) -> Vec<(f64, f64, f64)> {
    let text = String::from_utf8_lossy(pdf).into_owned();
    let mut out = Vec::new();
    for run in text.split("BT /").skip(1) {
        let mut words = run.split_whitespace();
        let size = words.nth(1).and_then(|w| w.parse::<f64>().ok());
        // `Tf 1 0 0 1 x y Tm`: five words between the size and x.
        let x = words.nth(5).and_then(|w| w.parse::<f64>().ok());
        let y = words.next().and_then(|w| w.parse::<f64>().ok());
        if let (Some(size), Some(x), Some(y)) = (size, x, y) {
            out.push((size, x, y));
        }
    }
    out
}

/// A document that states a type size and a margin, with enough words in one
/// paragraph to need several lines.
fn sized_document(class_options: &str, margin: &str) -> String {
    format!(
        "\\documentclass[{class_options}]{{extreport}}\n\
         \\usepackage[margin={margin}]{{geometry}}\n\
         \\begin{{document}}\n{}\n\\end{{document}}\n",
        "alpha bravo charlie delta echo foxtrot ".repeat(1500)
    )
}

#[test]
fn the_type_size_and_margins_the_preamble_states_reach_the_page() {
    // `\documentclass[11pt]` and `\usepackage[margin=0.95in]{geometry}` were
    // consumed and thrown away, so every book was set at plain.tex's 10pt on
    // 12pt leading with 1in margins whatever it asked for. Measured against
    // the lualatex-built scifi2/docs/book.pdf, whose body lines begin at
    // x=68.4 -- 0.95in of PDF points -- on baselines 13.549 apart, which is
    // the 13.6pt size11.clo:48 pairs with 11pt type.
    let pdf = texrs::run_pdf(&sized_document("11pt", "0.95in")).expect("pdf");
    let set = set_text(&pdf);
    let (size, x, first) = set[0];
    assert!(
        (x - 68.4).abs() < 1e-6,
        "the text starts at {x}, not at the 0.95in margin geometry was given"
    );
    // 11 of TeX's points is 10.9589 of PDF's: TeX's point is 1/72.27in and the
    // page is 612 by 792 of 1/72in.
    assert!(
        (size - 11.0 * 72.0 / 72.27).abs() < 1e-6,
        "the type is {size}pt, not the 11pt the class was given"
    );
    let second = set
        .iter()
        .find(|(_, _, y)| *y < first)
        .expect("a second line")
        .2;
    assert!(
        (first - second - 13.6 * 72.0 / 72.27).abs() < 1e-6,
        "the leading is {}, not the 13.6pt that goes with 11pt type",
        first - second
    );
}

#[test]
fn bigger_type_needs_more_pages_for_the_same_words() {
    // The size has to reach BOTH the measuring and the pagination, not just
    // the `Tf`: type set larger takes more lines to say the same thing and
    // fewer lines fit the page. Setting an 11pt book at 10pt is why texrs
    // fitted a third more text on a page than lualatex does.
    // The sample is large deliberately. Where no TeX installation is present
    // -- CI -- there is no cmr10.tfm to measure in and the widths fall back to
    // an estimate; on a few pages that can round 10pt and 11pt to the same
    // count and the property reads as false. Thousands of words apart, it
    // cannot: the difference is pages, not a rounding.
    let small = count_pages(&texrs::run_pdf(&sized_document("10pt", "0.95in")).expect("pdf"));
    let big = count_pages(&texrs::run_pdf(&sized_document("11pt", "0.95in")).expect("pdf"));
    assert!(
        big > small,
        "11pt type set in {big} pages where 10pt took {small}"
    );
}

#[test]
fn a_document_that_states_no_page_still_gets_plain_texs() {
    // The other half of the contract: a document that asks for nothing is
    // still set on plain.tex's page, so a reader comparing against a `tex` run
    // does not first have to account for a different one.
    let src = concat!(
        "\\documentclass{report}\n\\begin{document}\n",
        "alpha bravo charlie\n\\end{document}\n"
    );
    let pdf = texrs::run_pdf(src).expect("pdf");
    let (size, x, _) = set_text(&pdf)[0];
    assert_eq!(x, Layout::default().margin, "the margin moved");
    assert_eq!(size, Layout::default().size, "the type size moved");
}

#[test]
fn the_class_options_and_geometrys_are_read_where_they_are_written() {
    // Read straight, so the failure says which of the two halves broke. The
    // measure and the height come off 612 by 792 -- the paper `pdf::Page`
    // makes and the paper every corpus book is set on.
    let mut layout = Layout::default();
    layout.absorb_class_options("\n  11pt,\n");
    assert!((layout.size - 11.0 * 72.0 / 72.27).abs() < 1e-6);
    assert!((layout.leading - 13.6 * 72.0 / 72.27).abs() < 1e-6);
    layout.absorb_geometry_options("margin=0.95in");
    assert!((layout.margin - 68.4).abs() < 1e-6);
    assert!((layout.measure - (612.0 - 2.0 * 68.4)).abs() < 1e-6);
    assert!((layout.height - (792.0 - 2.0 * 68.4)).abs() < 1e-6);
    // An option that is not a size and a key that is not a margin change
    // nothing: `\documentclass[oneside,10pt]` and geometry's `includehead`
    // both arrive here.
    let untouched = layout.clone();
    layout.absorb_class_options("oneside,twocolumn");
    layout.absorb_geometry_options("includehead,headheight=12pt");
    assert_eq!(layout, untouched);
}

#[test]
fn options_passed_to_the_class_and_to_geometry_reach_the_page_too() {
    // `\PassOptionsToPackage` puts its options in the FIRST brace and names
    // its target in the second -- there is no `[...]` on it at all -- so
    // reading the bracket the other two directives carry would have found
    // nothing here. Pandoc writes a stack of these above `\documentclass`.
    let src = concat!(
        "\\PassOptionsToClass{11pt}{extreport}\n",
        "\\PassOptionsToPackage{margin=0.5in}{geometry}\n",
        "\\documentclass{extreport}\n\\usepackage{geometry}\n",
        "\\begin{document}\nalpha bravo charlie\n\\end{document}\n"
    );
    let pdf = texrs::run_pdf(src).expect("pdf");
    let (size, x, _) = set_text(&pdf)[0];
    assert!(
        (x - 36.0).abs() < 1e-6,
        "the text starts at {x}, not at 0.5in"
    );
    assert!(
        (size - 11.0 * 72.0 / 72.27).abs() < 1e-6,
        "the type is {size}pt, not the 11pt passed to the class"
    );
}

/// A pandoc code listing is a block of LINES, and the breaker used to reflow it
/// into the prose around it.
///
/// `\begin{Highlighting}` is not verbatim -- its body is `\NormalTok` and
/// siblings that have to expand -- so the code reached the breaker as ordinary
/// text and `split_whitespace` welded it. Measured: rubyrs/docs/book.tex set a
/// nine-line `dup_value` as three welded ones and came out in 208 pages where
/// lualatex sets it in 332; elisprs's `$ elisp -e ...` transcripts ran on one
/// line each, command and output together.
#[test]
fn a_code_listing_keeps_one_line_per_source_line() {
    let font = font_or_skip!();
    let src = "\\documentclass{article}\n\\newcommand{\\NormalTok}[1]{#1}\n\
               \\begin{document}\nBefore the listing.\n\n\
               \\begin{Shaded}\\begin{Highlighting}[]\n\
               \\NormalTok{let x = 1;}\n\
               \\NormalTok{let y = 2;}\n\
               \n\
               \\NormalTok{let z = 3;}\n\
               \\end{Highlighting}\\end{Shaded}\n\n\
               After the listing.\n\\end{document}\n";
    let text = texrs::run_text_marked(src).expect("run");
    let lines = break_lines(&text, &font, &Layout::default());
    let at = |want: &str| lines.iter().position(|l| l == want);
    let (Some(x), Some(y), Some(z)) = (at("let x = 1;"), at("let y = 2;"), at("let z = 3;")) else {
        panic!("each code line is a line of its own: {lines:?}");
    };
    assert_eq!(y, x + 1, "two code lines are two lines: {lines:?}");
    assert_eq!(
        z,
        y + 2,
        "a blank line in a listing is a blank code line, not a paragraph: {lines:?}"
    );
    assert!(
        !lines
            .iter()
            .any(|l| l.contains("listing.") && l.contains("let")),
        "code must not join the prose either side of it: {lines:?}"
    );
}

/// Splitting a listing into lines must not cost it its colour.
///
/// Pandoc's `\NormalTok` and its siblings ARE `\textcolor` calls -- that is why
/// `Highlighting` cannot be read as verbatim -- and the markers they leave are
/// what round 1 had just got onto the page. Each code line is lowered on its
/// own, so each one has to arrive carrying its own opening marker, its spec and
/// its close.
#[test]
fn a_listings_colour_markers_survive_the_line_split() {
    let src = "\\documentclass{article}\n\
               \\newcommand{\\NormalTok}[1]{\\textcolor[rgb]{0.25,0.44,0.63}{#1}}\n\
               \\begin{document}\n\
               \\begin{Shaded}\\begin{Highlighting}[]\n\
               \\NormalTok{let x = 1;}\n\
               \\NormalTok{let y = 2;}\n\
               \\end{Highlighting}\\end{Shaded}\n\\end{document}\n";
    let text = texrs::run_text_marked(src).expect("run");
    let para = text
        .split("\n\n")
        .find(|p| p.contains(LISTING_BREAK))
        .expect("the listing is a paragraph of its own");
    let lines: Vec<&str> = para.split_terminator(LISTING_BREAK).collect();
    assert_eq!(lines.len(), 2, "two code lines: {lines:?}");
    for line in &lines {
        assert!(
            line.contains('\u{1}') && line.contains("0.25,0.44,0.63") && line.contains('\u{3}'),
            "the line opens, specifies and closes its colour: {line:?}"
        );
    }
}

/// Every run a PDF draws, with the point it is drawn at.
///
/// `1 0 0 1 X Y Tm (text) Tj` is what the PDF path writes for a line, so the
/// position a line was given can be read back out of the file -- which is the
/// only way to ask whether a line was centred, as opposed to whether the
/// engine believes it centred it.
///
/// Read through `placed_faces` rather than by looking for `" Tm ("`: a
/// JUSTIFIED line has its word spacing between the two, `... Tm 0.45 Tw (...)`,
/// so that spelling saw only the ragged last line of every paragraph and
/// reported a four-line paragraph as one line.
fn placed(pdf: &[u8]) -> Vec<(f64, f64, String)> {
    placed_faces(pdf)
        .into_iter()
        .map(|(x, y, _, text)| (x, y, text))
        .collect()
}

/// Where a run containing `want` was drawn, and on what baseline.
fn at(runs: &[(f64, f64, String)], want: &str) -> (f64, f64) {
    runs.iter()
        .find(|(_, _, text)| text.contains(want))
        .map(|(x, y, _)| (*x, *y))
        .unwrap_or_else(|| panic!("nothing drawn containing {want:?}: {runs:?}"))
}

#[test]
fn a_centred_line_is_drawn_by_its_width_and_not_at_the_margin() {
    // `\begin{center}` expanded to nothing, so "centred line" and "left line"
    // were filled into ONE line at the margin. A title page is built out of
    // nothing but centred pieces, which is a large part of why a novel's front
    // matter collapsed into the prose after it.
    let src = concat!(
        "\\documentclass{article}\n\\begin{document}\n",
        "\\begin{center}\ncentred line\n\\end{center}\n",
        "left line\n\\end{document}\n"
    );
    let runs = placed(&texrs::run_pdf(src).expect("pdf"));
    let (centred_x, centred_y) = at(&runs, "centred line");
    let (left_x, left_y) = at(&runs, "left line");
    assert!(
        centred_y > left_y,
        "the centred line is its own line, above the next one: {runs:?}"
    );
    assert_eq!(left_x, 72.0, "an ordinary line starts at the margin");
    assert!(
        centred_x > left_x + 50.0,
        "a centred line is placed by its measured width, not at the margin: \
         got x={centred_x} against a margin of {left_x}"
    );
}

#[test]
fn centring_ends_with_the_environment_that_switched_it_on() {
    // `\centering` is a switch, and every LaTeX environment is a group that
    // ends it. Environments here are a macro pair rather than a group, so
    // without the `\end{...}` closing the region one `\centering` on a title
    // page would centre every remaining page of the book.
    let src = concat!(
        "\\documentclass{article}\n\\begin{document}\n",
        "\\begin{minipage}{\\linewidth}\\centering\ncentred line\n",
        "\\end{minipage}\n\nplain line\n\\end{document}\n"
    );
    let runs = placed(&texrs::run_pdf(src).expect("pdf"));
    assert!(at(&runs, "centred line").0 > 72.0, "centred: {runs:?}");
    assert_eq!(
        at(&runs, "plain line").0,
        72.0,
        "the region ended with the environment: {runs:?}"
    );
}

/// A document holding `body` between `\begin{document}` and `\end{document}`.
///
/// The list tests are all "these words, in this environment, land here", and
/// the preamble around them is the same every time.
fn document(body: &str) -> String {
    format!("\\documentclass{{article}}\n\\begin{{document}}\n{body}\n\\end{{document}}\n")
}

/// Every baseline a run at `x` was drawn on, nearest the top of the page first.
fn baselines_at(runs: &[(f64, f64, String)], x: f64) -> Vec<f64> {
    let mut ys: Vec<f64> = runs
        .iter()
        .filter(|(at, _, _)| (*at - x).abs() < 0.01)
        .map(|(_, y, _)| *y)
        .collect();
    ys.sort_by(|a, b| b.partial_cmp(a).expect("a baseline is a number"));
    ys.dedup();
    ys
}

/// The margin and the list indent, at the default layout: plain.tex's inch
/// margin, and `article.cls`'s `\leftmargini` of 2.5em at the 10pt type size.
const MARGIN: f64 = 72.0;
const FIRST_LEVEL: f64 = 97.0;
const SECOND_LEVEL: f64 = 122.0;

#[test]
fn each_item_of_a_list_starts_its_own_line_with_a_bullet_and_an_indent() {
    // `\begin{itemize}` expanded to nothing and `\item` to its optional
    // argument, so an itemize of two items read back as "alpha item bravo
    // item": one line, at the margin, with no bullet between them. `\item` is
    // 8,683 occurrences across the corpus, so this was the shape of a large
    // part of every book in it.
    let src = document(concat!(
        "before the list\n\n",
        "\\begin{itemize}\n\\tightlist\n",
        "\\item\n  alpha item\n",
        "\\item\n  bravo item\n",
        "\\end{itemize}\n\n",
        "after the list"
    ));
    let runs = placed(&texrs::run_pdf(&src).expect("pdf"));
    let (alpha_x, alpha_y) = at(&runs, "alpha item");
    let (bravo_x, bravo_y) = at(&runs, "bravo item");
    assert!(
        alpha_y > bravo_y,
        "each item starts its own line, one under the other: {runs:?}"
    );
    assert_eq!(
        (alpha_x, bravo_x),
        (FIRST_LEVEL, FIRST_LEVEL),
        "an item's line starts in from the margin: {runs:?}"
    );
    assert_eq!(
        (
            at(&runs, "before the list").0,
            at(&runs, "after the list").0
        ),
        (MARGIN, MARGIN),
        "the prose either side of the list is not moved: {runs:?}"
    );
    // WinAnsi puts the bullet at 0x95, and a PDF string spells a high byte as
    // an octal escape -- so `\225` in the content stream IS the bullet drawn.
    for want in ["alpha item", "bravo item"] {
        let (_, _, text) = runs
            .iter()
            .find(|(_, _, text)| text.contains(want))
            .expect("the item was drawn");
        assert!(
            text.starts_with("\\225 "),
            "the item carries its bullet: {text:?}"
        );
    }
}

#[test]
fn an_enumerate_numbers_its_items_and_a_description_sets_its_term_in_bold() {
    let src = document(concat!(
        "\\begin{enumerate}\n\\tightlist\n",
        "\\item\n  alpha item\n",
        "\\item\n  bravo item\n",
        "\\end{enumerate}\n\n",
        "\\begin{description}\n\\tightlist\n",
        "\\item[the term]\n  its meaning\n",
        "\\end{description}"
    ));
    let pdf = texrs::run_pdf(&src).expect("pdf");
    let runs = placed(&pdf);
    for (number, want) in [("1. ", "alpha item"), ("2. ", "bravo item")] {
        let (x, _, text) = runs
            .iter()
            .find(|(_, _, text)| text.contains(want))
            .expect("the item was drawn");
        assert!(
            text.starts_with(number),
            "an enumerate's item carries its number: {text:?}"
        );
        assert_eq!(*x, FIRST_LEVEL, "and sets at the list indent: {runs:?}");
    }
    // The term is the mark of a description item: it stands where the bullet
    // stands, in the bold face every description list sets it in, and the body
    // follows it on the same line.
    let faces = placed_faces(&pdf);
    let (term_x, term_y, term_face, _) = faces
        .iter()
        .find(|(_, _, _, text)| text.contains("the term"))
        .expect("the term was drawn");
    let (body_x, body_y, body_face, _) = faces
        .iter()
        .find(|(_, _, _, text)| text.contains("its meaning"))
        .expect("the body was drawn");
    assert_eq!(*term_x, FIRST_LEVEL, "the term is the mark: {faces:?}");
    assert_eq!(term_y, body_y, "the body runs on from it: {faces:?}");
    assert!(body_x > term_x, "and after it: {faces:?}");
    assert!(
        term_face.contains("Bold") && !body_face.contains("Bold"),
        "the term is bold and its meaning is not: {term_face} / {body_face}"
    );
}

/// The words of an item long enough to wrap several times.
const LONG_ITEM: &str = concat!(
    "a very long item that has to wrap because it runs well past the measure ",
    "and keeps going with plenty more words after it so that the breaker has ",
    "no choice at all but to put a second and a third and a fourth line under ",
    "the first one, every one of them inside the list rather than back out at ",
    "the page margin where the prose around the list is set"
);

/// A document set in big type inside big margins.
///
/// One level of indent is 2.5em, so at 20pt type it is a sixth of the 324pt
/// measure `margin=2in` leaves -- which is what makes the line count below an
/// assertion rather than a coin toss. The same words in the same measure would
/// break identically; the whole question is whether the measure narrowed.
fn wide_document(body: &str) -> String {
    format!(
        "\\documentclass[20pt]{{extreport}}\n\
         \\usepackage[margin=2in]{{geometry}}\n\
         \\begin{{document}}\n{body}\n\\end{{document}}\n"
    )
}

#[test]
fn a_long_item_wraps_inside_the_list_and_in_the_measure_the_indent_leaves() {
    // Two documents, the same words: once as prose at the margin, once as one
    // item. The item is set in a measure narrowed by exactly what it is moved
    // in by, so it takes MORE lines than the prose does -- an item that wrapped
    // back out to the page's right edge would take the same number.
    let margin = 2.0 * 72.0;
    // 2.5em at the 20pt the class asks for, in the PDF's points.
    let indent = 2.5 * 20.0 * 72.0 / 72.27;
    let prose = placed(&texrs::run_pdf(&wide_document(LONG_ITEM)).expect("pdf"));
    let item = placed(
        &texrs::run_pdf(&wide_document(&format!(
            "\\begin{{itemize}}\n\\item\n  {LONG_ITEM}\n\\end{{itemize}}"
        )))
        .expect("pdf"),
    );
    let prose_lines = baselines_at(&prose, margin);
    let item_lines = baselines_at(&item, margin + indent);
    assert!(
        prose_lines.len() > 2,
        "the paragraph is long enough to wrap: {prose:?}"
    );
    assert_eq!(
        baselines_at(&item, margin),
        Vec::<f64>::new(),
        "no line of the item is set at the margin: {item:?}"
    );
    assert!(
        item_lines.len() > prose_lines.len(),
        "the item wraps in the narrowed measure, so it takes more lines than \
         the same words as prose: {} against {}",
        item_lines.len(),
        prose_lines.len()
    );
}

#[test]
fn a_list_inside_a_list_sets_one_level_further_in() {
    let src = document(concat!(
        "\\begin{itemize}\n",
        "\\item outer item\n",
        "  \\begin{itemize}\n",
        "  \\item inner item\n",
        "  \\end{itemize}\n",
        "\\item second outer item\n",
        "\\end{itemize}\n\n",
        "after the lists"
    ));
    let runs = placed(&texrs::run_pdf(&src).expect("pdf"));
    assert_eq!(
        (
            at(&runs, "outer item").0,
            at(&runs, "inner item").0,
            at(&runs, "second outer item").0,
            at(&runs, "after the lists").0
        ),
        (FIRST_LEVEL, SECOND_LEVEL, FIRST_LEVEL, MARGIN),
        "each level indents further, and both are given back: {runs:?}"
    );
}

#[test]
fn centring_inside_a_list_still_centres_over_the_whole_measure() {
    // The one thing an earlier revision of this got wrong, and the reason the
    // order of the arms in `fill`'s `start` is spelt out there: the indent was
    // tested before the centring, so a centred line at any list depth lost its
    // centring marker and set flush at the list indent -- at 97.0 below rather
    // than by its own width.
    //
    // The two documents hold the SAME centred words, so the x they are placed
    // at is the same number, or the centring is being measured differently
    // inside a list -- which is what centring over the narrowed measure would
    // look like, and it is not what `\begin{center}` means.
    let centred = "\\begin{center}\ncentred words\n\\end{center}";
    let alone = placed(&texrs::run_pdf(&document(centred)).expect("pdf"));
    let inside = placed(
        &texrs::run_pdf(&document(&format!(
            "\\begin{{itemize}}\n\\item an item\n{centred}\n\\end{{itemize}}"
        )))
        .expect("pdf"),
    );
    let alone_x = at(&alone, "centred words").0;
    let inside_x = at(&inside, "centred words").0;
    assert!(
        alone_x > MARGIN + 50.0,
        "the centred line is placed by its width: {alone:?}"
    );
    assert_eq!(
        inside_x, alone_x,
        "a centred line inside a list is centred exactly as it is outside one: \
         {inside:?}"
    );
    assert_eq!(
        at(&inside, "an item").0,
        FIRST_LEVEL,
        "and the item around it is still indented: {inside:?}"
    );
}

#[test]
fn a_heading_is_given_vertical_space_above_and_below_it() {
    // A heading set hard against its paragraphs is indistinguishable from
    // them, and the page then holds lines that a real run spends on white
    // space: lualatex leaves 3.5ex above a section and 2.3ex below it.
    let src = concat!(
        "\\documentclass{article}\n\\begin{document}\n",
        "before the heading\n\\section{A Heading}\nafter the heading\n",
        "\\end{document}\n"
    );
    let runs = placed(&texrs::run_pdf(src).expect("pdf"));
    let leading = 12.0;
    let above = at(&runs, "before").1 - at(&runs, "A Heading").1;
    let below = at(&runs, "A Heading").1 - at(&runs, "after").1;
    assert!(
        above > leading && below > leading,
        "a heading sat one ordinary line from its paragraphs: above {above}, \
         below {below}, leading {leading}"
    );
}

#[test]
fn vertical_space_is_space_and_not_a_character() {
    // The space a heading asks for travels in the text as a vertical tab, the
    // way a forced break travels as a form feed. Drawn rather than skipped it
    // would come out as whatever the font has in that slot, in the middle of
    // the white space it was asked for.
    let src = concat!(
        "\\documentclass{article}\n\\begin{document}\n",
        "before\n\\section{Head}\nafter\n\\end{document}\n"
    );
    let pdf = texrs::run_pdf(src).expect("pdf");
    for (_, _, text) in placed(&pdf) {
        assert!(
            !text.chars().any(|c| c == texrs::typeset::VERTICAL_SPACE),
            "the marker was drawn as a glyph: {text:?}"
        );
    }
    let plain = texrs::run_text(src).expect("run");
    assert!(
        !plain.chars().any(|c| matches!(
            c,
            texrs::typeset::VERTICAL_SPACE | texrs::typeset::CENTRE | texrs::typeset::CENTRE_END
        )),
        "a --text run must not emit the markers: {plain:?}"
    );
}

/// Every `N 0 obj ... endobj` of a PDF, by its number.
///
/// The files texrs writes are uncompressed, so the objects can be read straight
/// out of them -- which is what the reader below needs and what a PDF reader
/// itself does.
fn objects(pdf: &str) -> Vec<(u32, &str)> {
    let mut out = Vec::new();
    for chunk in pdf.split("endobj") {
        let Some(at) = chunk.find(" 0 obj") else {
            continue;
        };
        let Ok(number) = chunk[..at].trim().rsplit('\n').next().unwrap_or("").parse() else {
            continue;
        };
        out.push((number, &chunk[at + " 0 obj".len()..]));
    }
    out.sort_by_key(|(n, _)| *n);
    out
}

/// The value written after `key`, up to whatever ends it.
fn value_of<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    let rest = body.split_once(key)?.1.trim_start();
    Some(rest.split([' ', '/', '>', '\n']).next().unwrap_or(rest))
}

/// Every run a PDF draws, with the `/BaseFont` it is drawn IN.
///
/// `drawn` above reads the colour operators; this reads the font ones, and the
/// two halves of a font operator cannot be read apart: `Tf` names a page
/// RESOURCE -- `/F2` -- and only that page's own resource dictionary says which
/// font object it is, so the same `/F2` is one font on one page and another on
/// the next.
fn faces(pdf: &[u8]) -> Vec<(String, String)> {
    placed_faces(pdf)
        .into_iter()
        .map(|(_, _, face, text)| (face, text))
        .collect()
}

/// The same, with the point each run was drawn at.
///
/// One reader rather than two: `placed` answers where and `faces` answers in
/// what, and a question about a COLUMN -- which face is set at which x, on
/// which baseline -- needs both of a single run at once.
fn placed_faces(pdf: &[u8]) -> Vec<(f64, f64, String, String)> {
    let pdf = String::from_utf8_lossy(pdf).into_owned();
    let objects = objects(&pdf);
    let by_number = |number: &str| objects.iter().find(|(n, _)| n.to_string() == number);
    let base = |number: &str| {
        by_number(number)
            .and_then(|(_, body)| value_of(body, "/BaseFont /"))
            .unwrap_or("?")
            .to_string()
    };
    let mut runs = Vec::new();
    // A page, and not the page TREE. Asked without depending on how the writer
    // spaces a dictionary: this read `/Type /Page>>` and stopped finding any
    // page at all the day the writer put a space before the closing bracket,
    // which made seventeen tests report that nothing had been drawn.
    let is_page = |b: &str| b.contains("/Type /Page") && !b.contains("/Type /Pages");
    for (_, page) in objects.iter().filter(|(_, b)| is_page(b)) {
        let Some((_, stream)) = value_of(page, "/Contents ").and_then(by_number) else {
            continue;
        };
        // `/F1 3 0 R /F2 8 0 R` -- the resource dictionary of THIS page.
        let named = |name: &str| {
            value_of(page, &format!("/{name} "))
                .map(base)
                .unwrap_or_else(|| "?".to_string())
        };
        for line in stream.lines() {
            let Some(at) = line.find(" Tf ") else {
                continue;
            };
            // `BT /F1 10 Tf ...`: the resource name is the token after the last
            // `/` before the operator, not everything after it.
            let head = line[..at].rsplit('/').next().unwrap_or("");
            let face = named(head.split_whitespace().next().unwrap_or(""));
            // `1 0 0 1 X Y Tm` -- the two numbers before the operator.
            let point = line.split_once(" Tm ").map(|(before, _)| before);
            let mut back = point.unwrap_or("").split_ascii_whitespace().rev();
            let (Some(Ok(y)), Some(Ok(x))) = (
                back.next().map(str::parse::<f64>),
                back.next().map(str::parse::<f64>),
            ) else {
                continue;
            };
            if let Some((body, _)) = line.rsplit_once(") Tj") {
                if let Some((_, text)) = body.rsplit_once('(') {
                    runs.push((x, y, face, text.to_string()));
                }
            }
        }
    }
    runs
}

/// The face a run of text came out in, found by what it says.
fn face_of<'a>(runs: &'a [(String, String)], want: &str) -> &'a str {
    runs.iter()
        .find(|(_, text)| text.contains(want))
        .map(|(face, _)| face.as_str())
        .unwrap_or_else(|| panic!("nothing drawn containing {want:?}: {runs:?}"))
}

#[test]
fn texttt_textbf_and_emph_are_set_in_the_faces_they_name() {
    // `\texttt` appears 683,577 times in the corpus, `\emph` 35,369 and
    // `\textbf` 34,159, and all three came out in the body face: a document
    // with one of each produced ONE font resource and ONE Tf operator, so every
    // code identifier in every book was set in the prose font.
    //
    // The families are named so that nothing installed can be found for them
    // and the same fallback is taken on every machine: `NoSuchSerifFace` is a
    // serif request and `NoSuchMonoFace` a monospace one, which is what
    // `base14_for` reads them as.
    let src = "\\documentclass{article}\n\\usepackage{fontspec}\n\
               \\setmainfont{NoSuchSerifFace}\n\\setmonofont{NoSuchMonoFace}\n\
               \\begin{document}\n\
               plain \\texttt{mono} \\textbf{bold} \\emph{italic} plain\n\\end{document}\n";
    let pdf = texrs::run_pdf(src).expect("pdf");
    let runs = faces(&pdf);
    assert_eq!(face_of(&runs, "plain"), "Times-Roman");
    assert_eq!(face_of(&runs, "mono"), "Courier");
    assert_eq!(face_of(&runs, "bold"), "Times-Bold");
    assert_eq!(face_of(&runs, "italic"), "Times-Italic");
    // Times' bold is not `Times-Roman-Bold`: the fourteen are named as they are
    // named, and a name no reader has is substituted without saying so.
    let s = String::from_utf8_lossy(&pdf);
    assert!(s.contains("/BaseFont /Times-Bold"), "{s}");
}

#[test]
fn a_book_that_redefines_texttt_still_reaches_the_mono_face() {
    // Every book in the corpus redefines `\texttt` to colour its inline code,
    // so dispatching on `\texttt` itself would never fire for the documents
    // this is for -- and if it did it would drop that colour. What the
    // redefinition writes is `{\ttfamily\color{...}#1}`, and the DECLARATION is
    // where the face is honoured, so both survive. The line below is
    // rubyrs/docs/book.tex:383 with its `\renewcommand` left out.
    let src = "\\documentclass{article}\n\\usepackage{fontspec}\n\
               \\setmainfont{NoSuchSerifFace}\n\\setmonofont{NoSuchMonoFace}\n\
               \\definecolor{neonCyan}{HTML}{00E5FF}\n\
               \\DeclareRobustCommand{\\texttt}[1]{{\\ttfamily\\color{neonCyan}#1}}\n\
               \\begin{document}\nalpha \\texttt{code} beta\n\\end{document}\n";
    let pdf = texrs::run_pdf(src).expect("pdf");
    let runs = faces(&pdf);
    assert_eq!(face_of(&runs, "code"), "Courier");
    assert_eq!(
        face_of(&runs, "beta"),
        "Times-Roman",
        "and the face ends with the group that set it"
    );
    let colour = drawn(&pdf)
        .into_iter()
        .find(|(_, text)| text.contains("code"))
        .map(|(colour, _)| colour)
        .expect("the code was drawn");
    assert_ne!(colour, "0 0 0", "the colour the book gives it survives too");
}

#[test]
fn a_face_marker_costs_nothing_on_the_line_and_is_never_set() {
    // A face marker is U+000E, ONE character naming the face, and U+000F. The
    // naming character is a LETTER: measured as text it pushes words onto later
    // pages, and drawn as text it sets an `m` in front of every one of the
    // 683,577 `\texttt`s in the corpus. With no monospace family named, the
    // mono face IS the main face, so the two documents below must break alike.
    let doc = |body: String| {
        format!("\\documentclass{{article}}\n\\begin{{document}}\n{body}\n\\end{{document}}\n")
    };
    let plain = texrs::run_pdf(&doc("alpha ".repeat(600))).expect("pdf");
    let faced = texrs::run_pdf(&doc("\\texttt{alpha} ".repeat(600))).expect("pdf");
    assert_eq!(
        count_pages(&faced),
        count_pages(&plain),
        "a face is not text and must not push words onto later pages"
    );
    for (_, text) in drawn(&faced) {
        assert!(
            !text.contains('\u{11}') && !text.contains('\u{12}') && !text.contains("malpha"),
            "a marker reached the page as a glyph: {text:?}"
        );
    }
}

#[test]
fn the_lowerer_keeps_the_files_the_preamble_named_for_every_face() {
    // The bold and italic files are named in the SAME option list as the
    // upright one, and the monospace family names its own. Reading past those
    // three keys is why `\texttt` and `\emph` could not be honoured for the
    // documents that matter: the family names alone resolve to nothing, since a
    // book ships its faces beside itself rather than installing them.
    let src = concat!(
        "\\documentclass{article}\n\\usepackage{fontspec}\n",
        "\\setmainfont{Arimo}[\n",
        "    Path=/somewhere/.fonts/,\n",
        "    Extension=.ttf,\n",
        "    UprightFont=Arimo-VF,\n",
        "    BoldFont=Arimo-VF,\n",
        "    BoldFeatures={RawFeature={axis={wght=700}}},\n",
        "    ItalicFont=Arimo-Italic-VF,\n",
        "]\n",
        "\\setmonofont{ShareTechMono}[\n",
        "    Path=/somewhere/.fonts/,\n",
        "    Extension=.ttf,\n",
        "    UprightFont=ShareTechMono-Regular,\n",
        "]\n",
        "\\begin{document}\nwords\n\\end{document}\n"
    );
    let mut lowerer = texrs::lower::Lowerer::new().with_text_output();
    lowerer.preload(texrs::latex::PRELUDE).expect("prelude");
    lowerer.lower(src).expect("lower");
    assert_eq!(lowerer.fonts.main_file.bold.as_deref(), Some("Arimo-VF"));
    assert_eq!(
        lowerer.fonts.main_file.italic.as_deref(),
        Some("Arimo-Italic-VF")
    );
    assert_eq!(
        lowerer.fonts.mono_file.upright.as_deref(),
        Some("ShareTechMono-Regular"),
        "the monospace file, which is what \\texttt is set from"
    );
}

#[test]
fn a_symbol_a_document_spells_as_a_macro_still_typesets_and_reaches_the_page() {
    // `\rightarrow` was undefined, and an undefined control sequence is not a
    // missing glyph: the run STOPS -- `! Undefined control sequence
    // \rightarrow.` -- and the document produces no page at all. The corpus
    // writes 2,123 arrows, and a document that draws a pipeline writes little
    // else.
    let src = "\\documentclass{article}\n\\begin{document}\n\
               step \\rightarrow next, at \\alpha\n\\end{document}\n";
    let pdf = texrs::run_pdf(src).expect("a document that writes an arrow still typesets");
    let runs = faces(&pdf);
    // No text face has an arrow and none ever will, so it comes from the
    // fallback font at the code that font's own encoding gives it: 174 for
    // `arrowright`, written into the content stream as the one byte `\256`.
    assert!(
        runs.iter()
            .any(|(face, text)| face == "Symbol" && text == "\\256"),
        "the arrow was not drawn from the fallback font: {runs:?}"
    );
    assert!(
        runs.iter()
            .any(|(face, text)| face == "Symbol" && text == "a"),
        "nor was the alpha, which is code 97 there: {runs:?}"
    );
    // And the prose around it stays in the document's own face rather than
    // being dragged into the fallback with it.
    assert_ne!(face_of(&runs, "step"), "Symbol");
}

#[test]
fn a_character_the_face_lacks_is_drawn_rather_than_written_out_as_its_utf8() {
    // A `char` pushed into the content stream is written as UTF-8, so the arrow
    // in `A -> B` went into the file as three bytes and a reader drew three
    // letters of whatever the face has at 0xE2, 0x86 and 0x92 -- `pdftotext`
    // read the page back with three wrong characters where the arrow was. Each
    // of these is ONE glyph on the page now, and which font draws it depends on
    // what the face has.
    let src = "\\documentclass{article}\n\\begin{document}\n\
               dash \u{2014} arrow \u{2192} rule \u{2500} end\n\\end{document}\n";
    let pdf = texrs::run_pdf(src).expect("pdf");
    let runs = faces(&pdf);
    let all: String = runs.iter().map(|(_, text)| text.as_str()).collect();
    assert!(
        !all.contains('\u{2192}') && !all.contains('\u{2014}'),
        "a character reached the content stream as itself: {runs:?}"
    );
    // The em dash IS in WinAnsi, at 0x97, and Helvetica has it -- so it stays
    // in the document's own face rather than being fetched from anywhere.
    assert_eq!(face_of(&runs, "\\227"), "Helvetica");
    // The arrow is in no text encoding at all and comes from the fallback.
    assert_eq!(face_of(&runs, "\\256"), "Symbol");
    // Box drawing is in neither, and Computer Modern has none of it either, so
    // it is set as the stand-in that keeps the line readable -- which is what
    // the DVI path has always done with the same character.
    assert!(
        all.contains("rule - end"),
        "the rule was dropped instead of standing in: {runs:?}"
    );
}

// ── Tables ────────────────────────────────────────────────────────────────
//
// A table used to be one paragraph of prose: `&` was lowered to a space and
// `\\` to a newline, so "Name & Value \\ alpha & 1 \\" arrived at the breaker
// as "Name Value alpha 1" and was filled into the sentence after it. The
// corpus depends on tables heavily -- pandoc emits a longtable for every
// markdown table, 132 of them in groovyrs/docs/book.tex.

/// A two-column table holding `body`, in the two base-fourteen faces the other
/// tests here use, so nothing installed on the machine can change the answer.
fn table_document(body: &str) -> String {
    format!(
        "\\documentclass{{article}}\n\\usepackage{{fontspec}}\n\
         \\setmainfont{{NoSuchSerifFace}}\n\\setmonofont{{NoSuchMonoFace}}\n\
         \\begin{{document}}\n\
         \\begin{{tabular}}{{ll}}\n\\toprule\n{body}\\bottomrule\n\\end{{tabular}}\n\n\
         afterwards\n\\end{{document}}\n"
    )
}

#[test]
fn a_table_is_set_as_rows_rather_than_filled_into_the_prose_around_it() {
    let src = table_document("Name & Value \\\\\n\\midrule\nalpha & 1 \\\\\nbeta & 2 \\\\\n");
    let runs = placed(&texrs::run_pdf(&src).expect("pdf"));
    let (_, header) = at(&runs, "Name");
    let (_, first) = at(&runs, "alpha");
    let (_, second) = at(&runs, "beta");
    let (_, after) = at(&runs, "afterwards");
    // Each row is a baseline of its own, in the order it was written, and the
    // sentence after the table is below all of them. Before this every one of
    // these was the same number: one line reading "Name Value alpha 1 beta 2
    // afterwards".
    assert!(
        header > first && first > second && second > after,
        "the rows are lines of their own, in order, above the prose after the \
         table: {runs:?}"
    );
    // And each row holds its own cells and only its own.
    let row = |want: &str| {
        runs.iter()
            .find(|(_, _, text)| text.contains(want))
            .map(|(_, _, text)| text.clone())
            .unwrap_or_default()
    };
    assert!(row("alpha").contains('1'), "got {:?}", row("alpha"));
    assert!(!row("alpha").contains("beta"), "got {:?}", row("alpha"));
}

#[test]
fn a_tables_columns_line_up_across_its_rows() {
    // The second cell of every row is coloured, which splits the line into runs
    // exactly where the column starts -- so the x a column was set at can be
    // read back out of the file rather than taken on trust. The first cells are
    // of very different widths on purpose: strung along one line, as they used
    // to be, these three x values are nowhere near each other.
    let src = table_document(
        "one & \\textcolor[rgb]{1,0,0}{x} \\\\\n\\midrule\n\
         a much longer first cell & \\textcolor[rgb]{1,0,0}{y} \\\\\n\
         mid & \\textcolor[rgb]{1,0,0}{z} \\\\\n",
    );
    let runs = placed(&texrs::run_pdf(&src).expect("pdf"));
    let column = |want: &str| at(&runs, want).0;
    let (x, y, z) = (column("x"), column("y"), column("z"));
    let spread = x.max(y).max(z) - x.min(y).min(z);
    // Padding is written in spaces, so a column stands within half a space of
    // where it was measured to be; half a space is under 3pt at any size these
    // documents are set in.
    assert!(
        spread < 3.0,
        "the second column is at x={x}, {y} and {z} on the three rows: {runs:?}"
    );
    // And it IS a column: indented past the widest cell of the first one,
    // rather than each row starting wherever the row before it ended.
    assert!(
        x > column("one") + 50.0,
        "the second column clears the first: x={x} against {}",
        column("one")
    );
}

/// The filled rectangles a PDF draws, as `(x, y, width, height)`.
///
/// `X Y W H re f` is what `pdf::Page::rule` writes, and a rule is the only
/// thing these documents fill -- so this is how to ask whether a booktabs rule
/// was DRAWN, as opposed to set as characters or dropped.
fn rules(pdf: &[u8]) -> Vec<(f64, f64, f64, f64)> {
    let s = String::from_utf8_lossy(pdf).into_owned();
    let mut found = Vec::new();
    for line in s.lines() {
        let Some(head) = line.strip_suffix(" re f") else {
            continue;
        };
        let numbers: Vec<f64> = head
            .split_ascii_whitespace()
            .rev()
            .take(4)
            .filter_map(|n| n.parse().ok())
            .collect();
        if let [h, w, y, x] = numbers[..] {
            found.push((x, y, w, h));
        }
    }
    found
}

#[test]
fn the_three_booktabs_rules_are_drawn_as_rules() {
    // `\toprule`, `\midrule` and `\bottomrule` were defined by the prelude to
    // expand to nothing, so a table had no rules at all -- and the corpus
    // writes `\toprule` 3,455 times.
    let src = table_document("Name & Value \\\\\n\\midrule\nalpha & 1 \\\\\n");
    let pdf = texrs::run_pdf(&src).expect("pdf");
    let drawn = rules(&pdf);
    assert_eq!(drawn.len(), 3, "three rules: {drawn:?}");
    let runs = placed(&pdf);
    let (_, header) = at(&runs, "Name");
    let (_, body) = at(&runs, "alpha");
    let ys: Vec<f64> = drawn.iter().map(|(_, y, _, _)| *y).collect();
    assert!(
        ys[0] > header && ys[1] < header && ys[1] > body && ys[2] < body,
        "top above the head, mid between head and body, bottom under the \
         body: rules at {ys:?}, head at {header}, body at {body}"
    );
    // Each runs the width of the table: not nothing, and not off the paper.
    for (x, _, w, h) in &drawn {
        assert!(*w > 20.0 && x + w < 612.0, "a rule of width {w} at x={x}");
        assert!(*h > 0.0 && *h < 2.0, "a rule is a line, not a band: {h}");
    }
    // booktabs sets the outer rules heavier than the inner one, which is the
    // whole visual difference between the three.
    assert!(
        drawn[0].3 > drawn[1].3 && drawn[2].3 > drawn[1].3,
        "the middle rule is the light one: {drawn:?}"
    );
}

#[test]
fn a_wrapped_table_row_does_not_leak_its_face_into_the_cell_beside_it() {
    // The defect a previous attempt at tables shipped. A row taller than one
    // line is set by putting the nth fragment of every column on the nth line,
    // and `to_pdf` walks ONE face stack down the page -- so a first column that
    // opened `\texttt` and left it open handed Courier to the prose in the
    // second column on every line of the row but the first.
    //
    // Both cells are long enough that the row is several lines tall, which is
    // what every wide pandoc table in the corpus looks like.
    let mono = "\\texttt{AwkFieldGet, AwkFieldSet, AwkNf, AwkSetRecord, \
                AwkGetFieldNum, AwkSpecialGet, AwkSpecialSet, AwkPrint}";
    let prose = "wrapping prose that keeps going for long enough to need \
                 several lines of its own beside the code";
    let src = table_document(&format!("{mono} & {prose} \\\\\n"));
    let pdf = texrs::run_pdf(&src).expect("pdf");
    let runs = placed_faces(&pdf);
    // First, that the row IS two columns several lines tall: on more than one
    // baseline, a Courier run at the margin and a Times-Roman run to the right
    // of it. Filled into one paragraph, as a table used to be, that happens on
    // exactly the one line where the code ends and the prose begins -- so this
    // is what says the assertion below is being asked of a real table.
    let side_by_side = runs
        .iter()
        .filter(|(_, _, face, _)| face == "Courier")
        .filter(|(_, y, _, _)| {
            runs.iter()
                .any(|(bx, by, bf, _)| by == y && bf == "Times-Roman" && *bx > 100.0)
        })
        .count();
    assert!(
        side_by_side > 1,
        "the row is one line, not a column of code beside a column of prose: \
         {runs:?}"
    );
    // Then the defect itself: every word of the prose column is in the prose
    // face, not just the first.
    let flat: Vec<(String, String)> = runs
        .iter()
        .map(|(_, _, face, text)| (face.clone(), text.clone()))
        .collect();
    for word in prose.split_whitespace() {
        assert_eq!(
            face_of(&flat, word),
            "Times-Roman",
            "{word:?} came out of the second column in the first column's \
             face: {runs:?}"
        );
    }
    // And the code is still monospace, so this is not passing by never
    // honouring the face at all.
    assert_eq!(face_of(&flat, "AwkFieldGet"), "Courier");
    assert_eq!(face_of(&flat, "AwkPrint"), "Courier");
}

#[test]
fn a_longtable_sets_its_foot_after_its_body_and_not_before_it() {
    // longtable states its head, then its foot, then its body: `\endhead`
    // closes the head and `\endlastfoot` the foot, so `\bottomrule` is WRITTEN
    // before the first row of data. Set in arrival order the bottom rule lands
    // under the head instead of under the table. This is the shape of every
    // markdown table pandoc emits, which is every table in the corpus.
    let src = "\\documentclass{article}\n\\begin{document}\n\
               \\begin{longtable}[]{@{}lr@{}}\n\
               \\toprule\\noalign{}\n& mean \\\\\n\\midrule\\noalign{}\n\
               \\endhead\n\\bottomrule\\noalign{}\n\\endlastfoot\n\
               awkrs & 23.7 ms \\\\\nmawk & 136.0 ms \\\\\n\
               \\end{longtable}\n\\end{document}\n";
    let pdf = texrs::run_pdf(src).expect("pdf");
    let runs = placed(&pdf);
    let (_, head) = at(&runs, "mean");
    let (_, last) = at(&runs, "mawk");
    let ys: Vec<f64> = rules(&pdf).iter().map(|(_, y, _, _)| *y).collect();
    assert_eq!(ys.len(), 3, "three rules: {ys:?}");
    assert!(
        ys[2] < last,
        "the bottom rule is under the last row of data, not under the head: \
         rules at {ys:?}, head at {head}, last row at {last}"
    );
    // The rows themselves are in the order they were written.
    assert!(head > at(&runs, "awkrs").1, "the head is above the body");
}
