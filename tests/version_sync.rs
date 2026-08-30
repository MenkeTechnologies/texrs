//! One version, in every file that states one.
//!
//! The crate version appears in six tracked files: `Cargo.toml`, the two
//! hand-written docs pages, the generated reference page, the two man pages,
//! and the IntelliJ plugin's `gradle.properties`. Nothing in a build or a test
//! run notices when they disagree — the code compiles, the pages render, the
//! man page formats — so drift is invisible until someone reads a page that
//! claims a version the binary has not been for three releases. That has
//! already happened here: v0.1.0 sat in the docs through v0.3.0, and the man
//! pages were three versions behind at v0.3.1.
//!
//! `scripts/bump.sh` stamps all six. This is what makes forgetting to run it a
//! failure rather than a slow drift, and it is why the bump script is safe to
//! trust: the gate, not the script, is the thing that cannot be skipped.

use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let p = repo().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// The version the crate is, from the compiler rather than from a re-parse of
/// the manifest — this is the number every other file has to match.
fn crate_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[test]
fn the_manifest_states_the_crate_version() {
    let manifest = read("Cargo.toml");
    let stated = manifest
        .lines()
        .find_map(|l| l.strip_prefix("version = \""))
        .and_then(|v| v.split('"').next())
        .expect("Cargo.toml has a package version");
    assert_eq!(stated, crate_version());
}

#[test]
fn every_docs_page_states_the_crate_version() {
    let want = format!("texrs v{}", crate_version());
    for page in ["docs/index.html", "docs/report.html", "docs/reference.html"] {
        let text = read(page);
        assert!(
            text.contains(&want),
            "{page} does not say `{want}` -- run scripts/bump.sh, or \
             `cargo run --bin gen-docs` for the generated page"
        );
        // And says no OTHER version, which is what catches a page that was
        // stamped in one place and left stale in another.
        let stale: Vec<&str> = text
            .match_indices("texrs v")
            .map(|(at, _)| {
                let rest = &text[at + "texrs v".len()..];
                let end = rest
                    .find(|c: char| !c.is_ascii_digit() && c != '.')
                    .unwrap_or(rest.len());
                &rest[..end]
            })
            .filter(|v| *v != crate_version())
            .collect();
        assert!(
            stale.is_empty(),
            "{page} also claims version(s) {stale:?}, which the binary is not"
        );
    }
}

#[test]
fn every_man_page_states_the_crate_version() {
    let want = format!("\"texrs {}\"", crate_version());
    for page in ["man/man1/texrs.1", "man/man1/texrsall.1"] {
        let text = read(page);
        assert!(
            text.contains(&want),
            "{page}'s .TH line does not say {want} -- the meta repo's \
             man-page-version-sync gate fails on this too"
        );
    }
}

#[test]
fn the_intellij_plugin_tracks_the_crate_version() {
    // The plugin drives this engine over LSP and DAP; a zip whose version is
    // three releases behind the binary it speaks to is a support question
    // nobody can answer.
    let path = repo().join("editors/intellij/gradle.properties");
    if !Path::new(&path).is_file() {
        return; // no plugin in this checkout
    }
    let text = std::fs::read_to_string(&path).expect("gradle.properties");
    let stated = text
        .lines()
        .find_map(|l| l.strip_prefix("pluginVersion="))
        .map(str::trim)
        .expect("gradle.properties states a pluginVersion");
    assert_eq!(
        stated,
        crate_version(),
        "the plugin version and the crate version have drifted apart"
    );
}
