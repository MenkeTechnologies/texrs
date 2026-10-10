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
//! * Nothing only plain sets: the oracle has plain loaded and texrs starts from
//!   INITEX, so `~` is active in one and an other character in the other, `^`
//!   and `$` differ the same way, and `\bgroup`, `\hp`, `\mp` and friends are
//!   macros in one and undefined in the other. Probe macros carry a prefix no
//!   plain macro starts with, and no probe uses a character plain makes active.
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

use texrs::parity::Oracle;

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

    let oracle = match texrs::parity::oracle(&repo()) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("parity-fuzz: {e}");
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
    fn word<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.next(xs.len())]
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

/// How many distinct constructs `probe` can emit.
const PROBE_KINDS: usize = 62;

/// Parameters the probes may assign: none of them changes what the terminal
/// shows, and none is read-only.
const INT_PARAMS: &[&str] = &[
    "pretolerance",
    "tolerance",
    "linepenalty",
    "hyphenpenalty",
    "exhyphenpenalty",
    "clubpenalty",
    "widowpenalty",
    "displaywidowpenalty",
    "brokenpenalty",
    "binoppenalty",
    "relpenalty",
    "predisplaypenalty",
    "postdisplaypenalty",
    "interlinepenalty",
    "doublehyphendemerits",
    "finalhyphendemerits",
    "adjdemerits",
    "delimiterfactor",
    "looseness",
    "hbadness",
    "vbadness",
    "maxdeadcycles",
    "hangafter",
    "floatingpenalty",
    "fam",
    "defaulthyphenchar",
    "defaultskewchar",
    "language",
    "lefthyphenmin",
    "righthyphenmin",
    "holdinginserts",
    "uchyph",
    "showboxbreadth",
    "showboxdepth",
    "outputpenalty",
];

const DIMEN_PARAMS: &[&str] = &[
    "parindent",
    "mathsurround",
    "lineskiplimit",
    "hsize",
    "vsize",
    "maxdepth",
    "splitmaxdepth",
    "boxmaxdepth",
    "hfuzz",
    "vfuzz",
    "delimitershortfall",
    "nulldelimiterspace",
    "scriptspace",
    "predisplaysize",
    "displaywidth",
    "displayindent",
    "overfullrule",
    "hangindent",
    "hoffset",
    "voffset",
    "emergencystretch",
];

const GLUE_PARAMS: &[&str] = &[
    "lineskip",
    "baselineskip",
    "parskip",
    "abovedisplayskip",
    "belowdisplayskip",
    "abovedisplayshortskip",
    "belowdisplayshortskip",
    "leftskip",
    "rightskip",
    "topskip",
    "splittopskip",
    "tabskip",
    "spaceskip",
    "xspaceskip",
    "parfillskip",
];

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
    let m = format!("\\fzm{}", letters(id));
    let h = format!("\\fzh{}", letters(id));
    match rng.next(PROBE_KINDS) {
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
        9 => format!(
            "\\def{m}{{{w}}}\\message{{p{id}:\\string{m} \\number{n} \\csname fzm{name}\\endcsname }}",
            n = rng.next(1000),
            name = letters(id)
        ),
        10 => format!("\\uppercase{{\\message{{p{id}:{w} abc 12}}}}\\lowercase{{\\message{{P{id}:{w} ABC}}}}"),
        11 => format!(
            "\\def{m}{{\\message{{p{id}:AFTER\\the\\count{r} }}}}\\afterassignment{m}\\count{r}={a} \\message{{p{id}:SET\\the\\count{r} }}",
            a = rng.next(100)
        ),
        12 => format!(
            "\\def{m}{{\\futurelet{h}{h}z}}\\def{h}z{{\\message{{p{id}:\\meaning{h} }}}}{m}{x}",
            x = rng.pick(&["\\relax", "\\par", "{}", "\\count1=0 "])
        ),
        13 => format!("\\def{m}#1#2.{{{w}#2#1}}\\message{{p{id}:\\meaning{m} }}\\message{{p{id}:{m} x y.}}"),
        14 => format!(
            "\\def{h}{{{w}}}\\def{m}{{\\expandafter\\def\\expandafter{h}\\expandafter{{X{h}}}}}{m}\\message{{p{id}:\\meaning{h} }}"
        ),
        15 => format!(
            "\\def{m}#1#2{{\\if#1#2T\\else F\\fi \\ifcat#1#2T\\else F\\fi \\ifx#1#2T\\else F\\fi}}\\message{{p{id}:{m} {a}{b} }}",
            a = rng.pick(&["a", "A", "1", "\\relax ", "@"]),
            b = rng.pick(&["a", "b", "1", "\\relax ", "@"])
        ),
        16 => format!(
            "\\dimen{r}={a}pt \\message{{p{id}:\\ifdim\\dimen{r}>{b}pt GT\\else LE\\fi \\ifdim\\dimen{r}={b}pt EQ\\fi }}",
            a = rng.next(30),
            b = rng.next(30)
        ),
        17 => format!(
            "\\dimen{r}={a}.{f}pt \\multiply\\dimen{r} by {c} \\advance\\dimen{r} by -{b}pt \\divide\\dimen{r} by {d} \\message{{p{id}:\\the\\dimen{r} }}",
            a = rng.next(100),
            f = rng.next(1000),
            b = rng.next(50),
            c = 1 + rng.next(9),
            d = 1 + rng.next(9)
        ),
        18 => format!(
            "\\skip{r}={a}pt plus {b}fil minus {c}pt \\advance\\skip{r} by {c}pt plus {a}fil \\message{{p{id}:\\the\\skip{r} }}",
            a = rng.next(20),
            b = rng.next(20),
            c = rng.next(20)
        ),
        19 => format!(
            "\\toks{r}={{{w}#{x}}}\\edef{m}{{\\the\\toks{r}}}\\message{{p{id}:\\the\\toks{r} \\meaning{m} }}",
            x = rng.next(9)
        ),
        20 => format!("\\message{{p{id}:\\romannumeral{n} \\number-{n} \\number\"{n:X} \\number'{n:o} }}", n = rng.next(4000)),
        21 => format!(
            "\\def{m}{{\\message{{p{id}:AG}}}}{{\\aftergroup{m}\\message{{p{id}:IN}}}}\\message{{p{id}:OUT}}"
        ),
        22 => format!(
            "{{\\catcode`\\@=11 \\message{{p{id}:\\the\\catcode`\\@ \\string @x}}}}\\message{{p{id}:\\the\\catcode`\\@ }}"
        ),
        23 => format!(
            "\\edef{m}{{\\noexpand{h}{w}\\string{h}}}\\message{{p{id}:\\meaning{m} }}"
        ),
        24 => format!(
            "\\count{r}={a} \\message{{p{id}:\\ifcase\\count{r} A\\or B\\or\\ifcase\\count{r} x\\or y\\or Z\\fi\\else E\\fi }}",
            a = rng.next(5) as i32 - 1
        ),
        25 => format!(
            "{{\\escapechar={e} \\message{{p{id}:\\string{h} \\meaning{m} }}}}",
            e = rng.pick(&["`\\!", "-1", "`\\A", "256"])
        ),
        26 => format!(
            "\\lccode`\\{u}=`\\{l} \\uccode`\\{l}=`\\{u} \\lowercase{{\\message{{p{id}:{u}{l}{u} }}}}\\uppercase{{\\message{{P{id}:{u}{l}{u} }}}}",
            u = rng.pick(&["X", "Y", "Z"]),
            l = rng.pick(&["x", "y", "z"])
        ),
        27 => format!(
            "\\count{r}=1 {{\\global\\advance\\count{r} by {a} \\count{r}=7 \\global\\count{r}=\\count{r} }}\\message{{p{id}:\\the\\count{r} }}",
            a = rng.next(9)
        ),
        28 => format!(
            "\\let{h}={a}\\def{m}{{{a}}}\\message{{p{id}:\\ifx{h}{m}S\\else D\\fi \\ifx{h}{a}S\\else D\\fi \\meaning{h} }}",
            a = rng.pick(&["\\relax", "\\par", "\\undefinedprobe", "A", "\\count"])
        ),
        29 => format!(
            "\\chardef{h}={n} \\countdef{m}={r} {m}={n} \\message{{p{id}:\\number{h} \\the{m} \\meaning{h} \\meaning{m} }}",
            n = rng.next(256)
        ),
        30 => format!(
            "\\expandafter\\ifx\\csname unprobe{n}\\endcsname\\relax \\message{{p{id}:UNDEF}}\\fi \\message{{p{id}:\\expandafter\\meaning\\csname unprobe{n}\\endcsname }}",
            n = letters(id)
        ),
        31 => format!(
            "\\def{m}#1{{\\ifx#1\\relax END\\else[#1]\\expandafter{m}\\fi}}\\message{{p{id}:{m} {w}\\relax }}"
        ),
        32 => format!(
            "\\def{m}#1#{{\\message{{p{id}:#1|}}\\def{h}}}{m} {w}x{{Q}}\\message{{p{id}:\\meaning{h} }}"
        ),
        33 => format!(
            "{{\\endlinechar={e} \\message{{p{id}:\\the\\endlinechar}}}}\\message{{p{id}:\\the\\endlinechar}}",
            e = rng.pick(&["-1", "`\\A", "13"])
        ),
        34 => format!(
            "\\count{r}={a} \\message{{p{id}:\\number\\count{r} \\ifnum\\count{r}<0 NEG\\fi \\ifnum-\\count{r}<{b} LT\\fi }}",
            a = rng.next(40) as i32 - 20,
            b = rng.next(10)
        ),
        // §236 integer parameters: set, read through `\the` and `\number`,
        // and changed by the three arithmetic commands.
        35 => {
            let p = rng.word(INT_PARAMS);
            format!(
                "\\{p}={a} \\advance\\{p} by {b} \\multiply\\{p} by {c} \\message{{p{id}:\\the\\{p} \\number\\{p} }}",
                a = rng.next(3000) as i32 - 1000,
                b = rng.next(100),
                c = 1 + rng.next(5)
            )
        }
        // §247 dimension parameters.
        36 => {
            let p = rng.word(DIMEN_PARAMS);
            format!(
                "\\{p}={a}.{f}pt \\advance\\{p} by {b}pt \\multiply\\{p} by {c} \\message{{p{id}:\\the\\{p} \\number\\{p} }}",
                a = rng.next(100),
                f = rng.next(1000),
                b = rng.next(20),
                c = 1 + rng.next(5)
            )
        }
        // §224 glue parameters, and the math glue ones in mu.
        37 => {
            let p = rng.word(GLUE_PARAMS);
            format!(
                "\\{p}={a}pt plus {b}{fil} minus {c}pt \\advance\\{p} by {c}pt \\message{{p{id}:\\the\\{p} }}",
                a = rng.next(30),
                b = rng.next(30),
                c = rng.next(30),
                fil = rng.word(&["pt", "fil", "fill", "filll"])
            )
        }
        38 => {
            let p = rng.word(&["thinmuskip", "medmuskip", "thickmuskip"]);
            format!(
                "\\{p}={a}mu plus {b}{fil} minus {c}mu \\message{{p{id}:\\the\\{p} }}",
                a = rng.next(30),
                b = rng.next(30),
                c = rng.next(30),
                fil = rng.word(&["mu", "fil", "fill"])
            )
        }
        // §230 token parameters and their registers.
        39 => {
            let p = rng.word(&["everypar", "everymath", "everydisplay", "everyhbox", "everyvbox", "everyjob", "everycr", "errhelp", "output"]);
            format!(
                "\\{p}={{{w}#{x}\\relax}}\\message{{p{id}:\\the\\{p} \\meaning\\{p} }}{{\\{p}={{}}\\message{{p{id}i:\\the\\{p}}}}}\\message{{p{id}o:\\the\\{p} }}",
                x = rng.next(9)
            )
        }
        // `\meaning` of a character of each category (§298), made by changing
        // a character's category rather than by writing the token.
        40 => {
            let cat = rng.pick(&[1, 2, 3, 4, 6, 7, 8, 10, 11, 12, 13]);
            format!(
                "{{\\catcode`\\!={cat} \\message{{p{id}:\\meaning!}}}}\\message{{p{id}:\\meaning!}}"
            )
        }
        // `\string` and `\meaning` of a control symbol, under `\escapechar`.
        41 => {
            let c = rng.word(&["\\relax", "\\par", "\\ ", "\\/", "\\-", "\\@", "\\:", "\\<", "\\(", "\\)", "\\[", "\\]"]);
            let e = rng.word(&["`\\\\", "`\\!", "-1", "0", "`\\a", "`\\ "]);
            format!(
                "{{\\escapechar={e} \\message{{p{id}:\\string{c} \\meaning{c}}}}}"
            )
        }
        // `\number` of a character constant (§442), including the ones that
        // end in the optional space.
        42 => {
            let c = rng.word(&["a", "A", "0", "\\a", "\\\\", "\\ ", "\\{", "\\}", "\\#", "\\%", "^^A", "^^?", "^^7f", "\"", "'"]);
            format!("\\catcode`\\^=7 \\message{{p{id}:\\number`{c} \\number`{c}\\relax x}}")
        }
        // Overflow in the three arithmetic commands (§1236), on integers.
        43 => {
            let v = rng.word(&["2147483647", "-2147483647", "1000000000", "65536", "46341", "-46341", "1"]);
            let op = rng.word(&["multiply", "divide"]);
            let by = rng.word(&["2", "3", "0", "-1", "46341", "2147483647", "65536"]);
            format!(
                "\\count{r}={v} \\{op}\\count{r} by {by} \\message{{p{id}:\\the\\count{r} }}"
            )
        }
        // The same on dimensions and glue: §1236 reports `Arithmetic
        // overflow`, and §460 `Dimension too large` where a constant is read.
        44 => {
            let v = rng.word(&["16383pt", "16383.99998pt", "-16383pt", "8192pt", "1sp", "10000pt"]);
            let op = rng.word(&["multiply", "divide", "advance"]);
            let by = match op {
                "advance" => rng.word(&["1pt", "16383pt", "-16383pt", "1sp"]),
                _ => rng.word(&["2", "3", "0", "-1", "65536", "7"]),
            };
            let kind = rng.word(&["dimen", "skip"]);
            format!(
                "\\{kind}{r}={v} \\{op}\\{kind}{r} by {by} \\message{{p{id}:\\the\\{kind}{r} }}"
            )
        }
        45 => {
            let t = rng.word(&["16384pt", "16383.999999pt", "16384.0pt", "1073741824sp", "1073741823sp", "99999pt", "1000000in", "2000mm", "300000cm", "1000000bp"]);
            format!("\\dimen{r}={t} \\message{{p{id}:\\the\\dimen{r} }}")
        }
        // The five code tables (§1232), their range checks, and what
        // `\uppercase` and `\lowercase` make of them.
        46 => {
            let table = rng.word(&["uccode", "lccode", "sfcode", "mathcode", "delcode", "catcode"]);
            let v = rng.word(&["0", "1", "65", "255", "256", "1000", "32768", "32769", "-1", "16777215", "16777216", "15", "16"]);
            format!(
                "\\{table}`\\!={v} \\message{{p{id}:\\the\\{table}`\\!}}\\uccode`\\a=`\\Q \\lccode`\\a=`\\z \\uppercase{{\\message{{P{id}:abAB}}}}\\lowercase{{\\message{{p{id}:abAB}}}}"
            )
        }
        47 => format!(
            "{{\\uccode`\\b=`\\Y \\uppercase{{\\message{{p{id}:ab}}}}}}\\uppercase{{\\message{{p{id}:ab}}}}\\uppercase{{\\def{m}{{ab\\message{{p{id}:x}}}}}}\\message{{p{id}:\\meaning{m} }}"
        ),
        // `\afterassignment` against the kinds of assignment (§1269).
        48 => {
            let a = rng.word(&[
                "\\def{h}{}",
                "\\let{h}=\\relax",
                "\\count1=5 ",
                "\\advance\\count1 by 1 ",
                "\\toks1={}",
                "\\chardef{h}=65 ",
                "\\global\\count1=3 ",
                "\\relax\\count1=1 ",
                "\\catcode`\\@=12 ",
                "\\uccode`\\a=`\\A ",
                "\\dimen1=1pt ",
                "\\skip1=1pt ",
                "\\tolerance=100 ",
                "\\everypar={}",
            ]);
            let a = a.replace("{h}", &h);
            format!(
                "\\def{m}{{\\message{{p{id}:AFTER}}}}\\afterassignment{m}{a}\\message{{p{id}:NEXT}}\\afterassignment{m}\\afterassignment\\relax{a}\\message{{p{id}:END}}"
            )
        }
        // `\aftergroup`, in order and across nested and `\begingroup` groups
        // (§1271).
        49 => {
            let open = rng.word(&["{", "\\begingroup "]);
            let close = if open == "{" { "}" } else { "\\endgroup " };
            format!(
                "\\def{m}{{\\message{{p{id}:A}}}}\\def{h}{{\\message{{p{id}:B}}}}{open}\\aftergroup{m}{{\\aftergroup{h}\\aftergroup{m}}}\\aftergroup{h}\\message{{p{id}:IN}}{close}\\message{{p{id}:OUT}}"
            )
        }
        // `\expandafter` and `\noexpand` chains (§366, §367).
        50 => {
            let n = rng.next(30);
            format!(
                "\\def{m}#1{{[#1]}}\\expandafter\\message\\expandafter{{\\romannumeral{n} \\number{n}{m}{{{w}}}}}\\expandafter\\expandafter\\expandafter\\message\\expandafter\\expandafter\\expandafter{{\\csname relax\\endcsname {w}}}"
            )
        }
        51 => format!(
            "\\def{m}{{{w}}}\\edef{h}{{\\expandafter\\noexpand\\csname fzq{name}\\endcsname\\noexpand{m}\\noexpand\\noexpand{m}\\expandafter\\noexpand{m}}}\\message{{p{id}:\\meaning{h} }}",
            name = letters(id)
        ),
        52 => format!(
            "\\def{m}{{{w}}}\\message{{p{id}:\\noexpand{m}\\expandafter\\noexpand\\expandafter{m}\\noexpand\\csname relax\\endcsname\\noexpand}}\\message{{p{id}:\\ifx\\noexpand{m}{m}T\\else F\\fi \\if\\noexpand{m}\\relax T\\else F\\fi \\ifcat\\noexpand{m}\\relax T\\else F\\fi}}"
        ),
        // Register and character numbers out of range (§433, §434, §1224).
        53 => {
            let n = rng.word(&["-1", "256", "300", "32767", "255", "0", "32768"]);
            let k = rng.word(&["count", "dimen", "skip", "toks", "muskip"]);
            let body = match k {
                "toks" => "{}",
                "muskip" => "1mu",
                "dimen" => "1pt",
                "skip" => "1pt",
                _ => "1",
            };
            format!(
                "\\{k}{n}={body} \\message{{p{id}:\\the\\{k}{n} }}\\{k}def{m}={n} \\message{{p{id}:\\meaning{m} }}"
            )
        }
        54 => {
            let n = rng.word(&["\"7FFF", "\"8000", "\"8001", "\"FFFF", "\"10000", "\"1000", "0", "32768", "-1"]);
            format!(
                "\\mathchardef{m}={n} \\message{{p{id}:\\meaning{m} \\number{m} \\the{m} }}\\chardef{h}={n} \\message{{p{id}:\\meaning{h} \\number{h} }}"
            )
        }
        // Number syntax: radix prefixes, signs, and what ends a number.
        55 => {
            let t = rng.word(&[
                "\"FF", "\"ff", "'777", "'8", "\"G", "--5", "-+-5", "+-+7", "5.5", "\"7FFFFFFF", "\"80000000", "'17777777777", "'20000000000", "2147483648", "-2147483648", "\\relax5", " 12 ", "007", "`a", "`ab",
            ]);
            format!("\\message{{p{id}:\\number{t}x \\romannumeral{t}x}}")
        }
        // A control sequence name made by `\csname` out of odd characters.
        56 => {
            let name = rng.word(&["a b", " ", "", "\\string\\x", "12", "{", "a\\relax"]);
            let name = if name.contains('{') { "a" } else { name };
            format!(
                "\\expandafter\\def\\csname {name}\\endcsname{{{w}}}\\expandafter\\message\\expandafter{{\\csname {name}\\endcsname \\expandafter\\string\\csname {name}\\endcsname}}"
            )
        }
        // Registers read where a quantity of another kind is scanned (§413
        // coercions), with signs.
        57 => {
            let a = rng.word(&["\\count{r} ", "\\dimen{r} ", "\\skip{r} ", "-\\count{r} ", "-\\dimen{r} ", "--\\skip{r} ", "\\catcode`\\a ", "\\hoffset ", "\\voffset "]);
            let into = rng.word(&["count", "dimen", "skip"]);
            let a = a.replace("{r}", &r.to_string());
            format!(
                "\\count{r}={x} \\dimen{r}={y}pt \\skip{r}={z}pt plus 1fil \\{into}{s}={a}\\message{{p{id}:\\the\\{into}{s} }}",
                x = rng.next(100),
                y = rng.next(100),
                z = rng.next(100),
                s = 1 + rng.next(9)
            )
        }
        58 => format!(
            "\\count{r}={a} \\dimen{r}=\\count{r}pt \\skip{r}=\\count{r}\\dimen{r} \\count{s}=\\dimen{r} \\message{{p{id}:\\the\\dimen{r} \\the\\skip{r} \\the\\count{s} }}",
            a = rng.next(40) as i32 - 20,
            s = 1 + rng.next(9)
        ),
        59 => format!(
            "\\toks{r}={{{w}}}\\toks{s}={{\\the\\toks{r}\\the\\toks{r} #}}\\edef{m}{{\\the\\toks{s}}}\\message{{p{id}:\\the\\toks{s} \\meaning{m} }}\\toks{t}=\\toks{r} \\message{{p{id}:\\the\\toks{t}}}",
            s = 1 + rng.next(9),
            t = 1 + rng.next(9)
        ),
        60 => {
            let l = rng.word(&["\\hfuzz=1pt", "\\count1=7", "\\toks1={a}", "\\catcode`\\a=11 ", "\\def\\x{}", "\\let\\x=\\relax"]);
            format!(
                "{{{l} \\global\\advance\\count{r} by 1 }}\\begingroup\\count{r}=9 {{\\count{r}=\\count{r}\\advance\\count{r} by -3 \\global\\count{s}=\\count{r}}}\\endgroup\\message{{p{id}:\\the\\count{r},\\the\\count{s}}}",
                s = 1 + rng.next(9)
            )
        }
        61 => {
            let k = rng.word(&["\\skip", "\\muskip"]);
            let u = if k == "\\skip" { "pt" } else { "mu" };
            format!(
                "{k}{r}=1{u} plus 2fil minus 3fill {k}{s}={a}{u} plus {b}{u} \\advance{k}{r} by {k}{s} \\multiply{k}{r} by {c} \\divide{k}{r} by {d} \\message{{p{id}:\\the{k}{r}}}",
                s = 1 + rng.next(9),
                a = rng.next(20),
                b = rng.next(20),
                c = 1 + rng.next(9),
                d = 1 + rng.next(9)
            )
        }
        _ => unreachable!("probe kind out of range"),
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
    // `\errorcontextlines` and `\newlinechar` are plain's 5 and -1, which is
    // what the oracle runs with; INITEX's are 0.
    let mut src = String::from(
        "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6 \\errorcontextlines=5 \\newlinechar=-1\n",
    );
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
    (
        texrs::parity::messages_of(&want),
        texrs::parity::messages_of(&got),
    )
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
