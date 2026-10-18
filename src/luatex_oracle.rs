//! The second oracle: what `luatex` says, for the primitives LuaTeX has and
//! TeX82 has not.
//!
//! `crate::parity` asks Knuth's `tex` with the plain format loaded. That is the
//! right reference for TeX82, but it cannot answer for `\numexpr`, `\expanded`
//! or `\attribute`, which the mission -- replacing `lualatex` -- needs answered
//! too. This asks `luatex` instead, in INITEX mode so its starting state is the
//! one texrs starts from (§222's undefined table, §232's sparse category
//! codes, every register zero), with every extra primitive enabled under its
//! own name the way a LaTeX format enables them:
//! `tex.enableprimitives('', tex.extraprimitives())` (LuaTeX manual, "The
//! `tex` library", `enableprimitives`). Without that line an INITEX luatex
//! knows only TeX82 and `\directlua`, which is measured -- `\meaning\numexpr`
//! is `undefined` there.
//!
//! Two things are read from it: the primitive table itself, by origin, for the
//! census `src/bin/primitive_census.rs` writes into `BUGS.md`; and the message
//! stream of each case in `tests/luatex_cases`, for the differential tests.

use std::path::Path;
use std::process::Command;

/// The version every frozen luatex expectation in the tree was measured
/// against. A different luatex reports a different divergence set, which
/// reads exactly like a regression in texrs, so it is refused rather than used.
pub const PINNED: &str = "1.24.0";

/// The program, from `LUATEX_ORACLE` or `luatex`, if it is the pinned version.
pub fn program() -> Result<String, String> {
    let program = std::env::var("LUATEX_ORACLE").unwrap_or_else(|_| "luatex".to_string());
    let out = Command::new(&program)
        .arg("--version")
        .output()
        .map_err(|_| format!("no `{program}' on PATH -- the harness has no oracle"))?;
    let banner = String::from_utf8_lossy(&out.stdout);
    let version = banner
        .lines()
        .next()
        .and_then(|l| l.split("Version ").nth(1))
        .and_then(|v| v.split_whitespace().next())
        .ok_or_else(|| format!("`{program} --version' did not report a version"))?;
    match version == PINNED || std::env::var("LUATEX_VERSION_EXPECT").as_deref() == Ok(version) {
        true => Ok(program),
        false => Err(format!(
            "oracle is luatex {version}, but the frozen expectations were measured \
             against {PINNED}; set LUATEX_VERSION_EXPECT={version} to accept it"
        )),
    }
}

/// The first line luatex is given: braces made group characters just long
/// enough for `\directlua` to read its argument, every extra primitive enabled
/// under its own name, the braces put back to INITEX's category 12, and then
/// the case. The case is what opens the log, so the job is named after it and
/// the `(./case.tex` that [`crate::parity::messages_of`] looks for is there.
const ENABLE_ALL: &str = "\\catcode123=1 \\catcode125=2 \
    \\directlua{tex.enableprimitives(\"\",tex.extraprimitives())}\
    \\catcode123=12 \\catcode125=12 \\input case.tex";

/// What luatex prints for `src`, as [`crate::parity::messages_of`] cuts it.
pub fn reference(program: &str, src: &str) -> String {
    let dir = scratch_dir();
    if std::fs::write(dir.join("case.tex"), src).is_err() {
        return String::new();
    }
    let out = Command::new(program)
        .args(["--ini", "-interaction=nonstopmode", ENABLE_ALL])
        // The same reason as `parity::reference`: a wrap at 79 columns lands
        // anywhere, including inside a name the census reads back.
        .env("max_print_line", "100000")
        .current_dir(&dir)
        .output();
    let text = out
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    crate::parity::messages_of(&text)
}

/// One primitive of luatex's table and where it comes from.
pub struct Primitive {
    /// `tex.extraprimitives`' origin: `tex`, `etex`, `luatex` or `core`, or
    /// `unlisted` for a name `tex.primitives()` has that no origin lists.
    pub origin: &'static str,
    pub name: String,
    /// What `\meaning` prints for it while it still means itself.
    pub meaning: String,
}

/// Every primitive luatex has, with its origin and its `\meaning`.
///
/// The list is luatex's own `tex.primitives()`, origins from
/// `tex.extraprimitives(origin)`, so nothing here is typed by hand.
pub fn primitives(program: &str) -> Result<Vec<Primitive>, String> {
    let dir = scratch_dir();
    let lua = "for _,o in ipairs({'tex','etex','luatex','core'}) do \
                 for _,v in ipairs(tex.extraprimitives(o)) do \
                   texio.write_nl('log','ORIGIN '..o..' ['..v..']') end end \
               for _,v in ipairs(tex.primitives()) do \
                 texio.write_nl('log','ALL ['..v..']') end";
    std::fs::write(dir.join("list.lua"), lua).map_err(|e| e.to_string())?;
    std::fs::write(
        dir.join("list.tex"),
        "\\catcode123=1 \\catcode125=2 \\directlua{dofile('list.lua')}\\end\n",
    )
    .map_err(|e| e.to_string())?;
    Command::new(program)
        .args(["--ini", "-interaction=nonstopmode", "list.tex"])
        .current_dir(&dir)
        .output()
        .map_err(|e| e.to_string())?;
    let log = std::fs::read_to_string(dir.join("list.log")).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);

    let mut origin_of = std::collections::HashMap::new();
    let mut all = Vec::new();
    // Each name is written between brackets, so the control space is `[ ]`
    // and whatever the log puts after the last one (the file's `)`) is not
    // read as part of it.
    let bracketed = |s: &str| {
        s.strip_prefix('[')
            .and_then(|s| s.rfind(']').map(|end| s[..end].to_string()))
    };
    for line in log.lines() {
        if let Some(rest) = line.strip_prefix("ORIGIN ") {
            let Some((o, name)) = rest.split_once(' ') else {
                continue;
            };
            let o: &'static str = match o {
                "tex" => "tex",
                "etex" => "etex",
                "luatex" => "luatex",
                _ => "core",
            };
            if let Some(name) = bracketed(name) {
                origin_of.entry(name).or_insert(o);
            }
        } else if let Some(name) = line.strip_prefix("ALL ").and_then(bracketed) {
            all.push(name);
        }
    }
    if all.is_empty() {
        return Err("luatex listed no primitives".to_string());
    }
    all.sort();
    all.dedup();

    let meanings = meanings_in(program, &all, reference)?;
    Ok(all
        .into_iter()
        .zip(meanings)
        .map(|(name, meaning)| Primitive {
            origin: origin_of.get(&name).copied().unwrap_or("unlisted"),
            name,
            meaning,
        })
        .collect())
}

/// Every primitive a document can reach in luatex, with the command code
/// luatex gives it (`token.create(name).cmdname`) and whether that command
/// expands (`.expandable`), sorted by name: the table
/// `src/luatex_primitives.rs` is written from. A name `tex.primitives()` lists
/// that is still `undefined_cs` once every origin is enabled is left out.
pub fn command_table(program: &str) -> Result<Vec<(String, String, bool)>, String> {
    let dir = scratch_dir();
    let lua = "for _,v in ipairs(tex.primitives()) do \
                 local t = token.create(v) \
                 texio.write_nl('log','CMD '..t.cmdname..' '..tostring(t.expandable)..' ['..v..']') \
               end";
    std::fs::write(dir.join("cmds.lua"), lua).map_err(|e| e.to_string())?;
    std::fs::write(
        dir.join("case.tex"),
        "\\catcode123=1 \\catcode125=2 \\directlua{dofile('cmds.lua')}\\end\n",
    )
    .map_err(|e| e.to_string())?;
    Command::new(program)
        .args(["--ini", "-interaction=nonstopmode", ENABLE_ALL])
        .current_dir(&dir)
        .output()
        .map_err(|e| e.to_string())?;
    let log = std::fs::read_to_string(dir.join("case.log")).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);
    let mut table: Vec<(String, String, bool)> = log
        .lines()
        .filter_map(|l| l.strip_prefix("CMD "))
        .filter_map(|rest| {
            let mut w = rest.splitn(3, ' ');
            let (cmd, expandable, name) = (w.next()?, w.next()?, w.next()?);
            let name = name.strip_prefix('[')?;
            let name = &name[..name.rfind(']')?];
            (cmd != "undefined_cs").then(|| (name.to_string(), cmd.to_string(), expandable == "true"))
        })
        .collect();
    table.sort();
    table.dedup();
    match table.is_empty() {
        true => Err("luatex reported no command codes".to_string()),
        false => Ok(table),
    }
}

/// `src/luatex_primitives.rs` as [`command_table`] makes it.
pub fn render_command_table(table: &[(String, String, bool)]) -> String {
    let mut out = format!(
        "//! Every primitive a document can reach in luatex {PINNED}, with the command\n\
         //! code luatex gives it (`token.create(name).cmdname`) and whether that\n\
         //! command expands. WRITTEN by `bash scripts/primitive-census.sh` out of\n\
         //! luatex's own table (`crate::luatex_oracle::command_table`); not edited\n\
         //! by hand, and `tests/luatex_primitives.rs` checks it against luatex.\n\
         \n\
         /// `(name, luatex command, expandable)`, sorted by name.\n\
         pub const TABLE: &[(&str, &str, bool)] = &[\n"
    );
    for (name, cmd, expandable) in table {
        out.push_str(&format!("    ({name:?}, {cmd:?}, {expandable}),\n"));
    }
    out.push_str("];\n");
    out
}

/// The control sequence `name` as a document spells it: a control symbol is
/// the escape and its one character, so the control space is `\ `.
pub fn spelt(name: &str) -> String {
    format!("\\{name}")
}

/// The source that makes an engine print `\meaning` of each of `names`, one
/// `\message` apiece, each between markers the reader below finds again.
pub fn meaning_probe(names: &[String]) -> String {
    let mut src = String::from("\\catcode`\\{=1 \\catcode`\\}=2\n");
    for n in names {
        src.push_str(&format!("\\message{{@@<\\meaning{}>@@}}\n", spelt(n)));
    }
    src.push_str("\\end\n");
    src
}

/// The meanings a probe's message stream holds, in order.
pub fn meanings_of(stream: &str) -> Vec<String> {
    stream
        .split("@@<")
        .skip(1)
        .map(|s| s.split(">@@").next().unwrap_or("").to_string())
        .collect()
}

/// `\meaning` of every one of `names`, by one run of `run` over the probe.
fn meanings_in(
    program: &str,
    names: &[String],
    run: fn(&str, &str) -> String,
) -> Result<Vec<String>, String> {
    let got = meanings_of(&run(program, &meaning_probe(names)));
    match got.len() == names.len() {
        true => Ok(got),
        false => Err(format!(
            "the meaning probe printed {} of {} meanings",
            got.len(),
            names.len()
        )),
    }
}

/// A directory of its own for one run: luatex writes its log beside the
/// input, and two runs in one directory would read each other's.
fn scratch_dir() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static RUN: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "texrs-luatex-{}-{}",
        std::process::id(),
        RUN.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// The primitives a case says it exercises: its `% primitives:` line, names
/// without their backslash.
pub fn primitives_of_case(src: &str) -> Vec<String> {
    src.lines()
        .filter_map(|l| l.trim().strip_prefix("% primitives:"))
        .flat_map(|rest| rest.split_whitespace())
        .map(|w| w.trim_start_matches('\\').to_string())
        .collect()
}

/// The frozen expectations for `tests/luatex_cases`.
pub fn frozen_path(repo: &Path) -> std::path::PathBuf {
    repo.join("tests/data/luatex_cases_expected.txt")
}
