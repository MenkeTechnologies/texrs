//! LuaTeX's category codes past 255: every Unicode character has one.
//!
//! The expectations were read off `luatex -ini` (LuaTeX 1.24.0, TeX Live
//! 2026) for the same input, with every extra primitive enabled, so this
//! needs no TeX installation to run. The catcode tables themselves are
//! `tests/luatex_cases/catcode_tables.tex`'s.

use std::process::Command;

fn run(name: &str, body: &str) -> String {
    let dir = std::env::temp_dir().join(format!("texrs-luatex-catcodes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join(format!("{name}.tex"));
    std::fs::write(&file, body).expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_texrs"))
        .args(["-ini", "-progname=luatex", "-interaction=nonstopmode"])
        .arg(&file)
        .current_dir(&dir)
        .output()
        .expect("run texrs");
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn a_character_past_255_has_a_category_code_of_its_own() {
    // luatex: `[12][12] [11]`, then `! Bad character code (1114112).` -- the
    // first character past Unicode's last.
    let out = run(
        "codes",
        concat!(
            "\\catcode`\\{=1 \\catcode`\\}=2\n",
            "\\message{[\\the\\catcode\"0410][\\the\\catcode\"10FFFF]}\n",
            "\\catcode\"0410=11 \\message{[\\the\\catcode\"0410]}\n",
            "\\catcode\"110000=11\n",
            "\\end\n",
        ),
    );
    assert!(out.contains("[12][12] [11]"), "{out}");
    assert!(out.contains("! Bad character code (1114112)."), "{out}");
}

#[test]
fn a_letter_past_255_lengthens_a_control_word() {
    // luatex prints `[ok]`: `\Жx` is one control sequence, defined, because
    // `Ж` is a letter here.
    let out = run(
        "wide",
        "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\Ж=11 \\def\\Жx{ok}\\message{[\\Жx]}\\end\n",
    );
    assert!(out.contains("[ok]"), "{out}");
}
