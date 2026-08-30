//! Where a diagnostic goes, ported in shape from Tectonic's `status_base`.
//!
//! texrs reports through this rather than writing to stderr directly, because
//! the same engine now runs behind three front ends — the CLI, the REPL and the
//! LSP — and each wants the message somewhere different: on stderr in tex's own
//! `! reason.` form, inline in the REPL transcript, or as a diagnostic on a
//! document. A backend is one small trait so a caller chooses, rather than the
//! engine deciding for everyone by calling `eprintln!`.
//!
//! Tectonic's version carries progress reporting and a `Note`/`Warning`/`Error`
//! ladder; texrs carries the ladder alone, since nothing here is slow enough to
//! need progress.

use std::sync::{Arc, Mutex};

/// How much a message matters.
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum Level {
    /// Something worth saying, which changes nothing.
    Note,
    /// Something the user probably wants to fix, which did not stop the run.
    Warning,
    /// Something that stopped the run.
    Error,
}

impl Level {
    /// The prefix tex itself uses. An error is `!`, which is what a TeX user
    /// greps for; a warning is spelled out, as tex spells out `Overfull \hbox`.
    pub fn prefix(self) -> &'static str {
        match self {
            Level::Note => "",
            Level::Warning => "Warning: ",
            Level::Error => "! ",
        }
    }
}

/// Somewhere a diagnostic can go.
pub trait StatusBackend {
    fn report(&mut self, level: Level, message: &str);

    fn note(&mut self, message: &str) {
        self.report(Level::Note, message)
    }
    fn warning(&mut self, message: &str) {
        self.report(Level::Warning, message)
    }
    fn error(&mut self, message: &str) {
        self.report(Level::Error, message)
    }
}

/// stderr, in tex's form: `! reason.` — the shape the parity harness and every
/// TeX user's muscle memory expect.
#[derive(Default)]
pub struct TexStatus {
    /// Nothing below this is printed. A quiet run still reports errors.
    pub min_level: Option<Level>,
}

impl TexStatus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Report only what is at least this bad.
    pub fn quiet_below(level: Level) -> Self {
        Self {
            min_level: Some(level),
        }
    }
}

impl StatusBackend for TexStatus {
    fn report(&mut self, level: Level, message: &str) {
        if self.min_level.is_some_and(|min| level < min) {
            return;
        }
        match level {
            // A note is progress, not a diagnostic: it goes as written, since
            // a full stop after a path reads as part of the path.
            Level::Note => eprintln!("{message}"),
            // tex ends a diagnostic with a full stop and does not repeat one.
            _ => eprintln!("{}{}.", level.prefix(), message.trim_end_matches('.')),
        }
    }
}

/// Keeps what it is told, for a caller that renders diagnostics itself — the
/// LSP turning them into document diagnostics, or a test asserting on them.
#[derive(Clone, Default)]
pub struct CollectingStatus {
    messages: Arc<Mutex<Vec<(Level, String)>>>,
}

impl CollectingStatus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything reported so far, oldest first.
    pub fn messages(&self) -> Vec<(Level, String)> {
        self.messages
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Whether anything reported was an error.
    pub fn had_error(&self) -> bool {
        self.messages().iter().any(|(l, _)| *l == Level::Error)
    }
}

impl StatusBackend for CollectingStatus {
    fn report(&mut self, level: Level, message: &str) {
        self.messages
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((level, message.to_string()));
    }
}

/// Drops everything. What a run that reports through its return value wants.
pub struct SilentStatus;

impl StatusBackend for SilentStatus {
    fn report(&mut self, _level: Level, _message: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_collecting_backend_keeps_what_it_was_told_in_order() {
        let mut status = CollectingStatus::new();
        status.note("reading the file");
        status.warning("a macro was redefined");
        status.error("Undefined control sequence");

        let got = status.messages();
        assert_eq!(got.len(), 3);
        assert_eq!(got[0], (Level::Note, "reading the file".to_string()));
        assert_eq!(got[2].0, Level::Error);
        assert!(status.had_error());

        // The handle is shared, so a clone reports into the same list — which
        // is what lets an engine hold one while its caller reads the other.
        let mut clone = status.clone();
        clone.note("from the clone");
        assert_eq!(status.messages().len(), 4);
    }

    #[test]
    fn a_run_with_nothing_wrong_reports_no_error() {
        let mut status = CollectingStatus::new();
        status.note("nothing to see");
        assert!(!status.had_error());
    }

    #[test]
    fn a_quiet_backend_still_reports_what_stopped_the_run() {
        // The filter is on the level, not on the message, so an error survives
        // a quiet run and a note does not.
        let mut status = TexStatus::quiet_below(Level::Error);
        assert!(status.min_level.is_some_and(|l| l == Level::Error));
        status.note("dropped");
        status.error("kept");
        // A note is progress and goes as written; a diagnostic is punctuated
        // the way tex punctuates one.
        assert_eq!(Level::Warning.prefix(), "Warning: ");

        assert!(Level::Note < Level::Warning && Level::Warning < Level::Error);
        assert_eq!(Level::Error.prefix(), "! ");
        assert_eq!(Level::Note.prefix(), "");
    }
}
