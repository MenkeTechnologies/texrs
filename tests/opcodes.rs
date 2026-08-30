//! Structural guard on the hand-assigned builtin id space.
//!
//! Every operation the VM cannot do natively is a fusevm builtin call whose id
//! is a hand-written `pub const` in `compiler::ops`, dispatched through
//! `vm.register_builtin(id, handler)` in `runtime.rs`. Two changes that each
//! append an op pick the same next number, the files merge without a conflict,
//! and `register_builtin` keeps only the LAST registration for a duplicated id
//! — silently routing one operation to the other's handler. Nothing in a normal
//! build or run reports that.
//!
//! The ids are worse than internal here, because two artefacts outlive the
//! build that wrote them: the bytecode cache stores compiled chunks on disk, and
//! `--aot` emits an object with the chunk serialized inside it. Both call
//! builtins BY NUMBER. Renumbering an op does not fail to compile; it makes
//! every cached chunk and every previously built binary call the wrong function.
//!
//! So this reads the constants back out of the source text — not out of the
//! compiled crate, where a duplicate is indistinguishable from an alias — and
//! fails on a duplicate value, an op that is declared but never registered, and
//! a builtin emitted from a bare integer rather than a named constant.

use std::collections::BTreeMap;
use std::path::PathBuf;

fn src(name: &str) -> String {
    let p: PathBuf = [env!("CARGO_MANIFEST_DIR"), "src", name].iter().collect();
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// Every `pub const NAME: u16 = N;` declared in `compiler::ops`, as
/// `(name, value)`.
fn declared_ops() -> Vec<(String, u16)> {
    let text = src("compiler.rs");
    let mut out = Vec::new();
    let mut in_ops = false;
    let mut depth = 0i32;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("pub mod ops") {
            in_ops = true;
            depth = 0;
        }
        if !in_ops {
            continue;
        }
        depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
        if let Some(rest) = trimmed.strip_prefix("pub const ") {
            let (name, rest) = rest.split_once(':').expect("a const has a type");
            let value = rest
                .split('=')
                .nth(1)
                .and_then(|v| v.trim().trim_end_matches(';').parse::<u16>().ok())
                .unwrap_or_else(|| panic!("{name} is not a plain u16 literal"));
            out.push((name.trim().to_string(), value));
        }
        if depth <= 0 && !trimmed.starts_with("pub mod ops") {
            in_ops = false;
        }
    }
    out
}

/// Every `ops::NAME` handed to `register_builtin`, in `runtime.rs`.
fn registered_ops() -> Vec<String> {
    let text = src("runtime.rs");
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(at) = line.find("register_builtin(") else {
            continue;
        };
        let rest = &line[at + "register_builtin(".len()..];
        let arg = rest.split(',').next().unwrap_or("").trim();
        out.push(arg.trim_start_matches("ops::").to_string());
    }
    out
}

#[test]
fn no_two_builtins_share_an_id() {
    let mut by_value: BTreeMap<u16, Vec<String>> = BTreeMap::new();
    for (name, value) in declared_ops() {
        by_value.entry(value).or_default().push(name);
    }
    let clashes: Vec<String> = by_value
        .iter()
        .filter(|(_, names)| names.len() > 1)
        .map(|(v, names)| format!("{v} is {}", names.join(" and ")))
        .collect();
    assert!(
        clashes.is_empty(),
        "two builtins share an id, so one silently routes to the other's \
         handler:\n  {}",
        clashes.join("\n  ")
    );
}

#[test]
fn every_declared_builtin_is_registered() {
    let declared = declared_ops();
    assert!(
        !declared.is_empty(),
        "no ops found -- the scanner is broken"
    );
    let registered = registered_ops();
    let missing: Vec<&str> = declared
        .iter()
        .map(|(n, _)| n.as_str())
        .filter(|n| !registered.iter().any(|r| r == n))
        .collect();
    assert!(
        missing.is_empty(),
        "declared but never registered, so a chunk calling one finds nothing: {}",
        missing.join(", ")
    );
}

#[test]
fn no_builtin_is_registered_twice() {
    let registered = registered_ops();
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for name in &registered {
        *seen.entry(name.as_str()).or_default() += 1;
    }
    let twice: Vec<&str> = seen
        .iter()
        .filter(|(_, n)| **n > 1)
        .map(|(name, _)| *name)
        .collect();
    assert!(
        twice.is_empty(),
        "registered more than once -- only the last handler survives: {}",
        twice.join(", ")
    );
}

#[test]
fn no_builtin_is_called_from_a_bare_number() {
    // `CallBuiltin(4000, 1)` compiles and runs and is wrong the moment the ids
    // move. Every emission has to go through the named constant.
    let text = src("compiler.rs");
    let bare: Vec<&str> = text
        .lines()
        .filter(|l| l.contains("CallBuiltin("))
        .filter(|l| {
            let after = l.split("CallBuiltin(").nth(1).unwrap_or("");
            after.trim_start().starts_with(|c: char| c.is_ascii_digit())
        })
        .collect();
    assert!(
        bare.is_empty(),
        "a builtin is called by number rather than by name:\n  {}",
        bare.join("\n  ")
    );
}

/// The ids that are already in artefacts on disk.
///
/// A cached chunk and an AOT object both call builtins by number, so these
/// values are a wire format: changing one does not fail to compile, it makes
/// every previously written artefact call the wrong function. Adding an op is
/// fine; renumbering an existing one has to be a deliberate act that also
/// invalidates the cache, which this test is here to force.
#[test]
fn the_published_ids_have_not_moved() {
    let declared: BTreeMap<String, u16> = declared_ops().into_iter().collect();
    for (name, value) in [
        ("MSG_APPEND", 4000u16),
        ("MSG_FLUSH", 4001),
        ("DBG_LINE", 4002),
        ("FFI_COMPILE", 4003),
        ("FFI_CALL", 4004),
    ] {
        assert_eq!(
            declared.get(name),
            Some(&value),
            "{name} moved. Every cached chunk and every --aot binary written \
             before this change calls {value}; if the move is deliberate, bump \
             the cache format version in src/script_cache.rs in the same commit."
        );
    }
}
