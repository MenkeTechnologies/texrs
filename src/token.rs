//! What the mouth produces: TeX has exactly two kinds of token.

use crate::catcode::Cat;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::RwLock;

/// A control sequence name, interned to a 32-bit id.
///
/// `tex.web` §256 does the same thing and for the same reason: the name is
/// looked up ONCE, in the hash table, and everything after that carries the
/// resulting `eqtb` pointer. A token then costs no allocation, comparing two is
/// an integer compare, and the meaning table is an array index rather than a
/// string hash.
///
/// It matters here because expansion copies token lists constantly -- every
/// macro use clones its body -- and a `String` per control sequence made each
/// copy a run of heap allocations. The names themselves are few (a document has
/// hundreds of distinct control sequences and expands them millions of times),
/// so interning trades a bounded table for an unbounded number of allocations.
#[derive(Clone, Copy)]
pub struct CsId(&'static str);

// Interning makes one canonical `&'static str` per distinct name, so two ids
// for the same name are the same pointer. Comparing and hashing THAT rather
// than the characters is what keeps a meaning lookup cheap: the table hashes a
// pointer, not a string, and `\ifx` compares two words.
impl PartialEq for CsId {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0.as_ptr(), other.0.as_ptr()) && self.0.len() == other.0.len()
    }
}
impl Eq for CsId {}
impl std::hash::Hash for CsId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (self.0.as_ptr() as usize).hash(state);
    }
}
impl std::fmt::Debug for CsId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cs({})", self.0)
    }
}

/// The name table. Written once per NEW name and read on every resolve, which
/// is what an `RwLock` is for; the write side is cold after warm-up.
///
/// Names are leaked to `&'static str` deliberately. A control sequence name,
/// once seen, is live for the rest of the run -- the meaning table, every token
/// carrying its id, and every diagnostic can refer to it -- so there is nothing
/// to free and no lifetime to thread through the engine. The set is bounded by
/// the document's DISTINCT control sequences (hundreds), not by how often they
/// are expanded (millions), which is exactly the trade interning exists to make.
struct Names {
    to_id: HashMap<&'static str, &'static str>,
    count: usize,
}

static NAMES: Lazy<RwLock<Names>> = Lazy::new(|| {
    RwLock::new(Names {
        to_id: HashMap::new(),
        count: 0,
    })
});

impl CsId {
    /// The id for a name, interning it if this is the first sighting.
    ///
    /// The lock is taken HERE and nowhere else on a hot path: interning happens
    /// once per control sequence the mouth reads, while resolving one happens
    /// once per expansion -- millions of times. An earlier revision stored an
    /// index and resolved it through the table, which put a lock acquire in
    /// front of every primitive dispatch and measured 25% SLOWER than the
    /// `String` it replaced. Carrying the canonical pointer means resolving is
    /// free.
    pub fn intern(name: &str) -> CsId {
        // A per-thread cache in front of the shared table. The mouth interns
        // every control sequence it reads, and a document says `\the` or a
        // macro's own name thousands of times, so the overwhelmingly common
        // case is a name this thread has already seen. Answering that without
        // touching the shared lock is what keeps interning from costing more
        // than the `String` it replaced.
        thread_local! {
            static SEEN: std::cell::RefCell<HashMap<Box<str>, &'static str>> =
                std::cell::RefCell::new(HashMap::new());
        }
        if let Some(hit) = SEEN.with(|c| c.borrow().get(name).copied()) {
            return CsId(hit);
        }
        let canonical = Self::intern_shared(name);
        SEEN.with(|c| c.borrow_mut().insert(name.into(), canonical.0));
        canonical
    }

    /// The shared table, taken only when this thread has not seen the name.
    fn intern_shared(name: &str) -> CsId {
        if let Some(s) = NAMES.read().expect("names").to_id.get(name) {
            return CsId(s);
        }
        let mut n = NAMES.write().expect("names");
        // Re-check: another thread may have interned it between the two locks.
        if let Some(s) = n.to_id.get(name) {
            return CsId(s);
        }
        let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
        n.to_id.insert(leaked, leaked);
        n.count += 1;
        CsId(leaked)
    }

    /// The name behind an id.
    ///
    /// The id IS the canonical name, so this is a field read -- no lock, no
    /// table, no allocation. The primitive dispatch stays a `match` on string
    /// literals while tokens stay 16 bytes of `Copy`.
    pub fn name(self) -> &'static str {
        self.0
    }

    /// How many distinct control sequences have been seen. For tests and
    /// `--cache-stats`; a document's count is small and bounded.
    pub fn interned_count() -> usize {
        NAMES.read().expect("names").count
    }
}

/// A token, as `tex.web` §289 defines one.
///
/// A character token carries the catcode it had WHEN IT WAS READ, not the
/// current one — that is why `\catcode`\{=1` after a macro was defined does not
/// retroactively change that macro's body. Keeping the pair together is what
/// makes the distinction representable at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Token {
    Char(char, Cat),
    /// A control sequence, stored WITHOUT its escape character, as an interned
    /// id rather than a name.
    Cs(CsId),
}

impl Token {
    /// A control sequence token from a name, interning it.
    pub fn cs(name: &str) -> Token {
        Token::Cs(CsId::intern(name))
    }

    /// The name of a control-sequence token, or `None` for a character.
    pub fn cs_name(&self) -> Option<&'static str> {
        match self {
            Token::Cs(id) => Some(id.name()),
            Token::Char(..) => None,
        }
    }

    /// Whether this is the named control sequence, without materialising the
    /// name: the comparison is against an interned id.
    pub fn is_cs(&self, name: &str) -> bool {
        matches!(self, Token::Cs(id) if *id == CsId::intern(name))
    }

    /// The text `\string` and `\message` produce for this token.
    ///
    /// A multi-letter control sequence prints with a trailing space and a
    /// single-character one does not (`tex.web` §294's `print_cs`) — the rule
    /// that makes `\message{\foo}` read `\foo ` and `\message{\!}` read `\!`.
    pub fn to_text(&self, escape: char) -> String {
        match self {
            Token::Char(c, _) => c.to_string(),
            Token::Cs(id) => {
                let name = id.name();
                let single = name.chars().count() == 1;
                match single {
                    true => format!("{escape}{name}"),
                    false => format!("{escape}{name} "),
                }
            }
        }
    }

    pub fn is_space(&self) -> bool {
        matches!(self, Token::Char(_, Cat::Space))
    }
}
