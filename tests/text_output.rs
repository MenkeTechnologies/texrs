//! `--text`: what the document SAYS, not what it announced.
//!
//! texrs emitted only the `\message` stream, so the words of a document went
//! nowhere -- an 880 KB book compiled to a program that printed 66 bytes, the
//! filename and nothing else. It "ran" in the sense that nothing errored, which
//! is not the sense anyone means. Ordinary character tokens now become
//! `Cmd::Text` and reach the output.
//!
//! This is not typesetting. There is no line breaking, no page, no font. It is
//! the text, in order, after every macro has been expanded.

fn text(src: &str) -> String {
    texrs::run_text(src).expect("run")
}

#[test]
fn a_plain_document_yields_its_words() {
    assert_eq!(
        text("\\catcode`\\{=1 \\catcode`\\}=2\nhello world\n\\end\n").trim(),
        "hello world"
    );
}

#[test]
fn a_macro_is_expanded_before_the_text_is_taken() {
    let src = "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6\n\
               \\def\\greet#1{hello #1}\n\\greet{world}\n\\end\n";
    assert_eq!(text(src).trim(), "hello world");
}

#[test]
fn a_latex_document_yields_its_prose_with_the_markup_resolved() {
    let src = "\\documentclass{article}\n\\begin{document}\n\
               \\section{Title}\nA paragraph with \\textbf{bold} and \\emph{stress}.\n\
               \\end{document}\n";
    let got = text(src);
    assert!(got.contains("Title"), "the heading is text too: {got:?}");
    assert!(
        got.contains("A paragraph with bold and stress"),
        "got {got:?}"
    );
}

#[test]
fn the_escaped_characters_come_through_as_themselves() {
    // `\%` and `\textless` are the two shapes: a control symbol and a named
    // character. `\%` is the one that cannot be written naively, because a per
    // cent sign in a macro body starts a comment and eats the closing brace.
    let src = "\\documentclass{article}\n\\begin{document}\n\
               100\\% and \\textless tag\\textgreater\n\\end{document}\n";
    let got = text(src);
    assert!(got.contains("100%"), "got {got:?}");
    assert!(got.contains("<tag>"), "got {got:?}");
}

#[test]
fn a_redefined_primitive_is_the_redefinition() {
    // LaTeX redefines \end to close an environment. Dispatching primitives by
    // name meant \end was always the run-stopping primitive, so a LaTeX
    // document stopped at its first \end{...} and produced its preamble only.
    let src = "\\documentclass{article}\n\\begin{document}\n\
               before \\end{document}\n";
    assert!(text(src).contains("before"), "the body must be reached");
}

#[test]
fn messages_are_not_the_text_and_the_text_is_not_the_messages() {
    // Two separate streams: `--text` must not start printing \message output,
    // or the differential suite's comparison against tex stops meaning anything.
    let src = "\\catcode`\\{=1 \\catcode`\\}=2\nwords \\message{announced}\n\\end\n";
    let t = text(src);
    assert!(t.contains("words"), "got {t:?}");
    assert!(!t.contains("announced"), "a message is not the text: {t:?}");
}

#[test]
fn a_group_that_only_carries_text_does_not_break_the_run() {
    // A group exists to save registers and scope the macro table; the table is
    // a compile-time fact, so a group assigning no register has nothing to do
    // at run time. Keeping it split the text either side into separate
    // constants, and a document's braces are everywhere -- every
    // `\NormalTok{...}` is one -- so a 4 MB book exhausted fusevm's
    // 65,536-entry constant pool and the compile PANICKED.
    let mut src = String::from("\\documentclass{article}\n\\begin{document}\n");
    for i in 0..5000 {
        src.push_str(&format!("{{word{i}}} "));
    }
    src.push_str("\n\\end{document}\n");
    let got = text(&src);
    assert!(got.contains("word0"), "the first group's text is there");
    assert!(
        got.contains("word4999"),
        "and the last one's: {} bytes",
        got.len()
    );
}

#[test]
fn a_verbatim_body_is_characters_and_not_tex() {
    // The point of the environment: a backslash in a listing is a backslash.
    // Reading it as TeX is why a book of code samples could not be read --
    // roff markup inside a listing, \fINAME, became a control sequence nobody
    // defined.
    let src = "\\documentclass{article}\n\\begin{document}\n\
               \\begin{verbatim}\n\\fINAME \\not{TeX} 100% raw\n\\end{verbatim}\n\
               after\n\\end{document}\n";
    let got = text(src);
    assert!(got.contains("\\fINAME"), "the backslash survives: {got:?}");
    assert!(
        got.contains("100% raw"),
        "a per cent is not a comment: {got:?}"
    );
    assert!(got.contains("after"), "and the document continues: {got:?}");
}

#[test]
fn pandoc_highlighting_expands_rather_than_passing_through() {
    // Highlighting and Shaded LOOK like code environments and are not: Pandoc
    // fills them with \NormalTok and friends, which have to expand for the code
    // to come out as code rather than as markup.
    let src = "\\documentclass{article}\n\\newcommand{\\NormalTok}[1]{#1}\n\
               \\newenvironment{Highlighting}{}{}\n\\begin{document}\n\
               \\begin{Highlighting}\n\\NormalTok{let x = 1;}\n\\end{Highlighting}\n\
               \\end{document}\n";
    let got = text(src);
    assert!(
        got.contains("let x = 1;"),
        "the code, not the markup: {got:?}"
    );
    assert!(
        !got.contains("NormalTok"),
        "markup must not survive: {got:?}"
    );
}

#[test]
fn a_character_above_latin1_ends_a_control_word() {
    // TeX82 reads BYTES, so such a character is a run of Others and never part
    // of a control word. Calling them Letters made `\textgreater→key` lex as
    // ONE control sequence named `textgreater→key`, so a document full of
    // arrows failed on names nobody wrote.
    let src = "\\documentclass{article}\n\\begin{document}\n\\textgreater→key\n\\end{document}\n";
    let got = text(src);
    assert!(got.contains('>'), "the macro resolved: {got:?}");
    assert!(got.contains('→'), "and the arrow is text: {got:?}");
}

#[test]
fn a_blank_line_ends_the_paragraph() {
    // The mouth already synthesises a `\par` per blank line (§304) and the
    // lowerer dropped it on the floor, so a book arrived at the line breaker as
    // ONE paragraph: scifi2/docs/book.tex holds 3,163 blank lines and produced
    // 58 separators, 3,229 once it is kept. Two consequences in the PDF -- no
    // paragraph got the ragged last line it is entitled to, and the words on
    // either side of the suppressed break welded together, which is how a title
    // page came out `// A NOVEL OF DEEP TIME //TWO SHIPS IN THE DARK.`
    let src = "\\documentclass{article}\n\\begin{document}\n\
               first block\n\nsecond block\n\\end{document}\n";
    let got = text(src);
    let paras: Vec<&str> = got
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    assert_eq!(
        paras,
        vec!["first block", "second block"],
        "a blank line is a paragraph boundary, not a space: {got:?}"
    );
}

#[test]
fn an_explicit_par_ends_the_paragraph_too() {
    // Same break, written out. Pandoc's output uses `\par` directly in places
    // where a blank line would be swallowed by an environment, so the two
    // spellings have to mean the same thing.
    let src = "\\documentclass{article}\n\\begin{document}\n\
               first block\\par second block\n\\end{document}\n";
    let got = text(src);
    assert!(
        got.contains("first block\n\nsecond block"),
        "an explicit \\par breaks the paragraph: {got:?}"
    );
}

#[test]
fn an_alignment_tab_separates_cells_and_is_not_an_ampersand() {
    // plain.tex:14 makes `&` catcode 4; nothing here did, so every cell
    // separator stayed an ordinary character and printed itself. A book of
    // keybinding tables came out of --pdf with 8,941 stray ampersands where
    // the same source set by lualatex has 23 — every one of those the escaped
    // `\&`. There are no cells to set into yet, so a boundary is the space
    // that would stand between two cells.
    let src = "\\documentclass{article}\n\\begin{document}\n\
               \\begin{tabular}{ll}\nleft & right \\\\\n\\end{tabular}\n\
               \\end{document}\n";
    let got = text(src);
    assert!(!got.contains('&'), "a tab is not a character: {got:?}");
    assert!(got.contains("left"), "the first cell's text: {got:?}");
    assert!(got.contains("right"), "and the second's: {got:?}");
}

#[test]
fn an_escaped_ampersand_is_still_an_ampersand() {
    // The other half of giving `&` catcode 4: `\&` is defined one line before
    // the `\catcode` in the prelude, while `&` is still ordinary, so its body
    // holds the character and not an alignment tab. Defined after, `AT\&T`
    // would have printed as `AT T`.
    let src = "\\documentclass{article}\n\\begin{document}\nAT\\&T\n\\end{document}\n";
    let got = text(src);
    assert!(got.contains("AT&T"), "got {got:?}");
}

#[test]
fn csstring_reaches_running_text_as_string_does() {
    // `\csstring` was answered only inside a `\message`, so in the body of a
    // document it was an undefined control sequence -- and it is the only way
    // to write ONE backslash, since `\string\\` writes the escape character and
    // then the name `\`. That is what the prelude's `\textbackslash` needs.
    let src = "\\catcode`\\{=1 \\catcode`\\}=2\n\\def\\f{F}[\\csstring\\f][\\string\\f]\n\\end\n";
    assert_eq!(text(src).trim(), "[f][\\f]");
}

#[test]
fn a_list_reads_back_as_marked_items_rather_than_as_one_run_of_words() {
    // `\begin{itemize}` expanded to nothing and `\item` to its optional
    // argument, so a list came back as "alpha item bravo item" -- one line,
    // no mark, welded to the prose after it. The indent that separates them
    // on the page is a position and leaves nothing here; the MARK is text the
    // document means and stays.
    let src = "\\documentclass{article}\n\\begin{document}\n\
               \\begin{itemize}\n\\tightlist\n\\item\n  alpha item\n\
               \\item\n  bravo item\n\\end{itemize}\n\
               \\begin{enumerate}\n\\item\n  first\n\\item\n  second\n\
               \\end{enumerate}\n\
               \\begin{description}\n\\item[a term]\n  its meaning\n\
               \\end{description}\n\
               after the lists\n\\end{document}\n";
    let got = text(src);
    let lines: Vec<String> = got
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<&str>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect();
    let has = |want: &str| lines.iter().any(|line| line == want);
    assert!(
        has("\u{2022} alpha item") && has("\u{2022} bravo item"),
        "each item is a line of its own, carrying its bullet: {lines:?}"
    );
    assert!(
        has("1. first") && has("2. second"),
        "an enumerate's items carry their numbers: {lines:?}"
    );
    assert!(
        has("a term its meaning"),
        "a description's term is its mark: {lines:?}"
    );
    assert!(
        has("after the lists"),
        "and the prose after a list is not welded to its last item: {lines:?}"
    );
    // The indent marker says where a line starts on the page, which is not
    // something a reader of the text asked for -- and neither is the digit
    // after it, which would otherwise put a `1` in front of every item.
    let leaked: Vec<char> = got
        .chars()
        .filter(|c| c.is_control() && *c != '\n')
        .collect();
    assert!(leaked.is_empty(), "the markers left {leaked:?} in {got:?}");
}

/// No marker may reach the text a reader gets, for every marker there is.
///
/// This is the check three parallel implementations of this port each needed
/// and none had. Each added a marker to typeset.rs, taught the PDF path to draw
/// it, and left `without_marks` alone -- so the control character and its
/// argument were written straight into `texrs --text`: 122 of them in awkrs
/// from one of them, against zero at the commit before. Walking the registry
/// means the next one fails here instead.
#[test]
fn every_marker_is_stripped_from_the_text_a_reader_gets() {
    for (marker, has_argument) in texrs::typeset::MARKERS {
        let mut marked = String::from("alpha");
        marked.push(*marker);
        if *has_argument {
            // The argument is a letter, so leaving it would put an `m` in the
            // middle of the words either side.
            marked.push('m');
        }
        marked.push_str("bravo");
        let got = texrs::text_without_marks(&marked);
        let leaked: Vec<char> = got
            .chars()
            .filter(|c| c.is_control() && *c != '\n')
            .collect();
        assert!(
            leaked.is_empty(),
            "U+{:04X} left {leaked:?} in the text a reader gets: {got:?}",
            *marker as u32
        );
        assert!(
            !got.contains("alphambravo"),
            "U+{:04X} left its argument character in the words: {got:?}",
            *marker as u32
        );
    }
}

/// The colour spec between its markers is not text either.
#[test]
fn a_colour_spec_does_not_reach_the_reader_as_digits() {
    // `\u{1}0.5,0,0\u{2}words\u{3}` is one coloured run. The r,g,b between
    // the first two markers is an instruction; printing it would put "0.5,0,0"
    // in front of every coloured word.
    let marked = "before\u{1}0.5,0,0\u{2}words\u{3}after";
    let got = texrs::text_without_marks(marked);
    assert_eq!(got, "beforewordsafter", "got {got:?}");
}

#[test]
fn a_table_reads_back_as_rows_rather_than_as_one_run_of_words() {
    // `&` was a space and `\\` a newline that `split_whitespace` swallowed, so
    // a table came back as "Name Value alpha 1 beta 2" welded to the sentence
    // after it. A row ends at a line in text, which is what the row marker
    // means where there are no columns to set.
    let src = "\\documentclass{article}\n\\begin{document}\n\
               \\begin{tabular}{ll}\n\\toprule\nName & Value \\\\\n\\midrule\n\
               alpha & 1 \\\\\nbeta & 2 \\\\\n\\bottomrule\n\\end{tabular}\n\
               after the table\n\\end{document}\n";
    let got = text(src);
    // The spaces around the source's own `&` are the document's and stay; what
    // is being asked here is which LINE each cell came out on.
    let rows: Vec<String> = got
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<&str>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect();
    let has = |want: &str| rows.iter().any(|row| row == want);
    assert!(
        has("Name Value") && has("alpha 1") && has("beta 2"),
        "each row is a line of its own: {rows:?}"
    );
    assert!(
        has("after the table"),
        "and the prose after the table is not welded to its last row: {rows:?}"
    );
    // The rules are drawn on the page and are not words, so they leave nothing
    // -- not the mark, and not the letter naming which rule it was.
    let leaked: Vec<char> = got
        .chars()
        .filter(|c| c.is_control() && *c != '\n')
        .collect();
    assert!(
        leaked.is_empty(),
        "control characters reached the reader: {leaked:?}"
    );
}

/// `\ref` names the sectioning unit its label stands in, and the number is
/// counted the way the class counts one.
///
/// The prelude answered `\ref` with nothing, so `See chapter \ref{ch:one}.`
/// came out as `See chapter .` -- and a book full of "see chapter" with no
/// number reads as broken prose rather than as a missing feature. Measured on
/// this document before: `See chapter , section , chapter , and .`
#[test]
fn a_ref_sets_the_number_of_the_unit_its_label_stands_in() {
    let src = "\\documentclass{report}\n\\begin{document}\n\
               \\chapter{First}\\label{ch:one}\nText in the first chapter.\n\
               \\section{Alpha}\\label{sec:alpha}\nMore text.\n\
               \\chapter{Second}\\label{ch:two}\n\
               See chapter \\ref{ch:one}, section \\ref{sec:alpha}, \
               chapter \\ref{ch:two}, and \\ref{nope}.\n\
               \\end{document}\n";
    let got = text(src);
    let sentence = got.split_whitespace().collect::<Vec<&str>>().join(" ");
    assert!(
        sentence.contains("See chapter 1, section 1.1, chapter 2, and ??."),
        "a chapter counts 1, 2 and a section inside chapter 1 is 1.1; a label \
         the document never declared sets ?? the way LaTeX's own \\@setref \
         does: {sentence:?}"
    );
}

/// A label declares a name for a place and is not text.
///
/// It reaches the page -- that is how the page it fell on is read back for a
/// `\pageref` -- so the one place that decides what a reader gets has to know
/// about it. Three implementations in a row forgot that place; see
/// `typeset::MARKERS`.
#[test]
fn a_label_leaves_neither_its_key_nor_its_marker_in_the_words() {
    let src = "\\documentclass{report}\n\\begin{document}\n\
               \\chapter{First}\\label{ch:one}\n\
               Before \\label{mid-word-key} after.\n\
               \\end{document}\n";
    let got = text(src);
    assert!(
        !got.contains("mid-word-key") && !got.contains("ch:one"),
        "a label key is a name for a place, not a word: {got:?}"
    );
    let leaked: Vec<char> = got
        .chars()
        .filter(|c| c.is_control() && *c != '\n')
        .collect();
    assert!(
        leaked.is_empty(),
        "control characters reached the reader: {leaked:?}"
    );
}

/// A picture has no words, and none of its source is any.
///
/// The picture travels the text stream as an encoded marker span so the PDF
/// path can draw it; a reader asking for the document's TEXT must get the
/// prose either side and nothing from between them. Read as text before the
/// span was recognised, a single picture put several hundred characters of
/// base64 -- or, before that, the picture's own TikZ source -- into the middle
/// of the paragraph.
#[test]
fn a_picture_contributes_no_words_and_leaves_no_marker() {
    let src = "\\documentclass{article}\n\\usepackage{tikz}\n\\begin{document}\n\
               Before.\n\\begin{tikzpicture}\n\
               \\draw[thick] (0,0) -- (3,0) -- (3,2) -- cycle;\n\
               \\node at (1,1) {Inside};\n\
               \\end{tikzpicture}\n\
               After.\n\\end{document}\n";
    let got = text(src);
    assert!(
        got.contains("Before.") && got.contains("After."),
        "the prose either side survives: {got:?}"
    );
    for absent in ["draw", "cycle", "Inside", "tikzpicture"] {
        assert!(
            !got.contains(absent),
            "{absent:?} is picture source, not text: {got:?}"
        );
    }
    let leaked: Vec<char> = got
        .chars()
        .filter(|c| c.is_control() && *c != '\n')
        .collect();
    assert!(
        leaked.is_empty(),
        "control characters reached the reader: {leaked:?}"
    );
}

#[test]
fn a_pair_of_backticks_and_a_pair_of_apostrophes_are_the_curly_quotes() {
    // TeX's own spelling of the quotation marks, and the reason it is a
    // ligature rather than a font's business: the two characters are joined
    // into one BEFORE anything measures the line. A literal pair of backticks
    // is 6.660pt in Arimo at 10pt against 3.330pt for the quote it stands for,
    // so a quoted line measured from the literal is set too wide.
    let src = "\\documentclass{article}\n\\begin{document}\n\
               ``hello there''\n\\end{document}\n";
    let got = text(src);
    assert!(
        got.contains("\u{201c}hello there\u{201d}"),
        "the pairs are one character each: {got:?}"
    );
    assert!(!got.contains("``"), "no literal backticks left: {got:?}");
    assert!(!got.contains("''"), "no literal apostrophes left: {got:?}");
}

#[test]
fn two_hyphens_are_an_en_dash_and_three_are_an_em_dash() {
    let src = "\\documentclass{article}\n\\begin{document}\n\
               pages 1--10, and then--- a break.\n\\end{document}\n";
    let got = text(src);
    assert!(got.contains("1\u{2013}10"), "an en dash between: {got:?}");
    assert!(got.contains("then\u{2014} a"), "and an em dash: {got:?}");
    assert!(!got.contains("--"), "no literal run survives: {got:?}");
}

#[test]
fn a_lone_hyphen_or_quote_forms_no_ligature() {
    // A PAIR is what the ligature program joins. What a lone quote is drawn as
    // is the FONT's encoding -- cmr draws an apostrophe as a right single
    // quote -- which is a different question and not decided here, so nothing
    // in this document may come out as one of the four characters the program
    // makes.
    let src = "\\documentclass{article}\n\\begin{document}\n\
               a well-known don't `x'\n\\end{document}\n";
    let got = text(src);
    assert!(got.contains("well-known"), "the hyphen stays: {got:?}");
    assert!(
        !got.contains(['\u{2013}', '\u{2014}', '\u{201c}', '\u{201d}']),
        "and no pair was joined: {got:?}"
    );
}

#[test]
fn four_hyphens_are_an_em_dash_and_a_hyphen_the_way_tex_sets_them() {
    // The pairs are applied left to right and nothing joins an em dash to a
    // hyphen, so a longer run falls out of them: four are `---' and `-', five
    // are `---' and `--'. Documented here because the run has to mean
    // something rather than be undefined.
    let src = "\\documentclass{article}\n\\begin{document}\n\
               a---- b----- c\n\\end{document}\n";
    let got = text(src);
    assert!(got.contains("a\u{2014}- "), "em dash then hyphen: {got:?}");
    assert!(
        got.contains("b\u{2014}\u{2013} "),
        "em dash then en dash: {got:?}"
    );
}

#[test]
fn a_group_between_two_hyphens_keeps_them_apart() {
    // `-{}-' is how a LaTeX document has always asked for two hyphens where
    // two hyphens are meant, so the group has to break the pair.
    let src = "\\documentclass{article}\n\\begin{document}\n\
               a-{}-b\n\\end{document}\n";
    let got = text(src);
    assert!(got.contains("a--b"), "both hyphens survive: {got:?}");
    assert!(!got.contains('\u{2013}'), "and no en dash: {got:?}");
}

#[test]
fn a_flag_inside_texttt_keeps_both_of_its_hyphens() {
    // `\texttt' is a text command, not an environment: the prelude expands it
    // to face markers and its ARGUMENT's characters flow through the same
    // funnel prose does. Joining the pair would rename the flag.
    let src = "\\documentclass{article}\n\\begin{document}\n\
               run \\texttt{grep --color=auto} now, 1--2\n\\end{document}\n";
    let got = text(src);
    assert!(got.contains("grep --color=auto"), "code is code: {got:?}");
    assert!(
        got.contains("1\u{2013}2"),
        "and prose after it is still prose: {got:?}"
    );
}

#[test]
fn a_declared_mono_face_keeps_the_hyphens_inside_its_group() {
    // The declaration form of the same thing: `{\ttfamily ...}' is what every
    // book in the corpus redefines `\texttt' to reach.
    let src = "\\documentclass{article}\n\\begin{document}\n\
               {\\ttfamily make --jobs} and 1--2\n\\end{document}\n";
    let got = text(src);
    assert!(got.contains("make --jobs"), "code is code: {got:?}");
    assert!(got.contains("1\u{2013}2"), "prose is prose: {got:?}");
}

#[test]
fn a_pandoc_listing_keeps_the_dashes_and_quotes_of_its_source() {
    // Highlighting is deliberately not verbatim -- its lines are re-lexed so
    // that \NormalTok and the colour it carries still expand -- so a program's
    // characters reach the funnel exactly as prose does.
    let src = "\\documentclass{article}\n\\newcommand{\\NormalTok}[1]{#1}\n\
               \\newenvironment{Highlighting}{}{}\n\\begin{document}\n\
               \\begin{Highlighting}\n\\NormalTok{curl --silent ``u'' -- x}\n\
               \\end{Highlighting}\n1--2\n\\end{document}\n";
    let got = text(src);
    assert!(
        got.contains("curl --silent ``u'' -- x"),
        "the program is untouched: {got:?}"
    );
    let line = got
        .lines()
        .find(|l| l.contains("curl"))
        .expect("the listing's line");
    assert!(
        !line.contains(['\u{2013}', '\u{2014}', '\u{201c}', '\u{201d}']),
        "nothing in it was joined: {line:?}"
    );
    assert!(
        got.contains("1\u{2013}2"),
        "and prose after the listing is still prose: {got:?}"
    );
}

#[test]
fn a_verbatim_body_keeps_the_dashes_and_quotes_it_wrote() {
    // A verbatim body is pushed whole and never reaches the funnel, and the
    // character before `\begin{verbatim}' must not pair with the first of it
    // either.
    let src = "\\documentclass{article}\n\\begin{document}\n\
               -\\begin{verbatim}\n-- ``x'' ---\n\\end{verbatim}\nafter 1--2\n\
               \\end{document}\n";
    let got = text(src);
    assert!(
        got.contains("-- ``x'' ---"),
        "the body is characters: {got:?}"
    );
    assert!(
        got.contains("1\u{2013}2"),
        "and prose after it is still prose: {got:?}"
    );
}
