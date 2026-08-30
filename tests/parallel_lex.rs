//! Parallel lexing has to produce EXACTLY what the sequential mouth produces.
//!
//! The split is legal because `tex.web` §303's three mouth states are per-line:
//! a line end sets state `N`, so a chunk starting at a line boundary needs no
//! context from the chunk before it. That is an argument, not a proof, so these
//! check it against the sequential lexer on inputs built to stress exactly the
//! places where per-line state matters -- blank lines (which become `\par`),
//! runs of spaces (collapsed, but only mid-line), comments (which eat to the
//! line end), and control words (which swallow the spaces after them).

use texrs::catcode::CatTable;
use texrs::lexer::Lexer;
use texrs::parallel::{lex_parallel_flat, line_aligned_bounds};
use texrs::token::Token;

fn sequential(src: &str, cats: &CatTable) -> Vec<Token> {
    let mut lx = Lexer::new(src);
    let mut out = Vec::new();
    while let Some(t) = lx.next_token(cats) {
        out.push(t);
    }
    out
}

fn agrees(src: &str) {
    let cats = CatTable::new();
    let want = sequential(src, &cats);
    for threads in [1usize, 2, 3, 4, 8, 16] {
        let got = lex_parallel_flat(src, &cats, threads);
        assert_eq!(
            got, want,
            "parallel lexing with {threads} threads disagreed with the mouth"
        );
    }
}

#[test]
fn plain_text_agrees() {
    agrees("hello world\nsecond line here\nthird\n");
}

#[test]
fn blank_lines_become_par_the_same_way() {
    // A blank line is `\par`, and only in state N. If a chunk began mid-line
    // this would differ, which is the whole risk being checked.
    agrees("one\n\ntwo\n\n\nthree\n");
}

#[test]
fn space_runs_and_control_words_agree() {
    // A control word swallows following spaces (state S); a run of spaces
    // collapses to one, but only mid-line.
    agrees("\\foo   bar\n   leading spaces\n\\a\\b \\c   \n");
}

#[test]
fn comments_that_eat_the_line_agree() {
    agrees("real text % comment to end\nmore\n%whole line comment\nlast\n");
}

#[test]
fn a_large_mixed_document_agrees() {
    // Big enough that every thread count actually splits it several ways.
    let mut src = String::new();
    for i in 0..4000 {
        match i % 5 {
            0 => src.push_str("\\message{line}\n"),
            1 => src.push_str("\n"),
            2 => src.push_str("% comment\n"),
            3 => src.push_str("plain   words   here\n"),
            _ => src.push_str("\\def\\x{body}\n"),
        }
    }
    agrees(&src);
}

#[test]
fn chunk_bounds_always_land_on_line_starts() {
    // The soundness argument depends on this and nothing else, so it is worth
    // asserting directly rather than only through the token comparison.
    let src = "alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\neta\ntheta\n";
    for n in [2usize, 3, 4, 9] {
        for b in line_aligned_bounds(src, n) {
            assert!(
                b == 0 || src.as_bytes()[b - 1] == b'\n',
                "bound {b} is not just after a newline for n={n}"
            );
        }
    }
}

#[test]
fn an_empty_or_tiny_source_does_not_split() {
    agrees("");
    agrees("x");
    agrees("\n");
}
