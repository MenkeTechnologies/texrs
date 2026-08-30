//! Interactive prompt for `texrs --repl`.
//!
//! Two halves, deliberately separable: [`Session`], which is what a line of TeX
//! means when the ones before it are still in effect, and the reedline loop
//! around it. The loop needs a terminal; the session does not, which is what
//! `tests/repl.rs` exercises.
//!
//! **How state persists.** A TeX document is not a sequence of independent
//! statements: `\catcode` changes how the next line READS, `\def` changes what
//! it MEANS, and a register assignment is run-time state living in a VM slot. A
//! session therefore keeps the source it has been given and re-lowers and
//! re-runs the whole of it each turn, printing only the messages the newest line
//! produced. That is slower than incremental evaluation and exactly right: the
//! second run of a line sees precisely the state the first left, because it IS
//! the same program with one more line.
//!
//! History lives in `~/.texrs/history`, and Tab completes the primitives from
//! `src/corpus.rs` — the same table the language server and the reference page
//! answer from.

use std::borrow::Cow;

use nu_ansi_term::{Color, Style};
use reedline::{
    default_emacs_keybindings, ColumnarMenu, Completer, Emacs, FileBackedHistory, KeyCode,
    KeyModifiers, MenuBuilder, Prompt, PromptEditMode, PromptHistorySearch,
    PromptHistorySearchStatus, Reedline, ReedlineEvent, ReedlineMenu, Signal, Span, Suggestion,
};

use crate::corpus::CORPUS;

/// A running interactive session: the lines given so far, and what they printed.
#[derive(Default)]
pub struct Session {
    /// Every line accepted so far, in order.
    lines: Vec<String>,
    /// How many messages the previous turn's program had produced, so a turn
    /// reports only what its own line added.
    seen: usize,
}

/// What one line did.
#[derive(Debug, PartialEq, Eq)]
pub enum Turn {
    /// The line compiled and ran; these messages are new.
    Output(Vec<String>),
    /// The line did not compile or the run stopped. The session is unchanged —
    /// a line that failed does not become part of the document, or every
    /// following turn would fail with it.
    Error(String),
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// The source of the document as it stands, for `\show`-style inspection and
    /// for the tests.
    pub fn source(&self) -> String {
        self.lines.join("\n")
    }

    /// Feed one line, returning what it printed.
    pub fn eval(&mut self, line: &str) -> Turn {
        self.lines.push(line.to_string());
        // `\end` stops lowering, so a document that has already been ended
        // would ignore everything after it. The session drops it and keeps
        // going: at a prompt, `\end` means "I am done with this line", not
        // "ignore the rest of my session".
        let src = self
            .lines
            .iter()
            .map(|l| l.trim_end())
            .filter(|l| *l != "\\end")
            .collect::<Vec<_>>()
            .join("\n");
        match crate::run_messages_list(&src) {
            Ok(msgs) => {
                let new = msgs[self.seen.min(msgs.len())..].to_vec();
                self.seen = msgs.len();
                Turn::Output(new)
            }
            Err(e) => {
                // Roll the line back so the session stays runnable.
                self.lines.pop();
                Turn::Error(e.0)
            }
        }
    }
}

/// Run the prompt until EOF (Ctrl-D).
///
/// With stdin on a terminal this is the reedline editor; without one it is a
/// plain read-eval loop, because the line editor needs a terminal to open and
/// would fail with `Device not configured` on a pipe. `texrs --repl < doc.tex`
/// is a reasonable thing to type, so it works.
pub fn run() -> Result<(), String> {
    // SAFETY: isatty takes a file descriptor and never fails destructively.
    if unsafe { libc::isatty(0) } != 1 {
        return run_piped();
    }
    // The logo and the live-stats box, the way every sibling engine's prompt
    // opens. Colour only when a terminal is there to read it.
    crate::banner::print_banner(crate::banner::colored_stdout());
    let mut session = Session::new();

    let history = dirs::home_dir()
        .map(|h| h.join(".texrs").join("history"))
        .and_then(|p| {
            let _ = std::fs::create_dir_all(p.parent()?);
            FileBackedHistory::with_file(2000, p).ok()
        });

    let completer = Box::new(PrimitiveCompleter);
    let menu = Box::new(ColumnarMenu::default().with_name("completion_menu"));
    let mut keybindings = default_emacs_keybindings();
    keybindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Tab,
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::Menu("completion_menu".to_string()),
            ReedlineEvent::MenuNext,
        ]),
    );

    let mut editor = Reedline::create()
        .with_completer(completer)
        .with_menu(ReedlineMenu::EngineCompleter(menu))
        .with_edit_mode(Box::new(Emacs::new(keybindings)));
    if let Some(h) = history {
        editor = editor.with_history(Box::new(h));
    }

    let prompt = TexPrompt;
    loop {
        match editor.read_line(&prompt) {
            Ok(Signal::Success(line)) => {
                if line.trim().is_empty() {
                    continue;
                }
                match session.eval(&line) {
                    Turn::Output(msgs) => {
                        for m in msgs {
                            println!("{m}");
                        }
                    }
                    // The same shape a run writes: `! <reason>.` on stderr.
                    Turn::Error(e) => eprintln!("! {e}."),
                }
            }
            // Ctrl-C abandons the line, not the session.
            Ok(Signal::CtrlC) => continue,
            Ok(Signal::CtrlD) => break,
            // reedline may grow signals this loop does not know; treating an
            // unknown one as "carry on" is the only safe reading.
            Ok(_) => continue,
            Err(e) => return Err(format!("repl: {e}")),
        }
    }
    Ok(())
}

/// The same loop without the line editor, for stdin on a pipe.
///
/// No banner: piped output is being read by something, and a greeting in front
/// of it is noise.
fn run_piped() -> Result<(), String> {
    use std::io::BufRead;
    let mut session = Session::new();
    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|e| format!("repl: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }
        match session.eval(&line) {
            Turn::Output(msgs) => {
                for m in msgs {
                    println!("{m}");
                }
            }
            Turn::Error(e) => eprintln!("! {e}."),
        }
    }
    Ok(())
}

/// Tab completion over the primitives, matching on the word being typed.
struct PrimitiveCompleter;

impl Completer for PrimitiveCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        let head = &line[..pos.min(line.len())];
        // A control sequence starts at its backslash; anything else completes
        // from the last whitespace.
        let start = head
            .rfind('\\')
            .unwrap_or_else(|| head.rfind(char::is_whitespace).map(|i| i + 1).unwrap_or(0));
        let word = &head[start..];
        CORPUS
            .iter()
            .filter(|(name, ..)| name.starts_with(word))
            .map(|(name, chapter, doc, _example)| Suggestion {
                value: (*name).to_string(),
                description: Some(format!("{chapter} — {doc}")),
                style: None,
                extra: None,
                span: Span::new(start, pos),
                append_whitespace: false,
                display_override: None,
                match_indices: None,
            })
            .collect()
    }
}

struct TexPrompt;

impl Prompt for TexPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Owned(
            Style::new()
                .fg(Color::Cyan)
                .bold()
                .paint("tex❯ ")
                .to_string(),
        )
    }
    fn render_prompt_right(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }
    fn render_prompt_indicator(&self, _mode: PromptEditMode) -> Cow<'_, str> {
        Cow::Borrowed("")
    }
    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed("... ")
    }
    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> Cow<'_, str> {
        let prefix = match history_search.status {
            PromptHistorySearchStatus::Passing => "",
            PromptHistorySearchStatus::Failing => "failing ",
        };
        Cow::Owned(format!(
            "({prefix}reverse-search: {}) ",
            history_search.term
        ))
    }
}
