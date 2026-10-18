//! The `kpse` library, and `require` through it.
//!
//! A port of luatexdir/lua/lkpselib.c over `crate::kpse`: where LuaTeX calls
//! into kpathsea in-process, texrs asks `kpsewhich`, which is kpathsea behind
//! a command line, so every answer is the one the same TeX Live gives LuaTeX.
//! The lookups go through [`crate::kpse::find_as`], one interactive process
//! per program name and format, and the variable expansions through
//! [`crate::kpse::query`]; both are memoised, since nothing a run does changes
//! the TeX tree under it.
//!
//! `require` is the other half. luainit.c's `setup_lua_path` replaces
//! `package.searchers[2]` with `luatex_kpse_lua_find`, which looks a module up
//! as a file of the `lua` format -- first with its dots turned into slashes,
//! then as written -- and loads it with `luaL_loadfile`. That is what makes
//! `require("luaotfload-main")` find
//! `texmf-dist/tex/luatex/luaotfload/luaotfload-main.lua`, and it is ported
//! here exactly, failure message included. The C-module searcher
//! (`searchers[3]`) needs unrestricted shell escape and is removed by
//! `luatex-core.lua` in every other configuration, which is this one.
//!
//! ## What is approximate
//!
//! - **`kpse.new`** answers a table carrying the same methods over its own
//!   program name, rather than a `kpathsea` instance: kpsewhich is the
//!   instance.
//! - **`kpse.lookup`'s `path`, `subdir`, `debug` and `mktex*` options** are
//!   kpathsea internals that `kpsewhich` does not expose one for one; `all`
//!   is `kpsewhich -all`, `format` and `mustexist` are honoured, and the rest
//!   are refused by name rather than ignored.
//! - **Glyph lookups** (`pk`, `gf`, `bitmap font`) go through `kpsewhich
//!   -format`, not kpathsea's `kpse_find_glyph` with its dpi search.

use mlua::{Lua, Table, Value, Variadic};
use std::cell::RefCell;

thread_local! {
    /// The program name kpathsea searches for: `kpse.set_program_name`'s, or
    /// the engine's. LuaTeX sets it from `argv[0]` before any Lua runs, so a
    /// document's chunk never sees the library unset.
    static PROGNAME: RefCell<String> = RefCell::new(String::from("luatex"));
}

/// The program name `kpse` searches under from here on: `lualatex` for a
/// LaTeX document, as LuaTeX's own `lualatex` invocation sets it, `luatex`
/// for plain. It decides `TEXINPUTS.<progname>`, so `kpse.find_file` of a
/// `.sty` under `luatex` and under `lualatex` are different searches.
pub fn set_progname(name: &str) {
    PROGNAME.with(|p| *p.borrow_mut() = name.to_string());
}

fn progname() -> String {
    PROGNAME.with(|p| p.borrow().clone())
}

/// lkpselib.c's `filetypenames[]`, the names `find_file` and `show_path`
/// accept, in its order. Each is a `kpsewhich -format` name as well.
const FILETYPE_NAMES: &[&str] = &[
    "gf",
    "pk",
    "bitmap font",
    "tfm",
    "afm",
    "base",
    "bib",
    "bst",
    "cnf",
    "ls-R",
    "fmt",
    "map",
    "mem",
    "mf",
    "mfpool",
    "mft",
    "mp",
    "mppool",
    "MetaPost support",
    "ocp",
    "ofm",
    "opl",
    "otp",
    "ovf",
    "ovp",
    "graphic/figure",
    "tex",
    "TeX system documentation",
    "texpool",
    "TeX system sources",
    "PostScript header",
    "Troff fonts",
    "type1 fonts",
    "vf",
    "dvips config",
    "ist",
    "truetype fonts",
    "type42 fonts",
    "web2c files",
    "other text files",
    "other binary files",
    "misc fonts",
    "web",
    "cweb",
    "enc files",
    "cmap files",
    "subfont definition files",
    "opentype fonts",
    "pdftex config",
    "lig files",
    "texmfscripts",
    "lua",
    "font feature files",
    "cid maps",
    "mlbib",
    "mlbst",
    "clua",
];

/// `luaL_checkoption(L, i, NULL, filetypenames)`, with its message.
fn check_format(v: &mlua::LuaString, arg: usize, func: &str) -> mlua::Result<&'static str> {
    let s = v.to_str()?;
    FILETYPE_NAMES
        .iter()
        .find(|n| **n == &*s)
        .copied()
        .ok_or_else(|| {
            mlua::Error::runtime(format!(
                "bad argument #{arg} to '{func}' (invalid option '{}')",
                &*s
            ))
        })
}

/// `find_file(name [, format] [, mustexist])`: the optional arguments may come
/// in any order, a string being the format and a boolean or number the
/// must-exist flag (lkpselib.c scans them from the last back).
fn find_file(prog: &str, args: &[Value], first: usize, func: &str) -> mlua::Result<Option<String>> {
    let Some(Value::String(name)) = args.get(first) else {
        return Err(mlua::Error::runtime("not a file name"));
    };
    let mut format = "tex";
    let mut must_exist = false;
    for (i, v) in args.iter().enumerate().skip(first + 1).rev() {
        match v {
            Value::Boolean(b) => must_exist = *b,
            Value::Integer(n) => must_exist = *n > 0,
            Value::Number(n) => must_exist = (*n as i64) > 0,
            Value::String(s) => format = check_format(s, i + 1, func)?,
            _ => {}
        }
    }
    let name = name.to_str()?.to_string();
    Ok(crate::kpse::find_as(&name, format, prog, must_exist)
        .map(|p| p.to_string_lossy().into_owned()))
}

/// kpathsea's `kpathsea_name_ok` for WRITING (tex-file.c), which is what
/// `out_name_ok` and its variants are: reading is always allowed ("As of
/// 2026, if ACTION is ok_reading, we simply return true").
///
/// `openout_any` decides: `a` allows anything; otherwise a name with a dot
/// file anywhere in it is refused; `r` stops there; `p` (the default) also
/// refuses an absolute name outside `TEXMF_OUTPUT_DIRECTORY` and `TEXMFOUTPUT`
/// -- or, `extended`, `TEXMFVAR` and `TEXMFSYSVAR` -- and any `..` component.
fn out_name_ok(prog: &str, fname: &str, extended: bool) -> bool {
    let choice = crate::kpse::query(prog, "-var-value", "openout_any")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "p".to_string());
    let c = choice.as_bytes()[0];
    if matches!(c, b'a' | b'y' | b'1') {
        return true;
    }
    let expanded = match fname.contains(['$', '~']) {
        true => crate::kpse::query(prog, "-expand-var", fname).unwrap_or_else(|| fname.to_string()),
        false => fname.to_string(),
    };
    let absolute = expanded.starts_with('/');
    let b = fname.as_bytes();
    for (q, _) in fname.match_indices('.') {
        let starts = q == 0 || b[q - 1] == b'/';
        let next = b.get(q + 1).copied();
        let after = b.get(q + 2).copied();
        if starts
            && next != Some(b'/')
            && !(next == Some(b'.') && after == Some(b'/'))
            && !(extended && absolute)
        {
            return false;
        }
    }
    if matches!(c, b'r' | b'n' | b'0') {
        return true;
    }
    if absolute {
        let ok_in = |dir: Option<String>| {
            dir.is_some_and(|d| {
                !d.is_empty()
                    && expanded.starts_with(&d)
                    && (expanded.len() == d.len() || expanded.as_bytes()[d.len()] == b'/')
            })
        };
        let passed = ok_in(std::env::var("TEXMF_OUTPUT_DIRECTORY").ok())
            || ok_in(crate::kpse::query(prog, "-var-value", "TEXMFOUTPUT"))
            || (extended
                && (ok_in(crate::kpse::query(prog, "-var-value", "TEXMFVAR"))
                    || ok_in(crate::kpse::query(prog, "-var-value", "TEXMFSYSVAR"))));
        if !passed {
            return false;
        }
    }
    if fname.starts_with("../") {
        return false;
    }
    for (at, _) in fname.match_indices("..") {
        if at > 0 && b.get(at + 2) == Some(&b'/') && b[at - 1] == b'/' {
            return false;
        }
    }
    true
}

/// One of the variable expanders: `var_value`, `expand_var`, `expand_path`,
/// `expand_braces`, `show_path`.
fn expander(lua: &Lua, t: &Table, name: &'static str, option: &'static str) -> mlua::Result<()> {
    t.set(
        name,
        lua.create_function(move |_, s: mlua::LuaString| {
            Ok(crate::kpse::query(&progname(), option, &s.to_str()?))
        })?,
    )
}

/// The `kpse` table.
pub(super) fn table(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set(
        "find_file",
        lua.create_function(|_, args: Variadic<Value>| {
            find_file(&progname(), &args, 0, "find_file")
        })?,
    )?;
    expander(lua, &t, "var_value", "-var-value")?;
    expander(lua, &t, "expand_var", "-expand-var")?;
    expander(lua, &t, "expand_path", "-expand-path")?;
    expander(lua, &t, "expand_braces", "-expand-braces")?;
    // `show_path(format)`: `luaL_checkoption(L, -1, "tex", filetypenames)`.
    t.set(
        "show_path",
        lua.create_function(|_, f: Option<mlua::LuaString>| {
            let format = match &f {
                Some(s) => check_format(s, 1, "show_path")?,
                None => "tex",
            };
            Ok(crate::kpse::query(&progname(), "-show-path", format))
        })?,
    )?;
    // Reading is always allowed (see `out_name_ok`).
    for name in ["in_name_ok", "in_name_ok_silent_extended"] {
        t.set(name, lua.create_function(|_, _: mlua::LuaString| Ok(true))?)?;
    }
    t.set(
        "out_name_ok",
        lua.create_function(|_, s: mlua::LuaString| {
            Ok(out_name_ok(&progname(), &s.to_str()?, false))
        })?,
    )?;
    t.set(
        "out_name_ok_silent_extended",
        lua.create_function(|_, s: mlua::LuaString| {
            Ok(out_name_ok(&progname(), &s.to_str()?, true))
        })?,
    )?;
    // `kpse_readable_file`: the name if it is a regular file that can be read.
    t.set(
        "readable_file",
        lua.create_function(|_, s: mlua::LuaString| {
            let name = s.to_str()?.to_string();
            let ok = std::fs::metadata(&name).is_ok_and(|m| m.is_file())
                && std::fs::File::open(&name).is_ok();
            Ok(ok.then_some(name))
        })?,
    )?;
    // `set_program_name(exe [, prog])`: the second names the program kpathsea
    // searches for, defaulting to the first.
    t.set(
        "set_program_name",
        lua.create_function(|lua, (exe, prog): (String, Option<String>)| {
            set_progname(&prog.unwrap_or(exe));
            // "fix up the texconfig entry"
            if let Ok(tc) = lua.globals().get::<Table>("texconfig") {
                tc.raw_set("kpse_init", false)?;
            }
            Ok(())
        })?,
    )?;
    // `init_prog` sets the glyph-search defaults (`MAKETEX_MODE` and the
    // fallback resolutions); glyph lookups here go through kpsewhich, which
    // keeps its own, so there is nothing for it to set.
    t.set(
        "init_prog",
        lua.create_function(|_, _: Variadic<Value>| Ok(()))?,
    )?;
    t.set(
        "version",
        lua.create_function(|_, ()| Ok(kpathsea_version()))?,
    )?;
    t.set(
        "default_texmfcnf",
        lua.create_function(|_, ()| Ok(crate::kpse::query(&progname(), "-var-value", "TEXMFCNF")))?,
    )?;
    // The recorder (`-recorder`) is not something texrs writes, so a name to
    // record has nowhere to go.
    for name in ["record_input_file", "record_output_file"] {
        t.set(name, lua.create_function(|_, _: Variadic<Value>| Ok(()))?)?;
    }
    // "check_permission": shell escape is disabled, so lkpselib.c's second
    // branch answers.
    t.set(
        "check_permission",
        lua.create_function(|_, _: mlua::LuaString| {
            Ok((false, "all command execution is disabled"))
        })?,
    )?;
    t.set(
        "lookup",
        lua.create_function(|lua, (name, opts): (mlua::LuaString, Option<Table>)| {
            lookup(lua, &progname(), &name.to_str()?, opts.as_ref())
        })?,
    )?;
    t.set(
        "new",
        lua.create_function(|lua, (exe, prog): (String, Option<String>)| {
            instance(lua, prog.unwrap_or(exe))
        })?,
    )?;
    Ok(t)
}

/// `kpathsea_version_string`, which is the first line `kpsewhich --version`
/// prints: `kpathsea version 6.4.2` in TeX Live 2026.
fn kpathsea_version() -> String {
    static V: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        std::process::Command::new("kpsewhich")
            .arg("--version")
            .output()
            .ok()
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .next()
                    .map(str::to_string)
            })
            .unwrap_or_default()
    })
    .clone()
}

/// `kpse.lookup(name [, options])`: every match when `all` is set, the first
/// otherwise, nil for none.
fn lookup(
    lua: &Lua,
    prog: &str,
    name: &str,
    opts: Option<&Table>,
) -> mlua::Result<Variadic<Value>> {
    let mut format = None;
    let mut must_exist = false;
    let mut all = false;
    if let Some(o) = opts {
        if let Some(f) = o.get::<Option<mlua::LuaString>>("format")? {
            format = Some(check_format(&f, 2, "lookup")?);
        }
        must_exist = o.get::<Option<bool>>("mustexist")?.unwrap_or(false);
        all = o.get::<Option<bool>>("all")?.unwrap_or(false);
        for unsupported in [
            "path", "subdir", "debug", "dpi", "mktexpk", "mktextex", "mktexmf", "mktextfm",
        ] {
            if !matches!(o.get::<Value>(unsupported)?, Value::Nil) {
                return Err(mlua::Error::runtime(format!(
                    "kpse.lookup's '{unsupported}' option is a kpathsea internal that texrs \
                     reaches through kpsewhich, which has no switch for it"
                )));
            }
        }
    }
    let mut args = vec![format!("-progname={prog}")];
    if let Some(f) = format {
        args.push(format!("-format={f}"));
    }
    if must_exist {
        args.push("-must-exist".to_string());
    }
    if all {
        args.push("-all".to_string());
    }
    let out = std::process::Command::new("kpsewhich")
        .args(&args)
        .arg(name)
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(mlua::Error::external)?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    let lines = match all {
        true => lines,
        false => lines.into_iter().take(1).collect(),
    };
    let mut out = Variadic::new();
    for l in lines {
        out.push(Value::String(lua.create_string(l)?));
    }
    if out.is_empty() {
        out.push(Value::Nil);
    }
    Ok(out)
}

/// `kpse.new(exe [, prog])`: an object whose methods are the library's own,
/// searching under its own program name.
fn instance(lua: &Lua, prog: String) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    let p = prog.clone();
    t.set(
        "find_file",
        lua.create_function(move |_, args: Variadic<Value>| find_file(&p, &args, 1, "find_file"))?,
    )?;
    for (name, option) in [
        ("var_value", "-var-value"),
        ("expand_var", "-expand-var"),
        ("expand_path", "-expand-path"),
        ("expand_braces", "-expand-braces"),
    ] {
        let p = prog.clone();
        t.set(
            name,
            lua.create_function(move |_, (_, s): (Value, mlua::LuaString)| {
                Ok(crate::kpse::query(&p, option, &s.to_str()?))
            })?,
        )?;
    }
    let p = prog.clone();
    t.set(
        "show_path",
        lua.create_function(move |_, (_, f): (Value, Option<mlua::LuaString>)| {
            let format = match &f {
                Some(s) => check_format(s, 2, "show_path")?,
                None => "tex",
            };
            Ok(crate::kpse::query(&p, "-show-path", format))
        })?,
    )?;
    let p = prog.clone();
    t.set(
        "lookup",
        lua.create_function(
            move |lua, (_, name, opts): (Value, mlua::LuaString, Option<Table>)| {
                lookup(lua, &p, &name.to_str()?, opts.as_ref())
            },
        )?,
    )?;
    Ok(t)
}

/// `luatex_kpse_lua_find` (luainit.c), installed as `package.searchers[2]`.
///
/// "Lua convention": the module name with its dots as slashes is tried
/// first, then the name as written; a module that is not there leaves the
/// searcher's own line in `require`'s message, and one that is there but
/// does not load is an error naming the file.
pub(super) fn install_searcher(lua: &Lua) -> mlua::Result<()> {
    let searchers: Table = lua.globals().get::<Table>("package")?.get("searchers")?;
    let find = lua.create_function(|lua, name: mlua::LuaString| {
        let name = name.to_str()?.to_string();
        let prog = progname();
        let alt = name.replace('.', "/");
        let found = crate::kpse::find_as(&alt, "lua", &prog, false)
            .or_else(|| crate::kpse::find_as(&name, "lua", &prog, false));
        let Some(path) = found else {
            return Ok(Value::String(lua.create_string(format!(
                "\n\t[kpse lua searcher] file not found: '{name}'"
            ))?));
        };
        let text = std::fs::read(&path).map_err(mlua::Error::external)?;
        let chunk = lua
            .load(text)
            .set_name(format!("@{}", path.display()))
            .into_function()
            .map_err(|e| {
                mlua::Error::runtime(format!(
                    "error loading module {name} from file {}:\n\t{e}",
                    path.display()
                ))
            })?;
        Ok(Value::Function(chunk))
    })?;
    searchers.raw_set(2, find)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writing_is_paranoid_about_dot_files_and_parent_directories() {
        // `openout_any = p` is TeX Live's default; when kpsewhich is not
        // installed the query answers nothing and the same default applies.
        assert!(out_name_ok("luatex", "plain.log", false));
        assert!(out_name_ok("luatex", "sub/dir/x.aux", false));
        assert!(!out_name_ok("luatex", ".rhosts", false));
        assert!(!out_name_ok("luatex", "a/.ssh/config", false));
        assert!(!out_name_ok("luatex", "../escape.tex", false));
        assert!(!out_name_ok("luatex", "a/../../b.tex", false));
        assert!(!out_name_ok("luatex", "/etc/passwd", false));
        // `./` and `../`-free relative names with a dot inside are fine.
        assert!(out_name_ok("luatex", "./x.tex", false));
    }
}
