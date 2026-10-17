//! Ahead-of-time compilation: a TeX document as a standalone native binary.
//!
//! `texrs --aot doc.tex` lowers the document to a fusevm `Chunk`, emits it as a
//! relocatable native object through `fusevm::aot::compile_object`, and links it
//! against the texrs runtime staticlib into an executable that runs the document
//! with no interpreter dispatch loop and no texrs on the machine.
//!
//! A macro is gone by the time the VM starts — expansion happened at compile
//! time — so the chunk carries no closures, but it is not the whole program: it
//! produces the messages and the text, and the page builder sets that text with
//! fonts, a page shape and a page colour the lowering read. The chunk compiled
//! here is the one an ordinary PDF run executes ([`crate::compile_pdf_program`],
//! text output on, `.aux` read), and those page inputs travel beside it,
//! encoded by [`crate::aot_runtime::Embedded`] into the C entry stub.
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
    // What the lowering reports on stderr is held, so the binary can say it
    // again at run time, and released here too, so the compile says it now.
    let (program, reports) =
        crate::latex::held_reports(|| crate::compile_pdf_program(Some(Path::new(file)), &src));
    crate::latex::release_reports(&reports);
    let crate::PdfProgram {
        mut chunk,
        families,
        layout,
        page_colour,
    } = program.map_err(|e| e.0)?;
    chunk.source = file.to_string();
    let embedded = crate::aot_runtime::Embedded {
        name: file.to_string(),
        families,
        layout,
        page_colour,
        ended: crate::source_ends_run(&src),
        reports,
    };

    let runtime_lib = runtime_staticlib()?;
    let obj = out.with_extension("o");
    fusevm::aot::compile_object(&chunk, &obj).map_err(|e| format!("--aot: {e}"))?;

    // The C entry exists so the link has a `main` in the language the linker
    // expects one from, and it is where the page inputs live: an array handed
    // to the Rust entry, which is all it does.
    let stub = out.with_extension("aot_main.c");
    std::fs::write(&stub, entry_stub(&embedded.encode()))
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

/// The C `main`, with `page` as the array it hands `texrs_aot_main`.
fn entry_stub(page: &[u8]) -> String {
    use std::fmt::Write;
    let mut c = String::from(
        "extern long texrs_aot_main(const unsigned char *, unsigned long);\n\
         static const unsigned char page[] = {",
    );
    for (i, b) in page.iter().enumerate() {
        let sep = match i % 32 {
            0 => "\n",
            _ => "",
        };
        let _ = write!(c, "{sep}{b},");
    }
    c.push_str("\n};\nint main(void){return (int)texrs_aot_main(page, sizeof page);}\n");
    c
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
