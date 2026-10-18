//! `primitive-census` -- every primitive `luatex` has, and how far texrs is
//! with each one.
//!
//! The list is luatex's own (`tex.primitives()`, origins from
//! `tex.extraprimitives`); nothing in it is typed by hand. Each primitive gets
//! one of three verdicts, all measured:
//!
//!   * `missing`  -- `\meaning` of it is `undefined` in texrs.
//!   * `partial`  -- texrs knows the name, but its `\meaning` differs from
//!     luatex's, or no case in `tests/luatex_cases` exercises it and passes.
//!   * `complete` -- the `\meaning` agrees, and every case whose `% primitives:`
//!     line names it prints what luatex prints, and there is at least one.
//!
//! ```sh
//! cargo run --bin primitive-census              # the table, to stdout
//! cargo run --bin primitive-census -- --bugs    # and into BUGS.md
//! cargo run --bin primitive-census -- --freeze  # record luatex's case output
//! cargo run --bin primitive-census -- --table   # write src/luatex_primitives.rs
//! cargo run --bin primitive-census -- --one f.tex # both streams for one file
//! ```
//! `scripts/primitive-census.sh` runs the last two.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use texrs::luatex_oracle::{self as oracle, Primitive};

const BEGIN: &str = "<!-- primitive-census:begin -->";
const END: &str = "<!-- primitive-census:end -->";

/// What texrs prints for `src`, with a panic counted as an answer rather than
/// the end of the census.
fn subject(src: &str) -> String {
    let src = src.to_string();
    std::panic::catch_unwind(move || texrs::luatex_oracle::subject(&src))
        .unwrap_or_else(|_| "PANIC".to_string())
}

/// texrs's `\meaning` of each name: one probe for the lot, and one per name if
/// the lot did not come back whole (an error part way stops a probe).
fn texrs_meanings(names: &[String]) -> Vec<String> {
    let all = oracle::meanings_of(&subject(&oracle::meaning_probe(names)));
    if all.len() == names.len() {
        return all;
    }
    names
        .iter()
        .map(|n| {
            oracle::meanings_of(&subject(&oracle::meaning_probe(std::slice::from_ref(n))))
                .pop()
                .unwrap_or_else(|| "ERROR".to_string())
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Status {
    Missing,
    Partial,
    Complete,
}

impl Status {
    fn word(self) -> &'static str {
        match self {
            Status::Missing => "missing",
            Status::Partial => "partial",
            Status::Complete => "complete",
        }
    }
}

struct Row {
    p: Primitive,
    status: Status,
    /// Why a partial one is partial.
    why: String,
}

fn main() -> ExitCode {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let program = match oracle::program() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("primitive-census: {e}");
            return ExitCode::from(2);
        }
    };

    // `--one FILE`: both engines' streams for one file, for working on a case.
    if let Some(at) = args.iter().position(|a| a == "--one") {
        let Some(src) = args
            .get(at + 1)
            .and_then(|f| std::fs::read_to_string(f).ok())
        else {
            eprintln!("primitive-census: --one needs a readable file");
            return ExitCode::from(2);
        };
        let (want, got) = (oracle::reference(&program, &src), subject(&src));
        println!(
            "luatex: {want}\ntexrs : {got}\n{}",
            if want == got { "SAME" } else { "DIFFERS" }
        );
        return ExitCode::SUCCESS;
    }

    // The cases, and what each engine prints for them.
    let cases = texrs::parity::cases_in(&repo.join("tests/luatex_cases"));
    let mut frozen = String::from(
        "# Frozen output of luatex, written by `cargo run --bin primitive-census -- --freeze`.\n\
         # One block per case in tests/luatex_cases; tests/luatex_primitives.rs replays\n\
         # these with no TeX installed.\n",
    );
    // Per primitive: (cases naming it, of those the ones that pass).
    let mut covered: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut failing = Vec::new();
    for case in &cases {
        let name = case.file_name().unwrap().to_string_lossy().into_owned();
        let src = std::fs::read_to_string(case).unwrap_or_default();
        let want = oracle::reference(&program, &src);
        let got = subject(&src);
        frozen.push_str(&format!("{}{name}\n{want}\n", texrs::parity::FREEZE_SEP));
        let pass = want == got;
        if !pass {
            failing.push(format!("{name}\n  luatex: {want:?}\n  texrs : {got:?}"));
        }
        for p in oracle::primitives_of_case(&src) {
            let e = covered.entry(p).or_default();
            e.0 += 1;
            e.1 += pass as usize;
        }
    }

    let prims = match oracle::primitives(&program) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("primitive-census: {e}");
            return ExitCode::from(2);
        }
    };
    // A name `tex.primitives()` lists that is still undefined once every
    // origin is enabled is not one a document can reach: reported, not judged.
    let (prims, unreachable): (Vec<Primitive>, Vec<Primitive>) =
        prims.into_iter().partition(|p| p.meaning != "undefined");
    let names: Vec<String> = prims.iter().map(|p| p.name.clone()).collect();
    let ours = texrs_meanings(&names);
    let rows: Vec<Row> = prims
        .into_iter()
        .zip(ours)
        .map(|(p, texrs)| {
            let (n, passing) = covered.get(&p.name).copied().unwrap_or((0, 0));
            let (status, why) = if texrs == "undefined" {
                (Status::Missing, String::new())
            } else if texrs != p.meaning {
                (Status::Partial, format!("`\\meaning` is `{texrs}`"))
            } else if n == 0 {
                (Status::Partial, String::new())
            } else if passing < n {
                (
                    Status::Partial,
                    format!("{} of {n} cases diverge", n - passing),
                )
            } else {
                (Status::Complete, String::new())
            };
            Row { p, status, why }
        })
        .collect();

    let mut report = render(&rows);
    if !unreachable.is_empty() {
        let names: Vec<String> = unreachable
            .iter()
            .map(|p| format!("`{}`", oracle::spelt(&p.name)))
            .collect();
        report.push_str(&format!(
            "\nListed by `tex.primitives()` but undefined in luatex itself once every \
             origin is enabled, so not judged: {}\n",
            names.join(" ")
        ));
    }
    print!("{report}");
    for f in &failing {
        eprintln!("DIVERGES {f}");
    }

    if args.iter().any(|a| a == "--table") {
        let written = oracle::command_table(&program)
            .map(|t| oracle::render_command_table(&t))
            .and_then(|text| {
                std::fs::write(repo.join("src/luatex_primitives.rs"), text)
                    .map_err(|e| e.to_string())
            });
        if let Err(e) = written {
            eprintln!("primitive-census: {e}");
            return ExitCode::from(2);
        }
    }
    if args.iter().any(|a| a == "--freeze") {
        if let Err(e) = std::fs::write(oracle::frozen_path(&repo), &frozen) {
            eprintln!("primitive-census: {e}");
            return ExitCode::from(2);
        }
    }
    if args.iter().any(|a| a == "--bugs") {
        let path = repo.join("BUGS.md");
        let bugs = std::fs::read_to_string(&path).unwrap_or_default();
        let (Some(b), Some(e)) = (bugs.find(BEGIN), bugs.find(END)) else {
            eprintln!("primitive-census: BUGS.md has no {BEGIN} ... {END} section");
            return ExitCode::from(2);
        };
        let new = format!("{}{BEGIN}\n{report}{}", &bugs[..b], &bugs[e..]);
        if let Err(e) = std::fs::write(&path, new) {
            eprintln!("primitive-census: {e}");
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}

/// The census as Markdown: the counts, then every primitive by origin and
/// verdict.
fn render(rows: &[Row]) -> String {
    const ORIGINS: [&str; 5] = ["tex", "etex", "luatex", "core", "unlisted"];
    const STATUSES: [Status; 3] = [Status::Missing, Status::Partial, Status::Complete];
    let count = |o: &str, s: Status| {
        rows.iter()
            .filter(|r| r.p.origin == o && r.status == s)
            .count()
    };
    let mut out = String::new();
    out.push_str(&format!(
        "Generated by `scripts/primitive-census.sh` against luatex {}; {} primitives judged.\n\n",
        oracle::PINNED,
        rows.len()
    ));
    out.push_str("| origin | missing | partial | complete |\n|---|---:|---:|---:|\n");
    for o in ORIGINS {
        let cells: Vec<String> = STATUSES.iter().map(|s| count(o, *s).to_string()).collect();
        if cells.iter().all(|c| c == "0") {
            continue;
        }
        out.push_str(&format!("| {o} | {} |\n", cells.join(" | ")));
    }
    let totals: Vec<String> = STATUSES
        .iter()
        .map(|s| rows.iter().filter(|r| r.status == *s).count().to_string())
        .collect();
    out.push_str(&format!("| all | {} |\n", totals.join(" | ")));

    for o in ORIGINS {
        for s in STATUSES {
            let mut these: Vec<&Row> = rows
                .iter()
                .filter(|r| r.p.origin == o && r.status == s)
                .collect();
            if these.is_empty() {
                continue;
            }
            these.sort_by(|a, b| a.p.name.cmp(&b.p.name));
            out.push_str(&format!("\n**{o}, {}** ({}):\n", s.word(), these.len()));
            // The ones with a reason first, one to a line; the rest run on.
            for r in these.iter().filter(|r| !r.why.is_empty()) {
                out.push_str(&format!(
                    "- `{}` -- {}; luatex: `{}`\n",
                    oracle::spelt(&r.p.name),
                    r.why,
                    r.p.meaning
                ));
            }
            let plain: Vec<String> = these
                .iter()
                .filter(|r| r.why.is_empty())
                .map(|r| format!("`{}`", oracle::spelt(&r.p.name)))
                .collect();
            if !plain.is_empty() {
                out.push_str(&plain.join(" "));
                out.push('\n');
            }
        }
    }
    out
}
