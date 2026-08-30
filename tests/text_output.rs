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
