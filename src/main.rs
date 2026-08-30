//! The `texrs` driver.
//!
//! Running a file prints the `\message` stream the way `tex` prints it on the
//! terminal — `(./file.tex <messages> )` — which is the comparison
//! `scripts/parity.sh` makes against the real engine. The introspection flags
//! stop earlier in the same pipeline: `--dump-tokens` after the mouth,
//! `--disasm` after lowering, so each stage can be read on its own.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
usage: texrs [OPTIONS] FILE.tex
       texrs -X new [DIR]           make a document (Texrs.toml + index.tex)
       texrs -X build [--profile P] build the document this directory is in
       texrs -X watch [--profile P] rebuild it whenever an input changes

  --repl          start the interactive prompt
  --lsp           speak the Language Server Protocol over stdio
  --dap           speak the Debug Adapter Protocol over stdio
  --dump-tokens   print the mouth's token stream and exit
  --disasm        print the lowered fusevm bytecode and exit
  --aot           compile the document to a standalone native executable
  --tiers         run it, then report which fusevm tier took its bytecode
  --profile NAME  which output of the document to build (-X build, -X watch)
  --interval MS   how often -X watch looks for a change (default 250)
  --no-cache      compile this run rather than reading the bytecode cache
  --cache-stats   say what the cache holds and where, and exit
  --cache-clear   delete the cache and exit
  -h, --help      print this and exit
  --version       print the version banner and exit
";

fn main() -> ExitCode {
    // The document commands come first: `-X` takes over the whole invocation,
    // as tectonic's V2 interface does, so nothing below has to know about them.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("-X") {
        return run_document(&args[1..]);
    }

    let mut path: Option<String> = None;
    let mut dump_tokens = false;
    let mut disasm = false;
    let mut aot = false;
    let mut tiers = false;
    let mut no_cache = false;

    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--version" => {
                println!("{}", texrs::banner::version_banner());
                return ExitCode::SUCCESS;
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "--repl" => {
                return match texrs::repl::run() {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => fail(&e),
                }
            }
            "--lsp" => {
                return match texrs::lsp::run() {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => fail(&e),
                }
            }
            "--dap" => {
                return match texrs::dap::run() {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => fail(&e),
                }
            }
            "--dump-tokens" => dump_tokens = true,
            "--disasm" => disasm = true,
            "--aot" => aot = true,
            "--tiers" => tiers = true,
            "--no-cache" => no_cache = true,
            "--cache-stats" => return cache_stats(),
            "--cache-clear" => return cache_clear(),
            other if other.starts_with('-') && other.len() > 1 => {
                eprintln!("texrs: unknown option: {other}");
                return ExitCode::from(1);
            }
            other => path = Some(other.to_string()),
        }
    }

    let Some(path) = path else {
        eprint!("{USAGE}");
        return ExitCode::from(1);
    };
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

    // The cache keys on the file, so it is only used when there is one and the
    // run has not asked to go without it.
    let run = match no_cache {
        true => texrs::run_messages(&src),
        false => texrs::run_messages_cached(std::path::Path::new(&path), &src),
    };
    match run {
        Ok(msgs) => {
            let body = match msgs.is_empty() {
                true => String::new(),
                false => format!(" {msgs}"),
            };
            println!("(./{path}{body} )");
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e.0),
    }
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
        "new" => {
            let dir = args
                .get(1)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
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
        "build" | "watch" => {
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
        other => {
            eprintln!("texrs: unknown document command: {other}");
            eprint!("{USAGE}");
            ExitCode::from(1)
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

/// tex writes its errors as `! <reason>.`; so does this.
fn fail(reason: &str) -> ExitCode {
    eprintln!("! {reason}.");
    ExitCode::from(1)
}
