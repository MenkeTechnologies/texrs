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
fn what_a_chunk_prints_is_input_and_not_a_message() {
    // The chunk RUNS -- `tests/lua.rs` pins the arithmetic -- and what
    // `tex.print` writes is pushed back as INPUT, to be read as the document
    // continues. It is not a message, so it does not reach this stream: the
    // `x` here is set, and only `after` was sent to the terminal.
    let src = "\\documentclass{article}\n\\directlua{tex.print('x')}\n\\message{after}\n\\end\n";
    assert_eq!(out(src), "after");
    // And it really did run: a chunk that fails stops the run, which a
    // consumed chunk could not do.
    assert!(
        texrs::run_messages("\\documentclass{article}\n\\directlua{error('boom')}\n\\end\n")
            .is_err()
    );
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

/// `\textbackslash` is ONE backslash. It was `\string\\`, and `\\` is a control
/// sequence whose name is a backslash, so \string wrote the escape character
/// and then that name -- two of them. Every `\textbackslash` in a pandoc code
/// listing came out doubled: 468 of them in strykelang's book gave 936
/// backslashes in the text, and a path `C:\Users` printed as `C:\\Users`.
#[test]
fn textbackslash_is_a_single_backslash() {
    let src = "\\documentclass{article}\n\\begin{document}\n\
               [\\textbackslash{}] [C:\\textbackslash Users]\n\\end{document}\n";
    let got = texrs::run_text(src).expect("run");
    assert!(
        got.contains("[\\]") && got.contains("[C:\\Users]"),
        "one backslash per \\textbackslash, got {got:?}"
    );
    assert!(!got.contains("\\\\"), "doubled: {got:?}");
}

/// The same inside a `\message`, which reaches the primitive by another path.
#[test]
fn textbackslash_is_a_single_backslash_in_a_message() {
    assert_eq!(
        out("\\documentclass{article}\n\\message{[\\textbackslash]}\n\\end\n"),
        "[\\]"
    );
}

/// `\^` is the circumflex accent, and pandoc also writes a bare caret as `\^{}`
/// -- the accent over nothing. `#1` alone dropped it, so the regexes in these
/// books lost their anchors: `/\^{}END/` printed as `/END/`, which says
/// something else. An empty argument is the character itself; a letter still
/// comes through as the letter, since texrs composes no glyph.
#[test]
fn a_circumflex_over_nothing_is_a_caret() {
    let src = "\\documentclass{article}\n\\begin{document}\n\
               /\\^{}END/ \\^e \\^{o} \\textasciicircum{}\n\\end{document}\n";
    assert_eq!(
        texrs::run_text(src)
            .expect("run")
            .split_whitespace()
            .collect::<Vec<_>>(),
        ["/^END/", "e", "o", "^"]
    );
}

/// A counter is a `\count` register, and the five representations read it.
///
/// `\setcounter`, `\addtocounter` and `\stepcounter` were two-argument macros
/// producing nothing, so every counter in every document stayed at whatever it
/// was declared with and `\arabic` had nothing to read.
#[test]
fn a_declared_counter_is_set_stepped_and_read_back() {
    let src = "\\documentclass{article}\n\\newcounter{step}\n\
               \\setcounter{step}{4}\\stepcounter{step}\\addtocounter{step}{2}\n\
               \\message{[\\arabic{step}][\\alph{step}][\\Alph{step}]}\n\\end\n";
    assert_eq!(out(src), "[7][g][G]");
}

/// A counter with no register is not a counter: LaTeX raises
/// `! LaTeX Error: No counter 'a' defined.` and texrs, which has no error
/// recovery to raise it into, consumes the arguments and produces nothing.
///
/// The alternative is what the port did first: `\setcounter{a}{1}` expanded to
/// the characters `\count \cr@a =1`, and an assignment is not expandable, so a
/// `\message` wrote them out as the document's text.
#[test]
fn a_counter_that_was_never_declared_writes_nothing_and_eats_its_arguments() {
    let src = "\\documentclass{article}\n\
               \\message{[\\setcounter{nosuch}{1}\\addtocounter{nosuch}{2}]}\n\\end\n";
    assert_eq!(out(src), "[]");
}

/// `\section` steps its counter, which is what makes a `\label` inside it
/// record the section's number rather than nothing at all.
///
/// The starred form is not numbered, exactly as latex.ltx's `\@ssect` is not,
/// and the optional argument -- the contents entry -- is read and dropped.
#[test]
fn a_section_steps_its_counter_and_a_starred_one_does_not() {
    let src = "\\documentclass{article}\n\\begin{document}\n\
               \\section{One}\\section[toc]{Two}\\section*{Unnumbered}\n\
               \\message{[\\arabic{section}]}\n\\end{document}\n";
    assert_eq!(out(src), "[2]");
}

/// `\ref` to a label nothing wrote down answers `??`, and -- the part that was
/// broken -- answers it WITHOUT eating the text after the reference.
///
/// `\@setref` reached an undefined `\r@x` and `\@firstoftwo` took the two
/// tokens following the reference as its arguments instead: `\ref{x} on page`
/// came out as `n page`, the reference gone and the `o` of `on` with it.
#[test]
fn an_unresolved_reference_is_two_question_marks_and_eats_no_text() {
    let src = "\\documentclass{article}\n\\begin{document}\n\
               See \\ref{gone} on page \\pageref{gone} now.\n\\end{document}\n";
    assert_eq!(
        texrs::run_text(src)
            .expect("run")
            .split_whitespace()
            .collect::<Vec<_>>(),
        ["See", "??", "on", "page", "??", "now."]
    );
}

/// `\DeclareOption` records the code an option runs and `\ExecuteOptions` runs
/// it, walking the comma list.
///
/// latex.ltx walks it with `\@for`, which cannot run here -- `\@iforloop` calls
/// itself and the lowerer inlines a macro's body -- so the walk is unrolled.
/// Every class stopped at its own `\ExecuteOptions` line before this.
#[test]
fn declared_options_are_run_by_execute_options_in_the_order_written() {
    let src = "\\documentclass{article}\n\\makeatletter\n\
               \\DeclareOption{a}{\\message{A}}\\DeclareOption{b}{\\message{B}}\n\
               \\ExecuteOptions{b,a,b}\n\\makeatother\n\\end\n";
    assert_eq!(out(src), "B A B");
}

/// `\newenvironment` defines both bodies, including a `[n]` argument count.
/// They were both dropped, so a document that defined its own environment got
/// neither half of it.
#[test]
fn a_new_environment_runs_both_of_its_bodies() {
    let src = "\\documentclass{article}\n\
               \\newenvironment{note}[1]{<#1:}{>}\n\
               \\message{\\begin{note}{k}body\\end{note}}\n\\end\n";
    assert_eq!(out(src), "<k:body>");
}

/// A bibliography entry is numbered from the `.aux`, which is where LaTeX takes
/// a `\cite`'s number from too -- so the label and the citation always agree.
/// `\bibitem` was undefined, and an undefined control sequence ends the run:
/// everything after the bibliography was lost.
#[test]
fn a_bibliography_numbers_its_entries_and_its_citations_alike() {
    let src = "\\documentclass{article}\n\\begin{document}\n\
               Read \\cite{one} and \\cite{two}.\n\
               \\begin{thebibliography}{9}\n\
               \\bibitem{one} First.\n\\bibitem{two} Second.\n\
               \\end{thebibliography}\n\\end{document}\n";
    let got = texrs::run_text(src).expect("run");
    // Without a file to write the .aux into, every key is the `?` LaTeX prints
    // for a citation with no entry -- and the entries are still there.
    assert!(got.contains("Read [?] and [?]."), "{got}");
    assert!(got.contains("First.") && got.contains("Second."), "{got}");
}

/// The `.aux` round trip: a run writes what its labels resolved to, and the
/// next run reads them back. This is LaTeX's own two-pass model, and it is why
/// `latex` has to be run twice.
#[test]
fn a_label_written_to_the_aux_is_what_the_next_run_resolves() {
    let dir = std::env::temp_dir().join(format!("texrs-aux-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let doc = dir.join("crossref.tex");
    let src = "\\documentclass{article}\n\\begin{document}\n\
               \\section{First}\\label{sec:a}\n\
               Section \\ref{sec:a} and \\cite{k}.\n\
               \\begin{thebibliography}{9}\\bibitem{k} A book.\\end{thebibliography}\n\
               \\end{document}\n";
    std::fs::write(&doc, src).expect("write");
    let _ = std::fs::remove_file(dir.join("crossref.aux"));
    let got = texrs::run_text_at(&doc, src).expect("run");
    let aux = std::fs::read_to_string(dir.join("crossref.aux")).expect("aux written");
    assert!(aux.contains("\\newlabel{sec:a}{{1}{0}}"), "{aux}");
    assert!(aux.contains("\\bibcite{k}{1}"), "{aux}");
    // `0.1` rather than `1`: the number comes from `typeset::unit_numbers`,
    // which counts chapter, section and subsection and joins every level down
    // to the one asked for -- so a class with no chapters still carries the
    // chapter's nought. LaTeX writes `1` here, and BUGS.md records the
    // difference; what this test is about is that the reference RESOLVED,
    // against a `??` that means the run could not settle it.
    assert!(got.contains("Section 0.1 and [1]."), "{got}");
    assert!(!got.contains("??"), "every reference resolved: {got}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `\newif` gives a switch its two setters, and the ones the FORMAT declares
/// have to be there before a class reads them: article.cls says
/// `\@mparswitchfalse` on its first page, and the class would not load at all
/// without it -- `texrs: package article needs \@mparswitchfalse`.
#[test]
fn a_kernel_switch_has_both_of_its_setters() {
    let src = "\\documentclass{article}\n\\makeatletter\n\
               \\@mparswitchtrue\\if@mparswitch\\message{on}\\else\\message{off}\\fi\n\
               \\@mparswitchfalse\\if@mparswitch\\message{on}\\else\\message{off}\\fi\n\
               \\makeatother\n\\end\n";
    assert_eq!(out(src), "on off");
}

/// `\newcount` and its relatives hand out a REGISTER, not just a name.
///
/// The allocator itself cannot be written in this engine -- `src/latex/allocate.rs`
/// says why -- so the register numbers are decided before the run and the
/// declaration consumes the name it was already given. What that has to mean is
/// that the name behaves like the register it stands for: assignable, readable,
/// and its own rather than shared with the next declaration.
#[test]
fn the_newcount_family_gives_each_name_a_register_of_its_own() {
    let src = "\\documentclass{article}\n\\makeatletter\n\
               \\newcount\\testa\\newcount\\testb\\newdimen\\testd\\newskip\\tests\n\
               \\testa=7 \\testb=9 \\advance\\testa by 1 \\testd=3pt \\tests=4pt\n\
               \\message{[\\the\\testa][\\the\\testb][\\the\\testd][\\the\\tests]}\n\
               \\makeatother\n\\end\n";
    assert_eq!(out(src), "[8][9][3.0pt][4.0pt]");
}

/// `\newtoks` reaches the token registers, and `\newbox` is a number the way
/// plain TeX's `\chardef` allocator makes it.
#[test]
fn a_new_token_register_holds_a_list() {
    let src = "\\documentclass{article}\n\\makeatletter\n\
               \\newtoks\\testt \\testt={A B}\n\\message{[\\the\\testt]}\n\
               \\makeatother\n\\end\n";
    assert_eq!(out(src), "[A B]");
}

/// `\IfFileExists` answers the file system rather than always saying no.
///
/// Both arms matter and for different reasons: the false one is what a preamble
/// takes when it means "use this only if it is here", and the true one is what
/// `\InputIfFileExists` reads a file through. A name the scan could not resolve
/// -- one built by expansion -- has to keep taking the false arm, because the
/// dispatch is a comparison against a sentinel and there is nothing else it
/// could match.
#[test]
fn if_file_exists_answers_the_file_system() {
    // In the working directory rather than in a temp one, and under a name
    // nothing else uses: `\IfFileExists` searches where `\input` searches, which
    // begins at `.`, and changing the process's directory would race every other
    // test in this binary.
    let there = std::path::Path::new("texrs-iffileexists-probe.tex");
    std::fs::write(there, "\\message{[read]}\n").expect("write");
    let src = "\\documentclass{article}\n\
               \\IfFileExists{texrs-iffileexists-probe.tex}{\\message{[yes]}}{\\message{[no]}}\n\
               \\IfFileExists{texrs-iffileexists-absent.tex}{\\message{[yes]}}{\\message{[no]}}\n\
               \\InputIfFileExists{texrs-iffileexists-probe.tex}{}{\\message{[missing]}}\n\
               \\end\n";
    let got = texrs::run_messages(src);
    let _ = std::fs::remove_file(there);
    let got = got.expect("run");
    assert!(got.contains("[yes]"), "the file that is there: {got}");
    assert!(got.contains("[no]"), "the file that is not: {got}");
    assert!(!got.contains("[missing]"), "the found arm inputs it: {got}");
    assert!(got.contains("[read]"), "and what it read ran: {got}");
}

/// A class that loads all the way through reports nothing.
///
/// `minimal.cls` is the shortest real class there is -- `\ProvidesClass`, two
/// `\setlength`, `\renewcommand\normalsize`, `\pagenumbering` and `\pagestyle`
/// -- and it is the first one this layer reads end to end. The claim under test
/// is the ABSENCE of the report: `src/latex/load.rs` names every package that
/// would not go through, so a silent load is the only evidence that one did.
#[test]
fn the_minimal_class_loads_end_to_end() {
    let request = texrs::latex::load::Request {
        name: "minimal".into(),
        extension: "cls",
        options: String::new(),
    };
    if texrs::latex::load::resolve("minimal", "cls").is_none() {
        eprintln!("skipping: kpsewhich cannot find minimal.cls");
        return;
    }
    assert!(
        matches!(
            texrs::latex::load::attempt(&request),
            texrs::latex::load::Outcome::Loaded(_)
        ),
        "minimal.cls must load all the way through"
    );
}

/// `\@ifundefined` answers the question rather than assuming an answer.
///
/// It used to be `\@secondoftwo` -- every name treated as DEFINED -- because
/// nothing could ask. `\ifcsname` can, so the port is the real one now, and
/// what rests on it is everything a package uses to decide whether to supply
/// something: `\@ifpackageloaded` is `\@ifundefined{ver@NAME.sty}`.
#[test]
fn ifundefined_answers_both_ways_and_a_loaded_package_is_known() {
    let src = "\\documentclass{article}\n\\makeatletter\n\
               \\def\\@known{}\n\
               \\@ifundefined{@known}{\\message{[wrong]}}{\\message{[known]}}\n\
               \\@ifundefined{@nothingdefinesthis}{\\message{[absent]}}{\\message{[wrong]}}\n\
               \\makeatother\n\\end\n";
    assert_eq!(out(src), "[known] [absent]");
}

/// `\AtBeginDocument` defers its argument to `\begin{document}` and runs it
/// there, which needs the append `\g@addto@macro` performs.
///
/// Both halves are under test: that the body does not run where it is
/// registered, and that TWO registrations both survive -- the append is where
/// every earlier shape of this lost the first one.
#[test]
fn at_begin_document_defers_its_body_and_keeps_every_one() {
    let src = "\\documentclass{article}\n\
               \\AtBeginDocument{\\message{[first]}}\n\
               \\AtBeginDocument{\\message{[second]}}\n\
               \\message{[preamble]}\n\
               \\begin{document}\\message{[body]}\\end{document}\n\\end\n";
    assert_eq!(out(src), "[preamble] [first] [second] [body]");
}

/// The path commands the prelude answers outside a picture.
///
/// A `tikzpicture` body is read raw and never expands, so these are reached
/// only where a document writes a path command somewhere else -- and there
/// `\shade`, `\shadedraw`, `\filldraw` and `\coordinate` were undefined
/// control sequences that stopped the document dead, while `\draw`, `\fill`,
/// `\node`, `\path` and `\clip` were answered. Each is delimited by its
/// semicolon, so it consumes the whole command and nothing after it.
#[test]
fn every_tikz_path_command_is_answered_and_stops_at_its_semicolon() {
    let commands = [
        r"\draw[thick] (0,0) -- (1,1);",
        r"\fill[red] (0,0) circle (1);",
        r"\filldraw[fill=blue] (0,0) rectangle (1,1);",
        r"\shade[left color=red,right color=blue] (0,0) rectangle (2,1);",
        r"\shadedraw[ball color=green] (0,0) circle (1);",
        r"\node[draw] at (0,0) {text};",
        r"\coordinate (a) at (1,2);",
        r"\path (0,0) -- (1,1);",
        r"\clip (0,0) rectangle (1,1);",
    ];
    for command in commands {
        let src = format!(
            "\\documentclass{{article}}\n\\begin{{document}}\n\
             {command}\\message{{[after]}}\n\\end{{document}}\n\\end\n"
        );
        assert_eq!(out(&src), "[after]", "{command}");
    }
}
