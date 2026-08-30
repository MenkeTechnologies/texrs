//! Lexing TeX in parallel.
//!
//! TeX's mouth looks strictly sequential -- it is a state machine over bytes
//! whose meaning depends on a mutable category-code table -- and every engine
//! since 1978 has run it on one core. Two properties make it splittable anyway:
//!
//! 1. **Every line starts in the same state.** `tex.web` §303's three states
//!    (`N` new line, `M` mid line, `S` skipping blanks) are per-line: a line end
//!    sets `N`, and so does a comment. So a chunk that begins at a line boundary
//!    begins in `N` no matter what came before it, and needs no context from the
//!    chunk before it. This is what makes the split legal at all.
//!
//! 2. **The catcode table is 256 bytes.** Handing every worker its own copy
//!    costs nothing, so each chunk lexes under the table as it stands now.
//!
//! What that does NOT survive is a `\catcode` assignment part way through the
//! file: everything after it was lexed under the wrong table. The caller is
//! responsible for noticing and re-lexing the tail: [`crate::catcode::CatTable`]
//! carries a generation counter, and `Lexer::next_token` drops its read-ahead
//! (counting the drop in [`DROP_GEN`]) the moment the table it was lexed under
//! is superseded. A document sets its catcodes in a preamble and leaves them alone, so
//! the speculation is right for the bulk of real input and wrong only where it
//! is cheap to find out.
//!
//! Uses `std::thread::scope` rather than a thread-pool crate: the work is a
//! single fan-out over borrowed input, which scoped threads express directly and
//! without adding a dependency to vendor.

use crate::catcode::CatTable;
use crate::lexer::Lexer;
use crate::token::Token;

/// Where a chunk began, so a caller can re-lex from a byte offset.
pub struct Chunk {
    /// Byte offset into the source this chunk started at.
    pub start: usize,
    pub tokens: Vec<Token>,
}

/// How many workers to use by default: one per core, and never more chunks
/// than there is work to justify them.
pub fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// Split at line boundaries into at most `n` roughly equal pieces.
///
/// Returns byte offsets where each piece begins. Splitting anywhere else would
/// break the per-line state invariant that makes this sound.
pub fn line_aligned_bounds(src: &str, n: usize) -> Vec<usize> {
    if n <= 1 || src.is_empty() {
        return vec![0];
    }
    let target = src.len() / n;
    if target == 0 {
        return vec![0];
    }
    let bytes = src.as_bytes();
    let mut bounds = vec![0usize];
    let mut want = target;
    while want < src.len() {
        // Walk forward to the first byte after the next newline.
        match bytes[want..].iter().position(|b| *b == b'\n') {
            Some(off) => {
                let at = want + off + 1;
                if at >= src.len() {
                    break;
                }
                if at > *bounds.last().expect("seeded") {
                    bounds.push(at);
                }
                want = at + target;
            }
            None => break,
        }
    }
    bounds
}

/// Lex the whole source in parallel, speculating that the catcode table does
/// not change part way through.
///
/// The result is the concatenation of the chunks' tokens, which equals what the
/// sequential mouth produces WHEN the speculation holds. `tests/parallel_lex.rs`
/// pins that equality against the sequential lexer rather than asserting it.
pub fn lex_parallel(src: &str, cats: &CatTable, threads: usize) -> Vec<Chunk> {
    let bounds = line_aligned_bounds(src, threads.max(1));
    if bounds.len() == 1 {
        return vec![Chunk {
            start: 0,
            tokens: lex_range(src, cats),
        }];
    }
    let mut out: Vec<Chunk> = Vec::with_capacity(bounds.len());
    std::thread::scope(|s| {
        let mut handles = Vec::with_capacity(bounds.len());
        for (i, &start) in bounds.iter().enumerate() {
            let end = bounds.get(i + 1).copied().unwrap_or(src.len());
            let piece = &src[start..end];
            handles.push((start, s.spawn(move || lex_range(piece, cats))));
        }
        for (start, h) in handles {
            let tokens = h.join().unwrap_or_default();
            out.push(Chunk { start, tokens });
        }
    });
    out
}

/// The sequential mouth over one piece, which is what each worker runs.
fn lex_range(piece: &str, cats: &CatTable) -> Vec<Token> {
    let mut lx = Lexer::new(piece);
    let mut toks = Vec::new();
    while let Some(t) = lx.next_token(cats) {
        toks.push(t);
    }
    toks
}

/// Every token, flattened, for a caller that just wants the stream.
pub fn lex_parallel_flat(src: &str, cats: &CatTable, threads: usize) -> Vec<Token> {
    lex_parallel(src, cats, threads)
        .into_iter()
        .flat_map(|c| c.tokens)
        .collect()
}

/// One token and the character index the mouth had reached after reading it.
///
/// The position is what makes a speculative stream abandonable: when
/// `\catcode` moves the table on, everything cached from that point is wrong,
/// and scanning has to resume at exactly the character the last good token
/// ended on.
pub type Placed = (Token, usize);

/// Character indices where a line begins, which are the only legal split points.
fn line_starts(chars: &[char], want: usize) -> Vec<usize> {
    if want <= 1 || chars.is_empty() {
        return vec![0];
    }
    let target = chars.len() / want;
    if target == 0 {
        return vec![0];
    }
    let mut bounds = vec![0usize];
    let mut i = target;
    while i < chars.len() {
        match chars[i..].iter().position(|c| *c == '\n') {
            Some(off) => {
                let at = i + off + 1;
                if at >= chars.len() {
                    break;
                }
                if at > *bounds.last().expect("seeded") {
                    bounds.push(at);
                }
                i = at + target;
            }
            None => break,
        }
    }
    bounds
}

/// Pre-lex `chars[from..]` across `threads` workers, under `cats`.
///
/// Returns every token with the character index it ended at, in source order.
/// Each worker gets a line-aligned slice, which is sound because the mouth's
/// three states are per-line (see the module docs).
pub static DROP_GEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub static DROP_EXHAUST: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub static PRELEX_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub static PRELEX_CHARS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn prelex(chars: &[char], from: usize, cats: &CatTable, threads: usize) -> Vec<Placed> {
    PRELEX_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    PRELEX_CHARS.fetch_add(
        chars.len().saturating_sub(from),
        std::sync::atomic::Ordering::Relaxed,
    );
    let rest = &chars[from.min(chars.len())..];
    if rest.is_empty() {
        return Vec::new();
    }
    let bounds = line_starts(rest, threads.max(1));
    if bounds.len() == 1 {
        return lex_slice(rest, from, cats);
    }
    let mut parts: Vec<Vec<Placed>> = Vec::with_capacity(bounds.len());
    std::thread::scope(|s| {
        let mut hs = Vec::with_capacity(bounds.len());
        for (i, &start) in bounds.iter().enumerate() {
            let end = bounds.get(i + 1).copied().unwrap_or(rest.len());
            let piece = &rest[start..end];
            let base = from + start;
            hs.push(s.spawn(move || lex_slice(piece, base, cats)));
        }
        for h in hs {
            parts.push(h.join().unwrap_or_default());
        }
    });
    parts.concat()
}

/// The sequential mouth over one slice, tagging each token with its absolute
/// end position.
fn lex_slice(piece: &[char], base: usize, cats: &CatTable) -> Vec<Placed> {
    let mut lx = Lexer::from_chars(piece.to_vec());
    let mut out = Vec::new();
    while let Some(t) = lx.next_token(cats) {
        out.push((t, base + lx.pos()));
    }
    out
}
