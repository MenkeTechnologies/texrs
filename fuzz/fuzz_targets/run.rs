//! Fuzz the whole pipeline: source to `\message` stream on the VM.
//!
//! Adds the two stages `lower` stops short of -- fusevm executing the chunk and
//! the host `\message` callbacks assembling their strings -- so a chunk that
//! compiles but that the VM rejects (a bad slot, an unbalanced stack) shows up
//! here rather than in a document.
//!
//! Same hang caveat as `lower`: bound it with `-timeout`.
//!
//!   cargo +nightly fuzz run run -- -timeout=10

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    if s.len() > 32_768 {
        return;
    }
    let _ = texrs::run_messages(s);
});
