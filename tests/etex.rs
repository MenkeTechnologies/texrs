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
