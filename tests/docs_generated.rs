//! `docs/reference.html` is generated, and the committed copy must match what
//! the generator produces right now.
//!
//! The page is what the project publishes; the corpus is what the engine and
//! the language server answer from. If someone hand-edits the page, the two
//! diverge silently and the site starts describing an engine that does not
//! exist. This test is what makes the file a build artifact in practice rather
//! than in intention.
//!
//! When it fails, the fix is `cargo run --bin gen-docs`, not an edit to the
//! HTML.

#[test]
fn the_committed_reference_matches_the_generator() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/reference.html");
    let committed = std::fs::read_to_string(&path).expect("docs/reference.html");
    let generated = texrs::docs::reference_html();
    assert!(
        committed == generated,
        "docs/reference.html is {} bytes and the generator produces {} -- run \
         `cargo run --bin gen-docs` and commit the result",
        committed.len(),
        generated.len()
    );
}

#[test]
fn every_chapter_reaches_the_page() {
    let page = texrs::docs::reference_html();
    for chapter in texrs::corpus::CHAPTERS {
        assert!(
            page.contains(&format!("<h2>{chapter}</h2>")),
            "chapter {chapter:?} has no section on the page"
        );
    }
    for (name, ..) in texrs::corpus::CORPUS {
        let escaped = name.replace('&', "&amp;").replace('<', "&lt;");
        assert!(
            page.contains(&format!("<code>{escaped}</code>")),
            "{name} is in the corpus but not on the page"
        );
    }
}

#[test]
fn the_page_stamps_the_crate_version() {
    let page = texrs::docs::reference_html();
    assert!(
        page.contains(&format!("texrs v{}", env!("CARGO_PKG_VERSION"))),
        "the page does not carry the current crate version -- the meta \
         version-sync gate compares docs/*.html against Cargo.toml"
    );
    assert!(
        !page.contains("__TEXRS_VERSION__"),
        "the version placeholder survived into the page"
    );
}
