//! Expandable primitives at the top level of a document.
//!
//! `tex.web` §366 has the expander handle `\expandafter`, `\csname` and the
//! rest wherever they occur. The lowerer dispatches top-level control sequences
//! itself, so it had no arm for them: they worked inside a macro body and were
//! "undefined control sequence" at the outermost level of a file. That is not a
//! corner -- `\expandafter\def\csname NAME\endcsname` is the idiom LaTeX's own
//! `\newcommand` is built out of, so nothing in that family could be written.
//!
//! `\let` had the neighbouring problem: it records that the alias MEANS a
//! primitive, and top-level dispatch matches on the primitive's name, so the
//! alias read as undefined while the thing it aliased worked.

fn out(src: &str) -> String {
    texrs::run_messages(src).expect("run")
}

const PREAMBLE: &str = "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6\n";

#[test]
fn expandafter_and_csname_define_a_macro_at_the_top_level() {
    // The exact shape \newcommand expands to.
    let src = format!(
        "{PREAMBLE}\\expandafter\\def\\csname foo\\endcsname{{OK}}\n\\message{{\\csname foo\\endcsname}}\n\\end\n"
    );
    assert_eq!(out(&src), "OK");
}

#[test]
fn a_csname_built_macro_takes_its_arguments() {
    let src = format!(
        "{PREAMBLE}\\expandafter\\def\\csname wrap\\endcsname#1{{[#1]}}\n\\message{{\\csname wrap\\endcsname{{x}}}}\n\\end\n"
    );
    assert_eq!(out(&src), "[x]");
}

#[test]
fn a_let_alias_is_callable_where_the_primitive_is() {
    let src = format!("{PREAMBLE}\\let\\g=\\message\n\\g{{via let}}\n\\end\n");
    assert_eq!(out(&src), "via let");
}

#[test]
fn expandafter_still_works_inside_a_macro_body() {
    // The fix passes the expansion context through rather than forcing one, so
    // the case that always worked has to keep working.
    let src = format!(
        "{PREAMBLE}\\def\\inner{{IN}}\n\\def\\outer{{\\expandafter\\message\\expandafter{{\\inner}}}}\n\\outer\n\\end\n"
    );
    assert_eq!(out(&src), "IN");
}

#[test]
fn an_undefined_control_sequence_is_still_undefined() {
    // Routing through the expander must not turn every unknown name into a
    // silent no-op -- that would hide real errors in a document.
    let src = format!("{PREAMBLE}\\notaprimitive\n\\end\n");
    let err = texrs::compile(&src).expect_err("must still fail");
    assert!(
        err.0.contains("Undefined control sequence"),
        "got {:?}",
        err.0
    );
}
