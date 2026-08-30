//! Advice on macro expansion: before, after and around, matched by glob.
//!
//! The same aspect-oriented idea the sibling engines carry (`zshrs`'s function
//! intercepts, `rubylang`'s method intercepts), placed where it means something
//! in TeX: a macro CALL. A document registers advice with `\intercept`, and
//! every expansion of a matching macro carries it.
//!
//! ```tex
//! \def\greet#1{HELLO-#1}
//! \def\trace{[in]}
//! \intercept{before}{greet}{\trace}
//! \message{\greet{WORLD}}      % => [in]HELLO-WORLD
//! ```
//!
//! **Where the weave happens.** Expansion is a COMPILE-time act in texrs — the
//! macro is gone by the time the VM starts — so advice is woven into the token
//! stream, not into anything at run time. `before` puts the handler's body in
//! front of the expansion, `after` puts it behind, and `around` replaces it,
//! with `\proceed` standing for what the macro would have expanded to.
//!
//! **Why glob rather than a name.** The useful intercepts are categorical:
//! `\intercept{before}{sec*}{\logsection}` catches every sectioning macro a
//! package defines, including the ones defined after the advice was registered.
//! A registry keyed by exact name would need the document to know every name up
//! front, which is the thing a macro package makes impossible.

use glob::Pattern;

/// When advice runs relative to the expansion it advises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Advice {
    Before,
    After,
    Around,
}

impl Advice {
    /// The keyword a document writes, or `None` if it wrote something else.
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "before" => Some(Advice::Before),
            "after" => Some(Advice::After),
            "around" => Some(Advice::Around),
            _ => None,
        }
    }
}

/// One registration: a pattern, when it fires, and whose body to weave.
#[derive(Debug, Clone)]
pub struct Intercept {
    pub pattern: Pattern,
    pub advice: Advice,
    /// The handler macro's name, without the escape character.
    pub handler: String,
}

/// Every registration, in the order they were made.
///
/// Order is the semantics: two `before` advices on the same macro run in the
/// order they were registered, and nested `around` advices wrap in that order
/// too, so the first registered is the outermost.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    list: Vec<Intercept>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether anything is registered at all.
    ///
    /// The expander asks this before doing any work per macro call, so a
    /// document that registers no advice pays one boolean for the feature.
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Register `handler` to run as `advice` on every macro whose name matches
    /// the glob `pattern`.
    pub fn register(&mut self, pattern: &str, advice: Advice, handler: &str) -> Result<(), String> {
        let pat =
            Pattern::new(pattern).map_err(|e| format!("Bad intercept pattern `{pattern}': {e}"))?;
        self.list.push(Intercept {
            pattern: pat,
            advice,
            handler: handler.to_string(),
        });
        Ok(())
    }

    /// The advice registered for a macro name, in registration order.
    pub fn matching(&self, name: &str) -> Vec<&Intercept> {
        self.list
            .iter()
            .filter(|i| i.pattern.matches(name))
            .collect()
    }

    /// Forget every registration. A group restores this, the way it restores
    /// the macro table.
    pub fn clear(&mut self) {
        self.list.clear();
    }
}
