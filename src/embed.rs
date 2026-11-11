//! The embedding entry point: run a whole document with a given terminal input
//! and get back its text.

/// The stack each run gets, the same the LaTeX label pass runs on
/// (`latex::aux`): lowering a large document recurses deeply.
const STACK: usize = 64 << 20;

/// Run `src` as a complete program with `input` as its stdin and return
/// everything it wrote to stdout.
///
/// "Stdout" is the document's text, as [`crate::run_text`] returns it and
/// `texrs --text` prints it; the `\message` stream and error reports are not
/// part of it. Each call runs on its own thread with fresh thread-local state,
/// so nothing leaks between calls and concurrent calls never mix output.
///
/// - `input` is the terminal (see [`crate::expand::with_terminal`]): each of
///   its lines answers one terminal read, which is what `\read16`, `\read-1`
///   or a `\read` from any closed stream does (tex.web §484). The run starts in
///   scroll mode so those reads are allowed. Running out of lines is tex's
///   `*** (job aborted, no legal \end found)`.
/// - `Err` for anything [`crate::run_text`] refuses (an undefined control
///   sequence, a fatal error, a failed Lua chunk) and for a panic in the
///   engine.
pub fn run_program(src: &str, input: &str) -> Result<String, String> {
    let (src, input) = (src.to_owned(), input.to_owned());
    let run = std::thread::Builder::new()
        .name("texrs-embed".into())
        .stack_size(STACK)
        .spawn(move || {
            crate::expand::with_terminal(&input, || crate::run_text(&src)).map_err(|e| e.0)
        })
        .map_err(|e| format!("cannot start the engine thread: {e}"))?;
    run.join().unwrap_or_else(|panic| {
        let msg = panic
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".into());
        Err(format!("engine panicked: {msg}"))
    })
}
