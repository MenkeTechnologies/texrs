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
