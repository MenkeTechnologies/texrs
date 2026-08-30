//! `\futurelet`, and the `\ifx` comparison it exists to feed.
//!
//! `tex.web` §1221: read three tokens, `\let` the first take the meaning of the
//! THIRD, then put the second and third back so the stream is untouched. That
//! non-destructive peek is the whole basis of LaTeX's `\@ifnextchar`, and so of
//! every optional argument in the language — `\newcommand{\x}[1]{...}` cannot
//! be written without it.
//!
//! Both behaviours here were checked against `tex` 3.141592653 rather than
//! against what seemed right.

fn out(src: &str) -> String {
    texrs::run_messages(src).expect("run")
}

const P: &str = "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6 \\catcode`\\[=12\n";

#[test]
fn the_peeked_token_is_left_in_the_stream() {
    // `tex` prints `ate:X` — the peek must not consume the X.
    let src = format!("{P}\\def\\eat#1{{\\message{{ate:#1}}}}\n\\futurelet\\n\\eat X\n\\end\n");
    assert_eq!(out(&src), "ate:X");
}

#[test]
fn a_control_sequence_let_to_a_character_compares_equal_to_it() {
    // `\ifx\next A` after peeking an A. tex says the meanings match; comparing
    // only cs-to-cs made this always false, which would make `\@ifnextchar`
    // answer "no" for every optional argument there is.
    let src = format!(
        "{P}\\def\\peek{{\\futurelet\\next\\show}}\n\
         \\def\\show{{\\ifx\\next A\\message{{sawA}}\\else\\message{{other}}\\fi}}\n\
         \\peek A\n\\end\n"
    );
    assert_eq!(out(&src), "sawA");
}

#[test]
fn the_peek_distinguishes_a_bracket_from_a_brace() {
    // The exact test `\@ifnextchar[` performs: is an optional argument coming?
    let yes = format!(
        "{P}\\let\\wc=[\n\\def\\chk{{\\ifx\\nx\\wc \\message{{opt}}\\else\\message{{none}}\\fi}}\n\
         \\futurelet\\nx\\chk[\n\\end\n"
    );
    assert_eq!(out(&yes), "opt");
    let no = format!(
        "{P}\\let\\wc=[\n\\def\\chk{{\\ifx\\nx\\wc \\message{{opt}}\\else\\message{{none}}\\fi}}\n\
         \\futurelet\\nx\\chk z\n\\end\n"
    );
    assert_eq!(out(&no), "none");
}

#[test]
fn futurelet_over_a_control_sequence_takes_its_meaning() {
    let src = format!(
        "{P}\\def\\target{{T}}\n\\def\\chk{{\\ifx\\nx\\target \\message{{same}}\\else\\message{{diff}}\\fi}}\n\
         \\futurelet\\nx\\chk\\target\n\\end\n"
    );
    assert_eq!(out(&src), "same");
}
