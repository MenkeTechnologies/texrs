//! The Emacs mode's primitive table is generated from the engine's corpus, and
//! this is what keeps the committed file equal to what the generator prints.
//!
//! Without it the elisp is a hand-kept copy that drifts the moment a primitive
//! is added — silently, because nothing in Emacs would notice a name that is
//! missing from a completion list.

use std::path::Path;

#[test]
fn the_committed_elisp_is_what_the_generator_prints() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("editors/emacs/texrs-stdlib.el");
    let committed = std::fs::read_to_string(&path).expect("editors/emacs/texrs-stdlib.el");
    assert_eq!(
        committed,
        texrs::docs::emacs_stdlib_el(),
        "editors/emacs/texrs-stdlib.el is stale — run `cargo run --bin gen-emacs-stdlib`"
    );
}

#[test]
fn every_primitive_the_engine_documents_is_offered_to_the_editor() {
    let elisp = texrs::docs::emacs_stdlib_el();
    for (name, ..) in texrs::corpus::CORPUS {
        // The name appears in the completion list and again in the doc table,
        // so a primitive is both offerable and explainable.
        assert!(
            elisp.matches(&format!("{name:?}")).count() >= 2,
            "{name} is not both listed and documented in the generated elisp"
        );
    }
    // The elisp is loadable as a whole: it ends with the provide and the
    // conventional trailer, which Emacs' package tooling checks for.
    assert!(
        elisp.ends_with(";;; texrs-stdlib.el ends here\n"),
        "{elisp}"
    );
    assert!(elisp.contains("(provide 'texrs-stdlib)"));
}
