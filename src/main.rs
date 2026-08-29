//! The `texrs` driver.
//!
//! Running a file prints the `\message` stream the way `tex` prints it on the
//! terminal — `(./file.tex <messages> )` — which is the comparison
//! `scripts/parity.sh` makes against the real engine. The introspection flags
//! stop earlier in the same pipeline: `--dump-tokens` after the mouth,
//! `--disasm` after lowering, so each stage can be read on its own.

use std::process::ExitCode;

const USAGE: &str = "\
usage: texrs [OPTIONS] FILE.tex

  --lsp           speak the Language Server Protocol over stdio
  --dump-tokens   print the mouth's token stream and exit
  --disasm        print the lowered fusevm bytecode and exit
  --no-cache      compile this run rather than reading the bytecode cache
  --cache-stats   say what the cache holds and where, and exit
  --cache-clear   delete the cache and exit
  -h, --help      print this and exit
  --version       print the version banner and exit
";

fn main() -> ExitCode {
    let mut path: Option<String> = None;
    let mut dump_tokens = false;
    let mut disasm = false;
    let mut no_cache = false;

    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--version" => {
                println!(
                    "texrs {} (TeX 3.141592653 mouth+expander)",
                    env!("CARGO_PKG_VERSION")
                );
                return ExitCode::SUCCESS;
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "--lsp" => {
                return match texrs::lsp::run() {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => fail(&e),
                }
            }
            "--dump-tokens" => dump_tokens = true,
            "--disasm" => disasm = true,
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
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
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
        return match texrs::compile(&src) {
            Ok(chunk) => {
                print!("{}", chunk.disassemble());
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e.0),
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
