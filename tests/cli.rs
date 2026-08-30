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
            if word.starts_with("--") && word.len() > 2 {
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

    env(texrs().arg(&doc));
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
        env(texrs().arg(doc));
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

    // No arguments at all prints the usage rather than doing nothing.
    let out = texrs().output().expect("run");
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("usage: texrs"),
        "no usage on stderr"
    );
}
