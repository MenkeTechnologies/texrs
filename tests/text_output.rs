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
