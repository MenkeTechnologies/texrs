//! Offline generator for `docs/reference.html`.
//!
//! ```sh
//! cargo run --bin gen-docs
//! ```
//!
//! Source of truth: `texrs::corpus::CORPUS`, the same table the language server
//! answers completion and hover from. The page and the editor therefore cannot
//! drift: a primitive is documented only if the engine dispatches it, which
//! `tests/corpus_coverage.rs` gates in both directions.

fn main() {
    let out = "docs/reference.html";
    let page = texrs::docs::reference_html();
    if let Err(e) = std::fs::write(out, &page) {
        eprintln!("gen-docs: cannot write {out}: {e}");
        std::process::exit(1);
    }
    // Counted off what the language server served, since that is what the page
    // was rendered from: a primitive the server stops resolving should make this
    // number fall rather than keep reporting the corpus's optimistic total.
    println!(
        "wrote {out} ({} entries, {} chapters)",
        texrs::lsp::served().len(),
        texrs::corpus::CHAPTERS.len()
    );
}
