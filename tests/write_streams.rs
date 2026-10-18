//! `\openout`, `\write` and `\closeout`, as LuaTeX writes the files.
//!
//! The document below runs through the `texrs` binary in a directory of its
//! own, and every file it writes is compared with the bytes luatex 1.24.0 wrote
//! for the same text (measured; see `src/write.rs`). When `luatex` is installed
//! the same document is also run through it, and the two directories must agree
//! file for file -- so a change to luatex's behaviour shows up here rather than
//! being frozen into the expected bytes.

use std::path::Path;
use std::process::Command;

/// Everything the four extensions decide about a file: the name (`.tex` added
/// only when there is no extension, braces keeping a space, `\jobname`
/// expanded in the name), the stream range (128 of them), how the text is
/// expanded and printed (`\the`, `\noexpand`, a doubled `#`, a control
/// character and a UTF-8 letter written raw), that `\openout` on an open stream
/// closes it first, and that a file written now is read back as it now stands.
const DOCUMENT: &str = r"\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\^=7
\count1=7 \def\x{XX}\def\space{ }
\immediate\openout3=foo
\immediate\write3{a \the\count1\space \x\relax\par # \number\count1 \romannumeral\count1}
\immediate\write3{^^Aé\noexpand\x \string\x}
\immediate\closeout3
\immediate\openout4={bar baz}
\immediate\write4{in bar}
\immediate\openout5=q.aux
\immediate\write5{aux}
\immediate\openout6=\jobname.out
\immediate\write6{j}
\immediate\write7{closed seven}
\immediate\write-1{neg}
\immediate\write200{big}
\immediate\openout100=hundred \immediate\write100{h}
\immediate\openout9=back \immediate\write9{\noexpand\message{READ BACK}}\immediate\closeout9
\input back
\immediate\openout9=back \immediate\write9{\noexpand\message{SECOND}}
\immediate\openout9=back2 \immediate\write9{two}\immediate\closeout9
\input back
\message{after}
\end
";

/// What luatex 1.24.0 wrote, file by file.
const WRITTEN: &[(&str, &str)] = &[
    ("foo.tex", "a 7XX\\relax \\par ## 7vii\n\u{1}é\\x \\x\n"),
    ("bar baz.tex", "in bar\n"),
    ("q.aux", "aux\n"),
    ("case.out", "j\n"),
    ("hundred.tex", "h\n"),
    ("back.tex", "\\message {SECOND}\n"),
    ("back2.tex", "two\n"),
];

fn run(program: &str, args: &[&str], dir: &Path) -> String {
    std::fs::write(dir.join("case.tex"), DOCUMENT).expect("write the document");
    let out = Command::new(program)
        .args(args)
        .arg("case.tex")
        .current_dir(dir)
        .output()
        .expect("run the engine");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn every_file_is_written_as_luatex_writes_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let shown = run(env!("CARGO_BIN_EXE_texrs"), &[], dir.path());
    for (name, bytes) in WRITTEN {
        let got = std::fs::read_to_string(dir.path().join(name))
            .unwrap_or_else(|_| panic!("{name} was not written"));
        assert_eq!(&got, bytes, "{name}");
    }
    // A file written and then `\input` is read as the lowering left it: the
    // first read sees what was written before it, the second the rewrite --
    // with `\openout` on the open stream 9 having closed `back.tex` first.
    assert!(shown.contains("READ BACK"), "{shown}");
    assert!(shown.contains("SECOND"), "{shown}");
    // A closed stream and one past the last go to the terminal; a negative
    // one to the log alone.
    assert!(shown.contains("closed seven"), "{shown}");
    assert!(shown.contains("big"), "{shown}");
    assert!(!shown.contains("neg"), "{shown}");
}

#[test]
fn the_files_agree_with_luatex_when_it_is_installed() {
    let installed = Command::new("luatex")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !installed {
        eprintln!("skipped: no luatex on PATH");
        return;
    }
    let ours = tempfile::tempdir().expect("tempdir");
    let theirs = tempfile::tempdir().expect("tempdir");
    run(env!("CARGO_BIN_EXE_texrs"), &[], ours.path());
    run("luatex", &["-interaction=nonstopmode"], theirs.path());
    for (name, _) in WRITTEN {
        let a = std::fs::read(theirs.path().join(name)).expect("luatex wrote it");
        let b = std::fs::read(ours.path().join(name)).expect("texrs wrote it");
        assert_eq!(a, b, "{name}");
    }
}
