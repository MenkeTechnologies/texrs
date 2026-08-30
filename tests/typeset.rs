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
use texrs::typeset::{break_lines, find_font, to_dvi, to_dvi_chain, FontChain, Layout};

fn font() -> Tfm {
    let p = find_font("cmr10").expect("cmr10.tfm (is a TeX installation present?)");
    Tfm::open(&p).expect("read cmr10")
}

#[test]
fn a_dvi_file_is_produced_and_parses_as_one() {
    let f = font();
    let dvi = to_dvi("hello world", &f, "cmr10", &Layout::default());
    let parsed = texrs::dvi::Dvi::parse(&dvi).expect("texrs must read back what it wrote");
    assert_eq!(parsed.pages(), 1, "one line of text is one page");
}

#[test]
fn every_line_fits_the_measure() {
    // The whole job of line breaking. A line wider than the measure would run
    // off the page, and nothing downstream would catch it.
    let f = font();
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
    let f = font();
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
    let f = font();
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
    let f = font();
    let text = "word ".repeat(20_000);
    let dvi = to_dvi(&text, &f, "cmr10", &Layout::default());
    let parsed = texrs::dvi::Dvi::parse(&dvi).expect("parse");
    assert!(parsed.pages() > 1, "got {} pages", parsed.pages());
}

#[test]
fn the_text_survives_the_round_trip_through_dvi() {
    // The characters that went in are the characters a reader gets back, which
    // is what makes the page the document rather than a plausible shape.
    let f = font();
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
    let chain = FontChain::load("cmr10", &["cmsy10"]).expect("fonts");
    let (font, slot) = chain.resolve('→').expect("an arrow must resolve");
    assert_eq!(chain.fonts[font].name, "cmsy10");
    assert_eq!(slot, 33, "the slot tex itself sets for \\rightarrow");
}

#[test]
fn the_section_mark_comes_from_the_symbol_font_not_the_text_font() {
    // This table was written wrong the first time: `§` pointed at cmr10 slot
    // 120, which is an `x`, and the page said "x" where the document said "§"
    // without anything reporting a problem.
    let chain = FontChain::load("cmr10", &["cmsy10"]).expect("fonts");
    let (font, slot) = chain.resolve('§').expect("a section mark must resolve");
    assert_eq!(chain.fonts[font].name, "cmsy10");
    assert_eq!(slot, 120);
}

#[test]
fn ascii_still_comes_from_the_text_font() {
    let chain = FontChain::load("cmr10", &["cmsy10"]).expect("fonts");
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
    let chain = FontChain::load("cmr10", &["cmsy10"]).expect("fonts");
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
    let chain = FontChain::load("cmr10", &["cmsy10"]).expect("fonts");
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
    let chain = FontChain::load("cmr10", &["cmsy10"]).expect("fonts");
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
    let mono = texrs::run_pdf(&src("ShareTechMono")).expect("pdf");
    assert!(
        String::from_utf8_lossy(&mono).contains("/Courier"),
        "a monospace request must reach the file as Courier"
    );
    let serif = texrs::run_pdf(&src("Times New Roman")).expect("pdf");
    assert!(String::from_utf8_lossy(&serif).contains("/Times-Roman"));
}

#[test]
fn colour_survives_the_pdf_path_as_pdfs_own_operator() {
    let src = "\\documentclass{article}\n\\begin{document}\n\
               plain \\textcolor[rgb]{1.00,0.00,0.00}{RED} plain\n\\end{document}\n";
    let pdf = texrs::run_pdf(src).expect("pdf");
    let s = String::from_utf8_lossy(&pdf);
    assert!(s.contains("1 0 0 rg"), "the colour is set");
    assert!(s.contains("0 g"), "and put back after");
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
