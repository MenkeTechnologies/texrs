//! The engine's own rules, one behaviour per test.
//!
//! The corpus in `tests/cases` holds the byte-level contract end to end; this
//! is the layer under it, so that a break says WHICH rule broke rather than
//! which document changed. A regression in delimited-parameter matching should
//! read as "a delimited parameter matches up to its delimiter", not as
//! "macros.tex diverges".
//!
//! Every expectation here was produced by running the snippet through real
//! `tex` while writing it — the same rule the corpus follows, since an
//! expectation typed from memory is a belief about TeX rather than a
//! measurement of it. The snippets are small enough to re-check by hand with
//! `cargo run --bin parity -- <file>`.

/// Every snippet runs under the same preamble, because INITEX starts with `{`
/// and `}` as ordinary characters and `#` as neither.
fn run(body: &str) -> String {
    let src = format!("\\catcode`\\{{=1 \\catcode`\\}}=2 \\catcode`\\#=6\n{body}\\end\n");
    texrs::run_messages(&src).unwrap_or_else(|e| panic!("{body}\n  stopped: {}", e.0))
}

/// The mouth's SkipBlanks state. `\a B` is `AB`, not `A B` -- this is why every macro-heavy document does not print stray spaces.
#[test]
fn a_control_word_swallows_the_space_after_it() {
    assert_eq!(run("\\def\\a{A}\n\\message{[\\a B]}\n"), "[AB]");
}

/// The asymmetry that makes the rule above worth stating: a control SYMBOL is one character and the space after it survives.
#[test]
fn a_control_symbol_keeps_it() {
    assert_eq!(run("\\message{[\\ B]}\n"), "[\\ B]");
}

/// The NewLine state: a blank line is `\par`, not two spaces. Nothing prints for it.
#[test]
fn a_blank_line_is_a_paragraph_not_text() {
    assert_eq!(run("\\message{[X]}\n\n\\message{[Y]}\n"), "[X] [Y]");
}

/// TeX's pattern matching. The argument is whatever precedes the delimiter, which is what `\def\pair#1,#2.` means.
#[test]
fn a_delimited_parameter_matches_up_to_its_delimiter() {
    assert_eq!(
        run("\\def\\pair#1,#2.{[#1|#2]}\n\\message{\\pair one,two.}\n"),
        "[one|two]"
    );
}

/// `##` in a body is one `#` in the definition it builds -- the mechanism every macro-defining macro rests on.
#[test]
fn a_doubled_hash_is_one_literal_parameter_character() {
    assert_eq!(
        run("\\def\\outer{\\def\\inner##1{[##1]}}\n\\outer\\message{\\inner{X}}\n"),
        "[X]"
    );
}

/// `\let` copies the meaning at the time it runs. Redefining the source afterwards leaves the alias alone.
#[test]
fn let_takes_the_current_meaning_not_a_reference() {
    assert_eq!(
        run("\\def\\v{ONE}\n\\let\\w=\\v\n\\def\\v{TWO}\n\\message{\\w\\v}\n"),
        "ONETWO"
    );
}

/// Two macros with the same parameter text and body are equal however they were named.
#[test]
fn ifx_compares_meanings_not_names() {
    assert_eq!(run("\\def\\a{X}\\def\\b{X}\\def\\c{Y}\n\\message{\\ifx\\a\\b S\\else D\\fi\\ifx\\a\\c S\\else D\\fi}\n"), "SD");
}

/// A macro can name another macro, which is how a package builds an interface it does not write out.
#[test]
fn csname_builds_a_name_from_characters() {
    assert_eq!(
        run("\\def\\greeting{HI}\\def\\n{greeting}\n\\message{\\csname \\n\\endcsname}\n"),
        "HI"
    );
}

/// An unknown name becomes `\relax` rather than raising -- and `\message` prints it by name.
#[test]
fn csname_of_an_unknown_name_is_relax_not_an_error() {
    assert_eq!(
        run("\\message{[\\csname nosuchthing\\endcsname]}\n"),
        "[\\nosuchthing ]"
    );
}

/// `\def` defers: the body reads the register when the macro is USED, so a later assignment moves it.
#[test]
fn the_reads_a_register_at_the_time_it_is_used() {
    assert_eq!(
        run("\\count1=1 \\def\\live{\\the\\count1 }\\count1=2\n\\message{\\live}\n"),
        "2"
    );
}

/// `\edef` does not: the value is taken now, which is the whole difference between the two.
#[test]
fn edef_freezes_the_read_at_definition_time() {
    assert_eq!(
        run("\\count1=1 \\edef\\frozen{\\the\\count1 }\\count1=2\n\\message{\\frozen}\n"),
        "1"
    );
}

/// A group scopes the macro table AND the registers written inside it -- the second half is the one a frontend can get wrong, because a register lives in a VM slot rather than in the frontend.
#[test]
fn a_group_restores_a_macro_and_a_register() {
    assert_eq!(
        run("\\def\\v{OUT}\\count1=1\n{\\def\\v{IN}\\count1=2 }\n\\message{\\v\\the\\count1 }\n"),
        "OUT1"
    );
}

/// `\gdef` opts out of that scoping.
#[test]
fn global_escapes_the_group() {
    assert_eq!(
        run("\\def\\v{OUT}\n{\\gdef\\v{IN}}\n\\message{\\v}\n"),
        "IN"
    );
}

/// TeX's `\divide` is integer division truncating toward zero (tex.web 1236), not floor division: -7/2 is -3, not -4.
#[test]
fn divide_truncates_toward_zero() {
    assert_eq!(
        run("\\count1=-7 \\divide\\count1 by 2\n\\message{\\the\\count1 }\n"),
        "-3"
    );
}

/// Parity, not remainder: -3 is odd, and a test written as `n mod 2 = 1` would say otherwise.
#[test]
fn ifodd_is_true_for_a_negative_odd_number() {
    assert_eq!(
        run("\\count1=-3\n\\message{\\ifodd\\count1 ODD\\else EVEN\\fi}\n"),
        "ODD"
    );
}

/// `\ifcase` is a switch with a default, and a value past the last `\or` takes it.
#[test]
fn ifcase_falls_to_else_past_the_last_or() {
    assert_eq!(
        run("\\count1=5\n\\message{\\ifcase\\count1 Z\\or O\\else M\\fi}\n"),
        "M"
    );
}

/// One token of lookahead: `\string` is held back while `\a` expands once, so `\string` sees `\b` rather than `\a`.
#[test]
fn expandafter_expands_the_token_after_the_next_one() {
    assert_eq!(
        run("\\def\\a{\\b}\\def\\b{DEEP}\n\\message{\\expandafter\\string\\a}\n"),
        "\\b"
    );
}

/// The inverse of `\csname`, escape character included.
#[test]
fn string_prints_a_control_sequence_with_its_escape() {
    assert_eq!(run("\\message{\\string\\foo}\n"), "\\foo");
}

/// `\number` prints the value, not the digits it was written with.
#[test]
fn number_drops_leading_zeros() {
    assert_eq!(run("\\message{\\number007}\n"), "7");
}
