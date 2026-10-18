//! What etex.src -- the file `luatex.ini` builds the plain LuaTeX format
//! from -- needs of the mouth and the gullet: `\everyeof` at the end of a real
//! file, `\readline`, `\the` inside `\csname`, and LuaTeX's expandable Lua
//! primitives wherever the gullet reads, not only at the top of a file.
//!
//! The expectations were read off `luatex -ini` (LuaTeX 1.24.0, TeX Live
//! 2026) for the same input, so this needs no TeX installation to run.

use std::process::Command;

fn run(name: &str, files: &[(&str, &str)]) -> String {
    let dir = std::env::temp_dir().join(format!("texrs-luatex-gullet-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    for (file, body) in files {
        std::fs::write(dir.join(file), body).expect("write");
    }
    let out = Command::new(env!("CARGO_BIN_EXE_texrs"))
        .args(["-ini", "-progname=luatex", "-interaction=nonstopmode", "main.tex"])
        .current_dir(&dir)
        .output()
        .expect("run texrs");
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn everyeof_is_read_when_a_file_ends_and_ends_a_skip() {
    // etex.src's `\patterns`: open a group, set `\everyeof` to close it, and
    // skip the rest of the file with `\iffalse`. luatex prints
    // `(./sub.tex [skip]) [after][] (./sub2.tex [in2] [eof])`.
    let out = run(
        "eof",
        &[
            ("sub.tex", "A\\skipit\nB\n\\patterns{x}\nC\n"),
            ("sub2.tex", "\\message{[in2]}\n"),
            (
                "main.tex",
                concat!(
                    "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6\n",
                    "\\def\\skipit{\\begingroup\\everyeof={\\fi\\endgroup}\\message{[skip]}\\iffalse}\n",
                    "\\input sub\n",
                    "\\message{[after][\\the\\everyeof]}\n",
                    "\\everyeof={\\message{[eof]}}\\input sub2\n",
                    "\\end\n",
                ),
            ),
        ],
    );
    assert!(
        out.contains("(./sub.tex [skip]) [after][] (./sub2.tex [in2] [eof])"),
        "{out}"
    );
}

#[test]
fn readline_csname_and_lua_in_the_gullet() {
    // luatex: `[macro:->a {b} c] [N] [41] [macro:->z]`.
    let out = run(
        "gullet",
        &[
            ("rl.txt", "a {b} c\n"),
            (
                "main.tex",
                concat!(
                    "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6\n",
                    "\\openin1=rl.txt \\readline1 to \\x \\message{[\\meaning\\x]}\n",
                    "\\count2=7 \\expandafter\\def\\csname n@\\the\\count2\\endcsname{N}",
                    "\\message{[\\csname n@7\\endcsname]}\n",
                    "\\count3=\\directlua{tex.sprint(\"41\")}\\relax \\message{[\\the\\count3]}\n",
                    "\\edef\\y{\\directlua{tex.sprint(\"z\")}}\\message{[\\meaning\\y]}\n",
                    "\\end\n",
                ),
            ),
        ],
    );
    assert!(out.contains("[macro:->a {b} c] [N] [41] [macro:->z]"), "{out}");
}
