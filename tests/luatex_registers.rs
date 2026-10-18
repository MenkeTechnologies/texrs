//! LuaTeX's register ranges, which a run has when it is a LuaTeX by its
//! program name (`-progname=luatex`, as `pdftex -progname=pdflatex` picks a
//! personality) and which a LaTeX document always has.
//!
//! The expectations were read off `luatex -ini` (LuaTeX 1.24.0, TeX Live
//! 2026) for the same input, so this needs no TeX installation to run.

use std::process::Command;

fn run(name: &str, args: &[&str], body: &str) -> String {
    let dir = std::env::temp_dir().join(format!("texrs-luatex-registers-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    // One file per test: the tests run side by side in one directory.
    let file = dir.join(format!("{name}.tex"));
    std::fs::write(&file, body).expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_texrs"))
        .args(args)
        .arg("-interaction=nonstopmode")
        .arg(&file)
        .current_dir(&dir)
        .output()
        .expect("run texrs");
    String::from_utf8_lossy(&out.stdout).to_string()
}

const BODY: &str = "\\catcode`\\{=1 \\catcode`\\}=2 \\countdef\\e=300 \\e=7 \\dimendef\\d=700 \
\\d=2pt \\skip999=1pt plus 1fil \\count65535=4 \\advance\\count65535 by \\e \
\\edef\\x{\\the\\e/\\the\\d/\\the\\skip999/\\the\\count65535/\\meaning\\e/\\meaning\\d}\
\\message{\\x}\\count65536=1 \\end\n";

#[test]
fn a_luatex_run_has_65536_registers_of_each_kind() {
    let out = run("ranges", &["-ini", "-progname=luatex"], BODY);
    // luatex: `(./ranges.tex 7/2.0pt/1.0pt plus 1.0fil/11/\count300/\dimen700`
    // and then `! Bad register code (65536).` for the one past its last.
    assert!(
        out.contains("7/2.0pt/1.0pt plus 1.0fil/11/\\count300/\\dimen700"),
        "{out}"
    );
    assert!(out.contains("! Bad register code (65536)."), "{out}");
    assert!(!out.contains("Bad register code (300)"), "{out}");
}

#[test]
fn a_tex_run_keeps_tex82_s_256() {
    // The same input under plain `-ini`, which is tex's: 300 is out of range.
    let out = run("tex82", &["-ini"], BODY);
    assert!(out.contains("! Bad register code (300)."), "{out}");
}

#[test]
fn a_register_past_255_keeps_its_value_through_a_group() {
    // tex.web 283's save stack does not care how a register is numbered:
    // `\count300` assigned inside a group is restored at its end, and a
    // `\global` one is not -- luatex prints `[5][9]`.
    let out = run(
        "grouped",
        &["-ini", "-progname=luatex"],
        "\\catcode`\\{=1 \\catcode`\\}=2 \\count300=5 \\count301=1 \
         {\\count300=6 \\global\\count301=9 }\\message{[\\the\\count300][\\the\\count301]}\\end\n",
    );
    assert!(out.contains("[5][9]"), "{out}");
}

#[test]
fn attribute_registers_start_unset_and_are_scoped_like_counts() {
    // luatex: `[-2147483647][-2147483647] [3][\attribute5][3] [9] [3][4] [5]`,
    // then `[-2147483647]` for an explicitly unset one, then
    // `! Bad register code (65536).` and `five`.
    let out = run(
        "attributes",
        &["-ini", "-progname=luatex"],
        concat!(
            "\\catcode`\\{=1 \\catcode`\\}=2\n",
            "\\message{[\\the\\attribute0][\\number\\attribute7]}\n",
            "\\attributedef\\a=5 \\a=3 \\message{[\\the\\a][\\meaning\\a][\\the\\attribute5]}\n",
            "{\\a=9 \\global\\attribute6=4 \\message{[\\the\\a]}}\\message{[\\the\\a][\\the\\attribute6]}\n",
            "\\advance\\a by 2 \\message{[\\the\\a]}\n",
            "\\attribute8=-2147483647 \\message{[\\the\\attribute8]}\n",
            "\\attribute65535=1 \\attribute65536=1\n",
            "\\ifnum\\a=5 \\message{five}\\fi\n",
            "\\end\n",
        ),
    );
    assert!(
        out.contains("[-2147483647][-2147483647] [3][\\attribute5][3] [9] [3][4] [5] [-2147483647]"),
        "{out}"
    );
    assert!(out.contains("! Bad register code (65536)."), "{out}");
    assert!(out.contains("five"), "{out}");
}
