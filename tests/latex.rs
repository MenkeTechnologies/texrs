//! The LaTeX layer: what a document written for LaTeX gets from an engine that
//! has a mouth and an expander and no stomach.
//!
//! `\newcommand` is implemented natively rather than as the `\ifnum...\def`
//! chain latex.ltx uses, because that chain cannot run here: a `\def` inside an
//! arm of a run-time conditional is executed while lowering, and lowering emits
//! both arms (`tests/cases/def_in_conditional_arm.tex`). The behaviour is what
//! latex.ltx specifies; only the implementation differs, which is what a port
//! has to preserve.
//!
//! Everything else is `src/latex/prelude.tex`, loaded when a document says
//! `\documentclass` or `\usepackage`.

fn out(src: &str) -> String {
    texrs::run_messages(src).expect("run")
}

#[test]
fn newcommand_defines_a_macro_with_no_arguments() {
    assert_eq!(
        out("\\documentclass{article}\n\\newcommand{\\hi}{HELLO}\n\\message{\\hi}\n\\end\n"),
        "HELLO"
    );
}

#[test]
fn newcommand_defines_one_taking_arguments() {
    assert_eq!(
        out("\\documentclass{article}\n\\newcommand{\\w}[1]{[#1]}\n\\message{\\w{x}}\n\\end\n"),
        "[x]"
    );
    assert_eq!(
        out("\\documentclass{article}\n\\newcommand{\\p}[2]{<#1|#2>}\n\\message{\\p{a}{b}}\n\\end\n"),
        "<a|b>"
    );
}

#[test]
fn renewcommand_replaces_and_providecommand_does_not() {
    assert_eq!(
        out("\\documentclass{article}\n\\newcommand{\\a}{one}\\renewcommand{\\a}{two}\n\\message{\\a}\n\\end\n"),
        "two"
    );
    assert_eq!(
        out("\\documentclass{article}\n\\newcommand{\\a}{one}\\providecommand{\\a}{two}\n\\message{\\a}\n\\end\n"),
        "one"
    );
}

#[test]
fn a_preamble_directive_consumes_its_arguments() {
    // The point is that the document AFTER it is still read. Leaving the
    // arguments in the stream would put `[11pt]{article}` in the text.
    let src = "\\documentclass[11pt]{article}\n\\usepackage[utf8]{inputenc}\n\
               \\PassOptionsToPackage{x}{y}\n\\message{after}\n\\end\n";
    assert_eq!(out(src), "after");
}

#[test]
fn text_macros_yield_their_argument() {
    let src = "\\documentclass{article}\n\
               \\message{\\texttt{a}\\textbf{b}\\emph{c}\\texorpdfstring{d}{e}}\n\\end\n";
    assert_eq!(out(src), "abcd");
}

#[test]
fn character_macros_produce_their_character() {
    let src = "\\documentclass{article}\n\\message{\\textless\\textgreater\\textbar}\n\\end\n";
    assert_eq!(out(src), "<>|");
}

#[test]
fn state_macros_consume_their_arguments_and_produce_nothing() {
    let src = "\\documentclass{article}\n\\message{[\\label{x}\\setcounter{a}{1}]}\n\\end\n";
    assert_eq!(out(src), "[]");
}

#[test]
fn an_engine_test_takes_the_branch_for_an_engine_we_are_not() {
    // iftex spells these with \let to \iffalse. texrs is not pdfTeX, LuaTeX or
    // XeTeX, and the false arm is the one whose primitives it might have.
    let src = "\\documentclass{article}\n\\usepackage{iftex}\n\
               \\ifPDFTeX \\message{pdf}\\else \\message{other}\\fi\n\\end\n";
    assert_eq!(out(src), "other");
}

#[test]
fn a_plain_tex_document_is_left_alone() {
    // The prelude redefines names a plain document may own -- \section among
    // them -- so it must load only for documents that say they are LaTeX.
    let src = "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6\n\
               \\def\\section#1{PLAIN:#1}\n\\message{\\section{t}}\n\\end\n";
    assert_eq!(out(src), "PLAIN:t");
}

#[test]
fn a_pandoc_style_document_runs() {
    // The shape the publications are generated in: a class, packages, token
    // macros defined by the document, and text through them.
    let src = "\\documentclass[11pt]{article}\n\\usepackage{xcolor}\n\
               \\newcommand{\\NormalTok}[1]{#1}\n\\newcommand{\\KeywordTok}[1]{#1}\n\
               \\begin{document}\n\\section{Title}\n\
               \\message{\\texttt{\\KeywordTok{fn}\\NormalTok{ main}}}\n\
               \\end{document}\n";
    assert_eq!(out(src), "fn main");
}

#[test]
fn an_engine_test_for_dimensions_takes_the_false_branch() {
    // `\ifdim` is recognised-but-not-evaluated: there are no dimen registers to
    // compare. The prelude \lets it to \iffalse so a document that guards a
    // measurement with it takes the path that does not ask for one. Without
    // alias resolution in the expander this still reached the \ifdim arm and
    // was refused, because the dispatch is by name and the alias kept its own.
    let src =
        "\\documentclass{article}\n\\ifdim 1pt>0pt \\message{yes}\\else \\message{no}\\fi\n\\end\n";
    assert_eq!(out(src), "no");
}

#[test]
fn a_newcommand_whose_name_is_not_definable_does_not_stop_the_document() {
    // LaTeX raises an error and carries on. Refusing the whole file over one
    // definition loses everything after it; the definition is dropped, and its
    // arguments with it so they do not land in the text.
    let src = "\\documentclass{article}\n\\newcommand{notacs}{body}\n\\message{after}\n\\end\n";
    assert_eq!(out(src), "after");
}

#[test]
fn directlua_is_consumed_rather_than_run() {
    // texrs has no Lua. Consuming the chunk lets the document be read; a
    // document whose OUTPUT depended on what the Lua computed is wrong here
    // rather than refused, which is why this is stated in the README.
    let src = "\\documentclass{article}\n\\directlua{tex.print('x')}\n\\message{after}\n\\end\n";
    assert_eq!(out(src), "after");
}

#[test]
fn a_tikz_path_command_is_consumed_to_its_semicolon() {
    let src = "\\documentclass{article}\n\\usepackage{tikz}\n\
               \\draw[thick,red] (0,0) -- (1,1);\n\\message{after}\n\\end\n";
    assert_eq!(out(src), "after");
}
