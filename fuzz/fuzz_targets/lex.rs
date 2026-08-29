//! Fuzz the mouth alone: bytes to tokens.
//!
//! Invariant: any valid UTF-8 input tokenises to completion or stops, and never
//! panics. The mouth is the one stage with no expansion in it, so it always
//! terminates — which makes it the only target safe to point at arbitrary
//! mutations of a real file without a timeout doing the terminating.
//!
//! The interesting inputs are the ones that exercise the three-state line
//! scanner and `^^X` notation: a lone `^`, `^^` at end of input, a control
//! sequence with nothing after the backslash, a line ending mid-`^^X`.
//!
//!   cargo +nightly fuzz run lex

#![no_main]

use libfuzzer_sys::fuzz_target;
use texrs::catcode::CatTable;
use texrs::lexer::Lexer;

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    // 64 KB: past that the mutator is burning time on volume rather than on new
    // edges, and no committed case is within two orders of magnitude of it.
    if s.len() > 65_536 {
        return;
    }
    let cats = CatTable::new();
    let mut lx = Lexer::new(s);
    while lx.next_token(&cats).is_some() {}
});
