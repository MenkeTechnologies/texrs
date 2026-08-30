//! Ahead-of-time compilation: a TeX document as a standalone native binary.
//!
//! `texrs --aot doc.tex` lowers the document to a fusevm `Chunk`, emits it as a
//! relocatable native object through `fusevm::aot::compile_object`, and links it
//! against the texrs runtime staticlib into an executable that runs the document
//! with no interpreter dispatch loop and no texrs on the machine.
//!
//! What makes this short compared with the sibling frontends' AOT paths: their
//! closures live in a host-side table outside the bytecode, so their objects
//! have to smuggle a serialized image of it through the chunk's name table and
//! rebuild it before the driver runs. texrs has nothing outside the chunk. A
//! macro is gone by the time the VM starts — expansion happened at compile time
//! — and what remains is register writes, branches and `\message`, all of them
//! ops. The chunk IS the program.
//!
//! The staticlib is `libtexrs.a`, which cargo builds beside the ordinary
//! library because the crate declares `crate-type = ["lib", "staticlib"]`. It
//! carries `src/aot_runtime.rs`'s hook and entry, which is what resolves the
//! symbols fusevm's object imports.

use std::path::{Path, PathBuf};

/// AOT-compile `file` to a standalone native executable at `out`.
///
/// Returns the path written. Requires a C toolchain (`cc`) for the link step
/// and the texrs staticlib, which `cargo build` produces.
pub fn compile_executable(file: &str, out: &Path) -> Result<PathBuf, String> {
    let src = std::fs::read_to_string(file).map_err(|e| format!("cannot read {file}: {e}"))?;
    let mut chunk = crate::compile(&src).map_err(|e| e.0)?;
    // The compiled binary prints the same `(./file.tex … )` line an ordinary run
    // prints, and this is where it learns the name: fusevm carries `source`
    // through serialization, and the register hook reads it back.
    chunk.source = file.to_string();

    let runtime_lib = runtime_staticlib()?;
    let obj = out.with_extension("o");
    fusevm::aot::compile_object(&chunk, &obj).map_err(|e| format!("--aot: {e}"))?;

    // The C entry exists so the link has a `main` in the language the linker
    // expects one from; it does nothing but call the Rust entry.
    let stub = out.with_extension("aot_main.c");
    std::fs::write(
        &stub,
        b"extern long texrs_aot_main(void);\nint main(void){return (int)texrs_aot_main();}\n"
            as &[u8],
    )
    .map_err(|e| format!("--aot: write entry stub: {e}"))?;

    let mut cmd = std::process::Command::new("cc");
    cmd.arg(&stub).arg(&obj).arg(&runtime_lib);
    // The platform libraries a Rust staticlib pulls in.
    if cfg!(target_os = "macos") {
        cmd.args([
            "-framework",
            "CoreFoundation",
            "-framework",
            "Security",
            "-liconv",
            "-lc++",
        ]);
    } else {
        cmd.args(["-lpthread", "-ldl", "-lm", "-lrt"]);
    }
    cmd.arg("-o").arg(out);

    let output = cmd
        .output()
        .map_err(|e| format!("--aot: invoking cc: {e}"))?;
    // The intermediates are the compiler's business, not the user's.
    let _ = std::fs::remove_file(&stub);
    let _ = std::fs::remove_file(&obj);
    if !output.status.success() {
        return Err(format!(
            "--aot: link failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(out.to_path_buf())
}

/// Find `libtexrs.a`, the staticlib the AOT object links against.
///
/// `TEXRS_STATICLIB` overrides the search, which is what a cross-compile or an
/// installed copy needs; otherwise the target directory this build wrote is
/// searched, release before debug — a debug staticlib links and runs, it is just
/// slow, and saying which one was used beats silently picking either.
fn runtime_staticlib() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("TEXRS_STATICLIB") {
        let p = PathBuf::from(p);
        return match p.is_file() {
            true => Ok(p),
            false => Err(format!(
                "--aot: TEXRS_STATICLIB={} is not a file",
                p.display()
            )),
        };
    }
    // `target/<profile>/libtexrs.a`, from wherever the running binary sits.
    let exe = std::env::current_exe().map_err(|e| format!("--aot: current_exe: {e}"))?;
    let dir = exe
        .parent()
        .ok_or_else(|| "--aot: binary has no directory".to_string())?;
    let candidates = [
        dir.join("libtexrs.a"),
        dir.join("../release/libtexrs.a"),
        dir.join("../debug/libtexrs.a"),
    ];
    for c in candidates {
        if c.is_file() {
            return Ok(c);
        }
    }
    Err(
        "--aot: no libtexrs.a found -- run `cargo build` first, or set \
         TEXRS_STATICLIB"
            .to_string(),
    )
}
