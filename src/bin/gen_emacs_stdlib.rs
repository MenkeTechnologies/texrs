//! Offline generator for `editors/emacs/texrs-stdlib.el`.
//!
//! ```sh
//! cargo run --bin gen-emacs-stdlib
//! ```
//!
//! Source of truth: `texrs::corpus::CORPUS`, the same table the language server
//! answers completion and hover from and `docs/reference.html` is rendered from.
//! Generating the elisp rather than writing it means the Emacs mode's completion
//! list and eldoc strings cannot drift from what the engine dispatches;
//! `tests/emacs_stdlib.rs` fails if the committed file is not what this prints.

fn main() {
    let out = "editors/emacs/texrs-stdlib.el";
    let text = texrs::docs::emacs_stdlib_el();
    if let Err(e) = std::fs::write(out, &text) {
        eprintln!("gen-emacs-stdlib: cannot write {out}: {e}");
        std::process::exit(1);
    }
    println!("wrote {out} ({} primitives)", texrs::corpus::CORPUS.len());
}
