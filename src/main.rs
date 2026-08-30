//! The `texrs` driver.
//!
//! Running a file prints the `\message` stream the way `tex` prints it on the
//! terminal — `(./file.tex <messages> )` — which is the comparison
//! `scripts/parity.sh` makes against the real engine. The introspection flags
//! stop earlier in the same pipeline: `--dump-tokens` after the mouth,
//! `--disasm` after lowering, so each stage can be read on its own.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The option grammar, in the fleet's house style: `── SECTION ───` dividers
/// and `//` descriptions, printed under the logo the way `tp -h` prints its own.
const USAGE: &str = "\
\n  USAGE: texrs [OPTIONS] [FILE[.tex]]... [COMMANDS]
         texrs [OPTIONS] \\FIRST-LINE      // the arguments are the input
         texrs [OPTIONS] &FMT ARGS        // with a named format
         texrs                            // no arguments: the prompt

  ── TEX OPTIONS ────────────────────────────────────────
  -interaction=MODE
          // batchmode, nonstopmode, scrollmode or errorstopmode
  -jobname=NAME
          // Set the job name
  -output-directory=DIR
          // Write files in DIR
  -progname=NAME
          // Set the program name
  -fmt=NAME
          // Use a named format
  -ini
          // Be initex
  -halt-on-error
          // Stop at the first error
  -file-line-error, -no-file-line-error
          // file:line:error style messages
  -recorder
          // Record the files read
  -8bit
          // Write 8-bit characters as themselves

  ── RUNNING ────────────────────────────────────────────
  --repl
          // Start the interactive prompt
  --jobs=N
          // Compile N documents at once (default: one per core)
  --no-cache
          // Compile this run rather than reading the bytecode cache
  --aot
          // Compile the document to a standalone native executable

  ── LOOKING INSIDE ─────────────────────────────────────
  --dump-tokens
          // Print the mouth's token stream and exit
  --disasm
          // Print the lowered fusevm bytecode and exit
  --tiers
          // Run it, then report which fusevm tier took its bytecode

  ── EDITORS ────────────────────────────────────────────
  --lsp
          // Speak the Language Server Protocol over stdio
  --dap
          // Speak the Debug Adapter Protocol over stdio

  ── CACHE ──────────────────────────────────────────────
  --cache-stats
          // Say what the bytecode cache holds and where
  --cache-clear
          // Delete it

  ── DOCUMENTS ──────────────────────────────────────────
  -X new [DIR]            // Make one (Texrs.toml + index.tex)
  -X init                 // Make one here, named after this directory
  -X build [--profile P]  // Build the document this directory is in
  -X watch [--profile P]  // Rebuild it whenever an input changes
  -X show                 // Say what the document is and can produce
  -X dump [--profile P]   // Build to stdout, writing nothing
  -X bundle fetch URL     // Download a bundle into the cache
  -X bundle list          // Say which bundles have been fetched
  -X dvi FILE.dvi         // Read what real tex shipped for a document
  -X dvi A.dvi B.dvi      // Say whether two files are the same document
  --profile NAME          // Which output to build
  --interval MS           // How often -X watch looks (default 250)

  ── SYSTEM ─────────────────────────────────────────────
  -h, --help              // Print this
  --version               // Print the version banner
";

fn main() -> ExitCode {
    struct Stats;
    impl Drop for Stats {
        fn drop(&mut self) {
            if std::env::var_os("TEXRS_PRELEX_STATS").is_some() {
                prelex_stats();
            }
        }
    }
    let _stats = Stats;

    // The document commands come first: `-X` takes over the whole invocation,
    // as tectonic's V2 interface does, so nothing below has to know about them.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("-X") {
        return run_document(&args[1..]);
    }

    let cli = match texrs::cli::parse(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };

    match cli.mode {
        texrs::cli::Mode::Help => {
            texrs::banner::print_banner(true);
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        texrs::cli::Mode::Version => {
            println!("{}", texrs::banner::version_banner());
            return ExitCode::SUCCESS;
        }
        texrs::cli::Mode::Repl => {
            return match texrs::repl::run() {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => fail(&e),
            }
        }
        texrs::cli::Mode::Lsp => {
            return match texrs::lsp::run() {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => fail(&e),
            }
        }
        texrs::cli::Mode::Dap => {
            return match texrs::dap::run() {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => fail(&e),
            }
        }
        texrs::cli::Mode::CacheStats => return cache_stats(),
        texrs::cli::Mode::CacheClear => return cache_clear(),
        texrs::cli::Mode::Run => {}
    }

    let (dump_tokens, disasm, aot, tiers, no_cache, jobs) = (
        cli.dump_tokens,
        cli.disasm,
        cli.aot,
        cli.tiers,
        cli.no_cache,
        cli.jobs,
    );
    let mut paths = cli.files.clone();

    // `texrs '\message{hi}\end'` -- tex's second invocation form, where the
    // arguments ARE the input. There is no file, so there is no `(./name.tex …)`
    // wrapper either: tex prints the messages bare, and so does this.
    if paths.is_empty() {
        if let Some(src) = cli.command_line_source() {
            return run_command_line(&src, cli.interaction);
        }
    }

    // No file and nothing on the command line: the prompt. `tex` prompts for
    // input here too -- an engine invoked with nothing to do should ask, not
    // print its usage and quit.
    if paths.is_empty() {
        return match texrs::repl::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&e),
        };
    }
    // More than one document is the case that actually parallelises. Each is an
    // independent compile -- its own mouth, macro table and chunk -- sharing
    // nothing but the on-disk bytecode cache, so running them together is a
    // straight fan-out. `tex` cannot do this at all: one process compiles one
    // file, and a user wanting more reaches for `make -j`.
    if paths.len() > 1 && !dump_tokens && !disasm && !tiers {
        return run_many(&paths, no_cache, jobs);
    }
    let path = paths.remove(0);
    // The document is opened through the provider stack rather than read
    // directly: the stack is what `\input` will search, and opening the primary
    // through it means the run's record of what it read starts with the
    // document itself.
    let mut inputs = texrs::io::ProviderStack::new();
    inputs.push(Box::new(
        texrs::io::FilesystemProvider::with_roots(Vec::new()).with_primary(&path),
    ));
    let src = match texrs::io::InputProvider::input_open_primary(&mut inputs) {
        texrs::io::OpenResult::Ok(input) => input.content,
        texrs::io::OpenResult::NotAvailable => {
            eprintln!("texrs: {path}: no such file");
            return ExitCode::from(1);
        }
        texrs::io::OpenResult::Err(e) => {
            eprintln!("texrs: {path}: {e}");
            return ExitCode::from(1);
        }
    };

    if dump_tokens {
        // The mouth alone: no expansion, so what prints is what the category
        // codes made of the bytes and nothing more.
        let cats = texrs::catcode::CatTable::new();
        let mut lx = texrs::lexer::Lexer::new(&src);
        while let Some(t) = lx.next_token(&cats) {
            println!("{t:?}");
        }
        return ExitCode::SUCCESS;
    }

    if disasm {
        // The same bytecode an ordinary run would execute, so it goes to the
        // cache like one: a listing is a compile, and the next run should not
        // have to do it again.
        let compiled = match no_cache {
            true => texrs::compile(&src),
            false => texrs::compile_cached(std::path::Path::new(&path), &src),
        };
        return match compiled {
            Ok(chunk) => {
                print!("{}", chunk.disassemble());
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e.0),
        };
    }

    if aot {
        // The executable sits beside the document, named after it: `doc.tex`
        // compiles to `doc`, the way a C file compiles to a program.
        let out = std::path::Path::new(&path).with_extension("");
        return match texrs::aot::compile_executable(&path, &out) {
            Ok(p) => {
                println!("{}", p.display());
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e),
        };
    }

    if tiers {
        // The report runs the document, so its own output comes first: what is
        // measured is what an ordinary run does. It compiles rather than
        // reading the cache, because the question is about this bytecode.
        return match texrs::tiers::report(&src) {
            Ok(r) => {
                println!("{r}");
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e),
        };
    }

    // tex reads the command line as more input AFTER the file -- but only if
    // the file did not end the run. `\end` inside it stops everything, and the
    // trailing arguments are never seen.
    let trailing = cli
        .command_line_source()
        .filter(|_| !texrs::source_ends_run(&src));
    if let Some(extra) = trailing {
        return run_with_trailing(&path, &src, &extra, cli.interaction);
    }

    // The cache keys on the file, so it is only used when there is one and the
    // run has not asked to go without it.
    let run = match no_cache {
        true => texrs::run_messages(&src),
        false => texrs::run_messages_cached(std::path::Path::new(&path), &src),
    };
    match run {
        Ok(msgs) => {
            if cli.interaction.prints() {
                println!("{}", file_line(&path, &msgs, texrs::source_ends_run(&src)));
            }
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e.0),
    }
}

/// Compile and run several documents at once, one per core.
///
/// Each document is wholly independent -- its own mouth, catcode table, macro
/// table and chunk -- so this is a fan-out with nothing to synchronise but the
/// bytecode cache, which takes a writer lock of its own. `tex` has no
/// equivalent: one process compiles one file, and a user who wants more runs
/// `make -j` and gets a process per document instead of a thread.
///
/// Output is printed in ARGUMENT order however the threads finish, because a
/// build log that reorders itself between runs is not a log anyone can diff.
/// The exit code is the worst of the runs: one bad document fails the batch.
fn run_many(paths: &[String], no_cache: bool, jobs: Option<usize>) -> ExitCode {
    let workers = jobs
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
        })
        .max(1)
        .min(paths.len());

    let next = std::sync::atomic::AtomicUsize::new(0);
    let results: Vec<std::sync::Mutex<Option<Result<String, String>>>> = (0..paths.len())
        .map(|_| std::sync::Mutex::new(None))
        .collect();

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                // A shared cursor rather than a fixed slice each: documents
                // differ in size by orders of magnitude, so handing out equal
                // COUNTS would leave one worker with the big one and the rest
                // idle. Taking the next index as each finishes balances it.
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if i >= paths.len() {
                        break;
                    }
                    *results[i].lock().expect("result slot") =
                        Some(one_document(&paths[i], no_cache));
                }
            });
        }
    });

    let mut code = ExitCode::SUCCESS;
    let mut failed = false;
    for (i, slot) in results.iter().enumerate() {
        match slot.lock().expect("result slot").take() {
            Some(Ok(line)) => println!("{line}"),
            Some(Err(e)) => {
                eprintln!("! {e}.");
                failed = true;
            }
            None => {
                eprintln!("texrs: {}: not run", paths[i]);
                failed = true;
            }
        }
    }
    if failed {
        code = ExitCode::from(1);
    }
    code
}

/// One document, as the single-file path runs it, returning the line to print.
fn one_document(path: &str, no_cache: bool) -> Result<String, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let run = match no_cache {
        true => texrs::run_messages(&src),
        false => texrs::run_messages_cached(Path::new(path), &src),
    };
    let msgs = run.map_err(|e| e.0)?;
    let body = match msgs.is_empty() {
        true => String::new(),
        false => format!(" {msgs}"),
    };
    Ok(format!("(./{path}{body} )"))
}

/// `texrs -X …`: the document commands, ported in shape from tectonic's V2
/// interface — a document is a directory with a `Texrs.toml` in it, and the
/// commands act on the document rather than on a file.
fn run_document(args: &[String]) -> ExitCode {
    let Some(command) = args.first().map(String::as_str) else {
        eprint!("{USAGE}");
        return ExitCode::from(1);
    };
    match command {
        "new" | "init" => {
            // `new` takes a directory; `init` is `new .` under the name
            // tectonic gives it, since "make one here" is what a user reaches
            // for in a directory that already exists.
            let dir = match command {
                "init" => PathBuf::from("."),
                _ => args
                    .get(1)
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from(".")),
            };
            // The document is named after its directory, which is what the
            // user has already chosen by making one.
            let name = std::fs::canonicalize(&dir)
                .ok()
                .as_deref()
                .and_then(Path::file_name)
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "document".to_string());
            match texrs::document::scaffold(&dir, &name) {
                Ok(path) => {
                    println!("wrote {}", path.display());
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&e),
            }
        }
        "bundle" => match args.get(1).map(String::as_str) {
            Some("fetch") => {
                let Some(url) = args.get(2) else {
                    return fail("`-X bundle fetch` needs a URL");
                };
                // The one place texrs uses the network, and only because it was
                // asked to. A build never comes here.
                match texrs::geturl::fetch(url) {
                    Ok(got) => {
                        println!("fetched {} bytes → {}", got.bytes, got.path.display());
                        println!(
                            "name it in Texrs.toml as: bundle = \"sha256:{}\"",
                            got.digest
                        );
                        ExitCode::SUCCESS
                    }
                    Err(e) => fail(&e),
                }
            }
            Some("list") => {
                let fetched = texrs::geturl::fetched();
                if fetched.is_empty() {
                    println!("no bundles fetched");
                }
                for digest in fetched {
                    match texrs::geturl::path_for(&digest) {
                        Some(path) => println!("sha256:{digest}  {}", path.display()),
                        None => println!("sha256:{digest}"),
                    }
                }
                ExitCode::SUCCESS
            }
            other => {
                eprintln!(
                    "texrs: `-X bundle` takes fetch or list, not {}",
                    other.unwrap_or("nothing")
                );
                ExitCode::from(1)
            }
        },
        "dvi" => {
            let Some(file) = args.get(1) else {
                return fail("`-X dvi` needs a .dvi file");
            };
            let dvi = match texrs::dvi::Dvi::open(file) {
                Ok(dvi) => dvi,
                Err(e) => return fail(&e),
            };
            // With a second file, the question is whether the two are the same
            // document rather than what one of them holds.
            let Some(other) = args.get(2) else {
                print!("{}", dvi.summary());
                return ExitCode::SUCCESS;
            };
            let against = match texrs::dvi::Dvi::open(other) {
                Ok(dvi) => dvi,
                Err(e) => return fail(&e),
            };
            let differences = dvi.compare(&against);
            if differences.is_empty() {
                println!("the same document");
                return ExitCode::SUCCESS;
            }
            for difference in &differences {
                println!("{difference:?}");
            }
            // A divergence is a failure: this is what a harness calls.
            ExitCode::from(1)
        }
        "show" => {
            let here = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            match texrs::document::Document::find_from(here) {
                Ok(document) => {
                    print!("{}", document.show());
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&e),
            }
        }
        "build" | "watch" | "dump" => {
            let mut profile: Option<String> = None;
            let mut interval = texrs::document::WATCH_INTERVAL;
            let mut rest = args[1..].iter();
            while let Some(arg) = rest.next() {
                match arg.as_str() {
                    "--profile" => match rest.next() {
                        Some(name) => profile = Some(name.clone()),
                        None => return fail("--profile needs a name"),
                    },
                    "--interval" => match rest.next().and_then(|ms| ms.parse().ok()) {
                        Some(ms) => interval = std::time::Duration::from_millis(ms),
                        None => return fail("--interval needs a number of milliseconds"),
                    },
                    other => return fail(&format!("unknown argument: {other}")),
                }
            }
            let here = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let document = match texrs::document::Document::find_from(here) {
                Ok(d) => d,
                Err(e) => return fail(&e),
            };
            if command == "dump" {
                return match document.dump(profile.as_deref()) {
                    Ok(text) => {
                        print!("{text}");
                        if !text.ends_with('\n') {
                            println!();
                        }
                        ExitCode::SUCCESS
                    }
                    Err(e) => fail(&e),
                };
            }
            if command == "build" {
                return match document.build(profile.as_deref()) {
                    Ok(built) => {
                        println!(
                            "built {} from {} input(s) → {}",
                            built.profile,
                            built.inputs.len(),
                            built.path.display()
                        );
                        ExitCode::SUCCESS
                    }
                    Err(e) => fail(&e),
                };
            }
            // Watch until the terminal interrupts it: the loop has no other
            // ending, which is what a watch is.
            let mut status = texrs::status::TexStatus::new();
            match document.watch(profile.as_deref(), interval, &mut status, &|| false) {
                Ok(_) => ExitCode::SUCCESS,
                Err(e) => fail(&e),
            }
        }
        // An unknown command is looked for on PATH as `texrs-<name>`, which is
        // how tectonic lets a command be added without changing the binary.
        other => match external_command(other, &args[1..]) {
            Some(code) => code,
            None => {
                eprintln!("texrs: unknown document command: {other}");
                eprint!("{USAGE}");
                ExitCode::from(1)
            }
        },
    }
}

/// Run `texrs-<name>` from PATH with `args`, if there is one.
///
/// The extension point tectonic has: a command nobody built into the binary is
/// a program on PATH, so `texrs -X lint` runs `texrs-lint`. `None` means there
/// is no such program, which is what keeps the unknown-command error reachable.
fn external_command(name: &str, args: &[String]) -> Option<ExitCode> {
    let program = format!("texrs-{name}");
    match std::process::Command::new(&program).args(args).status() {
        Ok(status) => Some(match status.code() {
            Some(0) => ExitCode::SUCCESS,
            Some(code) => ExitCode::from(code.min(255) as u8),
            // Killed by a signal. Any non-zero code beats reporting success.
            None => ExitCode::from(1),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            eprintln!("texrs: {program}: {e}");
            Some(ExitCode::from(1))
        }
    }
}

/// `--cache-stats`: what the cache holds and where it is, for a user asking why
/// a run was fast, or whether the cache is being used at all.
fn cache_stats() -> ExitCode {
    let path = texrs::script_cache::default_cache_path();
    if !texrs::script_cache::cache_enabled() {
        println!("cache: off (TEXRS_CACHE)");
        println!("would be: {}", path.display());
        return ExitCode::SUCCESS;
    }
    let Ok(cache) = texrs::script_cache::ScriptCache::open(&path) else {
        eprintln!("texrs: cannot open {}", path.display());
        return ExitCode::from(1);
    };
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    println!("cache:     {}", path.display());
    println!("documents: {}", cache.len());
    println!("size:      {size} bytes");
    ExitCode::SUCCESS
}

/// `--cache-clear`: delete the shard. The next run of every document compiles.
fn cache_clear() -> ExitCode {
    let path = texrs::script_cache::default_cache_path();
    let Ok(cache) = texrs::script_cache::ScriptCache::open(&path) else {
        eprintln!("texrs: cannot open {}", path.display());
        return ExitCode::from(1);
    };
    match cache.clear() {
        Ok(()) => {
            println!("cleared {}", path.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("texrs: {}: {e}", path.display());
            ExitCode::from(1)
        }
    }
}

/// The `(./doc.tex …)` line tex prints for a file.
///
/// The trailing space belongs to `\end`: tex prints `(./doc.tex MSGS )` when
/// `\end` stopped the run inside the file, and `(./doc.tex MSGS)` when the file
/// simply ran out. The difference is visible in any build log that greps for
/// one, so it is reproduced rather than normalised away.
fn file_line(path: &str, msgs: &str, ended: bool) -> String {
    let body = match msgs.is_empty() {
        true => String::new(),
        false => format!(" {msgs}"),
    };
    let close = match ended {
        true => " )",
        false => ")",
    };
    format!("(./{path}{body}{close}")
}

/// A file, then the command line as more input after it.
///
/// The file is run twice: once alone, to learn where its own output ends, and
/// once with the trailing input appended, because the two have to print on
/// opposite sides of the closing paren and the engine reports one message list.
/// It costs a second compile in the one case that asks for it -- a document
/// followed by command-line input -- and nothing at all otherwise.
fn run_with_trailing(
    path: &str,
    src: &str,
    extra: &str,
    interaction: texrs::cli::Interaction,
) -> ExitCode {
    let from_file = match texrs::run_messages_list(src) {
        Ok(m) => m,
        Err(e) => return fail(&e.0),
    };
    let all = match texrs::run_messages_list(&format!("{src}\n{extra}")) {
        Ok(m) => m,
        Err(e) => return fail(&e.0),
    };
    if interaction.prints() {
        let after = all[from_file.len().min(all.len())..].join(" ");
        let line = file_line(path, &from_file.join(" "), false);
        match after.is_empty() {
            true => println!("{line}"),
            false => println!("{line} {after}"),
        }
    }
    ExitCode::SUCCESS
}

/// Run TeX source that came from the command line rather than from a file.
///
/// tex prints the messages bare here — there is no file to open, so there is no
/// `(./name.tex …)` to print — which is what makes
/// `texrs '\catcode`\{=1 \message{hi}\end'` usable from a Makefile.
fn run_command_line(src: &str, interaction: texrs::cli::Interaction) -> ExitCode {
    match texrs::run_messages(src) {
        Ok(msgs) => {
            if interaction.prints() && !msgs.is_empty() {
                println!("{msgs}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e.0),
    }
}

/// tex writes its errors as `! <reason>.`; so does this.
fn fail(reason: &str) -> ExitCode {
    eprintln!("! {reason}.");
    ExitCode::from(1)
}

/// Print how often the parallel read-ahead ran, for `TEXRS_PRELEX_STATS`.
pub fn prelex_stats() {
    use std::sync::atomic::Ordering;
    eprintln!(
        "prelex calls={} chars={} drop_gen={} drop_exhaust={}",
        texrs::parallel::PRELEX_CALLS.load(Ordering::Relaxed),
        texrs::parallel::PRELEX_CHARS.load(Ordering::Relaxed),
        texrs::parallel::DROP_GEN.load(Ordering::Relaxed),
        texrs::parallel::DROP_EXHAUST.load(Ordering::Relaxed)
    );
}
