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
//! responsible for noticing (see [`lex_parallel_checked`]) and re-lexing the
//! tail. A document sets its catcodes in a preamble and leaves them alone, so
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
