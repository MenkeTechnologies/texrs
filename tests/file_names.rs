//! §526's `scan_file_name`, as web2c and LuaTeX read a name: expanded as it
//! is read, ended by a space or a non-character, and quoted with `"` when it
//! has a space in it. latex.ltx's first `\IfFileExists` is
//! `\openin\@inputcheck"#1" `, so a quote that is not understood leaves every
//! file it asks about missing.
//!
//! The answers are `tex -ini`'s and `luatex -ini`'s for the same input: both
//! print `OPEN OPEN OPEN` and then
//! `(./other.tex [x])`.

use std::process::Command;

#[test]
fn a_file_name_is_expanded_quoted_and_ended_as_tex_reads_it() {
    let dir = std::env::temp_dir().join(format!("texrs-file-names-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::write(dir.join("a b.tex"), "hello\n").expect("write");
    std::fs::write(dir.join("other.tex"), "\\message{[x]}\n").expect("write");
    std::fs::write(
        dir.join("names.tex"),
        concat!(
            "\\catcode`\\{=1 \\catcode`\\}=2\n",
            "\\def\\n{other}\n",
            // A quoted name, as latex.ltx writes one.
            "\\openin1\"other.tex\" \\ifeof1 \\message{EOF}\\else\\message{OPEN}\\fi\n",
            // A name built by a macro, which `get_x_token` expands.
            "\\openin2=\\n.tex \\ifeof2 \\message{EOF}\\else\\message{OPEN}\\fi\n",
            // A quoted name with a space in it.
            "\\openin3=\"a b\" \\ifeof3 \\message{EOF}\\else\\message{OPEN}\\fi\n",
            // `\relax` ends the name and is put back.
            "\\input \\n\\relax\n",
            "\\end\n",
        ),
    )
    .expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_texrs"))
        .args(["-ini", "-interaction=nonstopmode", "names.tex"])
        .current_dir(&dir)
        .output()
        .expect("run texrs");
    let out = String::from_utf8_lossy(&out.stdout);
    assert!(out.contains("OPEN OPEN OPEN"), "{out}");
    assert!(out.contains("[x]"), "{out}");
}
