//! Where LuaTeX's input callbacks reach the engine: `find_read_file`,
//! `open_read_file` and `process_input_buffer` (the LuaTeX manual, "Callbacks",
//! "File discovery" and "Data processing").
//!
//! One slot per callback, for the thread a run is on, empty unless the Lua
//! side has registered something; `crate::lua` fills them. The engine asks
//! through the functions here at the points luatex calls the callback: the
//! name of every file `\input` or `\openin` opens, the reader that stands in
//! for the file, and every line the mouth reads from a file before it
//! tokenises the line.

use std::cell::RefCell;

/// `find_read_file(id, asked_name)`: the file to open, or `None` for none.
/// `id` is 0 for `\input` and `n+1` for `\openin n`.
pub type FindReadFile = Box<dyn FnMut(i64, &str) -> Option<String>>;
/// `open_read_file(file_name)`: a reader that hands back the file a line at
/// a time, `None` at its end -- or `None` to read the file as it is.
pub type OpenReadFile = Box<dyn FnMut(&str) -> Option<Box<dyn FnMut() -> Option<String>>>>;
/// `process_input_buffer(line)`: the line to read instead, or `None` to read
/// it as it is. The line comes without its end.
pub type ProcessInputBuffer = Box<dyn FnMut(&str) -> Option<String>>;

thread_local! {
    static FIND_READ_FILE: RefCell<Option<FindReadFile>> = const { RefCell::new(None) };
    static OPEN_READ_FILE: RefCell<Option<OpenReadFile>> = const { RefCell::new(None) };
    static PROCESS_INPUT_BUFFER: RefCell<Option<ProcessInputBuffer>> = const { RefCell::new(None) };
}

/// Install (or, with `None`, remove) the `find_read_file` callback.
pub fn set_find_read_file(f: Option<FindReadFile>) {
    FIND_READ_FILE.with(|s| *s.borrow_mut() = f);
}

/// Install (or remove) the `open_read_file` callback.
pub fn set_open_read_file(f: Option<OpenReadFile>) {
    OPEN_READ_FILE.with(|s| *s.borrow_mut() = f);
}

/// Install (or remove) the `process_input_buffer` callback.
pub fn set_process_input_buffer(f: Option<ProcessInputBuffer>) {
    PROCESS_INPUT_BUFFER.with(|s| *s.borrow_mut() = f);
}

/// The name a file asked for as `asked` by `id` is opened under: the
/// callback's answer if one is installed (`Some(None)` when it found none),
/// `None` when no callback is.
pub fn find_read_file(id: i64, asked: &str) -> Option<Option<String>> {
    FIND_READ_FILE.with(|s| s.borrow_mut().as_mut().map(|f| f(id, asked)))
}

/// The text of `file` as the `open_read_file` callback's reader gives it, its
/// lines ended with line feeds; `None` when no callback is installed or it
/// declined the file.
pub fn open_read_file(file: &str) -> Option<String> {
    let mut reader = OPEN_READ_FILE.with(|s| s.borrow_mut().as_mut().and_then(|f| f(file)))?;
    let mut text = String::new();
    while let Some(line) = reader() {
        text.push_str(&line);
        text.push('\n');
    }
    Some(text)
}

/// Whether a `process_input_buffer` callback is installed: the mouth asks
/// this once per line, so it is all a run without one pays.
pub fn has_process_input_buffer() -> bool {
    PROCESS_INPUT_BUFFER.with(|s| s.borrow().is_some())
}

/// The line `line` becomes under the `process_input_buffer` callback, or
/// `None` when it is read as it is.
pub fn process_input_buffer(line: &str) -> Option<String> {
    PROCESS_INPUT_BUFFER.with(|s| s.borrow_mut().as_mut().and_then(|f| f(line)))
}
