//! The driver's own contract: what the flags do, and that the three places a
//! user meets them agree.
//!
//! The binary, the zsh completion and the man page each list the options. They
//! drift silently — a flag added to one and not the others is a flag nobody
//! discovers — so they are compared here rather than by remembering to.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Flags that are arguments to a `-X` command, not to a run of a file.
const DOCUMENT_FLAGS: &[&str] = &["--profile", "--interval"];

fn texrs() -> Command {
    Command::new(env!("CARGO_BIN_EXE_texrs"))
}

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// A cache of this test's own, so a run never reads or writes the one the
/// user's own texrs runs are using.
fn scratch_cache(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("texrs_cli_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn stdout_of(cmd: &mut Command) -> String {
    let out = cmd.output().expect("run texrs");
    assert!(
        out.status.success(),
        "texrs failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Every option the binary accepts, read from its own usage text — the list the
/// other two are held against.
///
/// An option that takes a value is written `--jobs=N` in the usage line, and the
/// two places it has to appear spell that differently: roff wants the value as
/// its own argument (`.BI \-\-jobs= N`) and a zsh completion writes the flag
/// with the value's spec after it (`--jobs=[…]:count:`). Comparing on the flag
/// NAME — everything up to the `=` — is what makes the gate ask the question it
/// means to ask ("is this flag documented?") rather than "is the usage line's
/// exact spelling present?". An undocumented flag still fails, which is the
/// point of the gate.
fn options_from_usage() -> Vec<String> {
    let usage = stdout_of(texrs().arg("--help"));
    let mut out: Vec<String> = Vec::new();
    for line in usage.lines() {
        for word in line.split_whitespace() {
            let word = word.trim_end_matches(',');
            // `--jobs=N` is the flag `--jobs`; the `N` is its value.
            let word = word.split('=').next().unwrap_or(word);
            // Both spellings count. tex's own options take one dash
            // (`-interaction`, `-jobname`), texrs's take two, and a user types
            // whichever the usage text showed them -- so both have to be
            // documented and completed.
            let is_long = word.starts_with("--") && word.len() > 2;
            let is_tex_style = word.starts_with('-')
                && !word.starts_with("--")
                && word.len() > 2
                && word[1..]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-');
            if is_long || is_tex_style {
                out.push(word.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn the_completion_offers_every_option_the_binary_takes() {
    let completion = std::fs::read_to_string(repo("completions/_texrs")).expect("completions");
    for opt in options_from_usage() {
        // A flag is offered either on its own (`--disasm[…]`) or inside a brace
        // group with its short form (`{-h,--help}'[…]`), so what is looked for
        // is the flag followed by anything that cannot continue it.
        let offered = completion.match_indices(&opt).any(|(at, _)| {
            completion[at + opt.len()..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_ascii_alphanumeric() && c != '-')
        });
        assert!(offered, "the zsh completion does not offer {opt}");
    }
    // And offers nothing the binary would refuse. The flags in
    // DOCUMENT_FLAGS belong to the `-X` commands rather than to a plain run,
    // so they are checked there instead — see
    // a_document_is_made_and_built_from_anywhere_inside_it.
    for line in completion.lines() {
        let Some(start) = line.find("'--") else {
            continue;
        };
        let rest = &line[start + 1..];
        let flag: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        // A flag written `--name=` in the completion takes its value inline
        // and cannot be run bare; one in DOCUMENT_FLAGS belongs to a `-X`
        // command and is checked there instead.
        let takes_inline_value = rest[flag.len()..].starts_with('=');
        if takes_inline_value || DOCUMENT_FLAGS.contains(&flag.as_str()) {
            continue;
        }
        let out = texrs().arg(&flag).arg("--help").output().expect("run");
        assert!(
            out.status.success(),
            "the completion offers {flag}, which the binary refuses"
        );
    }
}

/// The `-X` commands, read from the usage text the same way the options are.
/// `-X` is two characters, so `options_from_usage` skips it and its commands
/// went uncompleted and half-documented until this asked.
fn subcommands_from_usage() -> Vec<String> {
    let usage = stdout_of(texrs().arg("--help"));
    let mut out: Vec<String> = Vec::new();
    for line in usage.lines() {
        let Some(at) = line.find("-X ") else { continue };
        let name: String = line[at + 3..]
            .chars()
            .take_while(|c| c.is_ascii_lowercase())
            .collect();
        if !name.is_empty() {
            out.push(name);
        }
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn the_completion_and_the_man_page_know_every_x_command() {
    let completion = std::fs::read_to_string(repo("completions/_texrs")).expect("completions");
    let man = std::fs::read_to_string(repo("man/man1/texrs.1")).expect("man page");
    let plain = man.replace("\\-", "-");

    // The names the completion offers after `-X`, from its own value list.
    let at = completion.find("'-X[").expect("the completion offers -X");
    let list = completion[at..]
        .split(":command:(")
        .nth(1)
        .expect("a value list");
    let offered: Vec<&str> = list[..list.find(')').expect("closed")]
        .split_whitespace()
        .collect();

    let commands = subcommands_from_usage();
    // The guard is worth nothing if the usage text stopped listing commands.
    assert!(
        commands.len() >= 8,
        "the usage text lists only {commands:?}"
    );
    for name in &commands {
        assert!(
            offered.contains(&name.as_str()),
            "the zsh completion does not offer -X {name}, only {offered:?}"
        );
        assert!(
            plain.contains(&format!("-X {name}")) || plain.contains(&format!("-X \" {name}")),
            "the man page does not document -X {name}"
        );
    }
    // And nothing is offered that the usage text does not list, which is the
    // drift that leaves a completion promising a command that was renamed.
    for name in &offered {
        assert!(
            commands.iter().any(|c| c == name),
            "the completion offers -X {name}, which the binary does not list"
        );
    }

    // Each one is a command the binary knows. Run from a scratch directory
    // because two of them write there. `watch` is left out on purpose: it is
    // the one that does not return.
    let dir = scratch_cache("x_commands");
    for name in commands.iter().filter(|c| *c != "watch") {
        let said = String::from_utf8_lossy(
            &texrs()
                .current_dir(&dir)
                .arg("-X")
                .arg(name)
                .output()
                .expect("run")
                .stderr,
        )
        .to_string();
        assert!(
            !said.contains("unknown document command"),
            "-X {name} is listed but the binary does not know it"
        );
    }
}

#[test]
fn the_man_page_documents_every_option_the_binary_takes() {
    let man = std::fs::read_to_string(repo("man/man1/texrs.1")).expect("man page");
    // Options are escaped in roff (`\-\-disasm`), so the hyphens are unescaped
    // before looking for them.
    let plain = man.replace("\\-", "-");
    for opt in options_from_usage() {
        assert!(plain.contains(&opt), "the man page does not document {opt}");
    }
}

#[test]
fn a_document_runs_the_same_with_the_cache_and_without_it() {
    let cache = scratch_cache("same");
    let doc = cache.join("doc.tex");
    std::fs::write(
        &doc,
        "\\catcode`\\{=1 \\catcode`\\}=2 \\message{FROM-THE-CACHE}\n\\end\n",
    )
    .unwrap();

    let cold = stdout_of(
        texrs()
            .env("XDG_CACHE_HOME", &cache)
            .env("HOME", &cache)
            .arg(&doc),
    );
    assert!(cold.contains("FROM-THE-CACHE"), "{cold}");

    // The second run reads the cache; the third is told not to. All three print
    // the same thing, which is the only promise a cache may make.
    let warm = stdout_of(
        texrs()
            .env("XDG_CACHE_HOME", &cache)
            .env("HOME", &cache)
            .arg(&doc),
    );
    let uncached = stdout_of(
        texrs()
            .env("XDG_CACHE_HOME", &cache)
            .env("HOME", &cache)
            .arg("--no-cache")
            .arg(&doc),
    );
    assert_eq!(cold, warm, "a cached run prints what the cold run printed");
    assert_eq!(cold, uncached, "and so does one that skips the cache");
    let _ = std::fs::remove_dir_all(&cache);
}

#[test]
fn the_cache_can_be_inspected_and_cleared_from_the_command_line() {
    let cache = scratch_cache("stats");
    let doc = cache.join("doc.tex");
    std::fs::write(
        &doc,
        "\\catcode`\\{=1 \\catcode`\\}=2 \\message{X}\n\\end\n",
    )
    .unwrap();
    let env = |cmd: &mut Command| -> String {
        stdout_of(cmd.env("XDG_CACHE_HOME", &cache).env("HOME", &cache))
    };

    // Nothing has run, so the cache holds nothing but still says where it is.
    let before = env(texrs().arg("--cache-stats"));
    assert!(before.contains("documents: 0"), "{before}");
    assert!(before.contains("scripts.rkyv"), "{before}");

    // A listing compiles and caches what it compiled. The bare invocation
    // typesets now, and a typesetting run does not use the shard: it needs the
    // chunk that carries the document's text, and the fonts, page colour and
    // layout that go with it are read while lowering and are not in the cache.
    // Measured: --disasm and --dvi fill the shard, the bare form does not. See
    // BUGS.md.
    env(texrs().arg("--disasm").arg(&doc));
    let after = env(texrs().arg("--cache-stats"));
    assert!(
        after.contains("documents: 1"),
        "the run was cached: {after}"
    );

    // Clearing empties it, and the next run fills it again.
    let cleared = env(texrs().arg("--cache-clear"));
    assert!(cleared.contains("cleared"), "{cleared}");
    let empty = env(texrs().arg("--cache-stats"));
    assert!(empty.contains("documents: 0"), "{empty}");

    // Turned off, it reports itself off rather than pretending to be empty.
    let off = stdout_of(
        texrs()
            .env("XDG_CACHE_HOME", &cache)
            .env("HOME", &cache)
            .env("TEXRS_CACHE", "0")
            .arg("--cache-stats"),
    );
    assert!(off.contains("cache: off"), "{off}");
    let _ = std::fs::remove_dir_all(&cache);
}

/// Every document the binary compiles has to reach the shard, by whichever
/// route it was compiled — otherwise the cache is a promise kept only for the
/// paths someone remembered to wire up.
#[test]
fn every_document_the_binary_compiles_lands_in_the_shard() {
    let cache = scratch_cache("all_documents");
    let examples: Vec<PathBuf> = std::fs::read_dir(repo("examples"))
        .expect("examples/")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "tex"))
        .collect();
    assert!(!examples.is_empty(), "there are examples to run");

    let env = |cmd: &mut Command| -> String {
        stdout_of(cmd.env("XDG_CACHE_HOME", &cache).env("HOME", &cache))
    };
    for doc in &examples {
        env(texrs().arg("--disasm").arg(doc));
    }
    let stats = env(texrs().arg("--cache-stats"));
    assert!(
        stats.contains(&format!("documents: {}", examples.len())),
        "every one of the {} examples is cached: {stats}",
        examples.len()
    );

    // A listing is a compile too, so it caches what it compiled: after
    // clearing, `--disasm` alone leaves the document in the shard, and the run
    // that follows is a hit rather than a second entry.
    env(texrs().arg("--cache-clear"));
    let doc = &examples[0];
    env(texrs().arg("--disasm").arg(doc));
    let after_disasm = env(texrs().arg("--cache-stats"));
    assert!(
        after_disasm.contains("documents: 1"),
        "a --disasm run caches the chunk it compiled: {after_disasm}"
    );
    env(texrs().arg(doc));
    let after_run = env(texrs().arg("--cache-stats"));
    assert!(
        after_run.contains("documents: 1"),
        "and the run that follows reads it rather than adding another: {after_run}"
    );

    // --no-cache compiles without writing, so a cleared shard stays empty.
    env(texrs().arg("--cache-clear"));
    env(texrs().arg("--no-cache").arg(doc));
    let untouched = env(texrs().arg("--cache-stats"));
    assert!(
        untouched.contains("documents: 0"),
        "--no-cache leaves the shard alone: {untouched}"
    );

    // A document that does not compile has nothing to cache.
    let broken = cache.join("broken.tex");
    std::fs::write(
        &broken,
        "\\undefined@sequence{
",
    )
    .unwrap();
    let out = texrs()
        .env("XDG_CACHE_HOME", &cache)
        .env("HOME", &cache)
        .arg(&broken)
        .output()
        .expect("run");
    assert!(!out.status.success(), "the document is rejected");
    let still_empty = env(texrs().arg("--cache-stats"));
    assert!(
        still_empty.contains("documents: 0"),
        "a failed compile caches nothing: {still_empty}"
    );

    let _ = std::fs::remove_dir_all(&cache);
}

/// A document is a directory with a `Texrs.toml` in it, and the `-X` commands
/// act on the document rather than on a file.
#[test]
fn a_document_is_made_and_built_from_anywhere_inside_it() {
    let dir = scratch_cache("document");
    let run = |args: &[&str], cwd: &Path| -> std::process::Output {
        texrs()
            .args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &dir)
            .env("HOME", &dir)
            .output()
            .expect("run texrs")
    };

    // A new document is one command, and it builds as it stands.
    let made = run(&["-X", "new", "."], &dir);
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    assert!(dir.join("Texrs.toml").is_file());
    assert!(dir.join("index.tex").is_file());

    let original = std::fs::read_to_string(dir.join("index.tex")).unwrap();
    let built = run(&["-X", "build"], &dir);
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let output = dir.join("build").join(format!(
        "{}.txt",
        dir.file_name().unwrap().to_string_lossy()
    ));
    assert!(output.is_file(), "the build wrote {output:?}");
    assert_eq!(
        std::fs::read_to_string(&output).unwrap().trim(),
        "hello from texrs"
    );

    // The document is found by walking up, so a build works from inside it.
    let nested = dir.join("chapters").join("one");
    std::fs::create_dir_all(&nested).unwrap();
    assert!(run(&["-X", "build"], &nested).status.success());

    // A profile that is not there says which ones are.
    let missing = run(&["-X", "build", "--profile", "nope"], &dir);
    assert!(!missing.status.success());
    let err = String::from_utf8_lossy(&missing.stderr);
    assert!(
        err.contains("no output named") && err.contains("default"),
        "{err}"
    );

    // Outside a document, the failure names what is missing.
    let bare = scratch_cache("no_document");
    let out = run(&["-X", "build"], &bare);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("Texrs.toml"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Every flag the completion offers for a document command is accepted by
    // one — the other half of the check the completion test defers here.
    for flag in DOCUMENT_FLAGS {
        let value = if *flag == "--interval" {
            "10"
        } else {
            "default"
        };
        let out = run(&["-X", "build", flag, value], &dir);
        assert!(
            out.status.success(),
            "{flag} is offered but refused: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    // An interval that is not a number is refused rather than defaulted.
    let bad = run(&["-X", "build", "--interval", "soon"], &dir);
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("milliseconds"));

    // `-X watch` is not started here: the loop is covered by unit tests in
    // src/document.rs, and a background process in this suite would be a
    // flake rather than a check.

    // A rebuild of an unchanged document is served from the cache, and says
    // the same thing: a cache that changed the answer would be worse than none.
    let again = run(&["-X", "build"], &dir);
    assert!(again.status.success());
    assert_eq!(
        std::fs::read_to_string(&output).unwrap().trim(),
        "hello from texrs",
        "the cached build produces what the first one did"
    );
    // And an edit is not served the old chunk: the key is the content.
    std::fs::write(
        dir.join("index.tex"),
        "\\catcode`\\{=1 \\catcode`\\}=2 \\message{EDITED}\n\\end\n",
    )
    .unwrap();
    assert!(run(&["-X", "build"], &dir).status.success());
    assert_eq!(
        std::fs::read_to_string(&output).unwrap().trim(),
        "EDITED",
        "an edited document is compiled again rather than served the old chunk"
    );
    std::fs::write(dir.join("index.tex"), original).unwrap();
    assert!(run(&["-X", "build"], &dir).status.success());

    // `-X show` reads the document rather than remembering it: the digest it
    // prints is the input's.
    let shown = run(&["-X", "show"], &dir);
    assert!(shown.status.success());
    let text = String::from_utf8_lossy(&shown.stdout);
    assert!(
        text.contains("index.tex") && text.contains("default"),
        "{text}"
    );

    // `-X dump` prints what a build writes and leaves nothing behind.
    let fresh = scratch_cache("dump_only");
    assert!(run(&["-X", "init"], &fresh).status.success());
    let dumped = run(&["-X", "dump"], &fresh);
    assert!(dumped.status.success());
    assert_eq!(
        String::from_utf8_lossy(&dumped.stdout).trim(),
        "hello from texrs"
    );
    assert!(
        !fresh.join("build").exists(),
        "a dump writes no build directory"
    );

    // An unknown command is looked for on PATH as `texrs-<name>`; with no such
    // program the error stands.
    let unknown = run(&["-X", "frobnicate"], &dir);
    assert!(!unknown.status.success());
    assert!(
        String::from_utf8_lossy(&unknown.stderr).contains("unknown document command"),
        "{}",
        String::from_utf8_lossy(&unknown.stderr)
    );

    // And with one, it runs, taking the arguments after the command name.
    let helper = dir.join("texrs-hello");
    std::fs::write(&helper, "#!/bin/sh\necho \"external ran: $*\"\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!(
            "{}:{}",
            dir.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let out = texrs()
            .args(["-X", "hello", "one", "two"])
            .current_dir(&dir)
            .env("PATH", path)
            .output()
            .expect("run texrs");
        assert!(out.status.success());
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            "external ran: one two"
        );
    }
    let _ = std::fs::remove_dir_all(&fresh);

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&bare);
}

/// `-X dvi` reads what real tex shipped. The file is produced here by tex
/// itself, so what is parsed is a real one rather than bytes a test invented.
#[test]
fn the_dvi_reader_reads_what_tex_wrote() {
    let dir = scratch_cache("dvi");
    std::fs::write(dir.join("t.tex"), "Hello DVI world.\n\\bye\n").unwrap();
    let tex = std::process::Command::new("tex")
        .arg("-interaction=batchmode")
        .arg("t.tex")
        .current_dir(&dir)
        .output();
    let Ok(_) = tex else {
        // No tex here; the parity suite needs one too.
        let _ = std::fs::remove_dir_all(&dir);
        return;
    };
    if !dir.join("t.dvi").is_file() {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }

    let out = texrs()
        .args(["-X", "dvi", "t.dvi"])
        .current_dir(&dir)
        .output()
        .expect("run texrs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("preamble"), "{text}");
    assert!(text.contains("1 page(s)"), "{text}");
    assert!(text.contains("Hello"), "the characters it set: {text}");

    // A file that is not DVI says so rather than printing nothing.
    std::fs::write(dir.join("not.dvi"), b"\xff").unwrap();
    let bad = texrs()
        .args(["-X", "dvi", "not.dvi"])
        .current_dir(&dir)
        .output()
        .expect("run texrs");
    assert!(!bad.status.success());
    assert!(
        String::from_utf8_lossy(&bad.stderr).contains("not a DVI opcode"),
        "{}",
        String::from_utf8_lossy(&bad.stderr)
    );

    // Two files of one document compare equal, and the exit status says so —
    // which is what a harness reads.
    std::fs::copy(dir.join("t.dvi"), dir.join("copy.dvi")).unwrap();
    let same = texrs()
        .args(["-X", "dvi", "t.dvi", "copy.dvi"])
        .current_dir(&dir)
        .output()
        .expect("run texrs");
    assert!(same.status.success());
    assert!(String::from_utf8_lossy(&same.stdout).contains("the same document"));

    // A different document exits non-zero and says what differs.
    std::fs::write(dir.join("u.tex"), "Something else.\n\\bye\n").unwrap();
    let _ = std::process::Command::new("tex")
        .arg("-interaction=batchmode")
        .arg("u.tex")
        .current_dir(&dir)
        .output();
    if dir.join("u.dvi").is_file() {
        let differ = texrs()
            .args(["-X", "dvi", "t.dvi", "u.dvi"])
            .current_dir(&dir)
            .output()
            .expect("run texrs");
        assert!(!differ.status.success(), "a divergence is a failure");
        assert!(
            String::from_utf8_lossy(&differ.stdout).contains("Text"),
            "{}",
            String::from_utf8_lossy(&differ.stdout)
        );
    }

    // And one that is not there names itself.
    let missing = texrs()
        .args(["-X", "dvi", "absent.dvi"])
        .current_dir(&dir)
        .output()
        .expect("run texrs");
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("absent.dvi"));

    let _ = std::fs::remove_dir_all(&dir);
}

/// `-X bib` reads a database, and says so when it could not read all of it.
#[test]
fn the_bib_reader_reads_a_database_and_reports_what_it_could_not() {
    let dir = scratch_cache("bib");
    std::fs::write(
        dir.join("refs.bib"),
        "@STRING{tug = \"TeX Users Group\"}\n\
         @Article{knuth1984,\n  author = {Knuth, Donald E.},\n\
           journal = tug # \" Journal\",\n  year = 1984\n}\n",
    )
    .unwrap();

    let out = texrs()
        .args(["-X", "bib", "refs.bib"])
        .current_dir(&dir)
        .output()
        .expect("run texrs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("article        knuth1984"), "{text}");
    assert!(
        text.contains("TeX Users Group Journal"),
        "the abbreviation and the concatenation: {text}"
    );

    // A database with a record that cannot be read still prints what it could,
    // and the exit status says something was wrong.
    std::fs::write(
        dir.join("bad.bib"),
        "@misc{k, title={one}}\n@misc{k, title={two}}\n",
    )
    .unwrap();
    let out = texrs()
        .args(["-X", "bib", "bad.bib"])
        .current_dir(&dir)
        .output()
        .expect("run texrs");
    assert!(!out.status.success(), "a warning is a non-zero exit");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("defined twice"), "{text}");
    assert!(text.contains("one"), "and the entry it kept: {text}");

    // An .aux asks the other question: what does this document cite?
    std::fs::write(
        dir.join("t.aux"),
        "\\relax\n\\citation{knuth1984}\n\\citation{nosuch}\n\\bibdata{refs}\n",
    )
    .unwrap();
    let cites = texrs()
        .args(["-X", "bib", "t.aux"])
        .current_dir(&dir)
        .output()
        .expect("run texrs");
    let text = String::from_utf8_lossy(&cites.stdout);
    assert!(text.contains("cited     knuth1984"), "{text}");
    assert!(text.contains("MISSING   nosuch"), "{text}");
    assert!(
        !cites.status.success(),
        "a citation nothing defines is a non-zero exit"
    );

    // With nothing missing it succeeds, and says what went uncited.
    std::fs::write(
        dir.join("ok.aux"),
        "\\citation{knuth1984}\n\\bibdata{refs}\n",
    )
    .unwrap();
    let ok = texrs()
        .args(["-X", "bib", "ok.aux"])
        .current_dir(&dir)
        .output()
        .expect("run texrs");
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stdout)
    );

    // An .aux with no \bibdata says so rather than reporting everything as
    // missing.
    std::fs::write(dir.join("bare.aux"), "\\citation{k}\n").unwrap();
    let bare = texrs()
        .args(["-X", "bib", "bare.aux"])
        .current_dir(&dir)
        .output()
        .expect("run texrs");
    assert!(!bare.status.success());
    assert!(String::from_utf8_lossy(&bare.stderr).contains("bibdata"));

    // A file that is not there names itself.
    let missing = texrs()
        .args(["-X", "bib", "absent.bib"])
        .current_dir(&dir)
        .output()
        .expect("run texrs");
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("absent.bib"));

    let _ = std::fs::remove_dir_all(&dir);
}

/// The bundle commands, without touching the network: what they refuse, and
/// what an empty cache looks like.
#[test]
fn the_bundle_commands_say_what_they_need() {
    let dir = scratch_cache("bundles");
    let run = |args: &[&str]| -> std::process::Output {
        texrs()
            .args(args)
            .current_dir(&dir)
            .env("XDG_CACHE_HOME", &dir)
            .env("HOME", &dir)
            .output()
            .expect("run texrs")
    };

    // Nothing fetched yet, and saying so beats printing nothing.
    let listed = run(&["-X", "bundle", "list"]);
    assert!(listed.status.success());
    assert!(String::from_utf8_lossy(&listed.stdout).contains("no bundles fetched"));

    // A fetch needs a URL, and only http(s) is one. Neither reaches the
    // network: the first is refused for want of an argument, the second by
    // scheme.
    let bare = run(&["-X", "bundle", "fetch"]);
    assert!(!bare.status.success());
    assert!(String::from_utf8_lossy(&bare.stderr).contains("needs a URL"));

    let local = run(&["-X", "bundle", "fetch", "/tmp/support.zip"]);
    assert!(!local.status.success());
    assert!(
        String::from_utf8_lossy(&local.stderr).contains("only http and https"),
        "{}",
        String::from_utf8_lossy(&local.stderr)
    );

    // An unknown subcommand names the two there are.
    let wrong = run(&["-X", "bundle", "frobnicate"]);
    assert!(!wrong.status.success());
    assert!(String::from_utf8_lossy(&wrong.stderr).contains("fetch or list"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_unknown_option_is_refused_and_a_missing_file_is_reported() {
    let out = texrs().arg("--nope").output().expect("run");
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("unknown option"),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );

    let out = texrs().arg("/no/such/file.tex").output().expect("run");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("/no/such/file.tex"), "{err}");

    // No arguments at all starts the prompt, which is what `tex` does too --
    // it asks for input rather than printing its usage and quitting. With stdin
    // closed there is nothing to read, so the session ends immediately and
    // successfully, having printed nothing.
    let child = texrs()
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    let out = child.wait_with_output().expect("run");
    assert!(
        out.status.success(),
        "no arguments should open the prompt, not fail: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stdout.is_empty(),
        "a prompt reading from a closed stdin prints nothing: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// `-X bst` reads a style and names what nothing defines.
///
/// The value of the command is the second half: bibtex reports an undefined
/// name at run time, one per run, so a style with three of them costs three
/// builds to find. This asks once.
#[test]
fn the_bst_reader_reads_a_style_and_names_what_nothing_defines() {
    let dir = scratch_cache("bst");

    // A style the standard ones are shaped like, small enough to read.
    std::fs::write(
        dir.join("mine.bst"),
        "% a style\n\
         ENTRY { author title year } { } { label }\n\
         INTEGERS { state }\n\
         MACRO {jan} {\"January\"}\n\
         FUNCTION {output} { title write$ newline$ }\n\
         READ\n SORT\n ITERATE {output}\n",
    )
    .unwrap();
    let out = texrs()
        .args(["-X", "bst", "mine.bst"])
        .current_dir(&dir)
        .output()
        .expect("run texrs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("fields     author title year"), "{text}");
    assert!(text.contains("macros     jan"), "{text}");
    assert!(text.contains("functions  1"), "{text}");
    assert!(text.contains("ITERATE    output"), "{text}");

    // One that calls a name nothing defines says which, and fails.
    std::fs::write(
        dir.join("broken.bst"),
        "FUNCTION {a} { format.title write$ }\nITERATE {a}\n",
    )
    .unwrap();
    let out = texrs()
        .args(["-X", "bst", "broken.bst"])
        .current_dir(&dir)
        .output()
        .expect("run texrs");
    assert!(!out.status.success(), "an undefined name is a failure");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("UNDEFINED  format.title"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );

    // And a style that is not there names itself.
    let missing = texrs()
        .args(["-X", "bst", "absent.bst"])
        .current_dir(&dir)
        .output()
        .expect("run texrs");
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("absent.bst"));

    // The one that matters: the style every plain document uses, read whole
    // with nothing undefined in it.
    if let Ok(found) = std::process::Command::new("kpsewhich")
        .arg("plain.bst")
        .output()
    {
        let path = String::from_utf8_lossy(&found.stdout).trim().to_string();
        if !path.is_empty() {
            let out = texrs()
                .args(["-X", "bst", &path])
                .output()
                .expect("run texrs");
            assert!(
                out.status.success(),
                "plain.bst: {}",
                String::from_utf8_lossy(&out.stdout)
            );
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(text.contains("ITERATE    call.type$"), "{text}");
            assert!(text.contains("READ"), "{text}");
        }
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// `-X tfm` reads a font's metrics, by path or by name.
#[test]
fn the_tfm_reader_reads_a_font_by_name_and_by_path() {
    // A font that is not there names what was asked for.
    let missing = texrs()
        .args(["-X", "tfm", "nosuchfont"])
        .output()
        .expect("run");
    assert!(!missing.status.success());
    assert!(
        String::from_utf8_lossy(&missing.stderr).contains("nosuchfont"),
        "{}",
        String::from_utf8_lossy(&missing.stderr)
    );

    let Ok(found) = std::process::Command::new("kpsewhich")
        .arg("cmr10.tfm")
        .output()
    else {
        return;
    };
    let path = String::from_utf8_lossy(&found.stdout).trim().to_string();
    if path.is_empty() {
        return;
    }

    // By path and by name, the same font.
    let by_path = stdout_of(texrs().args(["-X", "tfm", &path]));
    let by_name = stdout_of(texrs().args(["-X", "tfm", "cmr10"]));
    assert_eq!(by_path, by_name, "a name is looked up, not read as a path");
    assert!(by_path.contains("family        CMR"), "{by_path}");
    assert!(by_path.contains("designsize    10.000000pt"), "{by_path}");
    assert!(by_path.contains("characters    128"), "{by_path}");

    // One character, with the program it takes part in: f makes three
    // ligatures in cmr10, which is why "office" sets the way it does.
    let f = stdout_of(texrs().args(["-X", "tfm", "cmr10", "f"]));
    assert!(f.contains("width 0.305557"), "{f}");
    assert!(f.contains("lig  i -> 0o14"), "{f}");
    assert!(f.contains("kern ! +0.077779"), "{f}");
}

/// `-X bibtex` writes the `.bbl` the real `bibtex` writes.
///
/// The unit tests hold the builtins to what BibTeX does; this holds the
/// command that strings them together to it, through the file system: the same
/// `.aux`, the same database, the same installed style, and the two `.bbl`
/// files compared.
#[test]
fn the_bibtex_command_writes_what_bibtex_writes() {
    let dir = scratch_cache("bibtex");
    std::fs::write(
        dir.join("refs.bib"),
        "@article{k, author={Knuth, Donald E.}, title={Literate Programming},\n\
          journal={The Computer Journal}, year=1984, volume=27, number=2,\n\
          pages={97--111}, month=may}\n\
         @book{b, author={van Beethoven, Ludwig and de la Vega, Maria},\n\
          title={The Ninth}, publisher={DG}, year=1826}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("t.aux"),
        "\\relax\n\\citation{k}\n\\citation{b}\n\\bibstyle{plain}\n\\bibdata{refs}\n",
    )
    .unwrap();

    // What the real bibtex writes, if this machine has one.
    let Ok(real) = std::process::Command::new("bibtex")
        .arg("t")
        .current_dir(&dir)
        .output()
    else {
        return;
    };
    let _ = real;
    let Ok(want) = std::fs::read_to_string(dir.join("t.bbl")) else {
        return;
    };
    std::fs::remove_file(dir.join("t.bbl")).unwrap();

    let out = texrs()
        .args(["-X", "bibtex", "t.aux"])
        .current_dir(&dir)
        .output()
        .expect("run texrs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = std::fs::read_to_string(dir.join("t.bbl")).expect("texrs wrote a .bbl");
    assert!(
        got.contains("Donald~E. Knuth"),
        "the run reached the names: {got}"
    );
    assert_eq!(got, want, "texrs and bibtex wrote different bibliographies");

    // An .aux with no style says so rather than writing half a file.
    std::fs::write(
        dir.join("n.aux"),
        "\\relax\n\\citation{k}\n\\bibdata{refs}\n",
    )
    .unwrap();
    let out = texrs()
        .args(["-X", "bibtex", "n.aux"])
        .current_dir(&dir)
        .output()
        .expect("run texrs");
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("bibstyle"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// `-X vf` reads a virtual font, by name or by path.
#[test]
fn the_vf_reader_says_what_a_virtual_character_really_sets() {
    let missing = texrs()
        .args(["-X", "vf", "nosuchfont"])
        .output()
        .expect("run");
    assert!(!missing.status.success());
    assert!(
        String::from_utf8_lossy(&missing.stderr).contains("nosuchfont"),
        "{}",
        String::from_utf8_lossy(&missing.stderr)
    );

    let Ok(found) = std::process::Command::new("kpsewhich")
        .arg("ptmr7t.vf")
        .output()
    else {
        return;
    };
    let path = String::from_utf8_lossy(&found.stdout).trim().to_string();
    if path.is_empty() {
        return;
    }

    let by_path = stdout_of(texrs().args(["-X", "vf", &path]));
    let by_name = stdout_of(texrs().args(["-X", "vf", "ptmr7t"]));
    assert_eq!(by_path, by_name, "a name is looked up, not read as a path");
    // It sets everything in the real Times, in that font's own encoding.
    assert!(by_path.contains("font 0        ptmr8r"), "{by_path}");
    assert!(by_path.contains("designsize    10.000000pt"), "{by_path}");

    // The ff ligature is two f's moved together: a character TeX has and the
    // real font does not.
    let ff = stdout_of(texrs().args(["-X", "vf", "ptmr7t", "\u{b}"]));
    assert!(ff.contains("set 0o146"), "{ff}");
    assert!(
        ff.contains("right -0.025000"),
        "the kern between them: {ff}"
    );
}

/// `-X pk` reads a packed font and draws a character.
#[test]
fn the_pk_reader_draws_a_character() {
    let missing = texrs()
        .args(["-X", "pk", "nosuchfont"])
        .output()
        .expect("run");
    assert!(!missing.status.success());
    assert!(
        String::from_utf8_lossy(&missing.stderr).contains("nosuchfont"),
        "{}",
        String::from_utf8_lossy(&missing.stderr)
    );

    let Ok(found) = std::process::Command::new("kpsewhich")
        .args(["-format=pk", "cmr10.600pk"])
        .output()
    else {
        return;
    };
    let path = String::from_utf8_lossy(&found.stdout).trim().to_string();
    if path.is_empty() {
        return;
    }

    let by_path = stdout_of(texrs().args(["-X", "pk", &path]));
    let by_name = stdout_of(texrs().args(["-X", "pk", "cmr10"]));
    assert_eq!(by_path, by_name, "a name is looked up at the shipped size");
    assert!(by_path.contains("resolution    600 dpi"), "{by_path}");
    assert!(by_path.contains("characters    128"), "{by_path}");
    // The packed font carries the same checksum as the metrics, which is how a
    // driver tells that the two belong to each other.
    assert!(by_path.contains("checksum      0o11374260171"), "{by_path}");

    // A drawn T: a bar across the top, and a stem down the middle that is
    // narrower than the bar.
    let t = stdout_of(texrs().args(["-X", "pk", "cmr10", "T"]));
    let rows: Vec<&str> = t.lines().skip(1).collect();
    assert!(t.starts_with("'T'  53x57 pixels"), "{t}");
    let ink = |row: &str| row.chars().filter(|&c| c == '*').count();
    assert!(ink(rows[0]) > 45, "the bar across the top: {:?}", rows[0]);
    assert!(
        ink(rows[rows.len() / 2]) < 15,
        "the stem is narrower: {:?}",
        rows[rows.len() / 2]
    );
}

/// `-X otf` reads an OpenType font, by name or by path.
#[test]
fn the_otf_reader_says_which_glyph_a_character_becomes() {
    let missing = texrs()
        .args(["-X", "otf", "nosuchfont.otf"])
        .output()
        .expect("run");
    assert!(!missing.status.success());
    assert!(
        String::from_utf8_lossy(&missing.stderr).contains("nosuchfont"),
        "{}",
        String::from_utf8_lossy(&missing.stderr)
    );

    let Ok(found) = std::process::Command::new("kpsewhich")
        .arg("lmroman10-regular.otf")
        .output()
    else {
        return;
    };
    let path = String::from_utf8_lossy(&found.stdout).trim().to_string();
    if path.is_empty() {
        return;
    }

    let by_path = stdout_of(texrs().args(["-X", "otf", &path]));
    let by_name = stdout_of(texrs().args(["-X", "otf", "lmroman10-regular.otf"]));
    assert_eq!(by_path, by_name, "a name is looked up, not read as a path");
    assert!(by_path.contains("family        LM Roman 10"), "{by_path}");
    assert!(by_path.contains("outlines      CFF"), "{by_path}");
    assert!(by_path.contains("units per em  1000"), "{by_path}");
    // The table directory is listed, with the tag that carries a trailing
    // space spelled as the font spells it.
    assert!(by_path.contains("CFF "), "{by_path}");
    assert!(by_path.contains("cmap"), "{by_path}");

    // Latin Modern is Computer Modern, so an A is 0.75 em here as it is in
    // cmr10.tfm -- the same number by a different road.
    let a = stdout_of(texrs().args(["-X", "otf", "lmroman10-regular.otf", "A"]));
    assert!(a.contains("width 750 of 1000"), "{a}");
    assert!(a.contains("0.7500 em"), "{a}");
}

/// `-X pfb` reads a Type 1 font, by name or by path.
#[test]
fn the_pfb_reader_says_what_a_glyph_costs() {
    let missing = texrs()
        .args(["-X", "pfb", "nosuchfont"])
        .output()
        .expect("run");
    assert!(!missing.status.success());
    assert!(
        String::from_utf8_lossy(&missing.stderr).contains("nosuchfont"),
        "{}",
        String::from_utf8_lossy(&missing.stderr)
    );

    let Ok(found) = std::process::Command::new("kpsewhich")
        .arg("cmr10.pfb")
        .output()
    else {
        return;
    };
    let path = String::from_utf8_lossy(&found.stdout).trim().to_string();
    if path.is_empty() {
        return;
    }

    let by_path = stdout_of(texrs().args(["-X", "pfb", &path]));
    let by_name = stdout_of(texrs().args(["-X", "pfb", "cmr10"]));
    assert_eq!(by_path, by_name, "a name is looked up, not read as a path");
    assert!(by_path.contains("font name     CMR10"), "{by_path}");
    assert!(
        by_path.contains("matrix        0.001 0 0 0.001 0 0"),
        "{by_path}"
    );
    // Computer Modern carries its own encoding rather than Adobe's, which is
    // what lets TeX put a ligature at position 11.
    assert!(by_path.contains("the font's own"), "{by_path}");

    // The width in the charstring is the width in the metrics: 750, which is
    // cmr10.tfm's 0.750002 of the design size.
    let a = stdout_of(texrs().args(["-X", "pfb", "cmr10", "A"]));
    assert!(a.contains("width 750"), "{a}");
    assert!(a.contains("side bearing 32"), "{a}");
}

/// `-X map` and `-X enc`: what a TeX font name means, and what a code is
/// called.
#[test]
fn the_map_and_encoding_readers_join_a_name_to_a_font() {
    let missing = texrs()
        .args(["-X", "map", "nosuch.map"])
        .output()
        .expect("run");
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("nosuch"));

    let Ok(found) = std::process::Command::new("kpsewhich")
        .arg("pdftex.map")
        .output()
    else {
        return;
    };
    let path = String::from_utf8_lossy(&found.stdout)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if path.is_empty() {
        return;
    }

    // The summary counts what the map does, and a real one does all of it.
    let summary = stdout_of(texrs().args(["-X", "map", &path]));
    assert!(summary.contains("fonts         "), "{summary}");
    assert!(summary.contains("re-encoded    "), "{summary}");
    assert!(
        !summary.contains("unreadable"),
        "every line reads: {summary}"
    );

    // One name: Times as TeX addresses it.
    let times = stdout_of(texrs().args(["-X", "map", &path, "ptmr8r"]));
    assert!(times.contains("encoding      8r.enc"), "{times}");
    assert!(times.contains(".pfb"), "{times}");

    // A name the map does not define is an error rather than an empty answer.
    let absent = texrs()
        .args(["-X", "map", &path, "nosuchfontname"])
        .output()
        .expect("run");
    assert!(!absent.status.success());

    // The encoding that name pointed at, looked up by its bare name.
    let enc = stdout_of(texrs().args(["-X", "enc", "8r"]));
    assert!(enc.contains("encoding      TeXBase1Encoding"), "{enc}");
    assert!(enc.contains("names         256"), "{enc}");
    assert!(enc.contains("65 'A'   A"), "an A is where an A is: {enc}");
}

/// `-X itar` indexes a tar bundle and reads one file out of it.
#[test]
fn the_itar_reader_indexes_a_bundle_and_reads_from_it() {
    let dir = scratch_cache("itar");
    let work = dir.join("work");
    std::fs::create_dir_all(work.join("deep")).unwrap();
    std::fs::write(work.join("macros.tex"), "\\def\\hi{HI}\n").unwrap();
    std::fs::write(work.join("deep/other.tex"), "% deep\n").unwrap();
    let made = std::process::Command::new("tar")
        .env("COPYFILE_DISABLE", "1")
        .args(["cf", "../b.tar", "macros.tex", "deep/other.tex"])
        .current_dir(&work)
        .status();
    let Ok(made) = made else { return };
    if !made.success() {
        return;
    }

    // The summary and the index: a line per file, with where it begins.
    let listing = stdout_of(texrs().args(["-X", "itar", "b.tar"]).current_dir(&dir));
    assert!(listing.contains("files         2"), "{listing}");
    let index: Vec<&str> = listing
        .lines()
        .filter(|line| line.contains(".tex "))
        .collect();
    assert_eq!(index.len(), 2, "{listing}");
    for line in &index {
        let words: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(words.len(), 3, "{line}");
        // A file's data begins on a block boundary, because a tar header is a
        // whole block.
        assert_eq!(
            words[1].parse::<u64>().expect("an offset") % 512,
            0,
            "{line}"
        );
    }

    // One file, by its last component, byte for byte.
    let out = stdout_of(
        texrs()
            .args(["-X", "itar", "b.tar", "other.tex"])
            .current_dir(&dir),
    );
    assert_eq!(out, "% deep\n");

    // A file that is not there is an error rather than nothing.
    let missing = texrs()
        .args(["-X", "itar", "b.tar", "nosuch.tex"])
        .current_dir(&dir)
        .output()
        .expect("run");
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("nosuch.tex"));

    let _ = std::fs::remove_dir_all(&dir);
}

/// `-X special` says what a `\special` means to a driver.
#[test]
fn the_special_reader_says_what_a_special_means() {
    let colour = stdout_of(texrs().args(["-X", "special", "color push rgb 1 0 0"]));
    assert!(colour.contains("colour push"), "{colour}");
    assert!(colour.contains("rgb 1 0 0"), "{colour}");

    // A paper size in TeX's own scaled points: A4 is 39158276 by 55380990,
    // which is what tex computes for 210mm by 297mm.
    let paper = stdout_of(texrs().args(["-X", "special", "papersize=210mm,297mm"]));
    assert!(paper.contains("39158276 by 55380990"), "{paper}");

    // The arguments are joined, so a special with spaces in it need not be
    // quoted as one word.
    let figure = stdout_of(texrs().args([
        "-X",
        "special",
        "PSfile=\"fig.eps\"",
        "llx=0",
        "lly=0",
        "urx=100",
        "ury=50",
        "rwi=1000",
    ]));
    assert!(figure.contains("figure        fig.eps"), "{figure}");
    assert!(
        figure.contains("width 100pt"),
        "dvips counts in tenths: {figure}"
    );

    // Something no driver knows comes back as it was rather than being dropped.
    let unknown = stdout_of(texrs().args(["-X", "special", "ps: gsave 0 0 moveto"]));
    assert!(
        unknown.contains("not read      ps: gsave 0 0 moveto"),
        "{unknown}"
    );

    // And nothing at all is an error rather than an empty answer.
    let empty = texrs().args(["-X", "special", ""]).output().expect("run");
    assert!(!empty.status.success());
}
