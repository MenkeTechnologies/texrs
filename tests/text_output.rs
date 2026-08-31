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
