//! Differential parity fuzzer: real `tex` against this engine.
//!
//! Ported in shape from the sibling frontends' `parity_fuzz` binaries, and for
//! their reason. The oracle is expensive — a `tex` invocation costs ~0.5s of
//! process start and format load — so a fuzzer that runs one construct per
//! invocation spends its whole budget on startup. Each generated program
//! therefore packs many independent PROBES (`--probes`, default 40), so one
//! invocation of each engine exercises dozens of constructs; on divergence the
//! probe list is minimized to the one that actually diverges before it is
//! reported.
//!
//! Determinism is the other half. Every program is a pure function of its index
//! and the seed, so a divergence replays exactly:
//!
//! ```sh
//! cargo run --bin parity-fuzz -- --seed 7 --once     # just program 7
//! cargo run --bin parity-fuzz -- --programs 200      # a sweep
//! ```
//!
//! **Scope invariants**, the same ones the siblings keep:
//!
//! * Only constructs texrs implements are emitted. An unimplemented one would
//!   reproduce a `BUGS.md` entry rather than find anything.
//! * The known gaps are not generated: no `\count0` (the oracle preloads plain,
//!   where it is the page number and holds 1), no conditional inside an `\edef`
//!   body (texrs does not freeze it yet), no undefined control sequence (texrs
//!   prints the name where tex raises). Generating a gap only re-finds it.
//! * No probe can collide with another: every macro and register a probe uses
//!   carries its own index, so packing forty into one document changes none of
//!   their answers.
//! * Nothing nondeterministic: a probe's output is a pure function of its text.
//!
//! This replaces the shell fuzzer it grew out of. One implementation of a
//! harness, in the language the engine is written in, with no bash or perl in
//! the loop — and forty times fewer oracle invocations for the same coverage.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// The reference engine, and the version every expectation here was measured
/// against.
struct Oracle {
    program: String,
    version: String,
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = match Config::parse(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return std::process::ExitCode::from(2);
        }
    };
    if cfg.help {
        print!("{USAGE}");
        return std::process::ExitCode::SUCCESS;
    }

    let oracle = match find_oracle() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            return std::process::ExitCode::from(2);
        }
    };
    println!(
        "oracle: tex {} · {} program(s) × {} probes · seed {}",
        oracle.version, cfg.programs, cfg.probes, cfg.seed
    );

    let Ok(dir) = scratch_dir() else {
        eprintln!("parity-fuzz: cannot make a scratch directory");
        return std::process::ExitCode::from(2);
    };

    let mut diverged = 0usize;
    for i in 0..cfg.programs {
        let index = match cfg.once {
            true => cfg.seed,
            false => cfg.seed.wrapping_add(i as u64),
        };
        let probes = generate(index, cfg.probes);
        if !diverges(&probes, &oracle, &dir, cfg.timeout) {
            continue;
        }
        diverged += 1;
        let minimal = minimize(&probes, &oracle, &dir, cfg.timeout);
        let source = document(&minimal);
        let (want, got) = run_both(&source, &oracle, &dir, cfg.timeout);
        println!(
            "\nDIVERGES  program {index}, minimized to {} probe(s)\n  tex   : {want}\n  texrs : {got}\n{}",
            minimal.len(),
            indent(&source)
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
    match diverged {
        0 => {
            println!("\nPARITY: {} program(s) agree with tex.", cfg.programs);
            std::process::ExitCode::SUCCESS
        }
        n => {
            println!("\n{n}/{} program(s) diverge from tex", cfg.programs);
            std::process::ExitCode::from(u8::try_from(n.min(250)).unwrap_or(250))
        }
    }
}

const USAGE: &str = "\
usage: parity-fuzz [OPTIONS]

  --seed N        first program index (default 1)
  --programs N    how many to generate (default 50)
  --probes N      constructs packed into each one (default 40)
  --once          run only the program named by --seed
  --timeout SECS  per-engine limit for one program (default 20)
  -h, --help      print this
";

struct Config {
    seed: u64,
    programs: usize,
    probes: usize,
    once: bool,
    timeout: Duration,
    help: bool,
}

impl Config {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut cfg = Config {
            seed: 1,
            programs: 50,
            probes: 40,
            once: false,
            timeout: Duration::from_secs(20),
            help: false,
        };
        let mut i = 0;
        while i < args.len() {
            let arg = args[i].as_str();
            i += 1;
            let mut value = |name: &str| -> Result<String, String> {
                let v = args
                    .get(i)
                    .cloned()
                    .ok_or_else(|| format!("parity-fuzz: {name} needs a value"))?;
                i += 1;
                Ok(v)
            };
            match arg {
                "-h" | "--help" => cfg.help = true,
                "--once" => cfg.once = true,
                "--seed" => cfg.seed = value("--seed")?.parse().map_err(|_| "bad --seed")?,
                "--programs" => {
                    cfg.programs = value("--programs")?.parse().map_err(|_| "bad --programs")?
                }
                "--probes" => {
                    cfg.probes = value("--probes")?.parse().map_err(|_| "bad --probes")?
                }
                "--timeout" => {
                    let secs: u64 = value("--timeout")?.parse().map_err(|_| "bad --timeout")?;
                    cfg.timeout = Duration::from_secs(secs);
                }
                other => return Err(format!("parity-fuzz: unknown option: {other}")),
            }
        }
        if cfg.once {
            cfg.programs = 1;
        }
        Ok(cfg)
    }
}

/// The pinned oracle version, read out of BUGS.md so it cannot drift from the
/// document that quotes it — the same gate `scripts/lib.sh` applies.
fn find_oracle() -> Result<Oracle, String> {
    let bugs = std::fs::read_to_string(repo().join("BUGS.md")).map_err(|e| e.to_string())?;
    let want = bugs
        .lines()
        .find_map(|l| l.split("measured against **tex ").nth(1))
        .and_then(|r| r.split("**").next())
        .ok_or("parity-fuzz: no `measured against **tex X.Y**` line in BUGS.md")?;

    let out = Command::new("tex")
        .arg("--version")
        .output()
        .map_err(|_| "parity-fuzz: no `tex` on PATH — the fuzzer has no oracle".to_string())?;
    let banner = String::from_utf8_lossy(&out.stdout);
    let version = banner
        .lines()
        .next()
        .and_then(|l| l.split("TeX ").nth(1))
        .and_then(|v| v.split_whitespace().next())
        .ok_or("parity-fuzz: `tex --version` did not report a version")?;
    if version != want {
        return Err(format!(
            "parity-fuzz: oracle is tex {version}, but everything here was measured against {want}.\n\
             A mismatched oracle reports a different divergence set, not an error."
        ));
    }
    Ok(Oracle {
        program: "tex".into(),
        version: version.to_string(),
    })
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn scratch_dir() -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("texrs-parity-fuzz-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

// ── the generator ───────────────────────────────────────────────────────────

/// Numerical Recipes' 32-bit LCG. Small, reproducible, and its low bits are
/// never used — the same generator the shell harness used, so a seed means the
/// same thing across both.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_add(1) & 0xFFFF_FFFF)
    }
    fn next(&mut self, m: usize) -> usize {
        self.0 = (self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223)) & 0xFFFF_FFFF;
        ((self.0 >> 16) as usize) % m.max(1)
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.next(xs.len())]
    }
}

/// The registers a probe may touch.
///
/// Never `\count0`: the oracle loads the plain format, where it is the page
/// number and already holds 1, while texrs starts every register at INITEX
/// zero. Reading it compares two engines that were never in the same state.
fn reg(rng: &mut Rng) -> usize {
    1 + rng.next(9)
}

/// A probe index as letters: 0 -> `a`, 25 -> `z`, 26 -> `ba`.
fn letters(mut id: usize) -> String {
    let mut out = String::new();
    loop {
        out.push((b'a' + (id % 26) as u8) as char);
        id /= 26;
        if id == 0 {
            return out;
        }
    }
}

const WORDS: &[&str] = &[
    "ALPHA", "BETA", "GAMMA", "DELTA", "EPS", "ZETA", "ETA", "THETA",
];

/// One probe: a self-contained construct, tagged with its index so a failure
/// names itself and two probes in one document cannot collide.
fn probe(rng: &mut Rng, id: usize) -> String {
    let w = rng.pick(WORDS);
    let r = reg(rng);
    // A control WORD is letters only, so `\\m7` is `\\m` followed by a `7` --
    // every probe would define the same macro and the delimiters would decide
    // what expanded. The name is spelled in letters so forty probes really are
    // forty macros.
    let m = format!("\\m{}", letters(id));
    match rng.next(10) {
        0 => format!("\\count{r}={n} \\message{{p{id}:\\the\\count{r} }}", n = rng.next(2000)),
        1 => format!(
            "\\count{r}={a} \\advance\\count{r} by {b} \\multiply\\count{r} by {c} \\message{{p{id}:\\the\\count{r} }}",
            a = rng.next(200),
            b = rng.next(100),
            c = 1 + rng.next(9)
        ),
        2 => format!(
            "\\count{r}={a} \\divide\\count{r} by {d} \\message{{p{id}:\\the\\count{r} }}",
            a = rng.next(1000),
            d = 1 + rng.next(9)
        ),
        3 => format!(
            "\\count{r}={a} \\message{{p{id}:\\ifnum\\count{r}>{b} BIG\\else SMALL\\fi }}",
            a = rng.next(50),
            b = rng.next(50)
        ),
        4 => format!(
            "\\count{r}={a} \\message{{p{id}:\\ifodd\\count{r} ODD\\else EVEN\\fi }}",
            a = rng.next(50)
        ),
        5 => format!(
            "\\count{r}={a} \\message{{p{id}:\\ifcase\\count{r} Z\\or O\\or T\\else M\\fi }}",
            a = rng.next(5)
        ),
        6 => format!("\\def{m}#1{{{w}-#1}}\\message{{p{id}:{m}{{{x}}} }}", x = rng.pick(WORDS)),
        7 => format!(
            "\\def{m}{{{w}}}\\message{{p{id}:{m} }}{{\\def{m}{{{x}}}\\message{{p{id}i:{m} }}}}\\message{{p{id}o:{m} }}",
            x = rng.pick(WORDS)
        ),
        8 => format!(
            "\\count{r}={a} \\edef{m}{{\\the\\count{r} }}\\count{r}={b} \\message{{p{id}:{m}\\the\\count{r} }}",
            a = rng.next(100),
            b = rng.next(100)
        ),
        _ => format!(
            "\\def{m}{{{w}}}\\message{{p{id}:\\string{m} \\number{n} \\csname m{name}\\endcsname }}",
            n = rng.next(1000),
            name = letters(id)
        ),
    }
}

/// A program's probes, as a pure function of its index.
fn generate(index: u64, count: usize) -> Vec<String> {
    let mut rng = Rng::new(index);
    (0..count).map(|id| probe(&mut rng, id)).collect()
}

/// The document a probe list becomes: the preamble every probe assumes, then
/// the probes, then `\end`.
fn document(probes: &[String]) -> String {
    let mut src = String::from("\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6\n");
    for p in probes {
        src.push_str(p);
        src.push('\n');
    }
    src.push_str("\\end\n");
    src
}

// ── running both engines ────────────────────────────────────────────────────

/// What tex and texrs each print for `source`.
fn run_both(source: &str, oracle: &Oracle, dir: &Path, timeout: Duration) -> (String, String) {
    let path = dir.join("case.tex");
    if std::fs::write(&path, source).is_err() {
        return ("<unwritable>".into(), "<unwritable>".into());
    }
    let want = run_with_timeout(
        Command::new(&oracle.program)
            .arg("-interaction=nonstopmode")
            .arg("case.tex")
            // tex wraps at 79 columns otherwise, and the comparison would be
            // with the wrapping rather than with the output.
            .env("max_print_line", "8000")
            .current_dir(dir),
        timeout,
    );
    let got = run_with_timeout(
        Command::new(texrs_binary()).arg(&path).current_dir(dir),
        timeout,
    );
    (messages_of(&want), messages_of(&got))
}

/// This binary lives beside the `texrs` one cargo built.
fn texrs_binary() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("texrs")))
        .unwrap_or_else(|| PathBuf::from("texrs"))
}

fn run_with_timeout(cmd: &mut Command, timeout: Duration) -> String {
    use std::process::Stdio;
    let Ok(mut child) = cmd.stdout(Stdio::piped()).stderr(Stdio::null()).spawn() else {
        return "<unstartable>".into();
    };
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                return "<timeout>".into();
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => return "<unwaitable>".into(),
        }
    }
    let out = child
        .wait_with_output()
        .map(|o| o.stdout)
        .unwrap_or_default();
    String::from_utf8_lossy(&out).into_owned()
}

/// The `\message` stream out of `(./case.tex … )`.
///
/// The same extraction `tests/common/mod.rs` makes, for the same reason: two
/// harnesses that extract differently are asking the oracle two questions.
fn messages_of(out: &str) -> String {
    let Some(at) = out.find("(./") else {
        return String::new();
    };
    let rest = &out[at + 3..];
    let Some((_, after)) = rest.split_once(".tex") else {
        return String::new();
    };
    let body = match after.rfind(')') {
        Some(end) => &after[..end],
        None => after,
    };
    body.replace('\n', "").trim().to_string()
}

fn diverges(probes: &[String], oracle: &Oracle, dir: &Path, timeout: Duration) -> bool {
    let (want, got) = run_both(&document(probes), oracle, dir, timeout);
    want != got
}

/// The smallest probe list that still diverges.
///
/// One probe alone first, because that is the usual case and it is one run per
/// probe; then a greedy drop for the divergence that needs two probes to
/// interact.
fn minimize(probes: &[String], oracle: &Oracle, dir: &Path, timeout: Duration) -> Vec<String> {
    for p in probes {
        let one = vec![p.clone()];
        if diverges(&one, oracle, dir, timeout) {
            return one;
        }
    }
    let mut cur = probes.to_vec();
    let mut i = 0;
    while i < cur.len() && cur.len() > 1 {
        let mut trial = cur.clone();
        trial.remove(i);
        match diverges(&trial, oracle, dir, timeout) {
            true => cur = trial,
            false => i += 1,
        }
    }
    cur
}

fn indent(source: &str) -> String {
    source
        .lines()
        .map(|l| format!("    {l}\n"))
        .collect::<String>()
}

/// Flush anything buffered before a long run, so a watched sweep prints as it
/// goes rather than at the end.
#[allow(dead_code)]
fn flush() {
    let _ = std::io::stdout().flush();
}
