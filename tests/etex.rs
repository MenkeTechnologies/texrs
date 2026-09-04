//! The eTeX expression primitives, compared against LuaTeX.
//!
//! These are not in `tex` 3.141592653, so the parity corpus cannot hold them:
//! its oracle would report every one as an undefined control sequence. The
//! oracle here is `luatex`, which carries them — with one asymmetry worth
//! stating, because it looks like cheating otherwise: `luatex -ini` does not
//! enable its extra primitives until asked, so the oracle's document opens with
//! `\directlua{tex.enableprimitives...}` and texrs's does not. Everything after
//! that line is identical text.
//!
//! Skipped, loudly, when no `luatex` is installed.

use std::process::Command;

/// `luatex`, if it is installed.
fn luatex() -> Option<String> {
    let out = Command::new("luatex").arg("--version").output().ok()?;
    out.status.success().then(|| "luatex".to_string())
}

/// Run `body` through both engines and return `(luatex, texrs)` output.
fn both(lua: &str, body: &str) -> (String, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let cats = "\\catcode`\\{=1 \\catcode`\\}=2\n";
    std::fs::write(
        dir.path().join("oracle.tex"),
        format!(
            "{cats}\\directlua{{tex.enableprimitives('', tex.extraprimitives())}}\n{body}\n\\end\n"
        ),
    )
    .expect("write");
    std::fs::write(
        dir.path().join("case.tex"),
        format!("{cats}{body}\n\\end\n"),
    )
    .expect("write");

    let bracketed = |out: Vec<u8>| {
        let text = String::from_utf8_lossy(&out).into_owned();
        // The engines' banners differ and are not the subject; what both write
        // is the bracketed message.
        match (text.find('['), text.rfind(']')) {
            (Some(a), Some(b)) if b > a => text[a..=b].to_string(),
            _ => String::new(),
        }
    };
    let reference = Command::new(lua)
        .args(["-ini", "-interaction=nonstopmode", "oracle.tex"])
        .env("max_print_line", "8000")
        .current_dir(dir.path())
        .output()
        .expect("run luatex");
    let subject = Command::new(env!("CARGO_BIN_EXE_texrs"))
        .arg("case.tex")
        .current_dir(dir.path())
        .output()
        .expect("run texrs");
    (bracketed(reference.stdout), bracketed(subject.stdout))
}

#[test]
fn numexpr_follows_precedence_and_parentheses() {
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    for body in [
        "\\count0=\\numexpr 2+3*4\\relax \\message{[\\the\\count0]}",
        "\\count0=\\numexpr (2+3)*4\\relax \\message{[\\the\\count0]}",
        "\\count0=\\numexpr 1+2-3\\relax \\message{[\\the\\count0]}",
    ] {
        let (want, got) = both(&lua, body);
        assert_eq!(got, want, "{body}");
    }
}

#[test]
fn numexpr_division_rounds_half_away_from_zero() {
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    // The rule that separates \numexpr from \divide: 7/2 is 4 here and 3
    // there. Getting this wrong is silent -- both answers look plausible --
    // so every sign and both roundings are pinned.
    for body in [
        "\\count0=\\numexpr 7/2\\relax \\message{[\\the\\count0]}",
        "\\count0=\\numexpr -7/2\\relax \\message{[\\the\\count0]}",
        "\\count0=\\numexpr 5/2\\relax \\message{[\\the\\count0]}",
        "\\count0=\\numexpr 2*3/4\\relax \\message{[\\the\\count0]}",
    ] {
        let (want, got) = both(&lua, body);
        assert_eq!(got, want, "{body}");
    }
}

#[test]
fn dimexpr_computes_in_scaled_points() {
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    for body in [
        "\\dimen0=\\dimexpr 1pt+2pt\\relax \\message{[\\the\\dimen0]}",
        "\\dimen0=\\dimexpr 1pt*3\\relax \\message{[\\the\\dimen0]}",
        "\\dimen0=\\dimexpr 1in/2\\relax \\message{[\\the\\dimen0]}",
    ] {
        let (want, got) = both(&lua, body);
        assert_eq!(got, want, "{body}");
    }
}

#[test]
fn divide_still_truncates() {
    // Not a differential test: the point is that the two operations differ,
    // and `tex` is the authority for \divide -- it is pinned in the parity
    // corpus. Here it is enough that texrs does not quietly use one rule for
    // both.
    let src = "\\catcode`\\{=1 \\catcode`\\}=2\n\\count1=7 \\divide\\count1 by 2\n\\count2=\\numexpr 7/2\\relax\n\\message{[\\the\\count1][\\the\\count2]}\n";
    let got = texrs::run_messages(src).expect("run");
    assert_eq!(got, "[3][4]", "\\divide truncates, \\numexpr rounds");
}

#[test]
fn unless_runs_the_other_arm() {
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    // Both directions, because swapping the arms of a conditional is right
    // only if it is the arms that swap and not the test.
    for body in [
        "\\message{[\\unless\\ifnum 1>2 A\\else B\\fi]}",
        "\\message{[\\unless\\ifnum 3>2 A\\else B\\fi]}",
        "\\message{[\\ifnum 1>2 A\\else B\\fi]}",
    ] {
        let (want, got) = both(&lua, body);
        assert_eq!(got, want, "{body}");
    }
}

#[test]
fn csstring_and_uchar_and_detokenize() {
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    for body in [
        "\\def\\f{F}\\message{[\\csstring\\f][\\string\\f]}",
        "\\message{[\\Uchar65][\\Uchar97]}",
        "\\message{[\\detokenize{\\a b}]}",
    ] {
        let (want, got) = both(&lua, body);
        assert_eq!(got, want, "{body}");
    }
}

#[test]
fn a_protected_macro_is_not_frozen_by_edef() {
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    // The observable difference without \meaning: redefine the macro after
    // the \edef. A protected one is called afresh and yields the new body.
    let body = "\\protected\\def\\p{P}\\edef\\e{\\p}\\def\\p{Z}\\message{[\\e]}";
    let (want, got) = both(&lua, body);
    assert_eq!(got, want, "a protected macro survives \\edef as itself");
}

#[test]
fn expanded_and_unexpanded_are_opposites() {
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    // The pair only parts company inside an \edef: there \unexpanded's group
    // survives as tokens and is called later, so redefining afterwards changes
    // the answer, while \expanded's is expanded now and frozen.
    for body in [
        "\\def\\q{Q}\\message{[\\expanded{\\q}]}",
        "\\def\\a{A}\\def\\b{\\a}\\message{[\\expanded{\\b}]}",
        "\\def\\q{Q}\\message{[\\unexpanded{\\q}]}",
        "\\def\\q{Q}\\edef\\e{\\unexpanded{\\q}}\\def\\q{Z}\\message{[\\e]}",
        "\\def\\q{Q}\\edef\\e{\\expanded{\\q}}\\def\\q{Z}\\message{[\\e]}",
    ] {
        let (want, got) = both(&lua, body);
        assert_eq!(got, want, "{body}");
    }
}

#[test]
fn begincsname_does_not_define_what_it_does_not_find() {
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    let body =
        "\\def\\foo{F}\\message{[\\begincsname foo\\endcsname][\\begincsname nope\\endcsname]}";
    let (want, got) = both(&lua, body);
    assert_eq!(
        got, want,
        "an unknown name expands to nothing, not to \\relax"
    );
}

#[test]
fn glueexpr_combines_components_by_order() {
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    // The order rule is the part that is not arithmetic: an infinite component
    // beats a finite one however large, a higher infinity beats a lower, and
    // only equal orders add. All three are here because getting one right by
    // accident is easy.
    for body in [
        "\\skip0=\\glueexpr 1pt plus 2pt+3pt plus 4pt\\relax \\message{[\\the\\skip0]}",
        "\\skip0=\\glueexpr 5pt plus 6pt-2pt plus 1pt\\relax \\message{[\\the\\skip0]}",
        "\\skip0=\\glueexpr 2pt plus 1fil*3\\relax \\message{[\\the\\skip0]}",
        "\\skip0=\\glueexpr 6pt plus 6pt/2\\relax \\message{[\\the\\skip0]}",
        "\\skip0=\\glueexpr 1pt plus 2pt+3pt plus 4fil\\relax \\message{[\\the\\skip0]}",
        "\\skip0=\\glueexpr 1pt plus 2fill+3pt plus 4fil\\relax \\message{[\\the\\skip0]}",
    ] {
        let (want, got) = both(&lua, body);
        assert_eq!(got, want, "{body}");
    }
}

#[test]
fn ifcsname_asks_without_answering() {
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    // The whole primitive is in the third bracket. `\csname` DEFINES a name it
    // does not find (`tex.web` §372 makes it `\relax`), so asking with it
    // changes the answer for every later ask; etex.ch's `if_cs_code` looks the
    // name up with `no_new_control_sequence` still true and leaves the hash
    // table alone, so the second ask about `\nope` must still say NO.
    let body = "\\def\\foo{F}\
        \\ifcsname foo\\endcsname\\message{[YES]}\\else\\message{[NO]}\\fi\
        \\ifcsname nope\\endcsname\\message{[YES]}\\else\\message{[NO]}\\fi\
        \\ifcsname nope\\endcsname\\message{[YES]}\\else\\message{[NO]}\\fi";
    let (want, got) = both(&lua, body);
    assert_eq!(
        got, want,
        "\\ifcsname must not define what it does not find"
    );
}

#[test]
fn ifcsname_and_ifdefined_decide_inside_a_message() {
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    // §1279 expands a `\message` body, so a conditional in one is decided
    // rather than printed. Both of these read the macro table, which is a
    // frontend fact, so neither becomes a run-time branch.
    let body = "\\def\\foo{F}\
        \\message{[\\ifdefined\\foo YES\\else NO\\fi][\\ifdefined\\bar YES\\else NO\\fi]}\
        \\message{[\\ifcsname foo\\endcsname YES\\else NO\\fi]\
        [\\ifcsname bar\\endcsname YES\\else NO\\fi]}";
    let (want, got) = both(&lua, body);
    assert_eq!(got, want);
}

#[test]
fn a_csname_that_found_nothing_compares_equal_to_relax() {
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    // LaTeX's older `\@ifundefined` is this line and nothing else, and it is
    // the reason `\ifx` has to see a `\csname`-made `\relax` and the primitive
    // `\relax` as the same command. `\let\a=\relax` is the same comparison
    // reached the other way.
    let body = "\\def\\foo{F}\\let\\a=\\relax\
        \\expandafter\\ifx\\csname nope\\endcsname\\relax\\message{[UNDEF]}\\else\\message{[DEF]}\\fi\
        \\expandafter\\ifx\\csname foo\\endcsname\\relax\\message{[UNDEF]}\\else\\message{[DEF]}\\fi\
        \\ifx\\a\\relax\\message{[LET]}\\else\\message{[NOTLET]}\\fi";
    let (want, got) = both(&lua, body);
    assert_eq!(got, want);
}

#[test]
fn muskip_is_a_glue_register_measured_in_mu() {
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    // Everything a `\skip` does, in the other unit: the components print with
    // `mu` after them, an infinite one is still `fil`, a copy carries all four,
    // `\advance` combines by order and `\multiply` scales every component. The
    // `\muskipdef` name has to work in the same positions the spelt-out form
    // does, and a register nothing has written is `0.0mu` rather than `0.0pt`.
    let body = "\\muskip0=3mu plus 1mu minus 2mu\\message{[\\the\\muskip0]}\
        \\muskip1=1mu plus 2fil\\message{[\\the\\muskip1]}\
        \\muskip2=\\muskip0 \\advance\\muskip2 by 1mu plus 1mu\\message{[\\the\\muskip2]}\
        \\multiply\\muskip2 by 2\\message{[\\the\\muskip2]}\
        \\muskipdef\\mymu=3 \\mymu=7mu\\message{[\\the\\mymu][\\the\\muskip3]}\
        \\message{[\\the\\muskip9]}";
    let (want, got) = both(&lua, body);
    assert_eq!(got, want);
}

#[test]
fn a_muskip_is_restored_by_the_group_that_set_it() {
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    // A math glue lives in the same slot file every other register does, so it
    // is saved and restored by the same machinery. Worth its own test because
    // it is four slots, and restoring three of four would look right in every
    // probe that only reads the natural component back.
    let body = "\\muskip0=3mu plus 1mu\
        {\\muskip0=99mu minus 4mu \\message{[\\the\\muskip0]}}\\message{[\\the\\muskip0]}";
    let (want, got) = both(&lua, body);
    assert_eq!(got, want);
}

#[test]
fn muexpr_is_glueexpr_in_the_other_unit() {
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    let body = "\\muskip0=\\muexpr 3mu plus 1mu + 2mu plus 2mu\\relax\\message{[\\the\\muskip0]}\
        \\muskip1=\\muexpr 2mu plus 1fil*3\\relax\\message{[\\the\\muskip1]}\
        \\muskip2=\\muexpr 6mu plus 6mu/2\\relax\\message{[\\the\\muskip2]}";
    let (want, got) = both(&lua, body);
    assert_eq!(got, want);
}

#[test]
fn latexs_ifundefined_answers_and_keeps_answering() {
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    // LaTeX's `\@ifundefined`, spelt without the `@` because that character is
    // catcode 12 in INITEX and both engines would read `\@` as its own control
    // sequence. This is the whole reason `\ifcsname` is worth having: the test
    // must not DEFINE the name it asks about, or the second ask answers
    // differently from the first. The three `\expandafter`s are the other half
    // -- they expand the `\fi` and the `\else` away so the two-argument macro
    // takes the arguments that follow the conditional.
    let body = "\\catcode`\\#=6 \\def\\firstoftwo#1#2{#1}\\def\\secondoftwo#1#2{#2}\
        \\def\\ifundef#1{\\ifcsname #1\\endcsname\
        \\expandafter\\ifx\\csname #1\\endcsname\\relax\
        \\expandafter\\expandafter\\expandafter\\firstoftwo\
        \\else\\expandafter\\expandafter\\expandafter\\secondoftwo\\fi\
        \\else\\expandafter\\firstoftwo\\fi}\
        \\def\\known{K}\
        \\ifundef{known}{\\message{[UNDEF]}}{\\message{[DEF]}}\
        \\ifundef{missing}{\\message{[UNDEF]}}{\\message{[DEF]}}\
        \\ifundef{missing}{\\message{[UNDEF2]}}{\\message{[DEF2]}}";
    let (want, got) = both(&lua, body);
    assert_eq!(got, want);
}

#[test]
fn unless_negates_whichever_engine_decides_the_conditional() {
    let Some(lua) = luatex() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    // `\unless` is one flag spent by the conditional that follows, and the
    // conditionals are settled in two different places here: `\ifnum` becomes
    // a run-time branch the lowerer builds, `\ifdefined` and `\ifcsname` are
    // decided by the expander. A flag either of them could leave behind would
    // negate the NEXT conditional instead, which is why all three are in one
    // document rather than three.
    let body = "\\def\\k{K}\
        \\unless\\ifdefined\\k \\message{[A-NO]}\\else\\message{[A-YES]}\\fi\
        \\unless\\ifcsname k\\endcsname \\message{[B-NO]}\\else\\message{[B-YES]}\\fi\
        \\unless\\ifcsname zz\\endcsname \\message{[C-NO]}\\else\\message{[C-YES]}\\fi\
        \\count1=5 \\unless\\ifnum\\count1>3 \\message{[D-NO]}\\else\\message{[D-YES]}\\fi";
    let (want, got) = both(&lua, body);
    assert_eq!(got, want);
}
