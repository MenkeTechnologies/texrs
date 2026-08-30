//! The runtime hook a standalone AOT binary needs.
//!
//! fusevm's AOT model puts the bincode-serialized `Chunk` in the object and, at
//! load, deserializes it and runs it on a `VM` (`fusevm_aot_run_embedded`).
//! Before running, it calls back into the frontend through the C symbol
//! `fusevm_aot_register_builtins` to install the frontend's builtins on that VM.
//! A standalone texrs binary is that object plus this runtime (the hook and the
//! `\message` builtins) plus a `main` that calls the entry below.
//!
//! texrs needs only the two message builtins, and the whole document is in the
//! chunk — there is no host-side table to rebuild, unlike the sibling frontends
//! whose closures live outside their bytecode. That is what makes this file
//! twenty lines rather than two hundred: everything a TeX document does at run
//! time is register writes, branches and `\message`, and all three are ops.

use fusevm::VM;

thread_local! {
    /// The document the embedded chunk was compiled from, recovered by the
    /// register hook so the entry can print the same `(./file.tex … )` line an
    /// ordinary run prints. fusevm hands the entry an exit code, not the chunk,
    /// so the hook — which does get the VM, and therefore the chunk — is the
    /// only place this can be read.
    static AOT_SOURCE: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

/// Install texrs's builtins on the AOT run's VM.
///
/// # Safety
///
/// Called by fusevm's AOT entry with the VM it is about to run.
#[no_mangle]
pub unsafe extern "C" fn fusevm_aot_register_builtins(vm: *mut VM) {
    if vm.is_null() {
        return;
    }
    let vm = &mut *vm;
    let source = vm.chunk.source.clone();
    AOT_SOURCE.with(|s| *s.borrow_mut() = source);
    crate::runtime::register_message_builtins(vm);
}

/// The `main` a linked texrs AOT binary calls.
///
/// Runs the embedded chunk, then prints the `\message` stream the way an
/// ordinary run prints it — the compiled document has to be observationally the
/// same program, or "compiled ahead of time" would mean "behaves differently".
#[no_mangle]
pub extern "C" fn texrs_aot_main() -> i64 {
    let code = fusevm::aot::fusevm_aot_run_embedded();
    let msgs = crate::runtime::take_messages();
    let body = match msgs.is_empty() {
        true => String::new(),
        false => format!(" {}", msgs.join(" ")),
    };
    let name = AOT_SOURCE.with(|s| s.borrow().clone());
    println!("(./{name}{body} )");
    code
}
