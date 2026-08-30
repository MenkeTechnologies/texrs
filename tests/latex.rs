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

/// `\newcommand*` is `\newcommand` with a restriction nothing here reads. The
/// star used to make the name scan give up, which dropped the DEFINITION --
/// pandoc writes `\newcommand*\pandocbounded[1]{...}`, so every use of it was
/// then undefined and the document stopped.
#[test]
fn the_starred_newcommand_defines_the_command() {
    let src = "\\documentclass{article}\n\\newcommand*\\a[1]{[#1]}\n\
               \\newcommand*{\\b}[1]{<#1>}\n\\message{\\a{x}\\b{y}}\n\\end\n";
    assert_eq!(out(src), "[x]<y>");
}

/// `\newcommand{\x}[n][default]` gives the first parameter a default, and a
/// call may leave the brackets out. The default was recorded and never read, so
/// the bracket group reached the text and shifted every argument after it.
#[test]
fn an_optional_argument_is_matched_at_the_call() {
    let src = "\\documentclass{article}\n\\newcommand{\\c}[3][def]{(#1|#2|#3)}\n\
               \\message{\\c[opt]{a}{b} \\c{a}{b}}\n\\end\n";
    assert_eq!(out(src), "(opt|a|b) (def|a|b)");
}

/// The two the books rest on: `\textcolor` with and without its colour model,
/// and `\includegraphics` with its options.
#[test]
fn the_optional_argument_forms_the_documents_write() {
    let src = "\\documentclass{article}\n\\begin{document}\n\
               \\textcolor[rgb]{1.00,0.00,0.00}{RED}\\textcolor{blue}{BLUE}\n\
               \\includegraphics[keepaspectratio]{f.pdf}\\hyperref[lab]{LINK}\n\
               \\end{document}\n";
    assert_eq!(
        texrs::run_text(src)
            .expect("run")
            .split_whitespace()
            .collect::<Vec<_>>(),
        ["REDBLUE", "LINK"]
    );
}

/// A colour model reaches `\textcolor` from INSIDE an expansion, because pandoc
/// calls it from `\NormalTok` and its siblings. The bracket is matched where
/// the arguments are matched, so the source of the tokens does not matter.
#[test]
fn an_optional_argument_is_matched_inside_an_expansion() {
    let src = "\\documentclass{article}\n\
               \\newcommand{\\NormalTok}[1]{\\textcolor[rgb]{0.25,0.44,0.63}{#1}}\n\
               \\begin{document}\n\\NormalTok{code}\n\\end{document}\n";
    assert_eq!(texrs::run_text(src).expect("run").trim(), "code");
}

/// A decided conditional runs the assignments of the arm it took, and only
/// those. Lowering both arms ran both `\let`s and the second won whichever way
/// the test went, which is what stopped LaTeX's `\@ifnextchar` from working --
/// and with it every optional argument written the way LaTeX writes one.
/// `\ifnum` still lowers both arms; its test is a register read.
#[test]
fn only_the_taken_arm_of_a_decided_conditional_assigns() {
    let same = "\\documentclass{article}\n\\def\\a{A}\\def\\b{A}\n\
                \\ifx\\a\\b\\def\\r{TAKEN}\\else\\def\\r{SKIPPED}\\fi\n\\message{\\r}\n\\end\n";
    assert_eq!(out(same), "TAKEN");
    let differ = "\\documentclass{article}\n\\def\\a{A}\\def\\b{B}\n\
                  \\ifx\\a\\b\\def\\r{TAKEN}\\else\\def\\r{SKIPPED}\\fi\n\\message{\\r}\n\\end\n";
    assert_eq!(out(differ), "SKIPPED");
}

/// `\@ifnextchar` is what every optional argument in LaTeX is written on, and
/// `\@ifstar` is the peek for a starred form. Both are in the prelude, and both
/// need the arm that did not run to have made no assignment.
#[test]
fn a_starred_form_dispatches_on_the_star() {
    let src = "\\documentclass{article}\n\\makeatletter\n\
               \\def\\x{\\@ifstar\\xstar\\xplain}\n\
               \\def\\xstar#1{[STAR #1]}\\def\\xplain#1{[PLAIN #1]}\n\
               \\begin{document}\n\\x*{a}\\x{b}\n\\end{document}\n";
    assert_eq!(
        texrs::run_text(src)
            .expect("run")
            .split_whitespace()
            .collect::<Vec<_>>(),
        ["[STAR", "a][PLAIN", "b]"]
    );
}

/// An environment that takes options takes them at `\begin`. pandoc opens every
/// code block with `\begin{Highlighting}[]` -- which is NOT a verbatim
/// environment, because its body is `\NormalTok` and siblings that have to
/// expand -- and an argumentless stub left the brackets in the text.
#[test]
fn an_environment_takes_its_options_at_begin() {
    let src = "\\documentclass{article}\n\\begin{document}\n\
               \\begin{Shaded}\\begin{Highlighting}[]\ncode\n\
               \\end{Highlighting}\\end{Shaded}\n\
               \\begin{minipage}[b]{0.5\\linewidth}mini\\end{minipage}\n\\end{document}\n";
    let got = texrs::run_text(src).expect("run");
    assert!(!got.contains('['), "no options reached the text: {got:?}");
    assert!(got.contains("code") && got.contains("mini"), "{got:?}");
}

/// A header fragment included with `--include-in-header` has no preamble of its
/// own -- it IS preamble -- so the LaTeX layer has to recognise it by the names
/// it uses. Read as plain TeX, its `\@ifundefined` stopped it.
#[test]
fn a_header_fragment_is_recognised_as_latex() {
    let src = "\\makeatletter\n\\@ifundefined{Shaded}{\\newenvironment{Shaded}{}{}}{}\n\
               \\makeatother\n\\message{after}\n\\end\n";
    assert_eq!(out(src), "after");
}

/// fontspec takes its features on either side of the font name, and these books
/// write `\setmainfont{Arimo}[Path=...]`. A peek composed IN FRONT of the
/// argument macro looks at that macro rather than at the bracket behind it, so
/// the trailing group was text; the optional argument is declared instead.
#[test]
fn a_font_declaration_consumes_features_on_either_side() {
    let src = "\\documentclass{article}\n\\begin{document}\n\
               \\setmainfont{Arimo}[Path=/fonts/,Extension=.ttf]\n\
               \\setsansfont[Scale=1]{Orbitron}\\setmathfont[]{STIX Two Math}\n\
               \\vspace*{1cm}\\titlespacing*{\\chapter}{0pt}{20pt}{20pt}\nWORDS\n\
               \\end{document}\n";
    assert_eq!(
        texrs::run_text(src)
            .expect("run")
            .split_whitespace()
            .collect::<Vec<_>>(),
        ["WORDS"]
    );
}
