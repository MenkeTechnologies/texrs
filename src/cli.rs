//! The command line, in `tex`'s own grammar.
//!
//! Every sibling frontend parses the reference tool's option grammar rather than
//! inventing one — `rubylang` mirrors MRI's `proc_options`, `phplang` mirrors
//! `php`'s — because a drop-in replacement that cannot be invoked the way the
//! thing it replaces is invoked is not a drop-in replacement. This is `tex`'s,
//! from `tex --help` and web2c's option table, plus texrs's own long options.
//!
//! The three shapes `tex(1)` accepts:
//!
//! ```text
//! tex [OPTION]... [TEXNAME[.tex]] [COMMANDS]   run a file, then more input
//! tex [OPTION]... \FIRST-LINE                  the arguments ARE the input
//! tex [OPTION]... &FMT ARGS                    with a named format
//! ```
//!
//! and the details that decide what a build script sees:
//!
//! * `TEXNAME` gets `.tex` appended when it has no extension, so `texrs doc`
//!   and `texrs doc.tex` are the same run.
//! * A first non-option argument starting with `\` makes the WHOLE argument
//!   list a line of TeX input, with no file at all — which is how a Makefile
//!   passes a one-liner.
//! * Arguments after the file are more TeX input, read after it.
//! * `-interaction=batchmode` writes nothing to the terminal.
//!
//! Options may be spelled with one dash or two (`-jobname=x`, `--jobname=x`),
//! as web2c's getopt accepts both, and a value may follow either an `=` or a
//! space.
//!
//! **Two deliberate divergences**, both from texrs taking several files where
//! `tex` takes one:
//!
//! 1. A non-option argument is a FILE unless it begins with `\`, in which case
//!    it is input — rather than tex's "the first is a file, everything after it
//!    is input". A file name beginning with a backslash is not a thing that
//!    exists; a second file to compile is.
//! 2. Options are recognised ANYWHERE, not only before the first file. tex
//!    stops parsing options at the first non-option argument, so `tex doc
//!    -halt-on-error` reads `-halt-on-error` as input to typeset. That is
//!    faithful and useless; `texrs doc -halt-on-error` sets the flag. A file
//!    genuinely named `-doc.tex` is still reachable as `./-doc.tex`.

use std::path::PathBuf;

/// What `-interaction=` selects.
///
/// texrs never stops to ask — there is no terminal interaction to have — so the
/// three non-batch modes differ only in what they would do at an error, and
/// batchmode is the one that changes observable behaviour: it silences the
/// terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Interaction {
    Batch,
    NonStop,
    Scroll,
    #[default]
    ErrorStop,
}

impl Interaction {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "batchmode" => Some(Interaction::Batch),
            "nonstopmode" => Some(Interaction::NonStop),
            "scrollmode" => Some(Interaction::Scroll),
            "errorstopmode" => Some(Interaction::ErrorStop),
            _ => None,
        }
    }

    /// Whether the terminal gets the `\message` stream at all.
    pub fn prints(self) -> bool {
        self != Interaction::Batch
    }
}

/// The one-shot modes: everything that runs instead of compiling a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Run,
    Help,
    Version,
    Repl,
    Lsp,
    Dap,
    CacheStats,
    CacheClear,
}

/// A parsed command line.
#[derive(Debug, Clone)]
pub struct Cli {
    pub mode: Mode,
    /// Files to compile, in the order given, `.tex` appended where it was left
    /// off.
    pub files: Vec<String>,
    /// TeX input from the command line itself: the `\FIRST-LINE` form, or the
    /// arguments that followed a file.
    pub commands: Vec<String>,
    /// `&FMT` or `-fmt=NAME`.
    pub format: Option<String>,

    // ---- tex's options ----------------------------------------------------
    pub interaction: Interaction,
    pub jobname: Option<String>,
    pub output_directory: Option<PathBuf>,
    pub progname: Option<String>,
    pub ini: bool,
    pub halt_on_error: bool,
    pub file_line_error: bool,
    pub recorder: bool,
    pub eight_bit: bool,

    // ---- texrs's own ------------------------------------------------------
    pub dump_tokens: bool,
    pub disasm: bool,
    pub tiers: bool,
    pub aot: bool,
    pub build: bool,
    pub no_cache: bool,
    pub jobs: Option<usize>,
}

impl Default for Cli {
    fn default() -> Self {
        Self {
            mode: Mode::Run,
            files: Vec::new(),
            commands: Vec::new(),
            format: None,
            interaction: Interaction::default(),
            jobname: None,
            output_directory: None,
            progname: None,
            ini: false,
            halt_on_error: false,
            file_line_error: false,
            recorder: false,
            eight_bit: false,
            dump_tokens: false,
            disasm: false,
            tiers: false,
            aot: false,
            build: false,
            no_cache: false,
            jobs: None,
        }
    }
}

impl Cli {
    /// The TeX source this invocation asks for, when it came from the command
    /// line rather than a file.
    ///
    /// tex joins the arguments with spaces, which is what makes
    /// `tex '\catcode`\{=1' '\message{hi}\end'` one line rather than three.
    pub fn command_line_source(&self) -> Option<String> {
        match self.commands.is_empty() {
            true => None,
            false => Some(self.commands.join(" ")),
        }
    }
}

/// Parse an argument list (without the program name).
pub fn parse(args: &[String]) -> Result<Cli, String> {
    let mut cli = Cli::default();
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];
        i += 1;

        if !is_option(arg) {
            classify_input(&mut cli, arg);
            continue;
        }

        // `-x` and `--x` are the same option, and a value may follow an `=` or
        // a space.
        let body = arg.trim_start_matches('-');
        let (name, inline) = match body.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (body, None),
        };
        let mut value = |cli_name: &str| -> Result<String, String> {
            match inline.clone() {
                Some(v) => Ok(v),
                None => match args.get(i) {
                    Some(v) => {
                        i += 1;
                        Ok(v.clone())
                    }
                    None => Err(format!("texrs: {cli_name} needs a value")),
                },
            }
        };

        match name {
            // ---- one-shot modes ------------------------------------------
            "h" | "help" => cli.mode = Mode::Help,
            "version" => cli.mode = Mode::Version,
            "repl" => cli.mode = Mode::Repl,
            "lsp" => cli.mode = Mode::Lsp,
            "dap" => cli.mode = Mode::Dap,
            "cache-stats" => cli.mode = Mode::CacheStats,
            "cache-clear" => cli.mode = Mode::CacheClear,

            // ---- tex's options -------------------------------------------
            "interaction" => {
                let v = value("-interaction")?;
                cli.interaction = Interaction::parse(&v).ok_or_else(|| {
                    format!(
                        "texrs: unknown interaction mode `{v}'; use batchmode, \
                         nonstopmode, scrollmode or errorstopmode"
                    )
                })?;
            }
            "jobname" => cli.jobname = Some(value("-jobname")?),
            "output-directory" => {
                cli.output_directory = Some(PathBuf::from(value("-output-directory")?))
            }
            "progname" => cli.progname = Some(value("-progname")?),
            "fmt" => cli.format = Some(value("-fmt")?),
            "ini" => cli.ini = true,
            "halt-on-error" => cli.halt_on_error = true,
            "file-line-error" => cli.file_line_error = true,
            "no-file-line-error" => cli.file_line_error = false,
            "recorder" => cli.recorder = true,
            "8bit" => cli.eight_bit = true,

            // ---- texrs's own ---------------------------------------------
            "dump-tokens" => cli.dump_tokens = true,
            "disasm" => cli.disasm = true,
            "tiers" => cli.tiers = true,
            "aot" => cli.aot = true,
            "build" => cli.build = true,
            "no-cache" => cli.no_cache = true,
            "jobs" => {
                let v = value("--jobs")?;
                match v.parse::<usize>() {
                    Ok(n) if n >= 1 => cli.jobs = Some(n),
                    _ => return Err("texrs: --jobs needs a positive count".into()),
                }
            }

            other => return Err(format!("texrs: unknown option: -{other}")),
        }
    }
    Ok(cli)
}

/// Whether `arg` is an option rather than input.
///
/// A lone `-` is not: tex reads stdin for it. Neither is anything starting with
/// `\` or `&`, which are the two input forms.
fn is_option(arg: &str) -> bool {
    arg.len() > 1 && arg.starts_with('-') && !arg.starts_with("-\\")
}

/// File, command line, or format name.
fn classify_input(cli: &mut Cli, arg: &str) {
    if let Some(fmt) = arg.strip_prefix('&') {
        cli.format = Some(fmt.to_string());
        return;
    }
    if arg.starts_with('\\') || !cli.commands.is_empty() {
        // Once input has started on the command line it continues: `tex file
        // '\message{a}' '\end'` is one line of input after the file.
        cli.commands.push(arg.to_string());
        return;
    }
    cli.files.push(with_tex_extension(arg));
}

/// `doc` becomes `doc.tex`; `doc.tex` and `doc.cls` are left alone.
///
/// tex appends the extension only when the name has none, which is why
/// `tex doc.sty` reads `doc.sty` rather than `doc.sty.tex`.
pub fn with_tex_extension(name: &str) -> String {
    let stem = std::path::Path::new(name);
    match stem.extension().is_some() {
        true => name.to_string(),
        false => format!("{name}.tex"),
    }
}

/// The option grammar, in the fleet's house style: `── SECTION ───` dividers
/// and `//` descriptions, printed under the logo the way `tp -h` prints its own.
pub const USAGE: &str = "\
\n  USAGE: texrs [OPTIONS] [FILE[.tex]]... [COMMANDS]
         texrs [OPTIONS] \\FIRST-LINE      // the arguments are the input
         texrs [OPTIONS] &FMT ARGS        // with a named format
         texrs                            // no arguments: the prompt

  ── TEX OPTIONS ────────────────────────────────────────
  -interaction=MODE
          // batchmode, nonstopmode, scrollmode or errorstopmode
  -jobname=NAME
          // Set the job name
  -output-directory=DIR
          // Write files in DIR
  -progname=NAME
          // Set the program name
  -fmt=NAME
          // Use a named format
  -ini
          // Be initex
  -halt-on-error
          // Stop at the first error
  -file-line-error, -no-file-line-error
          // file:line:error style messages
  -recorder
          // Record the files read
  -8bit
          // Write 8-bit characters as themselves

  ── RUNNING ────────────────────────────────────────────
  --repl
          // Start the interactive prompt
  --jobs=N
          // Compile N documents at once (default: one per core)
  --build
          // Compile into the bytecode cache and stop, without running
  --no-cache
          // Compile this run rather than reading the bytecode cache
  --aot
          // Compile the document to a standalone native executable

  ── LOOKING INSIDE ─────────────────────────────────────
  --dump-tokens
          // Print the mouth's token stream and exit
  --disasm
          // Print the lowered fusevm bytecode and exit
  --tiers
          // Run it, then report which fusevm tier took its bytecode

  ── EDITORS ────────────────────────────────────────────
  --lsp
          // Speak the Language Server Protocol over stdio
  --dap
          // Speak the Debug Adapter Protocol over stdio

  ── CACHE ──────────────────────────────────────────────
  --cache-stats
          // Say what the bytecode cache holds and where
  --cache-clear
          // Delete it

  ── DOCUMENTS ──────────────────────────────────────────
  -X new [DIR]            // Make one (Texrs.toml + index.tex)
  -X init                 // Make one here, named after this directory
  -X build [--profile P]  // Build the document this directory is in
  -X watch [--profile P]  // Rebuild it whenever an input changes
  -X show                 // Say what the document is and can produce
  -X dump [--profile P]   // Build to stdout, writing nothing
  -X bundle fetch URL     // Download a bundle into the cache
  -X bundle list          // Say which bundles have been fetched
  -X dvi FILE.dvi         // Read what real tex shipped for a document
  -X dvi A.dvi B.dvi      // Say whether two files are the same document
  -X bib FILE.bib         // Read a bibliography database
  -X bib FILE.aux         // Say what a document cites, and what is missing
  -X bst FILE.bst         // Read a bibliography style, and check its names
  -X bibtex FILE.aux      // Run the style: write the .bbl a document reads
  -X tfm FILE.tfm [C]     // Read a font's metrics, or one character's
  --profile NAME          // Which output to build
  --interval MS           // How often -X watch looks (default 250)

  ── SYSTEM ─────────────────────────────────────────────
  -h, --help              // Print this
  --version               // Print the version banner
";

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(args: &[&str]) -> Cli {
        parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()).expect("parses")
    }

    #[test]
    fn a_bare_name_gets_the_tex_extension() {
        assert_eq!(cli(&["doc"]).files, vec!["doc.tex".to_string()]);
        assert_eq!(cli(&["doc.tex"]).files, vec!["doc.tex".to_string()]);
        // A name that already has an extension keeps it, whatever it is.
        assert_eq!(cli(&["doc.sty"]).files, vec!["doc.sty".to_string()]);
    }

    #[test]
    fn a_backslash_argument_makes_the_whole_line_input() {
        let c = cli(&["\\message{hi}", "\\end"]);
        assert!(c.files.is_empty(), "a line of input is not a file");
        assert_eq!(c.command_line_source().unwrap(), "\\message{hi} \\end");
    }

    #[test]
    fn arguments_after_a_file_are_input_read_after_it() {
        let c = cli(&["doc", "\\message{after}", "\\end"]);
        assert_eq!(c.files, vec!["doc.tex".to_string()]);
        assert_eq!(c.command_line_source().unwrap(), "\\message{after} \\end");
    }

    #[test]
    fn several_files_are_several_documents() {
        // tex takes one file; texrs compiles a batch, which is the one place
        // the grammar deliberately differs.
        let c = cli(&["a", "b.tex", "c"]);
        assert_eq!(c.files, vec!["a.tex", "b.tex", "c.tex"]);
    }

    #[test]
    fn options_take_one_dash_or_two_and_a_value_either_way() {
        assert_eq!(cli(&["-jobname=x", "doc"]).jobname.unwrap(), "x");
        assert_eq!(cli(&["--jobname=x", "doc"]).jobname.unwrap(), "x");
        assert_eq!(cli(&["-jobname", "x", "doc"]).jobname.unwrap(), "x");
    }

    #[test]
    fn interaction_modes_are_texs_four() {
        assert_eq!(
            cli(&["-interaction=batchmode"]).interaction,
            Interaction::Batch
        );
        assert!(!cli(&["-interaction=batchmode"]).interaction.prints());
        assert!(cli(&["-interaction=nonstopmode"]).interaction.prints());
        let err = parse(&["-interaction=loud".to_string()]).unwrap_err();
        assert!(err.contains("batchmode"), "unhelpful error: {err}");
    }

    #[test]
    fn an_option_after_the_file_is_still_an_option() {
        // tex stops parsing options at the first non-option argument, so it
        // would typeset `-halt-on-error` as text. Faithful and useless: here it
        // sets the flag, which is what someone typing it meant.
        let c = cli(&["doc", "-halt-on-error"]);
        assert_eq!(c.files, vec!["doc.tex".to_string()]);
        assert!(c.halt_on_error);
        assert!(c.commands.is_empty());
    }

    #[test]
    fn a_format_argument_is_taken_and_is_not_a_file() {
        let c = cli(&["&plain", "doc"]);
        assert_eq!(c.format.as_deref(), Some("plain"));
        assert_eq!(c.files, vec!["doc.tex".to_string()]);
        assert_eq!(cli(&["-fmt=plain", "doc"]).format.as_deref(), Some("plain"));
    }

    #[test]
    fn texs_flags_are_accepted_rather_than_refused() {
        // A build script passes these to every engine it drives; refusing them
        // would make texrs undroppable into one, even though the milestone acts
        // on almost none of them.
        let c = cli(&[
            "-ini",
            "-halt-on-error",
            "-file-line-error",
            "-recorder",
            "-8bit",
            "-output-directory=out",
            "-progname=texrs",
            "doc",
        ]);
        assert!(c.ini && c.halt_on_error && c.file_line_error && c.recorder && c.eight_bit);
        assert_eq!(c.output_directory.unwrap().to_str().unwrap(), "out");
        assert_eq!(c.progname.as_deref(), Some("texrs"));
        assert_eq!(c.files, vec!["doc.tex".to_string()]);
    }

    #[test]
    fn an_unknown_option_is_still_refused() {
        let err = parse(&["--nope".to_string()]).unwrap_err();
        assert!(err.contains("unknown option"), "{err}");
    }

    #[test]
    fn no_file_line_error_turns_the_flag_back_off() {
        assert!(!cli(&["-file-line-error", "-no-file-line-error", "doc"]).file_line_error);
    }
}
