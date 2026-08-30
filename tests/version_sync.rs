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
//!
//! **What it does not police: prose.** A version string in a sentence —
//! "the expander gained \futurelet in v0.3.1" — is a true statement about
//! history, and a gate that failed on it would push whoever is holding the
//! release to edit a true sentence into a false one. Measured across the fleet,
//! that is not hypothetical: of 19 pages a naive "no other version anywhere"
//! rule flags, 14 are prose of exactly this kind. So the check is on version
//! SLOTS — the branded `texrs vX.Y.Z` of a build line, the `.TH` field, the
//! `pluginVersion=` line — and the convention that follows is worth stating
//! because the gate enforces it: write the product's current version as
//! `texrs vX.Y.Z`, and write history as a bare `vX.Y.Z`.

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

/// Every version SLOT in `text`: the branded `texrs vX.Y.Z` form a build line
/// or a stat card uses.
///
/// Deliberately not "every version-shaped string". Prose dates changes, and a
/// gate that read those as claims would demand they be falsified — which is
/// also how a careless stamp turns `0.0.0.0` in an IP address into `0.18.1.0`.
fn slots_in(text: &str) -> Vec<String> {
    text.match_indices("texrs v")
        .map(|(at, _)| {
            let rest = &text[at + "texrs v".len()..];
            let end = rest
                .find(|c: char| !c.is_ascii_digit() && c != '.')
                .unwrap_or(rest.len());
            rest[..end].trim_end_matches('.').to_string()
        })
        .filter(|v| v.split('.').count() == 3)
        .collect()
}

#[test]
fn a_version_in_prose_is_not_a_slot() {
    // The property the fleet measurement paid for: 14 of 19 flagged pages were
    // sentences like these, every one of them true.
    let prose = "<p>The expander gained <code>\\futurelet</code> in v0.3.1, and \
                 the v4.8.0 release added a cache.</p>";
    assert!(
        slots_in(prose).is_empty(),
        "a version in a sentence was read as a claim about this build"
    );

    let slot = "<p class=\"docs-build-line\">texrs v9.9.9 · TeX on fusevm</p>";
    assert_eq!(slots_in(slot), vec!["9.9.9".to_string()]);

    // Both at once: the slot is checked, the sentence is left alone.
    let both = format!("{slot}{prose}");
    assert_eq!(slots_in(&both), vec!["9.9.9".to_string()]);
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
        // And every OTHER slot on the page agrees, which is what catches a
        // page stamped in one place and left stale in another. Only slots: a
        // bare `vX.Y.Z` in a sentence is history, not a claim about what this
        // build is.
        let stale: Vec<String> = slots_in(&text)
            .into_iter()
            .filter(|v| v != crate_version())
            .collect();
        assert!(
            stale.is_empty(),
            "{page} has version slot(s) reading {stale:?} rather than {}. \
             (A version in prose is fine and is not counted -- write history as \
             a bare `vX.Y.Z`, and the product's current version as \
             `texrs vX.Y.Z`.)",
            crate_version()
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
