//! Fuzz the frontend: mouth + expander + lowering + bytecode emission.
//!
//! Invariant: an input either compiles to a `Chunk` or is rejected with a
//! `TexError`. A panic is a bug -- an unwrap on a token that was not there, a
//! subtraction under zero in the group depth, an index past the end of a macro
//! parameter list.
//!
//! Nothing is executed here, so the fuzzer measures the half of texrs that a
//! macro-heavy document spends its time in without also measuring fusevm.
//!
//! A runaway macro (`\def\x{\x}\x`) expands forever, exactly as real tex does --
//! neither engine has a step budget, so this is parity, not a bug. libfuzzer's
//! `-timeout` is what bounds it:
//!
//!   cargo +nightly fuzz run lower -- -timeout=10

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    if s.len() > 32_768 {
        return;
    }
    let _ = texrs::compile(s);
});
