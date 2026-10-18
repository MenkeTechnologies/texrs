//! Turn a TeX source into the command stream `crate::compiler` lowers.
//!
//! This is the frontend half: the mouth tokenises, macros expand, `\catcode` and
//! `\def` take effect HERE at compile time — they change how the rest of the
//! file reads, so they cannot be deferred to run time — and everything whose
//! value is only known when the program runs becomes a `Cmd`.
//!
//! A conditional is the interesting case. A tree-walker decides the branch as it
//! goes; a compiler cannot, because the registers it tests are runtime state. So
//! both arms are COLLECTED as token runs, lowered recursively, and emitted as a
//! real branch. That is what makes `\ifnum` compile to `NumLt` + `JumpIfFalse`
//! instead of a Rust `if`.

use crate::catcode::Cat;
use crate::expand::{Engine, GroupKind, Macro, Meaning, TexError};
use crate::ir::{Arith, Cmd, MsgOp, Num, Rel};
use crate::lexer::Lexer;
use crate::token::{CsId, Token};

type R<T> = Result<T, TexError>;

mod math;
mod stomach;

pub struct Lowerer {
    pub eng: Engine,
    /// The stomach's dispatch state: the mode nest, and what the program has
    /// been told (see `stomach`).
    st: stomach::State,
    /// Whether an `\end` in the source stopped the run.
    ///
    /// tex closes the file's paren differently depending on this: `\end` inside
    /// the file prints `(./doc.tex MSGS )` and stops reading, while a file that
    /// merely runs out prints `(./doc.tex MSGS)` and keeps reading — from the
    /// command line, if there is more there. `src/main.rs` needs to know which
    /// happened; nothing else does.
    pub ended: bool,
    /// Next hidden count register. `\edef` freezing `\the\count0` needs somewhere
    /// to put the value NOW, and a register is the only run-time store there is;
    /// TeX reserves the high registers for exactly this kind of scratch use.
    next_scratch: i64,
    /// Every register a `\global` assignment has written, in the order they
    /// were lowered.
    ///
    /// A group must not simply restore one. `tex.web` §283's `geq_word_define`
    /// sets `xeq_level` to `level_one`, and §282's `unsave` DESTROYS a saved
    /// value whose level no longer matches -- so a register assigned globally
    /// inside a group keeps what the LAST global assignment left, even when a
    /// local one came after it (that one saved the global value, and the `}`
    /// restores it). The enclosing group finds out which registers those are by
    /// the slice of this list its body added, and keeps them through a scratch
    /// slot rather than the save on its stack: see `Cmd::Group`'s `keeps`.
    globals: Vec<i64>,
    /// Registers the command just lowered assigned `\global`ly, waiting for a
    /// `Cmd::KeepGlobal` after the last of the commands it lowered to.
    pending_keeps: Vec<i64>,
    /// The next scratch slot a group can keep a globally assigned register
    /// in, past every slot a register occupies.
    next_keep_slot: u16,
    /// The register writes a preloaded preamble made, waiting to be put in
    /// front of the document's own commands. See [`Lowerer::preload`].
    prologue: Vec<Cmd>,
    /// The conditional whose arms [`Self::arms`] reads next: its name, its
    /// `\unless`, and the line it began on.
    next_arms: Option<(&'static str, bool, u32)>,
    /// Whether `lower_located` is reading the document itself (see `lower`).
    reading_document: bool,
    /// Errors the expander REPORTED rather than raised, waiting to be written
    /// into the message stream in front of whatever is printed next.
    ///
    /// tex prints an error on the terminal where it happens, and `print_ln`
    /// ends that display with no character of its own — so the `\message` that
    /// follows begins at column zero and gets none of the separating space
    /// §1279 would otherwise put in front of it. Prepending the report to that
    /// message's pieces is that arrangement: the report and the message are one
    /// terminal line, not two.
    ///
    /// Message pieces rather than strings, because `\errmessage` expands its
    /// text as `\message` does, and a register in it is read at run time.
    reports: Vec<MsgOp>,
    /// Whether anything was reported at all, for the line tex writes at the end
    /// of a run that had errors.
    reported: bool,
    /// §1335's `(\end occurred ...)` lines, each already parenthesised: the
    /// run-end notes for what `\end` found still open. Printed by the run's
    /// closing notice, after the file's own `)`.
    end_notes: Vec<String>,
    /// The typefaces the document asked for, by `\setmainfont` and its
    /// siblings.
    ///
    /// A document says `\setmainfont{Arimo}` and means it; setting the whole
    /// book in Computer Modern regardless is the complaint this exists to
    /// answer. Recorded while lowering because that is where the preamble is
    /// read, and handed to the typesetter, which decides what it can honour.
    pub fonts: crate::typeset::Families,
    /// The palette the document has defined, and the colour of the page.
    ///
    /// A document defines its colours once and refers to them by name from
    /// then on -- `\definecolor{neonCyan}{HTML}{05D9E8}` in the preamble and
    /// `\color{neonCyan}` wherever it wants it. Without the palette the name
    /// means nothing and every page comes out black.
    pub colours: crate::colour::Colours,
    /// What `\pagecolor` asked the page to be painted.
    ///
    /// Load-bearing rather than decorative: a document that sets a dark page
    /// also sets light text, and honouring the text without the page leaves
    /// white-on-white.
    pub page_colour: Option<crate::colour::Rgb>,
    /// The page the document asked for: the type size in its class options,
    /// the leading that goes with that size, and geometry's margins.
    ///
    /// Read while lowering for the same reason the families are -- that is
    /// where the preamble is -- and handed to the typesetter, which sets the
    /// page from it. Without this every document was set on plain.tex's page
    /// however loudly it said otherwise, which fitted about a third more text
    /// onto a page than the lualatex-built PDFs in the corpus carry.
    pub layout: crate::typeset::Layout,
    /// Carry the document's own text into the program.
    ///
    /// Off by default: the terminal output of a `tex` run is its `\message`
    /// stream, and the differential suite compares against exactly that. `--text`
    /// turns it on for a caller who wants what the document SAYS rather than
    /// what it announced.
    text_output: bool,
    /// Read formulas and set none: the label pass. See [`Self::passing_over_formulas`].
    formulas_passed_over: bool,
    /// How many `\input` files are open above this one, so a file that inputs
    /// itself stops with a diagnostic instead of exhausting the host stack.
    input_depth: usize,
    /// How deep a contents lists, which is LaTeX's `tocdepth`.
    ///
    /// 2 is the class default -- chapter, section, subsection. Every book in
    /// the corpus sets 0 immediately above its `\tableofcontents`, which is a
    /// contents of chapters, and setting one two levels deeper would list some
    /// four hundred headings where lualatex lists forty.
    pub toc_depth: usize,
    /// How deep `block` is currently nested.
    ///
    /// Lowering inlines a macro into the stream and lowers through its body, and
    /// it lowers BOTH arms of a run-time conditional because neither is decided
    /// yet. A macro whose body names itself therefore inlines into its own arm
    /// without end -- `\def\r{\ifnum\count0<3 \r \fi}` never terminates while
    /// lowering, whichever way the test would go at run time, and the Rust stack
    /// runs out before anything is emitted. Real TeX never meets this because it
    /// interprets: `pass_text` skips the arm it did not take without expanding
    /// it. Until a recursive macro lowers to a run-time call rather than an
    /// inline copy, this bounds the nesting so the failure is TeX's own
    /// "capacity exceeded" rather than a segfault.
    depth: usize,
    /// The stack address at the outermost level [`Lowerer::enter_level`]
    /// counted, so the recursion's use of the Rust stack can be bounded in
    /// bytes as well as in levels. See [`LOWER_STACK_BUDGET`].
    stack_base: usize,
    /// The formulas and pictures read so far and not yet seen set or
    /// measured; see [`crate::math::Formulas`].
    formulas: crate::math::Formulas,
    /// Where the document's finished paragraphs go while the rest of it is
    /// still being lowered, when a page builder is breaking them as they come:
    /// see [`crate::typeset::Feed`].
    feed: Option<crate::typeset::Feed>,
    /// The list environments open, outermost first, each with the item number
    /// it has reached.
    ///
    /// A field for the same reason `table_depth` is one: a group is lowered by
    /// a recursive call, and it decides both what `\item` means -- a bullet, a
    /// number, a term, or the prelude's own definition where no list is open --
    /// and how far in the item sets.
    lists: Vec<List>,
    /// How many `tabular` or `longtable` environments are open.
    ///
    /// A field rather than a local of `lower_into`, because a group is lowered
    /// by a recursive call and pandoc wraps every longtable in one:
    /// `{\def\LTcaptype{none} \begin{longtable}...}`. It decides what `&` and
    /// `\\` mean -- a cell boundary and a row end inside a table, a space and
    /// a line break outside one.
    table_depth: usize,
    /// What `\titleformat` said each sectioning command is set with.
    ///
    /// titlesec is how every book in the corpus styles its headings, and the
    /// prelude consumed the format argument and discarded it -- so a document
    /// that asked for `{\sffamily\bfseries\Huge}` got the class default face
    /// and the class default size. The tokens are kept whole and lowered with
    /// the title, so every declaration in them reaches the page through the
    /// arms that already read one.
    title_formats: std::collections::HashMap<String, Vec<Token>>,
    /// The face markers open in the text run, innermost last, each recording
    /// whether it is the mono face.
    ///
    /// A face reaches the text as marker CHARACTERS rather than as a command
    /// -- the prelude expands `\texttt{#1}` to `^^11m#1^^12` -- so this is how
    /// the ligature program below knows it is looking at code. A stack because
    /// the markers nest: `\texttt{a\textbf{b}c}` opens two and closes two.
    faces: Vec<bool>,
    /// How many code listings are being lowered.
    ///
    /// A listing is deliberately NOT read verbatim -- `lower_listing` re-lexes
    /// every line so that `\NormalTok` and the colour it carries still expand
    /// -- so a program's characters arrive through `push_text_char` exactly as
    /// prose does, and only this tells the two apart.
    listing_depth: usize,
    /// The character `push_text_char` last appended, while it is still the
    /// last character of the run and a ligature can form on it.
    ///
    /// A ligature joins two characters the DOCUMENT wrote side by side, so
    /// this records that it wrote one. Without it the pair would be taken from
    /// whatever happens to end the run -- a verbatim body's last character
    /// after `\end{verbatim}`, or a marker. It is cleared at every group
    /// boundary, which is what makes `-{}-` two hyphens: the spelling a LaTeX
    /// document has always used to ask for them.
    lig: Option<char>,
    /// What `\immediate\write` has written into each file this lowering
    /// opened, so `\input` and `\openin` read such a file as it now stands
    /// (see `crate::write`).
    written: crate::write::Written,
    /// The `\openout`, `\write`, `\closeout` and `\special` whatsits read
    /// without `\immediate`, in order (§1341-§1354).
    ///
    /// §1373 performs each one when the page holding it is shipped, and
    /// `ship_whatsit` is that step; what is missing is the list a whatsit sits
    /// in. Until pages are built from node lists here, they wait in this list
    /// and no shipout reaches them, so a deferred `\write` writes nothing.
    pub deferred_whatsits: Vec<crate::write::Whatsit>,
}

/// A list environment that is open, and how many items it has had.
struct List {
    kind: ListKind,
    count: usize,
}

/// What a list puts in front of each of its items.
#[derive(Clone, Copy)]
enum ListKind {
    /// `itemize`: a bullet.
    Bullet,
    /// `enumerate`: the item's number.
    Number,
    /// `description`: the term the item names, in bold.
    Term,
}

/// The nesting `block` refuses to go past.
///
/// Sits far above any real document -- a hand-written file nests groups and
/// conditionals a few dozen deep at the very most -- and below what the stack
/// can take. The ceiling is MEASURED, not guessed, and measured on a SPAWNED
/// thread rather than main: a spawned stack is a fraction of main's, and both
/// the test harness and any future worker pool run there. Matches the spirit
/// of the expander's 200_000-step ceiling: bound the runaway, name it as TeX
/// names it.
///
/// The measurement is of the STACK, so it moves when a frame on the recursive
/// path grows, and it has: it was 128-lowers/192-aborts when the bound was set
/// to 100, and adding a primitive to the number scanner made every level
/// fatter. Re-measured in a debug build on a spawned thread, 98 levels lower
/// and 100 abort -- the old bound was sitting exactly on the cliff, and
/// `mutual_recursion_is_still_bounded_rather_than_aborting` stopped reporting
/// "capacity exceeded" and started dying with a stack overflow instead, which
/// is the crash the bound exists to prevent. 64 leaves the margin back.
///
/// If this abbreviates a document that legitimately nests deeper, the fix is a
/// bigger stack for the lowering thread, not a bigger number here.
///
/// The count alone kept drifting off the cliff: `block_inner` is one function
/// with a match arm per command, and an unoptimised build gives every local of
/// every arm its own stack slot, so its frame is ~32 KiB and grows with each
/// command added. 63 levels of it overflowed the 2 MiB of a spawned thread
/// before the count reached 64. [`LOWER_STACK_BUDGET`] bounds the bytes too,
/// which is the quantity that actually runs out.
const MAX_LOWER_DEPTH: usize = 64;

/// The Rust stack the lowering recursion may use below its outermost level.
///
/// Three quarters of the 2 MiB a spawned thread gets by default (the test
/// harness, rayon's workers), leaving the rest for whatever the deepest level
/// calls into. An optimised build reaches [`MAX_LOWER_DEPTH`] first; an
/// unoptimised one, with its fatter frames, reaches this first -- either way
/// the runaway ends in TeX's capacity error rather than an abort.
const LOWER_STACK_BUDGET: usize = 1536 * 1024;

/// How many tokens the document level reads before offering its commands to
/// the paragraph feed again when they have not grown in number.
///
/// The feed takes only what can no longer change, so an offer that is skipped
/// hands nothing on wrongly: the same text goes at the next one, which is all
/// the breaking job can tell apart. The commands are offered whenever one is
/// added -- every source line adds a line directive -- and otherwise every
/// this many tokens, for a run of text that goes on without a line end.
/// Offering after every token, as it was, put a walk back over the trailing
/// line directives in front of every character of prose.
const OFFER_EVERY: usize = 1024;

/// The address of a local in the caller's frame. The stack grows down on
/// every target texrs builds for, so the difference between two of these is
/// the stack used between the two calls.
#[inline(always)]
fn stack_address() -> usize {
    let marker = 0u8;
    std::hint::black_box(&marker) as *const u8 as usize
}

impl Lowerer {
    pub fn new() -> Self {
        Self {
            st: stomach::State::default(),
            input_depth: 0,
            eng: Engine::new(),
            ended: false,
            next_scratch: 255,
            globals: Vec::new(),
            pending_keeps: Vec::new(),
            next_keep_slot: crate::compiler::TOTAL_SLOTS,
            prologue: Vec::new(),
            reports: Vec::new(),
            next_arms: None,
            reading_document: false,
            reported: false,
            end_notes: Vec::new(),
            fonts: crate::typeset::Families::default(),
            colours: crate::colour::Colours::new(),
            page_colour: None,
            layout: crate::typeset::Layout::default(),
            text_output: false,
            formulas_passed_over: false,
            depth: 0,
            stack_base: 0,
            formulas: Default::default(),
            feed: None,
            lists: Vec::new(),
            table_depth: 0,
            title_formats: std::collections::HashMap::new(),
            faces: Vec::new(),
            listing_depth: 0,
            lig: None,
            toc_depth: 2,
            written: crate::write::Written::default(),
            deferred_whatsits: Vec::new(),
        }
    }

    /// One macro argument as the characters in it.
    ///
    /// `read_balanced_group` scans an argument the way TeX does -- a brace
    /// group, or one token standing in for one -- so a `\setcounter` read here
    /// consumes exactly what the prelude's two-argument stub consumed. Control
    /// sequences inside it are dropped: a counter is named in letters.
    fn group_chars(&mut self, lx: &mut Lexer) -> R<String> {
        let toks = self.eng.read_balanced_group(lx)?;
        Ok(toks
            .iter()
            .filter_map(|t| match t {
                crate::token::Token::Char(c, _) => Some(*c),
                crate::token::Token::Cs(_) => None,
            })
            .collect())
    }

    /// Where the hidden scratch registers have got to.
    ///
    /// A format (`crate::format`) captures the engine after a preamble and
    /// applies it to a later run, and the scratch counter has to travel with
    /// it: a body that started again from 255 would write over a value the
    /// preamble's `\edef` had already frozen there.
    pub fn scratch_mark(&self) -> i64 {
        self.next_scratch
    }

    /// Resume the scratch counter where a captured format left it.
    pub fn set_scratch_mark(&mut self, at: i64) {
        self.next_scratch = at;
    }

    /// Emit the document's own text as well as its messages.
    /// Report what tex reports and carry on, where the library API would stop:
    /// see [`Engine::recover`].
    pub fn tex_errors(mut self, on: bool) -> Self {
        self.eng.recover = on;
        self
    }

    pub fn with_text_output(mut self) -> Self {
        self.text_output = true;
        self
    }

    /// Build the page's own list too: paragraphs (§1091-§1096), the page
    /// builder (§994) and `\output`, as the node path of `--dvi` needs. Off
    /// otherwise, where the outer list's text goes to the string path.
    pub fn with_list_path(mut self) -> Self {
        self.st_enable_paragraphs();
        self
    }

    /// Read every formula with the math parser and set none of it, keeping only
    /// the labels and equation numbers it carries: what the `.aux` pass needs
    /// (`src/latex/aux.rs`), which has no text stream to set them in.
    pub fn passing_over_formulas(mut self) -> Self {
        self.formulas_passed_over = true;
        self
    }

    /// Hand the document's text, a stretch of finished paragraphs at a time, to
    /// `feed` while it is lowered, and the formula settings those stretches
    /// name as each batch of them comes back. Lowering is unchanged by it; the
    /// feed closes when [`Self::lower`] returns.
    pub fn feed_paragraphs(&mut self, feed: crate::typeset::Feed) {
        self.formulas.tap(feed.tap());
        self.feed = Some(feed);
    }

    /// Compile a whole source to a command stream.
    pub fn lower(&mut self, src: &str) -> R<Vec<Cmd>> {
        // The document is a file, whose lines LuaTeX's `process_input_buffer`
        // sees; a preloaded preamble is not.
        self.reading_document = true;
        let lowered = self.lower_located(src);
        self.reading_document = false;
        // Whatever the feed has not handed on by now, the page builder breaks.
        self.feed = None;
        let mut cmds = lowered.map_err(|(e, _line)| e)?;
        // The last paragraph ends with the document rather than with a blank
        // line, and is broken with the parameters in force there.
        if let Some(mark) = self.paragraph_end_mark() {
            self.push_text(&mut cmds, &mark);
        }
        // The page builder reads its penalties as the document left them.
        self.layout.paging = crate::typeset::PageParams::from_intpars(&self.eng.intpars);
        // What a preloaded preamble WROTE to the registers runs first: see
        // `preload`.
        if !self.prologue.is_empty() {
            let mut out = std::mem::take(&mut self.prologue);
            out.append(&mut cmds);
            return Ok(out);
        }
        Ok(cmds)
    }

    /// Run the LaTeX prelude through this lowerer, keeping its definitions and
    /// the register values it established, and discarding its output.
    ///
    /// The prelude is nearly all definitions, and a definition is FRONTEND
    /// state -- it lands on `self.eng` and the document is lowered against it,
    /// which is why this returned nothing for as long as that was true. A
    /// register value is not: it lives in a VM slot, so `\p@=1pt` lowers to a
    /// `Cmd::SetCount` and dropping that command left `\p@` at INITEX's zero
    /// for the whole run. `kernel.tex` writes plain TeX's named constants that
    /// way -- `\p@=1pt`, `\z@=0pt`, `\z@skip=0pt plus0pt minus0pt` -- so
    /// size10.clo's `\abovedisplayskip 10\p@` was ten times nothing, silently,
    /// and `\@settopoint` divided by zero.
    ///
    /// So the SLOT WRITES are kept and everything the preamble printed is not.
    /// `lower` puts them in front of the document's own commands, which is
    /// where the preamble ran.
    pub fn preload(&mut self, src: &str) -> R<()> {
        // A LaTeX document: its prelude defines `\hbox` and its siblings as
        // text macros for the PDF path, so the list commands are not lowered
        // as lists.
        self.st.enabled = false;
        // What this path replaces is `lualatex`, so its registers are
        // LuaTeX's 65536 of each kind: latex.ltx itself `\countdef`s
        // register 258.
        self.eng.register_limit = crate::expand::LUATEX_REGISTER_LIMIT;
        let out = self.lower_located(src);
        // The prelude's own diagnostics belong to the prelude, and the document
        // is what tex is reporting on: an error the prelude recovered from must
        // not put `(see the transcript file …)` on the end of a clean run.
        self.reports.clear();
        self.reported = false;
        match out {
            Ok(cmds) => {
                state_only(&cmds, &mut self.prologue);
                Ok(())
            }
            Err((e, _)) => Err(e),
        }
    }

    /// The same, reporting the line the mouth had reached when it stopped.
    ///
    /// A `TexError` carries a reason and no position, which is all a terminal
    /// message needs (`! Undefined control sequence.`) and not enough for an
    /// editor: a diagnostic has to land on a line. The lexer knows where it is,
    /// so the position is taken from it at the point the error escapes rather
    /// than threaded through every `?` in the expander.
    pub fn lower_located(&mut self, src: &str) -> Result<Vec<Cmd>, (TexError, u32)> {
        let mut lx = Lexer::new(src);
        if self.reading_document {
            lx.mark_as_file();
        }
        match self.block(&mut lx, None) {
            Ok(mut cmds) => {
                self.eng.note_mouth(&lx);
                std::mem::take(&mut self.formulas).fill(&mut cmds);
                self.close_reports(&mut cmds);
                Ok(cmds)
            }
            Err(e) => {
                self.formulas = Default::default();
                // What was reported and recovered from before the run stopped
                // was printed by tex before whatever stopped it, and is the
                // reason a reader needs to make sense of a stop that follows.
                self.eng.note_mouth(&lx);
                let prior: String = self
                    .take_reports()
                    .into_iter()
                    .filter_map(|op| match op {
                        MsgOp::Report(text) => {
                            Some(match text.strip_prefix(crate::expand::DIAGNOSTIC) {
                                Some(shown) => shown.to_string(),
                                None if text == crate::expand::WARNING_ISSUED => String::new(),
                                None => text,
                            })
                        }
                        _ => None,
                    })
                    .collect();
                Err((TexError(format!("{prior}{}", e.0)), lx.line()))
            }
        }
    }

    /// Whatever was reported but never printed, and the line tex writes at the
    /// end of a run that had errors.
    ///
    /// `tex.web` §1335's `close_files_and_terminate` prints `(see the transcript
    /// file for additional information)` when `log_opened` and the run was not
    /// clean. The `)` in front of it is the document's own closing paren, which
    /// tex writes when the file ends and which is otherwise the last character
    /// of the run -- so it is only visible at all once something follows it.
    fn close_reports(&mut self, cmds: &mut Vec<Cmd>) {
        let held = self.take_reports();
        // A run that gave up closes nothing: no paren, no notice.
        if self.eng.halted {
            if !held.is_empty() {
                cmds.push(Cmd::Reports(held));
            }
            return;
        }
        if !self.reported {
            // Tracing output (§245) reported no error, and is printed all the
            // same: a trace after the last message reaches the terminal
            // before the file's `)`, as tex prints it.
            if !held.is_empty() {
                cmds.push(Cmd::Reports(held));
            }
            // Nothing was reported while LOWERING, which does not settle it:
            // §1236's checked arithmetic reports on the VM, and only the VM
            // knows whether it did. The notice becomes a command the run
            // decides, and writes nothing for a run that stayed clean.
            //
            // Not onto an EMPTY chunk, though. A source that lowered to no
            // commands ran no arithmetic, so there is nothing for the VM to
            // have reported and the notice could only ever write nothing --
            // and a preamble of pure definitions has to stay empty, because
            // that is how `src/format.rs` knows a format has lost nothing by
            // dumping it.
            if !cmds.is_empty() || !self.end_notes.is_empty() {
                cmds.push(self.transcript_notice());
            }
            return;
        }
        // What is still held is printed where it stands, and the notice is
        // the run's to decide: a report in an arm the run did not take was
        // never printed, and tex never made it.
        if !held.is_empty() {
            cmds.push(Cmd::Reports(held));
        }
        cmds.push(self.transcript_notice());
    }

    /// §1335's closing: the end notes, after the file's paren -- `)` hard
    /// against the last message in LuaTeX, ` )` in TeX82.
    fn transcript_notice(&mut self) -> Cmd {
        Cmd::TranscriptNotice {
            notes: std::mem::take(&mut self.end_notes),
            glued: self.eng.is_luatex(),
        }
    }

    /// §1335: `\end` with groups still open says how many, once, as the run
    /// closes. The level is the structural depth the lowering has reached.
    fn note_open_groups(&mut self) {
        let level = self.eng.group_level();
        if level > 0 {
            self.end_notes.push(format!(
                "({}end occurred inside a group at level {level})",
                self.eng.esc()
            ));
            // etex.ch's `final_cleanup` lists the groups too
            // (`show_save_groups`), which luatex prints and tex does not.
            if self.eng.is_luatex() {
                let listing = self.eng.save_groups_listing();
                self.end_notes.push(listing);
            }
        }
    }

    /// The reports waiting to be printed, as message pieces.
    ///
    /// Draining the expander's own list here is what puts a report in the right
    /// place: everything reported since the last message goes in front of the
    /// next one.
    fn take_reports(&mut self) -> Vec<MsgOp> {
        let fresh = self.eng.take_errors();
        // Tracing output (§245) is not an error and leaves the run clean.
        let errors = fresh
            .iter()
            .any(|r| !r.starts_with(crate::expand::DIAGNOSTIC));
        self.reported = self.reported || errors || !self.reports.is_empty();
        self.reports.extend(fresh.into_iter().map(MsgOp::Report));
        std::mem::take(&mut self.reports)
    }

    /// Print what was reported so far HERE, ahead of the arms of a conditional
    /// the run decides. Held for the next message instead, the reports landed
    /// in whichever arm was lowered first and were lost when the run took the
    /// other; tex printed them while it scanned the test, before either arm.
    fn flush_reports(&mut self, out: &mut Vec<Cmd>) {
        if self.eng.interaction == 0 {
            return self.withhold_reports(out);
        }
        let held = self.take_reports();
        if !held.is_empty() {
            out.push(Cmd::Reports(held));
        }
    }

    /// LuaTeX's `assign_internal_value` (maincontrol.c): `\hoffset` and
    /// `\pageleftoffset`, and `\voffset` and `\pagetopoffset`, are one
    /// setting in two spellings, an inch apart, so an assignment to either
    /// sets the other, under the same prefix. The inch is `one_true_inch`,
    /// which luatex leaves at `one_inch` (4736287sp, measured) until
    /// `prepare_mag` has seen a `\mag` other than 1000; texrs takes it as
    /// `one_inch` always, which BUGS.md records.
    fn couple_offsets(&mut self, out: &mut Vec<Cmd>, slot: i64) {
        let Some(name) = crate::params::name_of_slot(slot) else {
            return;
        };
        // Each spelling, its partner, and which way the inch goes.
        const PAIRS: [(&str, &str, i64); 4] = [
            ("pageleftoffset", "hoffset", -1),
            ("hoffset", "pageleftoffset", 1),
            ("pagetopoffset", "voffset", -1),
            ("voffset", "pagetopoffset", 1),
        ];
        let Some(&(_, partner, sign)) = PAIRS.iter().find(|(n, ..)| *n == name) else {
            return;
        };
        let Some(partner) = crate::params::register_slot(partner) else {
            return;
        };
        const ONE_INCH: i64 = 4_736_287;
        self.note_global(&[partner]);
        out.push(Cmd::SetCount(
            partner,
            Num::Expr {
                op: crate::etex_arith::Op::Add,
                max: i64::MAX / 2,
                args: vec![Num::Count(slot), Num::Literal(sign * ONE_INCH)],
            },
        ));
    }

    /// Batch mode's terminal: the reports waiting to be printed are not, but
    /// a run that had them still ends with §1335's pointer to the transcript.
    fn withhold_reports(&mut self, out: &mut Vec<Cmd>) {
        let errors = self
            .take_reports()
            .iter()
            .any(|op| !matches!(op, MsgOp::Report(r) if r.starts_with(crate::expand::DIAGNOSTIC)));
        if errors {
            out.push(Cmd::Reports(vec![MsgOp::Report(
                crate::expand::WARNING_ISSUED.to_string(),
            )]));
        }
    }

    /// §1264's `new_interaction`: `print_ln`, so whatever is printed next
    /// starts a line of its own, and the mode. What batch mode held back from
    /// the terminal is not printed when it ends.
    fn new_interaction(&mut self, out: &mut Vec<Cmd>, mode: i64) {
        if self.eng.interaction == 0 {
            self.withhold_reports(out);
        }
        out.push(Cmd::Write(Vec::new()));
        self.eng.interaction = mode;
    }

    /// Hold a report the lowerer built from pieces, behind whatever the
    /// expander reported before it.
    fn queue_report(&mut self, ops: Vec<MsgOp>) {
        let fresh = self.eng.take_errors();
        self.reports.extend(fresh.into_iter().map(MsgOp::Report));
        self.reports.extend(ops);
        self.reported = true;
    }

    /// Lower commands until the input ends, `\end` is seen, or one of `stop` is
    /// reached at nesting depth zero (used for a conditional's arms).
    /// Drop a line directive no command follows.
    ///
    /// A line whose tokens all vanish at compile time — a `\def`, a `\catcode`,
    /// a comment, `\end` — leaves no run-time work behind. A marker there is not
    /// merely useless: `--dap` verifies a breakpoint against the marker set, so
    /// one would let a client set a breakpoint on a line that can never be
    /// reached and report it verified.
    fn drop_empty_line_directives(cmds: Vec<Cmd>) -> Vec<Cmd> {
        let mut out: Vec<Cmd> = Vec::with_capacity(cmds.len());
        for cmd in cmds {
            if matches!(cmd, Cmd::Line(_)) && matches!(out.last(), Some(Cmd::Line(_))) {
                out.pop();
            }
            out.push(cmd);
        }
        if matches!(out.last(), Some(Cmd::Line(_))) {
            out.pop();
        }
        out
    }

    /// Append one character to the text run in progress.
    ///
    /// Looks PAST any line directives: they generate no code, but one is
    /// emitted per line, so treating them as breaks makes every line its own
    /// constant. A 4 MB book then exhausts fusevm's 65,536-entry constant pool
    /// — which a u16 operand cannot address past — and the compile panics.
    /// Coalescing keeps one constant per stretch of text between real commands.
    ///
    /// This is also where TeX's ligature program runs (`tex.web` S1034): the
    /// pairs a text font joins into one character are joined HERE, because the
    /// run in progress still holds the character before this one. `--` becomes
    /// an en dash, `---` an em dash, two backticks a left double quote and two
    /// apostrophes a right one. Doing it at the single point every prose
    /// character passes through is what keeps the measurement and the drawing
    /// agreeing: in Arimo at 10pt a literal pair of backticks is 6.660pt
    /// against 3.330pt for the one character it stands for, so a quoted line
    /// measured from the literal is set too wide for the page it is broken to.
    /// A character by code (§1110's `\char`, a `\chardef`ed name), written to
    /// the text stream when there is one. §434 bounds the code to 0..255.
    fn push_char_code(&mut self, out: &mut Vec<Cmd>, code: i64) -> R<()> {
        if !(0..=255).contains(&code) {
            return Err(TexError(format!("Bad character code ({code})")));
        }
        if self.text_output {
            self.push_text_char(out, char::from(code as u8));
        }
        Ok(())
    }

    fn push_text_char(&mut self, out: &mut Vec<Cmd>, c: char) {
        let mut at = out.len();
        while at > 0 && matches!(out[at - 1], Cmd::Line(_)) {
            at -= 1;
        }
        // The run in progress, found once: its last character is all that is
        // asked of it below, and the character goes on the end of it.
        let mut run = match at.checked_sub(1).and_then(|i| out.get_mut(i)) {
            Some(Cmd::Text(t)) => Some(t),
            _ => None,
        };
        let last = run.as_ref().and_then(|t| t.chars().next_back());
        // A text command's face arrives as characters, so it is seen here: the
        // marker, the one character naming the face, and the closing marker.
        if c == crate::typeset::FACE_POP {
            self.faces.pop();
        } else if last == Some(crate::typeset::FACE_PUSH) {
            self.faces.push(mono_after(c, self.faces.last().copied()));
        }
        // The pair a ligature joins is ONE character, so the first of it comes
        // back out of the run before the character they form goes in. `prev`
        // is taken whatever happens: a character that forms no ligature ends
        // the one that was building.
        let prev = std::mem::take(&mut self.lig);
        let code = self.in_code();
        let joined = match code {
            true => None,
            false => prev
                .filter(|p| last == Some(*p))
                .and_then(|p| ligature(p, c)),
        };
        let mut c = c;
        if let Some(j) = joined {
            if let Some(t) = run.as_mut() {
                t.pop();
            }
            c = j;
        }
        self.lig = (!code).then_some(c);
        match run {
            Some(t) => t.push(c),
            None => out.push(Cmd::Text(c.to_string())),
        }
    }

    /// Whether the characters arriving now are code rather than prose.
    ///
    /// Code is exempt from the ligature program: `\texttt{--flag}` names a
    /// flag that has two hyphens in it, and a listing is source, so joining
    /// the pair would rewrite the program the document is showing. A verbatim
    /// body needs nothing here -- it never reaches `push_text_char`, being
    /// pushed whole -- but a listing's body does, and so does the argument of
    /// a text command, because the prelude expands `\texttt` to markers around
    /// characters that then flow through here one at a time.
    fn in_code(&self) -> bool {
        self.listing_depth > 0 || self.faces.contains(&true)
    }

    /// Count one more level of the lowering recursion -- a group, or the arms
    /// of a run-time conditional, in a block or in a message -- refusing past
    /// [`MAX_LOWER_DEPTH`] levels or [`LOWER_STACK_BUDGET`] bytes of stack,
    /// whichever comes first. Both arms of an undecided conditional are
    /// lowered, so a macro that calls itself inside one never reaches a base
    /// case at lowering time. The caller decrements `depth` on the way out.
    fn enter_level(&mut self) -> R<()> {
        let here = stack_address();
        if self.depth == 0 {
            self.stack_base = here;
        }
        let used = self.stack_base.saturating_sub(here);
        if self.depth >= MAX_LOWER_DEPTH || used > LOWER_STACK_BUDGET {
            return Err(TexError(
                "TeX capacity exceeded, sorry [input stack size=100]".into(),
            ));
        }
        self.depth += 1;
        Ok(())
    }

    fn block(&mut self, lx: &mut Lexer, stop: Option<&[&str]>) -> R<Vec<Cmd>> {
        self.enter_level()?;
        // A group is a ligature boundary. `-{}-` is how a LaTeX document has
        // always asked for two hyphens where two hyphens are meant, and that
        // only works if the hyphen before the group cannot pair with the one
        // after it.
        self.lig = None;
        let out = self.block_inner(lx, stop);
        self.lig = None;
        self.depth -= 1;
        out
    }

    fn block_inner(&mut self, lx: &mut Lexer, stop: Option<&[&str]>) -> R<Vec<Cmd>> {
        let mut out = Vec::new();
        // Whether a `\color` in THIS group is still in force. It is a switch,
        // not a wrapper: it colours everything after it until the group that
        // holds it closes, so the run is ended here rather than by the command.
        let mut colour_open = false;
        // Whether a `\centering` in THIS group is still in force. Like colour
        // it is a switch that lasts until the group closes, so it is ended
        // here rather than by the command.
        let mut centre_open = false;
        // And whether a `\ttfamily` in this group is. The same kind of thing in
        // the same place: a face declaration is a switch that runs to the end
        // of its group, which is what `{\ttfamily code}` -- the body every book
        // redefines `\texttt` to -- depends on.
        let mut face_open = 0usize;
        // Where each open environment's declarations began, so its `\end`
        // closes its own and not those of an environment around it.
        let mut env_marks: Vec<(usize, usize)> = Vec::new();
        // A size declaration is scoped to its group exactly as a face is, and
        // is closed everywhere a face is closed.
        let mut size_open = 0usize;
        // The line the last directive named, so one is emitted per line rather
        // than per command: a `\count` assignment and the `\message` beside it
        // share a line and need only one.
        let mut marked = 0u32;
        // A `\global` register assignment the last command made is marked
        // here, AFTER every command it lowered to, so the mark sees its value.
        // How much of `out` the register tracker has followed.
        let mut followed = 0usize;
        // The length of `out` when the feed was last offered it, and how many
        // tokens have been read since: see `OFFER_EVERY`.
        let mut offered = (usize::MAX, 0usize);
        // §82: the hundredth error ends the run.
        while let Some(tok) = {
            self.flush_keeps(&mut out);
            // What the gullet made for the program while it expanded -- a
            // chunk's register writes -- goes in where the expansion stood.
            out.extend(self.eng.take_deferred());
            for cmd in &out[followed..] {
                self.eng.known.apply(cmd);
            }
            followed = out.len();
            // The document's own level is the one whose commands are its text
            // in order; a group's are not yet placed in it.
            if self.depth == 1 {
                if let Some(feed) = self.feed.as_mut() {
                    offered.1 += 1;
                    if out.len() != offered.0 || offered.1 >= OFFER_EVERY {
                        feed.offer(&out, &self.fonts, &self.layout);
                        offered = (out.len(), 0);
                    }
                }
            }
            // A formula's closing `$` and a `\right` end the block that was
            // lowering it, as a `}` ends a group's (`stomach`).
            let ended = std::mem::take(&mut self.st.end_block);
            let tok = match self.eng.halted || ended {
                true => None,
                false => lx.next_token(&self.eng.cats),
            };
            self.eng.note_mouth(lx);
            tok
        } {
            // §321's `push_input` overflows at `stack_size`: a macro that
            // calls itself and leaves a token behind each time -- the `\fi`
            // of `\def\a{\ifnum\count0<3 \b \fi}` -- grows the input stack by
            // a level per call, and tex stops it there. The message path holds
            // its own token list to the same bound.
            if lx.input_levels() > 10_000 {
                return Err(TexError(
                    "TeX capacity exceeded, sorry [input stack size=10000]".into(),
                ));
            }
            let line = lx.line();
            if line != marked {
                out.push(Cmd::Line(line));
                marked = line;
            }
            // An ACTIVE character is a command, not text: it is looked up in
            // the same table a control sequence is. Rewriting it to that
            // control sequence here means everything below -- expansion, the
            // tail-loop recogniser, `\ifx` -- sees one kind of token and needs
            // no second case for it.
            let tok = match &tok {
                Token::Char(c, Cat::Active) => match self.eng.active_meaning(*c) {
                    Some(id) => Token::Cs(id),
                    None => tok,
                },
                _ => tok,
            };
            // Where the stomach builds the lists, a `\let` copy of a character
            // is that character to `main_control` (its `cur_cmd` is the
            // category): `\bgroup` opens a group as `{` does.
            let tok = self.st_implicit(tok);
            // §1211: the token after a prefix is judged before it is acted on.
            if self.eng.recover && self.eng.prefix_gate(lx, &tok) {
                continue;
            }
            // Mathematics: `$…$`, `$$…$$`, LaTeX's `\(` and `\[`, and the
            // display environments. Caught HERE, before the macro table is
            // consulted, for the reason colour and the faces are: the prelude
            // defines `\(`, `\)`, `\[` and `\]` to expand to nothing, so by
            // the time a name reached the table the formula's delimiters had
            // gone. See `crate::math`, which holds all of it.
            if self.text_output {
                let size = self.layout.size;
                // `\displaywidth` (§1204): a display is centred on the measure
                // and an equation number sits at the far edge of it, so the
                // measure has to travel with the type size.
                let measure = self.layout.measure;
                if crate::math::lower_math(
                    &mut self.eng,
                    size,
                    measure,
                    lx,
                    &tok,
                    &mut out,
                    &mut self.formulas,
                )? {
                    continue;
                }
            }
            // The label pass reads a formula as the run above does and sets
            // nothing; see `crate::math::pass_over_math`.
            if self.formulas_passed_over && crate::math::pass_over_math(&mut self.eng, lx, &tok)? {
                continue;
            }
            // `$`, and the characters math mode gives cases of their own
            // (`stomach`).
            if let Token::Char(c, cat) = tok {
                if self.st_math_token(lx, c, cat, &mut out)? {
                    continue;
                }
            }
            let Token::Cs(name) = &tok else {
                // Braces group the macro table while lowering, so a `\def`
                // inside them is undone at the `}` exactly as TeX undoes it.
                match &tok {
                    Token::Char(_, Cat::BeginGroup) => {
                        // A group scopes the macro table AND the registers it
                        // writes; the latter is run-time state, so the body is
                        // lowered and wrapped in save/restore.
                        self.eng
                            .compile_time_begin_group(GroupKind::Simple, lx.line());
                        let mark = self.globals.len();
                        let known_mark = self.eng.known.mark();
                        let mut body = self.block(lx, Some(&["\u{0}endgroup"]))?;
                        self.eng.compile_time_end_group()?;
                        self.st_group_body(&mut body);
                        // Whatever `\aftergroup` held for this group is read
                        // next, which is what "after the group" means.
                        let after = self.eng.take_after_group();
                        lx.push_back(&after);
                        let (saves, keeps) = self.group_parts(&mut body, mark)?;
                        self.known_after_group(known_mark, &saves, &keeps);
                        // A group exists to save registers and to scope the
                        // macro table. The macro table is a compile-time fact
                        // and is already handled above, so a group that assigns
                        // no register has nothing left to do at run time --
                        // and keeping it breaks the text run either side of it.
                        // A document's braces are everywhere (every
                        // `\NormalTok{...}` is one), so each became its own
                        // constant and a 4 MB book exhausted the 65,536-entry
                        // pool. Flattening a group that only carries text keeps
                        // one constant per stretch.
                        let only_text = body
                            .iter()
                            .all(|c| matches!(c, Cmd::Text(_) | Cmd::Line(_)));
                        if saves.is_empty() && keeps.is_empty() && only_text {
                            for cmd in body {
                                match (&cmd, out.last_mut()) {
                                    (Cmd::Text(t), Some(Cmd::Text(prev))) => prev.push_str(t),
                                    _ => out.push(cmd),
                                }
                            }
                        } else {
                            out.push(Cmd::Group { saves, keeps, body });
                        }
                    }
                    // §1069: a `}` with no group open, or with a `\begingroup`'s,
                    // a formula's or a `\left`'s innermost, closes nothing; it
                    // is reported and dropped (§1069's `extra_right_brace`).
                    Token::Char(_, Cat::EndGroup)
                        if !self.eng.innermost_group().is_some_and(GroupKind::closed_by_brace) =>
                    {
                        let esc = self.eng.esc().to_string();
                        let msg = match self.eng.innermost_group() {
                            None => "Too many }'s".to_string(),
                            Some(GroupKind::MathShift) => "Extra }, or forgotten $".to_string(),
                            Some(GroupKind::MathLeft) => format!("Extra }}, or forgotten {esc}right"),
                            Some(_) => format!("Extra }}, or forgotten {esc}endgroup"),
                        };
                        self.eng.report(lx, &msg);
                    }
                    Token::Char(_, Cat::EndGroup) => {
                        self.close_colour(&mut out, &mut colour_open);
                        self.close_centre(&mut out, &mut centre_open);
                        self.close_face(&mut out, &mut face_open);
                        self.close_size(&mut out, &mut size_open);
                        return Ok(Self::drop_empty_line_directives(out));
                    }
                    // An alignment tab is a cell BOUNDARY, not a character the
                    // document wrote. The catch-all below takes characters of
                    // every catcode, so each one reached the page as a literal
                    // ampersand: 8,941 of them in zmax-reference.pdf, where
                    // lualatex sets the same source with 23 -- the 24 escaped
                    // `\&' it actually writes. Inside a table the boundary is
                    // now a boundary the typesetter builds columns from;
                    // outside one there are no columns, so it stays what
                    // separates two cells set side by side -- a space.
                    // A character in a list the stomach builds (`stomach`).
                    Token::Char(c, cat) if self.st_takes(*cat) => {
                        let (c, cat) = (*c, *cat);
                        self.st_char(lx, c, cat, &mut out)?;
                    }
                    Token::Char(_, Cat::AlignTab) if self.text_output => {
                        let boundary = match self.table_depth > 0 {
                            true => crate::typeset::TABLE_CELL,
                            false => ' ',
                        };
                        self.push_text_char(&mut out, boundary);
                    }
                    // The document's own words. Dropping these is why a book
                    // used to compile to a program that printed nothing.
                    Token::Char(c, _) if self.text_output => {
                        self.push_text_char(&mut out, *c);
                    }
                    _ => {}
                }
                continue;
            };
            let name = *name;
            // §358: a `\noexpand`ed control sequence is `\relax` here.
            if name.noexpand_origin().is_some() {
                continue;
            }
            if let Some(stops) = stop {
                if stops.contains(&name.name()) {
                    lx.push_back(&[Token::Cs(name)]);
                    self.close_colour(&mut out, &mut colour_open);
                    self.close_centre(&mut out, &mut centre_open);
                    self.close_face(&mut out, &mut face_open);
                    self.close_size(&mut out, &mut size_open);
                    return Ok(Self::drop_empty_line_directives(out));
                }
            }
            // A verbatim environment suspends the catcodes: everything up to
            // its \end is characters, not TeX. It has to be caught HERE,
            // before `\begin` expands, because expanding is exactly what must
            // not happen to the body -- a code listing is full of backslashes
            // that are not control sequences, and reading them as control
            // sequences is why a book of code samples could not be read.
            if name.name() == "begin" {
                if let Some(env) = self.peek_environment_name(lx) {
                    // A picture, caught in the same place and for the same
                    // reason a verbatim environment is: its body is TikZ, not
                    // TeX. `(0,0) -- (3,0)` is a path, `;` ends a command,
                    // `\draw` is a picture operator rather than a macro -- and
                    // the prelude's `\def\draw#1;{}` consumed every one of them
                    // and drew nothing, so a document's diagrams reached the
                    // page as blank space. See `picture_environment`.
                    if PICTURE_ENVIRONMENTS.contains(&env.as_str()) {
                        self.picture_environment(lx, &env, &mut out)?;
                        continue;
                    }
                    // Where this environment's declarations start. Closing
                    // every open face at every `\end` is wrong the moment
                    // environments nest, and pandoc nests them as a matter of
                    // course: `\begin{Shaded}\begin{Highlighting}...` closed
                    // Shaded's `\ttfamily` at the INNER end and set the rest
                    // of the listing in the body face.
                    //
                    // Recorded for the general case only. A picture, a listing
                    // and a verbatim body all read their own `\end` out of the
                    // stream above, so none of them reaches the close below.
                    env_marks.push((face_open, size_open));
                    // A listing is read raw for a DIFFERENT reason: its body is
                    // TeX and must expand, but the LINES in it are the author's
                    // and only the raw body still has them. See `lower_listing`.
                    let listing = LISTING_ENVIRONMENTS.contains(&env.as_str());
                    if listing || VERBATIM_ENVIRONMENTS.contains(&env.as_str()) {
                        // Consume the `{name}` that was only peeked at, or it
                        // lands in the output ahead of the body.
                        while let Some(t) = lx.next_token(&self.eng.cats) {
                            if matches!(t, Token::Char(_, Cat::EndGroup)) {
                                break;
                            }
                        }
                        let end = format!("\\end{{{env}}}");
                        let Some(body) = lx.read_raw_until(&end) else {
                            return Err(TexError(format!(
                                "Runaway argument: \\begin{{{env}}} never ends"
                            )));
                        };
                        if self.text_output {
                            // Neither body is prose. A verbatim one is pushed
                            // whole and never sees `push_text_char` at all, so
                            // all that is needed is that the run so far cannot
                            // pair a character with the first of it.
                            self.lig = None;
                            match listing {
                                true => self.lower_listing(&body, &mut out)?,
                                false => self.push_verbatim(&body, &mut out),
                            }
                        }
                        continue;
                    }
                }
            }
            // An environment is a GROUP, and a size its begin-code opened dies
            // with it. The prelude spells the pair `\def\begin#1{\csname
            // #1\endcsname}` and `\def\end#1{\csname end#1\endcsname}`, and
            // neither opens one -- LaTeX's own `\begin` does a `\begingroup`
            // (ltmiscen), which is what scopes a declaration to its
            // environment.
            //
            // Without this, pandoc's template -- which ends
            // `\renewenvironment{Shaded}{...\ttfamily\small}` -- puts the rest
            // of the book in `\small` after its FIRST code listing. cli-fleet
            // came out with 76.7% of its glyphs at 8.97pt from a source
            // holding two `\small`, against a reference that has no 8.97 in it
            // anywhere.
            //
            // The FACE is scoped here for the same reason and by the same
            // rule. It leaked identically all along, and was left alone while
            // nothing could show it: with no `\setmonofont` the mono face IS
            // the main face, so the leak drew the same glyphs.
            //
            // A document that names a monospace family shows it immediately.
            // pandoc's `\renewenvironment{Shaded}{\ttfamily\small}` put the
            // prose after every code listing in the MONO face -- and at the
            // mono family's `Scale=MatchLowercase`, so `arb` set body text at
            // 10.566pt in ShareTechMono where 10pt of Arimo belonged.
            if self.text_output && name.name() == "end" {
                // Down to where this environment opened, and no further: the
                // declarations of an environment still running outside it are
                // not this one's to close.
                let (faces, sizes) = env_marks.pop().unwrap_or((0, 0));
                while size_open > sizes {
                    self.close_size_one(&mut out, &mut size_open);
                }
                while face_open > faces {
                    self.close_face_one(&mut out, &mut face_open);
                }
            }
            // Colour, before the prelude's own definitions can swallow it. The
            // prelude defines these to consume their arguments and emit
            // nothing, which is why a book full of `\color{neonCyan}` came out
            // entirely black.
            if self.text_output && self.lower_colour(lx, name, &mut out, &mut colour_open)? {
                continue;
            }
            // The face, in the same place and for the same reason: the prelude
            // defines `\ttfamily` and its siblings to expand to nothing, so
            // every `\texttt` in every book was set in the body font.
            if self.text_output && self.lower_face(name, &mut out, &mut face_open) {
                continue;
            }
            if self.text_output && self.lower_size(name, &mut out, &mut size_open) {
                // The marker is the SIZE, and the size is not all these names
                // do. size10.clo defines every one of the ten (47-86) and each
                // body assigns: `\@setfontsize` sets \baselineskip and
                // \normalbaselineskip, and the four display skips follow it.
                // size10.clo:134 divides by \baselineskip, so swallowing the
                // call whole left that divisor at INITEX's zero. The definition
                // runs as well when there is one, and the ten bodies set
                // registers and define names -- none of them sets text, so the
                // marker stays the only thing the size contributes here.
                if !self.eng.is_macro(name) {
                    continue;
                }
            }
            // Page structure, for the same reason and in the same place.
            if self.text_output && self.lower_page_break(lx, name, &mut out)? {
                continue;
            }
            // Cross-references, likewise: the prelude answers `\ref` and
            // `\pageref` with nothing, so "See chapter \ref{ch:one} on page
            // \pageref{ch:one}." set as "See chapter  on page .".
            //
            // AFTER the headings, because the number a `\ref` answers with is
            // counted off the marks `\chapter` and `\section` write -- and a
            // `\label` is written straight after the heading it names.
            if self.text_output && self.lower_cross_ref(lx, name, &mut out)? {
                continue;
            }
            // Centring, likewise: the prelude answers `\centering` with
            // nothing, which is why a centred line and the line after it came
            // out as one.
            if self.text_output && self.lower_centre(lx, name, &mut out, &mut centre_open)? {
                continue;
            }
            // Tables, likewise: the prelude answers `\begin{tabular}` and every
            // booktabs rule with nothing, so a table arrived at the breaker as
            // one paragraph of prose.
            if self.text_output && self.lower_table(lx, name, &mut out)? {
                continue;
            }
            // Lists, likewise: the prelude answers `\begin{itemize}` with
            // nothing and `\item` with its optional argument, so every item of
            // every list ran into the one before it.
            //
            // AFTER centring, which reads the `\end...` of whichever
            // environment is open to close a `\centering` region -- including
            // this one's -- and hands the command back rather than consuming
            // it.
            if self.text_output && self.lower_list(lx, name, &mut out)? {
                continue;
            }
            // `\directlua` and its relatives, which RUN their chunk: a real
            // Lua 5.3 (the version LuaTeX embeds), with what the chunk printed
            // pushed back in front of the mouth to be read next. See
            // `crate::lua`. The one thing that was read out of a chunk before
            // anything could run it -- `luaotfload.add_fallback`, the only
            // statement a corpus book makes of WHICH faces a glyph its own face
            // lacks comes from -- is still read there, from the same text.
            //
            // `directlua` is named here as well as in `crate::lua`'s own table
            // because it is the one of these the corpus documents, and
            // `tests/corpus_coverage.rs` reads THIS file's string literals to
            // check that everything documented is still reachable. A name that
            // moved out of here would be reported as no longer dispatched while
            // it demonstrably runs.
            if name.name() == "directlua" || crate::lua::claims(name.name()) {
                crate::lua::lower(&mut self.eng, lx, name.name(), &mut out, &mut self.fonts)?;
                continue;
            }
            // A control sequence MEANS what it was last defined as. The
            // dispatch below is by NAME, so a document that redefines a
            // primitive was still getting the primitive. LaTeX redefines `\end`
            // to close an environment, so a LaTeX document stopped dead at its
            // first `\end{...}` -- which is why a whole book produced a page of
            // preamble text and nothing else.
            if matches!(self.eng.meanings.get(&name), Some(Meaning::Macro(_)))
                && self.meaning_wins(lx, name)
            {
                if let Some(parts) = self.tail_loop(name) {
                    out.extend(self.lower_tail_loop(parts)?);
                    continue;
                }
                self.eng.expand_macro_file(lx, name)?;
                continue;
            }
            // The other half of that: `\let\x=\iffalse` makes `\x` BE
            // `\iffalse` -- tex.web 1221 copies the MEANING, so the two are
            // indistinguishable afterwards. The dispatch below is by name, and
            // the alias was resolved only in its default arm, which a primitive
            // with an arm of its own never reaches. `\let\ifdim=\iffalse` then
            // took the `ifdim' arm and scanned a dimension where tex takes the
            // false branch; `\let\g=\message` worked only because `message` has
            // no arm. Asked AFTER the text-mode handlers above, which dispatch
            // on the name the prelude let: resolving first turned every
            // `\item` into its primitive and unmade the lists.
            //
            // A chain cannot loop: `\let` copies the meaning at definition
            // time, so a `Meaning::Primitive` always names a real primitive
            // rather than another alias.
            if let Some(Meaning::Primitive(p)) = self.eng.meanings.get(&name) {
                let p = *p;
                if p != name {
                    lx.push_back(&[Token::Cs(p)]);
                    continue;
                }
            }
            // The commands that build lists, dispatched on the mode they
            // meet (`stomach`).
            if self.st_command(lx, name, &mut out)? {
                continue;
            }
            match name.name() {
                "end" => {
                    self.note_open_groups();
                    self.end_notes.extend(self.eng.open_conditional_notes());
                    self.ended = true;
                    break;
                }
                // Compile-time: these change how the REST of the file reads.
                "def" | "gdef" => self.eng.compile_time_def(lx, name.name())?,
                "catcode" => self.eng.compile_time_catcode(lx)?,
                "catcodetable" => self.eng.compile_time_catcodetable(lx)?,
                "savecatcodetable" | "initcatcodetable" => {
                    let init = name.name() == "initcatcodetable";
                    self.eng.compile_time_catcode_table_copy(lx, init)?
                }
                // The other per-character tables. Like `\catcode` they change
                // how the rest of the file reads, so they are compile-time.
                // Named one by one rather than through a guard: the corpus
                // gate reads the dispatch to check that everything documented
                // is really reachable, and a guard hides the names from it.
                "mathcode" | "lccode" | "uccode" | "sfcode" | "delcode" => {
                    let t = crate::charcodes::Table::from_name(name.name())
                        .expect("one of the five just matched");
                    self.eng.compile_time_charcode(lx, t)?
                }
                // `\tolerance=200`, `\escapechar=-1`: a §236 parameter is
                // frontend state like a catcode (`crate::intpar` says why), so
                // the assignment happens while lowering.
                _ if self.eng.intpar_cs(name).is_some() => {
                    let i = self.eng.intpar_cs(name).expect("just matched");
                    self.eng.compile_time_intpar(lx, i)?
                }
                "count" => {
                    let reg = self
                        .eng
                        .scan_register_slot(lx, false, crate::compiler::RegFile::Count)?;
                    self.eng.skip_equals_file(lx)?;
                    let v = self.number(lx)?;
                    let global = self.note_global(&[reg]);
                    self.trace_register(reg, &v, global);
                    out.push(Cmd::SetCount(reg, v));
                }
                // `\dimen0=1pt`. A dimension register is a register in the same
                // slot file as the counts, offset past them, so everything that
                // already works for a count -- assignment, a group's save and
                // restore -- works for it with no second mechanism.
                // `\toks0={...}`. A token list is frontend state like a macro
                // body, so it is stored while lowering rather than in a slot,
                // and nothing in the braces expands.
                "toks" => {
                    let reg = self.eng.scan_register_number(lx, false)?;
                    self.eng.do_toks_assign(lx, reg)?;
                }
                // LuaTeX's attribute registers are frontend state, like the
                // integer parameters: assigned while lowering.
                "attribute" => {
                    let n = self.eng.scan_register_number(lx, false)?;
                    self.eng.compile_time_attribute(lx, n)?
                }
                _ if self.eng.attribute_of(name).is_some() => {
                    let n = self.eng.attribute_of(name).expect("just matched");
                    self.eng.compile_time_attribute(lx, n)?
                }
                // `\skip0=1pt plus 2pt minus 3pt`. Four slots, written
                // together, so the whole glue is one assignment.
                // `\muskip0=3mu plus 1mu`. The same four slots in the second
                // glue file: what differs is the unit the right-hand side is
                // read in, and `glue_assign` reads that off the base.
                "skip" | "muskip" => {
                    let file = crate::compiler::RegFile::of_name(name.name()).expect("skip or muskip");
                    let base = self.eng.scan_register_slot(lx, false, file)?;
                    self.eng.skip_equals_file(lx)?;
                    self.note_global(&[base, base + 1, base + 2, base + 3]);
                    out.extend(self.glue_assign(lx, base)?);
                }
                "dimen" => {
                    let slot = self
                        .eng
                        .scan_register_slot(lx, false, crate::compiler::RegFile::Dimen)?;
                    self.eng.skip_equals_file(lx)?;
                    // `\dimen1=\dimen0` copies a register whose value is only
                    // known at run time, and `\dimen1=\skip0` takes a glue's
                    // natural width (§430's coercion); reading a constant here
                    // refused both.
                    self.eng.clear_scaling_sites();
                    let v = self.dimen_number(lx, false)?;
                    let global = self.note_global(&[slot]);
                    self.trace_register(slot, &v, global);
                    self.note_scaling_site(&mut out, lx, &v);
                    out.push(Cmd::SetCount(slot, v));
                }
                // `\pageno=7`, where `\pageno` was `\countdef`'d: the name is
                // the register, so this is the `\count0=7` arm reached by
                // another spelling.
                // `\toks@={...}`, through a name \toksdef gave the register.
                _ if self.eng.toks_cs(name).is_some() => {
                    let reg = self.eng.toks_cs(name).expect("just matched");
                    self.eng.do_toks_assign(lx, reg)?;
                }
                _ if matches!(
                    self.eng.numeric_cs(name),
                    Some(crate::expand::NumericCs::Register(_))
                ) =>
                {
                    let Some(crate::expand::NumericCs::Register(reg)) = self.eng.numeric_cs(name)
                    else {
                        unreachable!("guarded by the arm's own pattern")
                    };
                    self.eng.skip_equals_file(lx)?;
                    // Which register it is decides what follows the `=`: a
                    // dimension register takes a dimension, and reading a bare
                    // number there would store `2pt` as two scaled points and
                    // leave the `pt` behind in the document.
                    // Which register it is decides what follows the `=`, and
                    // there are three kinds now: a glue takes a whole glue, a
                    // dimension a dimension, a count a number. Reading the
                    // wrong one stores a prefix of the value and leaves the
                    // rest in the document.
                    if crate::compiler::is_glue_slot(reg) {
                        self.note_global(&[reg, reg + 1, reg + 2, reg + 3]);
                        let cmds = self.glue_assign(lx, reg)?;
                        out.extend(cmds);
                    } else {
                        self.eng.clear_scaling_sites();
                        let v = match crate::compiler::is_count_slot(reg) {
                            false => self.dimen_number(lx, false)?,
                            true => self.number(lx)?,
                        };
                        let global = self.note_global(&[reg]);
                        self.trace_register(reg, &v, global);
                        self.note_scaling_site(&mut out, lx, &v);
                        out.push(Cmd::SetCount(reg, v));
                        self.couple_offsets(&mut out, reg);
                    }
                }
                // `\advance\tolerance by 5`: the parameter is frontend state,
                // so the arithmetic is done while lowering.
                "advance" | "multiply" | "divide" if self.intpar_follows(lx) => {
                    let Some(Token::Cs(what)) = lx.next_token(&self.eng.cats) else {
                        unreachable!("intpar_follows saw a control sequence")
                    };
                    let i = self.eng.intpar_cs(what).expect("intpar_follows matched it");
                    self.eng.compile_time_intpar_arith(lx, i, name.name())?
                }
                // The same for an attribute register, by number or by name.
                "advance" | "multiply" | "divide" if self.attribute_follows(lx) => {
                    let Some(Token::Cs(what)) = lx.next_token(&self.eng.cats) else {
                        unreachable!("attribute_follows saw a control sequence")
                    };
                    let n = match self.eng.attribute_of(what) {
                        Some(n) => n,
                        None => self.eng.scan_register_number(lx, false)?,
                    };
                    self.eng.compile_time_attribute_arith(lx, n, name.name())?
                }
                "advance" | "multiply" | "divide" => {
                    let op = match name.name() {
                        "advance" => Arith::Add,
                        "multiply" => Arith::Mul,
                        _ => Arith::Div,
                    };
                    let Some(Token::Cs(what)) = lx.next_token(&self.eng.cats) else {
                        return Err(TexError("You can't use this after \\advance".into()));
                    };
                    // `\advance\pageno by 1` names the register the same way
                    // `\advance\count0 by 1` does, and `\advance\dimen0 by 1pt`
                    // names a dimension register the same way again -- the base
                    // the slot falls in is what says which kind it is.
                    let reg = match self.eng.numeric_cs(what) {
                        Some(crate::expand::NumericCs::Register(r)) => r,
                        _ => match crate::compiler::RegFile::of_name(what.name()) {
                            Some(file) => self.eng.scan_register_slot(lx, false, file)?,
                            None => {
                                return Err(TexError(format!(
                                    "Unsupported register \\{}",
                                    what.name()
                                )))
                            }
                        },
                    };
                    self.eng.skip_by_file(lx)?;
                    // `tex.web` §1240: what follows `by` is read in the units
                    // of the register being changed for `\advance`, and as a
                    // plain integer for `\multiply` and `\divide` whatever the
                    // register is -- glue times a length is not a thing TeX
                    // has. Reading the wrong one stores a prefix of the value
                    // and leaves the rest in the document.
                    let glue = crate::compiler::is_glue_slot(reg);
                    let dimen = !glue && !crate::compiler::is_count_slot(reg);
                    if glue {
                        self.note_global(&[reg, reg + 1, reg + 2, reg + 3]);
                    } else {
                        self.note_global(&[reg]);
                    }
                    match (glue, matches!(op, Arith::Add)) {
                        (true, true) => {
                            // A glue REGISTER operand has orders that are the
                            // run's, so it cannot be scanned now (§461).
                            let mu = crate::compiler::is_mu_slot(reg);
                            if let Some((from, negative)) = self.peek_glue_register(lx, mu)? {
                                out.extend(advance_glue_by_register(reg, from, negative));
                            } else {
                                // Each width may be a `<factor><internal unit>`
                                // the run computes -- plain.tex's `\openup` adds
                                // `\dimen@` to `\baselineskip` -- so each is a
                                // `Num`, as `glue_assign` makes them; the orders
                                // are always known.
                                self.eng.clear_scaling_sites();
                                let g = match mu {
                                    true => self.eng.scan_muglue_parts(lx)?,
                                    false => self.eng.scan_glue_parts(lx)?,
                                };
                                let natural = scaled_num(g.natural);
                                self.note_scaling_site(&mut out, lx, &natural);
                                out.push(Cmd::Arith(Arith::Add, reg, natural));
                                out.extend(glue_add_orders(
                                    reg,
                                    (scaled_num(g.stretch), g.stretch_order),
                                    (scaled_num(g.shrink), g.shrink_order),
                                ));
                            }
                        }
                        // A glue scaled by an integer scales in every
                        // component and keeps both orders (§1240).
                        (true, false) => {
                            let v = self.number(lx)?;
                            self.note_error_site(&mut out, lx, op);
                            for part in 0..3 {
                                out.push(Cmd::Arith(op, reg + part, v.clone()));
                            }
                        }
                        (false, _) => {
                            self.eng.clear_scaling_sites();
                            let v = match dimen && matches!(op, Arith::Add) {
                                true => self.dimen_number(lx, false)?,
                                false => self.number(lx)?,
                            };
                            self.note_scaling_site(&mut out, lx, &v);
                            self.note_error_site(&mut out, lx, op);
                            out.push(Cmd::Arith(op, reg, v));
                            self.couple_offsets(&mut out, reg);
                        }
                    }
                }
                // `\show` takes the next token UNEXPANDED (§1294's `get_token`)
                // and prints what it means. The meaning is a frontend fact, so
                // the text is known now; it reaches the terminal the way a
                // report does, in front of whatever is printed next.
                "showtokens" => self.eng.show_tokens(lx)?,
                // LuaTeX's `\immediateassignment` runs the assignment after it
                // while expanding, and `\immediateassigned` the assignments in
                // its braces (maincontrol.c). Where the lowerer reads the file
                // they run where they stand anyway: the prefix vanishes, and the
                // braces come off without opening a group.
                "immediateassignment" => {}
                "immediateassigned" => {
                    let text = self.eng.scan_general_text(lx)?;
                    lx.push_back(&text);
                }
                "showgroups" => self.eng.show_groups(lx),
                "show" => {
                    let Some(tok) = lx.next_token(&self.eng.cats) else {
                        return Err(TexError("File ended while scanning \\show".into()));
                    };
                    self.eng.show_token(lx, &tok);
                }
                // §1297: `\showthe` prints `> `, what §465's `the_toks` makes
                // of the quantity, and `.` with the context, as `\show` does.
                // The quantity may be a register the run holds, so the text is
                // built the way a `\message` with `\the` in it is.
                "showthe" => {
                    let quantity = self.the_operand(lx)?;
                    self.eng.note_mouth(lx);
                    let mut work = Lexer::new("");
                    work.push_back(&quantity);
                    let parts = self.msg_ops(&mut work, &[])?;
                    let nl = self.eng.intpars.get(crate::intpar::NEW_LINE_CHAR);
                    let context = self.eng.error_context(lx).replace('\n', "");
                    let mut ops = vec![MsgOp::Report("> ".into())];
                    ops.extend(crate::ir::printed_ops(parts, nl));
                    ops.push(MsgOp::Report(format!(".{context}")));
                    self.queue_report(ops);
                }
                // §1283: the text is expanded as a `\message`'s is, printed as
                // an error's (`! ` and the text), and §82's `error` follows
                // with its `.` and the context. In nonstop mode the help goes to
                // the log alone, so the terminal shows nothing more.
                // §1264's `new_interaction` (luatex's maincontrol.c): `print_ln`,
                // then the mode. Global whatever group it is in, and a prefix
                // is allowed. `\interactionmode=n` is the same for a mode in
                // range, and `Bad interaction mode` otherwise.
                "batchmode" | "nonstopmode" | "scrollmode" | "errorstopmode" => {
                    let mode = ["batchmode", "nonstopmode", "scrollmode", "errorstopmode"]
                        .iter()
                        .position(|m| *m == name.name())
                        .expect("one of the four just matched");
                    self.new_interaction(&mut out, mode as i64);
                }
                "interactionmode" => {
                    self.eng.skip_equals_file(lx)?;
                    let mode = self.eng.scan_number_file(lx)?;
                    match (0..=3).contains(&mode) {
                        true => self.new_interaction(&mut out, mode),
                        false => self
                            .eng
                            .report(lx, &format!("Bad interaction mode ({mode})")),
                    }
                }
                "errmessage" => {
                    let parts = self.message_parts(lx)?;
                    self.eng.note_mouth(lx);
                    let nl = self.eng.intpars.get(crate::intpar::NEW_LINE_CHAR);
                    let context = self.eng.error_context(lx).replace('\n', "");
                    let mut ops = vec![MsgOp::Report("! ".into())];
                    ops.extend(crate::ir::printed_ops(parts, nl));
                    ops.push(MsgOp::Report(format!(".{context}")));
                    self.queue_report(ops);
                }
                "message" => {
                    let parts = self.message_parts(lx)?;
                    self.eng.note_mouth(lx);
                    // Printed to the terminal: §59's `print`, with the
                    // `\newlinechar` in force now.
                    let nl = self.eng.intpars.get(crate::intpar::NEW_LINE_CHAR);
                    let parts = crate::ir::printed_ops(parts, nl);
                    // Anything reported so far is printed HERE, in front of the
                    // message and with nothing between them: see `reports`.
                    // In batch mode the terminal gets none of it (§75).
                    if self.eng.interaction == 0 {
                        self.withhold_reports(&mut out);
                    } else {
                        let mut ops = self.take_reports();
                        ops.extend(parts);
                        out.push(Cmd::Message(ops));
                    }
                }
                // §1275: `\openin` closes the stream and opens it again on the
                // file the name finds -- `.tex` supplied as `\input` supplies it
                // -- leaving it closed when nothing has it; `\closein` closes
                // it. Neither waits for a shipout, and neither is undone by a
                // group. The file is read while lowering, as `\input` is.
                "openin" => {
                    let n = self.eng.scan_stream_number(lx, false)?;
                    self.eng.skip_equals_file(lx)?;
                    let name = self.scan_file_name(lx)?;
                    // `find_read_file` with id n+1, then `open_read_file`.
                    let text = match crate::input_hooks::find_read_file(n as i64 + 1, &name) {
                        Some(None) => None,
                        Some(Some(file)) => {
                            crate::input_hooks::open_read_file(&file).or_else(|| {
                                self.locate_written(&file)
                                    .or_else(|| Self::locate_input(&file))
                                    .map(|(_, src)| src)
                            })
                        }
                        None => crate::input_hooks::open_read_file(&name).or_else(|| {
                            self.locate_written(&name)
                                .or_else(|| Self::locate_input(&name))
                                .map(|(_, src)| src)
                        }),
                    };
                    self.eng.open_read_stream(n, text);
                }
                "closein" => {
                    let n = self.eng.scan_stream_number(lx, false)?;
                    self.eng.close_read_stream(n);
                }
                "read" => self.eng.compile_time_read(lx)?,
                "readline" => self.eng.compile_time_readline(lx)?,
                // §362: the file ends with the line being read. In the
                // document itself that leaves no `\end` to find unless it is on
                // the same line, as in tex.
                "endinput" => lx.end_file_after_line(),
                // §1375: `\immediate` reads the next token with `get_x_token`
                // and, when it is `\openout`, `\write` or `\closeout`, does it
                // now. Anything else goes back to be read as if `\immediate`
                // had not been there -- `\special` included, which LuaTeX's
                // `do_extension` makes a node either way.
                "immediate" => match self.eng.next_unexpandable(lx)? {
                    Some((_, Some(p))) if matches!(p.name(), "openout" | "write" | "closeout") => {
                        self.extension(lx, p.name(), true, &mut out)?
                    }
                    // LuaTeX's `do_extension` with `immediate` set: an object
                    // or a picture written now rather than when a page refers
                    // to it.
                    Some((_, Some(p)))
                        if matches!(
                            p.name(),
                            "pdfextension" | "saveimageresource" | "saveboxresource"
                        ) =>
                    {
                        self.backend_extension(lx, p.name(), true)?
                    }
                    Some((t, _)) => lx.push_back(&[t]),
                    None => {}
                },
                "openout" | "write" | "closeout" | "special" => {
                    self.extension(lx, name.name(), false, &mut out)?
                }
                "pdfextension" | "saveimageresource" | "useimageresource" => {
                    self.backend_extension(lx, name.name(), false)?
                }
                "saveboxresource" | "useboxresource" | "savepos" => {
                    self.backend_extension(lx, name.name(), false)?
                }
                // `\input FILE` reads another file HERE, sharing every piece of
                // state: a macro it defines is defined afterwards, a `\catcode`
                // it sets stays set. That is the whole point of it -- a real
                // document's first line loads a format or a package, and until
                // this existed no real document could run at all.
                "input" => {
                    let name = self.scan_file_name(lx)?;
                    let (shown, src) = self.open_input(&name)?;
                    out.push(Cmd::Message(vec![MsgOp::Text(format!("({shown}"))]));
                    self.input_depth += 1;
                    let inner = self.input_pass(&src);
                    self.input_depth -= 1;
                    out.extend(inner?);
                    // The file's own commands were followed as it was lowered.
                    followed = out.len();
                    out.push(Cmd::FileClose);
                    // `\end` inside the file stops the whole run, not just the
                    // file: tex closes every open paren and finishes.
                    if self.ended {
                        break;
                    }
                }
                // `\rustcompile <base64>\endrust`, which is what a `\rust{ … }`
                // block desugared to before the mouth ever read the file.
                n if n == crate::rust_ffi::COMPILE_CS => {
                    let b64 = self.rust_blob(lx)?;
                    out.push(Cmd::RustCompile(b64));
                }
                // A bare `\rustcall` in running text: the value is discarded,
                // which is how a document calls a Rust function for its effect.
                n if n == crate::rust_ffi::CALL_CS => {
                    let call = self.rust_call(lx, false)?;
                    out.push(Cmd::Message(vec![]));
                    // The call has to reach the VM, and a message with no
                    // pieces emits nothing but the flush -- so put the call in
                    // it and drop the rendered text by flushing an empty build.
                    if let Some(Cmd::Message(parts)) = out.last_mut() {
                        parts.push(MsgOp::Discard(call));
                    }
                }
                "ifnum" | "ifabsnum" => {
                    // `\unless` belongs to this conditional, so it is taken
                    // before the test can expand a conditional of its own.
                    let negated = self.eng.take_unless();
                    let line = lx.line();
                    let abs = name.name() == "ifabsnum";
                    // §498: the conditional is open while its test is read.
                    self.eng.begin_test();
                    let left = magnitude(abs, self.number(lx)?);
                    let rel = match self.eng.read_relation_file(lx, name.name())? {
                        '<' => Rel::Less,
                        '>' => Rel::Greater,
                        _ => Rel::Equal,
                    };
                    let right = magnitude(abs, self.number(lx)?);
                    // §498 decides the test when the mouth reaches it. When
                    // every register it reads has a value the lowerer has
                    // followed (`crate::known`), that is now, and only the arm
                    // taken is read: a `\def` in the other never runs, as in
                    // tex. Lowering both arms ran both definitions and the
                    // losing one won.
                    if let Some(truth) = self.known_truth(&left, rel, &right) {
                        let opened_by = (if abs { "ifabsnum" } else { "ifnum" }, negated, line);
                        self.eng
                            .enter_decided_file(lx, truth != negated, opened_by)?;
                        continue;
                    }
                    self.eng.end_test();
                    self.eng.set_unless(negated);
                    self.flush_reports(&mut out);
                    self.next_arms = Some((if abs { "ifabsnum" } else { "ifnum" }, negated, line));
                    let (then_branch, else_branch) = self.arms(lx)?;
                    out.push(Cmd::IfNum {
                        left,
                        rel,
                        right,
                        then_branch,
                        else_branch,
                    });
                }
                // `\ifdim` is `\ifnum` over dimensions (`tex.web` §503 shares
                // the comparison and differs only in the scanner), and a
                // dimension is an integer in a slot -- so it lowers to the same
                // real branch rather than being recognised and skipped.
                "ifdim" | "ifabsdim" => {
                    let negated = self.eng.take_unless();
                    let line = lx.line();
                    let abs = name.name() == "ifabsdim";
                    self.eng.begin_test();
                    let left = magnitude(abs, self.dimen_number(lx, false)?);
                    let rel = match self.eng.read_relation_file(lx, name.name())? {
                        '<' => Rel::Less,
                        '>' => Rel::Greater,
                        _ => Rel::Equal,
                    };
                    let right = magnitude(abs, self.dimen_number(lx, false)?);
                    // Decided now when both dimensions are known; see `\ifnum`.
                    if let Some(truth) = self.known_truth(&left, rel, &right) {
                        let opened_by = (if abs { "ifabsdim" } else { "ifdim" }, negated, line);
                        self.eng
                            .enter_decided_file(lx, truth != negated, opened_by)?;
                        continue;
                    }
                    self.eng.end_test();
                    self.eng.set_unless(negated);
                    self.flush_reports(&mut out);
                    self.next_arms = Some((if abs { "ifabsdim" } else { "ifdim" }, negated, line));
                    let (then_branch, else_branch) = self.arms(lx)?;
                    out.push(Cmd::IfNum {
                        left,
                        rel,
                        right,
                        then_branch,
                        else_branch,
                    });
                }
                // `tex.web` §509 over a selector the run decides. A constant
                // selector is the expander's, which skips the losing cases with
                // §494's `pass_text` and lowers only the one that is selected;
                // a register's value is the VM's, so the cases lower to the
                // same equality chain the `\message` arm builds. Leaving this
                // to the expander answered from its own copy of the register,
                // which no run-time `\count1=2` ever reaches.
                "ifcase" => match {
                    self.eng.begin_test();
                    let line = lx.line();
                    let selector = self.number(lx);
                    (line, selector?)
                } {
                    // A selector whose value is known -- a constant, or a
                    // register the lowerer has followed -- is the expander's.
                    (line, value) if self.eng.known.num(&value).is_some() => {
                        let n = self.eng.known.num(&value).expect("just matched");
                        self.eng.ifcase_file(lx, n, line)?
                    }
                    (_, value) => {
                        self.eng.end_test();
                        self.flush_reports(&mut out);
                        let branches = self.case_arms(lx)?;
                        out.extend(case_chain_cmds(value, branches));
                    }
                },
                "ifodd" => {
                    let negated = self.eng.take_unless();
                    let line = lx.line();
                    self.eng.begin_test();
                    let value = self.number(lx);
                    let value = value?;
                    if let Some(v) = self.eng.known.num(&value) {
                        self.eng.enter_decided_file(
                            lx,
                            (v % 2 != 0) != negated,
                            ("ifodd", negated, line),
                        )?;
                        continue;
                    }
                    self.eng.end_test();
                    self.eng.set_unless(negated);
                    self.flush_reports(&mut out);
                    self.next_arms = Some(("ifodd", negated, line));
                    let (then_branch, else_branch) = self.arms(lx)?;
                    out.push(Cmd::IfOdd {
                        value,
                        then_branch,
                        else_branch,
                    });
                }
                // `\iftrue`, `\iffalse`, `\ifx`, `\ifdefined`, `\ifcsname` and
                // the box conditionals have NO arm here, deliberately. Their
                // truth is a frontend fact -- the macro table, or the fact that
                // no box register has been filled -- so the EXPANDER decides
                // them, in `do_conditional`, and §494's `pass_text` skips the
                // arm that lost without lowering it. That is what keeps a
                // `\let` in the losing arm from running: lowering a branch
                // EXECUTES the compile-time assignments in it, and
                // `\ifx\a\b\let\x\y\else\let\x\z\fi` used to run both.
                //
                // They were decided HERE once, by a `decided_arms` that read
                // each arm as a bounded token region ending at its own `\fi`.
                // That boundary is what LaTeX's `\@ifundefined` breaks: it is
                // `\ifcsname` around `\expandafter\expandafter\expandafter
                // \firstoftwo`, three `\expandafter`s whose whole purpose is to
                // EXPAND the `\fi` and the `\else` away so the two-argument
                // macro lands outside the conditional and eats the arguments
                // that follow it. A `\fi` expanded like that popped §489's
                // condition stack, which `decided_arms` had never pushed to, so
                // the outer conditional's `\else` found the stack empty and the
                // run stopped with `! Extra \else.` One stack, kept by the
                // expander for every conditional, is what tex has.
                "let" => self.eng.compile_time_let(lx)?,
                // Both define a control sequence that stands for a number, and
                // both are compile-time: what they define changes how the rest
                // of the file READS, exactly as `\def` does.
                "chardef" | "countdef" | "mathchardef" | "dimendef" | "skipdef" | "muskipdef"
                | "toksdef" | "attributedef" => self.eng.compile_time_numeric_def(lx, name.name())?,
                "newcommand" | "renewcommand" | "providecommand" | "DeclareRobustCommand" => {
                    self.eng.compile_time_newcommand(lx, name.name())?
                }
                // Preamble directives naming files texrs cannot load. Their
                // arguments are consumed so the body of the document is still
                // read; see compile_time_preamble_directive.
                // The font the document asked for. Its argument is consumed
                // either way; the difference is that the name is kept.
                "setmainfont" | "setsansfont" | "setmonofont" | "setromanfont" => {
                    let k = name.name();
                    let _ = self.eng.read_optional_bracket(lx)?;
                    let name = self.eng.read_group_text_pub(lx)?;
                    // The options are where a document that SHIPS its font
                    // says so, with `Path=` and `UprightFont=`. Discarding
                    // them left only a family name nothing has installed.
                    let options = self.optional_text(lx)?.unwrap_or_default();
                    let name = name.trim();
                    // fontspec accepts the FILE's own name where a family name
                    // goes -- `\setmainfont{Arimo-VF.ttf}[Path=...]` -- and
                    // lualatex honours it. Read as a family it names nothing
                    // installed, so a document written that way was set in
                    // whatever fc-match answered with. The options still win:
                    // an explicit `UprightFont=`/`Extension=` is what the
                    // document said last and is not overwritten here.
                    let mut file = crate::typeset::FontFile::parse(&options);
                    file.absorb_filename(name);
                    match k {
                        "setmainfont" | "setromanfont" => self.fonts.main_file = file,
                        // The monospace face ships beside the document exactly
                        // as the main one does -- `UprightFont=
                        // ShareTechMono-Regular` in every corpus book -- so the
                        // family name on its own resolves to nothing and
                        // `\texttt` falls back to the body font it was meant to
                        // escape.
                        "setmonofont" => self.fonts.mono_file = file,
                        "setsansfont" => self.fonts.sans_file = file,
                        _ => {}
                    }
                    let slot = match k {
                        "setsansfont" => &mut self.fonts.sans,
                        "setmonofont" => &mut self.fonts.mono,
                        _ => &mut self.fonts.main,
                    };
                    // A filename is kept as its STEM: the family fallback
                    // compares names with the punctuation stripped out, so
                    // `Arimo-VF.ttf` became `arimovfttf` and matched no
                    // installed file either.
                    *slot = Some(crate::typeset::font_family_name(name).to_string());
                }
                // The class options carry the type size; geometry's options
                // carry the margins. Both used to be consumed and dropped,
                // which set an 11pt book on 0.95in margins as a 10pt book on
                // plain.tex's 1in page.
                "documentclass" => {
                    let (options, _) = self.eng.compile_time_preamble_directive(lx, 1)?;
                    self.layout.absorb_class_options(&options);
                }
                "usepackage" | "RequirePackage" => {
                    let (options, packages) = self.eng.compile_time_preamble_directive(lx, 1)?;
                    if packages.first().is_some_and(|p| p.trim() == "geometry") {
                        self.layout.absorb_geometry_options(&options);
                    }
                }
                "PassOptionsToPackage" | "PassOptionsToClass" => {
                    let to_class = name.name() == "PassOptionsToClass";
                    let (_, args) = self.eng.compile_time_preamble_directive(lx, 2)?;
                    // `\PassOptionsToPackage{margin=1in}{geometry}` carries the
                    // options in the FIRST brace and names its target in the
                    // second -- there is no `[...]` here at all. What it passes
                    // reaches the class or the package exactly as an option
                    // list written on them would, so it is read the same way.
                    let options = args.first().cloned().unwrap_or_default();
                    if to_class {
                        self.layout.absorb_class_options(&options);
                    } else if args.get(1).is_some_and(|p| p.trim() == "geometry") {
                        self.layout.absorb_geometry_options(&options);
                    }
                }
                "makeatletter" => self.eng.compile_time_set_at_letter(true),
                "makeatother" => self.eng.compile_time_set_at_letter(false),
                "futurelet" => self.eng.compile_time_futurelet(lx)?,
                // Advice registration is a compile-time act: it changes what
                // the macros after it expand to.
                "intercept" => self.eng.compile_time_intercept(lx)?,
                "edef" | "xdef" => {
                    // The prefix is spent either way; `\globaldefs` decides
                    // over both it and `\xdef`'s own globality (§1214).
                    let prefix = self.eng.take_global_prefix();
                    let global = self
                        .eng
                        .global_defs_applied(name.name() == "xdef" || prefix);
                    let snapshots = self.edef(lx, global)?;
                    out.extend(snapshots);
                }
                // `\begingroup` is a group in tex.web's sense (§1063's
                // `simple_group` reached by `new_save_level`), so it scopes the
                // REGISTERS as well as the macro table. Only the macro table
                // was scoped here, which is compile-time state; a register
                // lives in a VM slot, so saving it means lowering the body and
                // wrapping it the way a `{...}` body is wrapped. Without that
                // the two group forms disagreed: `{\count0=2}` restored and
                // `\begingroup\count0=2\endgroup` did not.
                "begingroup" => {
                    self.eng
                        .compile_time_begin_group(GroupKind::SemiSimple, lx.line());
                    let mark = self.globals.len();
                    let known_mark = self.eng.known.mark();
                    let mut body = self.block(lx, Some(&["endgroup"]))?;
                    self.st_group_body(&mut body);
                    // The `\endgroup` that stopped the block was pushed back
                    // for this arm to consume; at end of input there is none.
                    if !self.ended {
                        let _ = lx.next_token(&self.eng.cats);
                    }
                    self.eng.compile_time_end_group()?;
                    let after = self.eng.take_after_group();
                    lx.push_back(&after);
                    let (saves, keeps) = self.group_parts(&mut body, mark)?;
                    self.known_after_group(known_mark, &saves, &keeps);
                    // No register written means nothing is left for run time --
                    // the macro table was already scoped above -- so the body
                    // is spliced in rather than wrapped, which also keeps a
                    // stretch of text one constant instead of two.
                    if saves.is_empty() && keeps.is_empty() {
                        for cmd in body {
                            match (&cmd, out.last_mut()) {
                                (Cmd::Text(t), Some(Cmd::Text(prev))) => prev.push_str(t),
                                _ => out.push(cmd),
                            }
                        }
                    } else {
                        out.push(Cmd::Group { saves, keeps, body });
                    }
                    // `\end` inside the group stops the whole run, as it does
                    // inside an `\input` file.
                    if self.ended {
                        break;
                    }
                }
                // §1065 and §1064's `off_save`: an `\endgroup` with no group
                // open is reported and dropped.
                "endgroup" if self.eng.innermost_group().is_none() => {
                    let msg = format!("Extra {}endgroup", self.eng.esc());
                    self.eng.report(lx, &msg);
                }
                "endgroup" => self.eng.compile_time_end_group()?,
                // §1135: an `\endcsname` with no `\csname` open is reported and
                // dropped.
                "endcsname" => {
                    let msg = format!("Extra {}endcsname", self.eng.esc());
                    self.eng.report(lx, &msg);
                }
                // `tex.web` §1288: read the group unexpanded, put every
                // character through `\uccode`/`\lccode`, and push the result
                // back to be read again. The catcodes travel unchanged, which
                // is what LaTeX's `\MakeUppercase` is built on.
                "uppercase" | "lowercase" => {
                    let table = match name.name() {
                        "uppercase" => crate::charcodes::Table::Upper,
                        _ => crate::charcodes::Table::Lower,
                    };
                    self.eng.do_case_shift(lx, table)?;
                }
                // §326: hold the next token and insert it after the enclosing
                // group's `}`.
                "aftergroup" => {
                    let Some(t) = self.eng.take_file(lx) else {
                        return Err(TexError("Missing token for \\aftergroup".into()));
                    };
                    self.eng.after_group(t);
                }
                // §1269: hold the next token and insert it once the assignment
                // that follows has finished.
                "afterassignment" => {
                    let Some(t) = self.eng.take_file(lx) else {
                        return Err(TexError("Missing token for \\afterassignment".into()));
                    };
                    self.eng.set_after_assignment(t);
                }
                // §1060: skip the spaces that follow. Nothing here is in
                // horizontal mode, so it has only its mouth-level effect --
                // which is the whole of it for a `\message` stream.
                "ignorespaces" => self.skip_spaces_in_text(lx),
                // §465 and §470: expandable, so at the top level what they stand
                // for is characters, which are read next like any other. A token
                // register is tokens and is the expander's; any other quantity
                // is written out here the way a message writes it.
                "the" | "number" => self.expand_the_here(lx, name)?,
                "global" => {
                    self.eng.set_global_prefix(true);
                    self.eng.await_prefixed_command();
                }
                // The other two definition prefixes. Like `\global` they set a
                // flag the definition that follows reads and spends.
                "long" => {
                    self.eng.set_long_prefix(true);
                    self.eng.await_prefixed_command();
                }
                "outer" => {
                    self.eng.set_outer_prefix(true);
                    self.eng.await_prefixed_command();
                }
                "protected" => {
                    self.eng.set_protected_prefix(true);
                    self.eng.await_prefixed_command();
                }
                // `\unless\ifnum ...` runs the ELSE arm when the test holds.
                "unless" => self.eng.set_unless(true),
                "relax" => {}
                // §1110 and §1141: `\char` and a `\chardef`ed name set the
                // character of that code. The font draws it, as for any other.
                "char" => {
                    let Num::Literal(code) = self.number(lx)? else {
                        // A register's value is the run's, and a character is chosen here.
                        return Err(TexError(
                            "\\char needs a number known while lowering".into(),
                        ));
                    };
                    self.push_char_code(&mut out, code)?;
                }
                // A blank line IS a `\par` -- the mouth synthesises one per
                // blank line, §304 -- and dropping it is why a whole book came
                // out as a single paragraph. The line breaker starts a fresh
                // paragraph at "\n\n"; scifi2/docs/book.tex holds 3,163 blank
                // lines and reached the breaker with 58 separators, so every
                // paragraph lost the ragged last line it is entitled to, and
                // the words on either side of a suppressed break welded
                // together: `// A NOVEL OF DEEP TIME //TWO SHIPS IN THE DARK.`
                "par" => {
                    if self.text_output {
                        self.push_text(&mut out, "\n\n");
                    }
                }
                _ if matches!(self.eng.meanings.get(&name), Some(Meaning::CharDef(_))) => {
                    let Some(Meaning::CharDef(code)) = self.eng.meanings.get(&name) else {
                        unreachable!("just matched");
                    };
                    let code = *code;
                    self.push_char_code(&mut out, code)?;
                }
                // §1221's `\let\x=<character>`: the name is that character's
                // command and code wherever it is executed -- an implicit space
                // (expl3's `\exp_stop_f:`), an implicit brace (`\bgroup`), a
                // letter. In running text it is the character itself.
                _ if matches!(self.eng.meanings.get(&name), Some(Meaning::Char(..))) => {
                    let Some(Meaning::Char(c, cat)) = self.eng.meanings.get(&name) else {
                        unreachable!("just matched");
                    };
                    let char_token = Token::Char(*c, *cat);
                    lx.push_back(&[char_token]);
                }
                other => {
                    // TeX's loop idiom -- a macro whose last act is to call
                    // itself under a test -- becomes a real loop rather than an
                    // inline copy. Inlining it cannot terminate: the copy holds
                    // the call that gets copied.
                    if let Some(parts) = self.tail_loop(name) {
                        let cmds = self.lower_tail_loop(parts)?;
                        out.extend(cmds);
                        continue;
                    }
                    // A macro expands into the stream and lowering continues
                    // through its body -- expansion is a frontend concern.
                    if matches!(self.eng.meanings.get(&name), Some(Meaning::Macro(_))) {
                        self.eng.expand_macro_file(lx, name)?;
                        continue;
                    }
                    // Expandable primitives reach the top level too: TeX's
                    // expander handles `\expandafter`, `\csname` and friends
                    // wherever they occur, not only inside a macro body. Whatever
                    // they leave behind goes back through this loop.
                    if self.eng.expand_in_text(lx, name)? {
                        continue;
                    }
                    // `\let\g=\message` makes `\g` MEAN the primitive, and a
                    // primitive is dispatched by name here, so the alias has to
                    // resolve to the name before the match runs or `\g` reads as
                    // undefined while `\message` works.
                    if let Some(Meaning::Primitive(p)) = self.eng.meanings.get(&name) {
                        let p = *p;
                        if p != name {
                            lx.push_back(&[Token::Cs(p)]);
                            continue;
                        }
                    }
                    // A character the document spelled as a control sequence:
                    // `\rightarrow`, `\alpha`, `\S`. Nothing above defines
                    // them, so a document that writes one stopped dead with
                    // `! Undefined control sequence \rightarrow.` and produced
                    // no page at all. Asked LAST, so a document that defines
                    // its own `\star` -- or lets one to something -- keeps it;
                    // this only answers where the alternative is refusing the
                    // document. The character is all that is decided here:
                    // which font draws it is the typesetter's, and asking
                    // `crate::typeset` for it keeps that one table.
                    if let Some(ch) = crate::typeset::symbol_char(other) {
                        if self.text_output {
                            self.push_text_char(&mut out, ch);
                        }
                        continue;
                    }
                    // §370: a name with no meaning is reported, expands to
                    // nothing, and the run goes on. What is not undefined but
                    // is not done here -- a primitive this engine has no arm
                    // for -- still stops it.
                    if self.eng.recover && self.eng.is_undefined(name) {
                        self.eng.report(lx, "Undefined control sequence");
                        continue;
                    }
                    return Err(TexError(format!("Undefined control sequence \\{other}")));
                }
            }
            // `tex.web` §1269: a token `\afterassignment` held is put back once
            // the ASSIGNMENT that followed has finished -- at the end of
            // `prefixed_command` and nowhere else, which is why this is here
            // rather than at the top of the loop.
            let assigned = crate::expand::Engine::is_assignment(name.name())
                || self.eng.intpar_cs(name).is_some()
                || matches!(
                    self.eng.numeric_cs(name),
                    Some(crate::expand::NumericCs::Register(_))
                )
                || self.eng.toks_cs(name).is_some();
            if let Some(t) = self.eng.take_after_assignment(assigned) {
                lx.push_back(&[t]);
            }
        }
        self.flush_keeps(&mut out);
        self.close_colour(&mut out, &mut colour_open);
        self.close_centre(&mut out, &mut centre_open);
        self.close_face(&mut out, &mut face_open);
        self.close_size(&mut out, &mut size_open);
        Ok(Self::drop_empty_line_directives(out))
    }

    /// Carry §311's context display to the run, for a command that can report
    /// there.
    ///
    /// Only `\multiply` and `\divide` can: §1236 checks them and `\advance`
    /// wraps silently. The display is taken AFTER the operand has been scanned,
    /// because that is where tex's mouth stands when the check fails -- for
    /// `\multiply\count1 by 2` the whole line has been read, so the display is
    /// the line and an empty second half.
    fn note_error_site(&mut self, out: &mut Vec<Cmd>, lx: &Lexer, op: Arith) {
        if matches!(op, Arith::Add) {
            return;
        }
        if lx.context().is_some() {
            out.push(Cmd::ErrorSite(self.eng.error_context(lx)));
        }
    }

    /// Carry §311's context display to the run for a value that can be out of
    /// range when it is computed: a factor times an internal dimension, which
    /// §455 reports as `Dimension too large` once the whole dimension is read.
    fn note_scaling_site(&mut self, out: &mut Vec<Cmd>, lx: &Lexer, v: &Num) {
        if matches!(v, Num::Scaled { .. } | Num::ByCount { .. }) && lx.context().is_some() {
            if let Some(site) = self.eng.next_scaling_site() {
                out.push(Cmd::ErrorSite(site));
            }
        }
    }

    /// `\edef` and `\xdef`: §1218, with the body read by §473's
    /// `scan_toks(true, true)` -- expanded AS it is read, over the registers
    /// `crate::known` has followed. See `Engine::read_edef_body`.
    fn edef(&mut self, lx: &mut Lexer, global: bool) -> R<Vec<Cmd>> {
        let name = self.eng.get_r_token(lx, false)?;
        let mut frozen = crate::expand::Frozen {
            next_scratch: self.next_scratch,
            snapshots: Vec::new(),
        };
        let (params, body) = self.eng.read_edef_parts(lx, name, &mut frozen)?;
        self.next_scratch = frozen.next_scratch;
        self.eng.define_macro_with_params(name, params, body, global)?;
        Ok(frozen.snapshots)
    }

    /// The `\else` and `\fi` arms of a conditional, each lowered.
    /// `\textcolor[model]{spec}{text}`, as markers INSIDE the text run.
    ///
    /// Not as a `Cmd::Color` wrapping a block, which is what this did first: a
    /// colour region splits the text either side of it into separate commands,
    /// each of which becomes its own string constant, and Pandoc's syntax
    /// highlighting emits thousands of `\textcolor` calls per book. That
    /// exhausted fusevm's 65,536-constant pool on five of the larger documents
    /// -- the same ceiling the braces hit before, reached a different way.
    ///
    /// Markers in the stream keep the text coalescing as it did, and the
    /// typesetter turns them into the DVI `\special` a driver reads. Only the
    /// `rgb` model is understood, which is what a Pandoc document writes;
    /// anything else falls through to the ordinary macro path rather than being
    /// coloured wrongly.
    /// The commands that move to a new page, before the prelude's stubs can
    /// swallow them.
    ///
    /// The prelude defined `\newpage`, `\clearpage` and `\pagebreak` to expand
    /// to nothing, so a book's title page, copyright page and first chapter ran
    /// together as one stream of prose. `\chapter` was `#1` -- the heading text
    /// and nothing else -- so no chapter began a page either, and a 270-page
    /// book came out at 144.
    fn lower_page_break(
        &mut self,
        lx: &mut Lexer,
        name: crate::token::CsId,
        out: &mut Vec<Cmd>,
    ) -> R<bool> {
        match name.name() {
            // `\pagebreak[0-4]` takes an optional strength, which is advice
            // about how badly the break is wanted rather than a break itself;
            // taken as a break, since the document asked.
            "newpage" | "clearpage" | "cleardoublepage" | "pagebreak" => {
                let _ = self.eng.read_optional_bracket(lx)?;
                self.push_text(out, &crate::typeset::PAGE_BREAK.to_string());
                Ok(true)
            }
            // titlesec, caught here rather than left to the prelude's stub,
            // which consumed the format argument and discarded it.
            //
            // Every book in the corpus styles its headings this way --
            // `\titleformat{\chapter}[hang]{\sffamily\bfseries\Huge\color{..}}`
            // -- and with the format thrown away a chapter title got the class
            // default face at the class default size. Both are wrong at once
            // and neither is visible as an error.
            //
            // The two spellings differ in shape: the starred one is
            // `\titleformat*{\cmd}{fmt}`, and the plain one carries an
            // OPTIONAL argument after its first mandatory one --
            // `\titleformat{\cmd}[shape]{fmt}{label}{sep}{before}`. The format
            // is the one argument worth anything here; the label, separator
            // and before-code are titlesec's own layout and are dropped as
            // they were.
            "titleformat" => {
                let starred = self.eng.skip_optional_star(lx);
                let command = self.eng.read_group_text_pub(lx)?;
                if !starred {
                    let _ = self.eng.read_optional_bracket(lx)?;
                }
                let format = self.eng.read_balanced_group(lx)?;
                if !starred {
                    for _ in 0..3 {
                        let _ = self.eng.read_balanced_group(lx)?;
                    }
                }
                // `{\section}` arrives with its backslash on.
                let name = command.trim().trim_start_matches('\\').to_string();
                self.title_formats.insert(name, format);
                Ok(true)
            }
            // A figure, caught here rather than left to the prelude's stub.
            // Stubbed, the image was dropped AND reserved no room, so a
            // document with a figure and one without produced byte-identical
            // PDFs -- the caption survived alone and the page count was short
            // by every figure in the book.
            //
            // The lengths are read as the document wrote them: `\textwidth` is
            // a measure the lowerer does not have, so it travels as a fraction
            // and is resolved in `to_pdf`. See `IMAGE`.
            // pandoc wraps every figure it emits in this, and the prelude
            // defines it as a pass-through -- but the DOCUMENT defines it too,
            // and the document's definition wins. Pandoc's own is
            //
            //   \sbox\pandoc@box{#1} ... \usebox{\pandoc@box}
            //
            // which boxes the image, divides by its height with \Gscale@div
            // and sets it through \scalebox. None of that is on this path, so
            // the figure went into a box that was never set and every one of
            // them vanished -- 51 in `inventions` alone, silently, since the
            // caption is outside the wrapper and survived to look right.
            //
            // Caught here rather than left to expand, for the reason the arms
            // around it are: what the wrapper MEANS is "set this, scaled down
            // if it would overflow", and setting it unscaled is the faithful
            // half of that. Scaling is a box operation and is not pretended at.
            "pandocbounded" => {
                let raw = self.eng.read_balanced_group(lx)?;
                self.lower_into(&raw, out)?;
                Ok(true)
            }
            "includegraphics" => {
                let options = self.optional_text(lx)?.unwrap_or_default();
                let path = self.eng.read_group_text_pub(lx)?;
                let (width, height) = crate::typeset::image_options(&options);
                let mark = crate::typeset::image_mark(&width, &height, path.trim());
                self.push_text(out, &mark);
                Ok(true)
            }
            // A chapter starts a page. `\chapter*{...}` is the unnumbered form
            // and `\chapter[short]{long}` carries a running-head title; both
            // begin a page, and the long title is what gets set.
            "chapter" => {
                self.eng.skip_optional_star(lx);
                let _ = self.eng.read_optional_bracket(lx)?;
                self.push_text(out, &crate::typeset::PAGE_BREAK.to_string());
                self.push_heading(out, 6, 4, lx, 0)?;
                Ok(true)
            }
            // The section headings. Same treatment and for the same reason as
            // `\chapter`: the prelude set each to its own argument, so a
            // heading was the first words of the paragraph under it.
            "section" | "subsection" | "subsubsection" => {
                self.eng.skip_optional_star(lx);
                let _ = self.eng.read_optional_bracket(lx)?;
                // The depth the contents knows it by: `tocdepth` counts a
                // chapter 0 and each step down one more.
                let level = match name.name() {
                    "section" => 1,
                    "subsection" => 2,
                    _ => 3,
                };
                self.push_heading(out, 2, 2, lx, level)?;
                Ok(true)
            }
            // The contents, which the prelude answered with nothing: every
            // book in the corpus opens with one and none of them was set. What
            // goes here is the REQUEST -- the contents cannot be built until
            // the pages its entries name exist, so the typesetter builds it
            // (`typeset::contents_set`).
            "tableofcontents" => {
                // The mark alone, with no paragraph break around it: the
                // contents the typesetter puts in its place opens and closes
                // with the breaks it needs, and a document read as TEXT --
                // which has no pages and so no contents -- is then exactly
                // the text it was before.
                self.push_text(out, &crate::typeset::toc_request_mark(self.toc_depth));
                Ok(true)
            }
            // `\setcounter{tocdepth}{n}` says how deep the contents goes, and
            // it is the one counter read here: the prelude swallows
            // `\setcounter` whole, and every book sets this one to 0 -- a
            // contents of chapters -- immediately above its
            // `\tableofcontents`. Both arguments are consumed either way, as
            // the prelude's two-argument stub consumed them, so a counter this
            // does not know is set exactly as it was before.
            "setcounter" => {
                let counter = self.group_chars(lx)?;
                let value = self.group_chars(lx)?;
                if counter.trim() == "tocdepth" {
                    if let Ok(depth) = value.trim().parse::<usize>() {
                        self.toc_depth = depth;
                    }
                }
                Ok(true)
            }
            // `\end{titlepage}` is `\newpage` and then, unless the class is
            // two-sided, `\setcounter{page}\@ne` (extreport.cls:514-518). So
            // the cover sheet is not one of the document's numbered pages and
            // the contents entries are a page lower than the sheets they stand
            // on. The mark goes BEFORE the break, so that it lands on the
            // cover sheet itself -- which is what it is about; the command is
            // handed back rather than consumed, because an `\end...` is also
            // what closes a `\centering` region and a title page is built out
            // of centred pieces.
            "endtitlepage" => {
                let mark = crate::typeset::toc_page_one_mark();
                self.push_text(out, &format!("{mark}{}", crate::typeset::PAGE_BREAK));
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    /// `\label`, `\ref` and `\pageref`, before the prelude's stubs can swallow
    /// them.
    ///
    /// The prelude reads all three and produces nothing, which is right for a
    /// label -- it is a name for a place, not text -- and wrong for the other
    /// two: `See chapter \ref{ch:one} on page \pageref{ch:one}.` set as `See
    /// chapter  on page .`, which reads as broken prose. `\label` is 88,341
    /// occurrences across the corpus.
    ///
    /// None of the three can be answered here. A `\ref` names a unit that may
    /// not have been read yet and a `\pageref` a page that does not exist
    /// until the document has been broken and paginated, so what is written is
    /// the question -- see `typeset::REF` -- and the typesetter answers it.
    fn lower_cross_ref(
        &mut self,
        lx: &mut Lexer,
        name: crate::token::CsId,
        out: &mut Vec<Cmd>,
    ) -> R<bool> {
        let code = match name.name() {
            "label" => crate::typeset::REF_LABEL,
            "ref" => crate::typeset::REF_NUMBER,
            "pageref" => crate::typeset::REF_PAGE,
            _ => return Ok(false),
        };
        // Read exactly what the prelude's one-argument stub read, so a
        // reference in a document that never reaches the typesetter consumes
        // its key as it did before.
        let key = self.group_chars(lx)?;
        self.push_text(out, &crate::typeset::ref_mark(code, &key));
        Ok(true)
    }

    /// A heading's text, with `above` and `below` units of vertical space
    /// around it.
    ///
    /// The unit is `typeset::PARAGRAPH_SPACE` -- half a leading, the space
    /// LaTeX leaves between two paragraphs -- because that is the smallest
    /// length the page breaker spends and so the one the rest are counted in.
    /// LaTeX spends 50pt above a chapter title and 40pt below it, and roughly
    /// a line and a half either side of a section; a heading set with none of
    /// that is indistinguishable from the paragraph it introduces, and the
    /// page holds lines that lualatex spends on white space.
    ///
    /// `level` is what the contents knows the heading by -- 0 for a chapter,
    /// one more for each step down. The mark naming it goes at the head of the
    /// heading's own paragraph, so the title is everything from the mark to
    /// the paragraph break: that is what `typeset::contents_entries` reads.
    fn push_heading(
        &mut self,
        out: &mut Vec<Cmd>,
        above: usize,
        below: usize,
        lx: &mut Lexer,
        level: usize,
    ) -> R<()> {
        let space = crate::typeset::VERTICAL_SPACE.to_string();
        // The heading owns its own lines rather than running into the
        // paragraphs either side of it, so the space is bracketed by the
        // paragraph breaks that end them.
        self.push_text(out, "\n\n");
        self.push_text(out, &space.repeat(above));
        self.push_text(out, "\n\n");
        self.push_text(out, &crate::typeset::toc_entry_mark(level));
        // The size book.cls sets each level at: `\Huge` for a chapter title
        // (book.cls:387), `\Large` for a section (407), `\large` for a
        // subsection (411), the body size below that. Set here rather than in
        // the prelude because a heading is read as an argument and the size
        // has to wrap what that argument lowers to.
        //
        // This is the whole of the page-count gap: a heading set at the body
        // size does not wrap where lualatex's does, and takes one body line
        // where lualatex gives it a larger box.
        let step = match level {
            0 => "Huge",
            1 => "Large",
            2 => "large",
            _ => "normalsize",
        };
        // What `\titleformat` said this level is set with, if the document
        // said anything. It REPLACES the class default rather than adding to
        // it, which is what titlesec does: a format naming no size leaves the
        // heading at the body size, exactly as it would under LaTeX.
        let named = match level {
            0 => "chapter",
            1 => "section",
            2 => "subsection",
            _ => "subsubsection",
        };
        let format = self.title_formats.get(named).cloned();
        let sized = match format.is_some() {
            true => None,
            false => crate::typeset::size_step(step, self.layout.size),
        };
        if let Some(size) = sized {
            self.push_text(
                out,
                &format!(
                    "{}{};{}{}",
                    crate::typeset::SIZE_PUSH,
                    size.size,
                    size.leading,
                    crate::typeset::SIZE_PUSH
                ),
            );
        }
        let raw = self.eng.read_balanced_group(lx)?;
        // The format and the title go down in ONE call, so the group that
        // `lower_into` opens closes the format's declarations after the title
        // rather than before it. Lowered separately, a `\sffamily` in the
        // format would end at the end of the format.
        match format {
            Some(format) => {
                let mut both = format;
                both.extend_from_slice(&raw);
                self.lower_into(&both, out)?;
            }
            None => self.lower_into(&raw, out)?,
        }
        if sized.is_some() {
            self.push_text(out, &crate::typeset::SIZE_POP.to_string());
        }
        self.push_text(out, "\n\n");
        self.push_text(out, &space.repeat(below));
        self.push_text(out, "\n\n");
        Ok(())
    }

    /// Centring, before the prelude's stubs can swallow it.
    ///
    /// `\begin{center}` and `\centering` were defined to expand to nothing, so
    /// "centred line" and "left line" came out as one flowing line -- and a
    /// title page, which is built out of nothing but centred pieces, ran into
    /// the prose after it. The region reaches the page as the markers in
    /// `typeset`, which is where a line finds out it is positioned by its
    /// width rather than at the margin.
    ///
    /// Returns whether the command was one of these and has been dealt with.
    fn lower_centre(
        &mut self,
        lx: &mut Lexer,
        name: crate::token::CsId,
        out: &mut Vec<Cmd>,
        centre_open: &mut bool,
    ) -> R<bool> {
        let open = crate::typeset::CENTRE.to_string();
        let close = crate::typeset::CENTRE_END.to_string();
        match name.name() {
            // `\begin{center}` runs `\center`, as latex.ltx has it, and
            // `\end{center}` runs `\endcenter`. `\centering` is the switch
            // form: it centres everything up to the end of the group or the
            // environment holding it, which is what the arm below closes.
            "center" | "centering" => {
                self.push_text(out, &open);
                *centre_open = true;
                Ok(true)
            }
            "endcenter" => {
                self.push_text(out, &close);
                *centre_open = false;
                Ok(true)
            }
            // `\centerline{...}` centres exactly its argument, on its own
            // line: it is a box, not a switch.
            "centerline" => {
                self.push_text(out, &format!("\n\n{open}"));
                let raw = self.eng.read_balanced_group(lx)?;
                self.lower_into(&raw, out)?;
                self.push_text(out, &format!("{close}\n\n"));
                Ok(true)
            }
            // Every LaTeX environment is a group, and `\centering` lasts until
            // the group holding it closes. Environments here are a macro pair
            // rather than a group, so the `\end{...}` of whichever one is open
            // -- `minipage`, `titlepage`, `figure` -- is what ends the region.
            // Without this a single `\centering` in a title page centred every
            // remaining page of the book. The command itself is left to the
            // ordinary dispatch: this closes a region, it does not consume an
            // environment.
            //
            // `\endcsname` is spelt like one and is not one -- it is the
            // second half of the `\csname` that BUILDS the `\end...` about to
            // arrive, so counting it closed every region one command early and
            // wrote the marker twice.
            other if *centre_open && other.starts_with("end") && other != "endcsname" => {
                self.push_text(out, &close);
                *centre_open = false;
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    /// The table environments and the marks inside them, before the prelude's
    /// stubs can swallow them.
    ///
    /// `\begin{tabular}` and `\begin{longtable}` expanded to nothing, `&` to a
    /// space and `\\` to a newline, so a table reached the breaker as one
    /// paragraph of prose -- "Name Value alpha 1 beta 2", welded to the
    /// sentence after it. The corpus leans on this heavily: pandoc emits a
    /// longtable for every markdown table, 132 of them in
    /// groovyrs/docs/book.tex and 43 in awkrs/docs/book.tex.
    ///
    /// What reaches the typesetter is the STRUCTURE and not the setting: a cell
    /// boundary, a row end, and a mark for each booktabs rule and each
    /// longtable section boundary. Column widths are measured where the font
    /// is, in `typeset::table_lines`.
    ///
    /// Returns whether the command was one of these and has been dealt with.
    fn lower_table(
        &mut self,
        lx: &mut Lexer,
        name: crate::token::CsId,
        out: &mut Vec<Cmd>,
    ) -> R<bool> {
        use crate::typeset::{
            FIRST_HEAD_END, FOOT_END, HEAD_END, LAST_FOOT_END, RULE_BOTTOM, RULE_MID, RULE_TOP,
            TABLE_MARK, TABLE_ROW,
        };
        // Every arm below that means something only inside a table asks this
        // in its BODY rather than in a match guard, and hands the command back
        // when there is no table open. The corpus gate reads the HEAD of each
        // arm to check that everything the engine dispatches is documented,
        // and a guard hides every name after the first one from it.
        let inside = self.table_depth > 0;
        match name.name() {
            // `\begin{tabular}[pos]{cols}`, and longtable's identical shape.
            // The arguments are read HERE because the prelude's stub is what
            // would otherwise eat them and it is never reached: this returns
            // first, exactly as `\centering` is taken before its stub.
            "tabular" | "longtable" => {
                let _ = self.eng.read_optional_bracket(lx)?;
                let _ = self.eng.read_balanced_group(lx)?;
                // A table is its own block, so the prose either side of it is
                // not filled into its first and last row.
                self.push_text(out, "\n\n");
                self.table_depth += 1;
                Ok(true)
            }
            "endtabular" | "endlongtable" => {
                self.table_depth = self.table_depth.saturating_sub(1);
                // A row end, in case the last row was written without one. A
                // row of nothing but blank cells is dropped when the table is
                // set, so one that was not needed costs nothing.
                self.push_text(out, &format!("{TABLE_ROW}\n\n"));
                Ok(true)
            }
            // booktabs' three rules, and `\hline`, which is the kernel's own
            // spelling of the same line. `\midrule[width]` takes an optional
            // thickness; the rule is drawn at booktabs' weight either way.
            "toprule" | "midrule" | "bottomrule" | "hline" => {
                if !inside {
                    return Ok(false);
                }
                let code = match name.name() {
                    "toprule" => RULE_TOP,
                    "bottomrule" => RULE_BOTTOM,
                    _ => RULE_MID,
                };
                let _ = self.eng.read_optional_bracket(lx)?;
                self.push_text(out, &format!("{TABLE_MARK}{code}"));
                Ok(true)
            }
            // longtable writes its head and its foot BEFORE its body, so these
            // boundaries are what lets the table be set in the order a reader
            // gets it rather than the order it was written.
            "endhead" | "endfirsthead" | "endfoot" | "endlastfoot" => {
                if !inside {
                    return Ok(false);
                }
                let code = match name.name() {
                    "endhead" => HEAD_END,
                    "endfirsthead" => FIRST_HEAD_END,
                    "endfoot" => FOOT_END,
                    // The last foot is not the repeating one: it is set once,
                    // under the end of the table, where the repeating foot is
                    // set at the bottom of every page the table runs past.
                    _ => LAST_FOOT_END,
                };
                self.push_text(out, &format!("{TABLE_MARK}{code}"));
                Ok(true)
            }
            // A row ends at `\\`, which is a line break everywhere else and is
            // one here too -- of a row rather than of a line. `\\*` forbids a
            // break after it and `\\[2pt]` asks for extra space; neither
            // changes where the row ends.
            "\\" | "tabularnewline" => {
                if !inside {
                    return Ok(false);
                }
                self.eng.skip_optional_star(lx);
                let _ = self.eng.read_optional_bracket(lx)?;
                self.push_text(out, &TABLE_ROW.to_string());
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// The list environments and the items in them, before the prelude's stubs
    /// can swallow them.
    ///
    /// `\begin{itemize}` expanded to nothing and `\item` to its optional
    /// argument, so a list of two items read back as "first item second item"
    /// -- one line, no bullet, no break. `\item` is 8,683 occurrences across
    /// the corpus, so this is the shape of a large part of every book in it.
    ///
    /// What reaches the typesetter is the STRUCTURE and not the setting: a
    /// paragraph break so each item is its own line, the depth marker saying
    /// which left edge that line hangs from, and the item's MARK as ordinary
    /// text -- a bullet, a number, or a description's term in bold. How far in
    /// a depth is, and how much of the measure is left after it, are decided
    /// where the font is, in `typeset::fill`.
    ///
    /// Returns whether the command was one of these and has been dealt with.
    fn lower_list(
        &mut self,
        lx: &mut Lexer,
        name: crate::token::CsId,
        out: &mut Vec<Cmd>,
    ) -> R<bool> {
        // As in `lower_table`, every arm that means something only inside a
        // list asks so in its BODY rather than in a match guard: the corpus
        // gate reads the HEAD of each arm, and a guard would hide every name
        // after the first one from it.
        match name.name() {
            // `\begin{itemize}` runs `\itemize`, the way latex.ltx has it.
            "itemize" | "enumerate" | "description" => {
                let kind = match name.name() {
                    "itemize" => ListKind::Bullet,
                    "enumerate" => ListKind::Number,
                    _ => ListKind::Term,
                };
                self.lists.push(List { kind, count: 0 });
                self.start_list_line(out);
                Ok(true)
            }
            // And `\end{itemize}` runs `\enditemize`. An `\end` with no list
            // open is somebody else's -- `\end{document}` reaches here too --
            // and is handed straight back.
            "enditemize" | "endenumerate" | "enddescription" => {
                if self.lists.pop().is_none() {
                    return Ok(false);
                }
                self.start_list_line(out);
                Ok(true)
            }
            // `\item` outside a list is the prelude's, which yields its
            // optional argument: that is what a document's own `\item`-like
            // macro expects, and it is not this to take.
            "item" => {
                let Some(list) = self.lists.last_mut() else {
                    return Ok(false);
                };
                list.count += 1;
                let (kind, count) = (list.kind, list.count);
                // `\item[term]`. The term is TeX and is LOWERED rather than
                // flattened -- pandoc writes `\item[\texttt{-{}-flag}]`, and
                // flattening it would set the macro name.
                let term = self.eng.read_optional_bracket(lx)?;
                self.start_list_line(out);
                match (kind, term) {
                    // A description's term is the mark, and every description
                    // list sets it bold.
                    (ListKind::Term, Some(term)) => {
                        self.push_text(out, &format!("{}b", crate::typeset::FACE_PUSH));
                        self.lower_into(&term, out)?;
                        self.push_text(out, &format!("{} ", crate::typeset::FACE_POP));
                    }
                    // Elsewhere an explicit label REPLACES the mark, which is
                    // what `\item[$\ast$]` in an itemize asks for.
                    (_, Some(term)) => {
                        self.lower_into(&term, out)?;
                        self.push_text(out, " ");
                    }
                    (ListKind::Bullet, None) => self.push_text(out, "\u{2022} "),
                    (ListKind::Number, None) => self.push_text(out, &format!("{count}. ")),
                    // A description item with no term has no mark: the body is
                    // all it wrote.
                    (ListKind::Term, None) => {}
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// End the line in hand and start the next one at the depth now open.
    ///
    /// The three list boundaries -- the `\begin`, each `\item`, the `\end` --
    /// are one thing to the breaker: a line ends here, and what follows hangs
    /// from that depth. Depth 0 is a list closing, and puts the prose after it
    /// back at the margin.
    fn start_list_line(&mut self, out: &mut Vec<Cmd>) {
        self.push_text(out, "\n\n");
        let mark = crate::typeset::indent_mark(self.lists.len());
        self.push_text(out, &mark);
    }

    /// End a centred region that is still in force, if there is one.
    fn close_centre(&self, out: &mut Vec<Cmd>, centre_open: &mut bool) {
        if std::mem::take(centre_open) {
            self.push_text(out, &crate::typeset::CENTRE_END.to_string());
        }
    }

    /// The colour commands, before the prelude's stubs can swallow them.
    ///
    /// Returns whether the command was one of these and has been dealt with.
    /// The colour reaches the page as a marker in the text -- `\u{1}r,g,b\u{2}`
    /// to start and `\u{3}` to end -- because both backends draw colour as
    /// something wrapped around a run of characters rather than as a character.
    fn lower_colour(
        &mut self,
        lx: &mut Lexer,
        name: crate::token::CsId,
        out: &mut Vec<Cmd>,
        colour_open: &mut bool,
    ) -> R<bool> {
        match name.name() {
            // `\definecolor{name}{model}{spec}` -- the palette, which is what
            // every later `\color{name}` is looked up in.
            "definecolor" | "providecolor" | "colorlet" => {
                let _ = self.eng.read_optional_bracket(lx)?;
                let defined = self.eng.read_group_text_pub(lx)?;
                let second = self.eng.read_group_text_pub(lx)?;
                match name.name() {
                    // `\colorlet{new}{old}` names an existing colour again.
                    "colorlet" => {
                        if let Some(rgb) = self.colours.get(&second) {
                            self.colours.define_rgb(&defined, rgb);
                        }
                    }
                    other => {
                        let spec = self.eng.read_group_text_pub(lx)?;
                        // `\providecolor` defines only what is not defined.
                        let fresh = self.colours.get(&defined).is_none();
                        if other == "definecolor" || fresh {
                            self.colours.define(&defined, &second, &spec);
                        }
                    }
                }
                Ok(true)
            }
            // `\pagecolor{name}` paints the page. Recorded rather than drawn:
            // the backend paints it under everything else, which is the only
            // order in which the text stays on top of it.
            "pagecolor" => {
                let model = self.optional_text(lx)?;
                let spec = self.eng.read_group_text_pub(lx)?;
                if let Some(rgb) = self.colours.resolve(model.as_deref(), &spec) {
                    self.page_colour = Some(rgb);
                }
                Ok(true)
            }
            // `\color{name}` is a SWITCH: everything after it is that colour
            // until the group holding it closes. `\textcolor{name}{text}`
            // colours exactly its argument.
            "color" | "textcolor" => {
                let model = self.optional_text(lx)?;
                let spec = self.eng.read_group_text_pub(lx)?;
                let Some((r, g, b)) = self.colours.resolve(model.as_deref(), &spec) else {
                    // An unknown colour leaves the text in the colour it had.
                    // `\textcolor` still owes its argument to the page.
                    if name.name() == "textcolor" {
                        let raw = self.eng.read_balanced_group(lx)?;
                        self.lower_into(&raw, out)?;
                    }
                    return Ok(true);
                };
                if name.name() == "color" {
                    // A second `\color` in one group replaces the first rather
                    // than nesting: TeX has one current colour, not a stack.
                    self.close_colour(out, colour_open);
                    self.push_text(out, &format!("\u{1}{r},{g},{b}\u{2}"));
                    *colour_open = true;
                    return Ok(true);
                }
                self.push_text(out, &format!("\u{1}{r},{g},{b}\u{2}"));
                let raw = self.eng.read_balanced_group(lx)?;
                self.lower_into(&raw, out)?;
                self.push_text(out, "\u{3}");
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// End a `\color` that is still in force, if there is one.
    fn close_colour(&self, out: &mut Vec<Cmd>, colour_open: &mut bool) {
        if std::mem::take(colour_open) {
            self.push_text(out, "\u{3}");
        }
    }

    /// The face declarations, before the prelude's stubs can swallow them.
    ///
    /// These are LaTeX's font switches, and every one of them is what a text
    /// command is made of: `\texttt{x}` is `{\ttfamily x}`, which is how a book
    /// that redefines `\texttt` -- and the corpus books all do, to colour their
    /// inline code -- still reaches the mono face. The face travels to the page
    /// as a marker in the text, `\u{11}` and a code to open and `\u{12}` to
    /// close, because a face wraps a run of characters rather than being one.
    ///
    /// Returns whether the command was one of these.
    fn lower_face(
        &mut self,
        name: crate::token::CsId,
        out: &mut Vec<Cmd>,
        face_open: &mut usize,
    ) -> bool {
        // Each of these sets ONE axis and says nothing about the other two,
        // which is what LaTeX's declarations do. The marker carries the same
        // letter, read as a modifier of the face already in force -- see
        // `Face::from_code`.
        let code = match name.name() {
            "ttfamily" => 'm',
            "sffamily" => 's',
            "rmfamily" => 'r',
            "bfseries" => 'b',
            "itshape" | "slshape" => 'i',
            // The one a heading writes to undo all three at once.
            "normalfont" => 'n',
            _ => return false,
        };
        // A second switch in one group JOINS the first rather than replacing
        // it. Replacing is what made `{\sffamily\bfseries\Huge}` -- how every
        // book in the corpus writes a heading -- come out bold and not sans.
        // Each push is its own pop, so the count is what the group closes.
        self.faces
            .push(mono_after(code, self.faces.last().copied()));
        self.push_text(out, &format!("{}{}", crate::typeset::FACE_PUSH, code));
        *face_open += 1;
        true
    }

    /// End every face declaration still in force in this group.
    fn close_face(&mut self, out: &mut Vec<Cmd>, face_open: &mut usize) {
        for _ in 0..std::mem::take(face_open) {
            self.faces.pop();
            self.push_text(out, &crate::typeset::FACE_POP.to_string());
        }
    }

    /// `\large`, `\Large`, `\huge` and the other seven, as a size marker.
    ///
    /// A size is a declaration exactly as a face is -- `{\Large text}` rather
    /// than `\Large{text}` -- so it is caught here and scoped to its group the
    /// same way, rather than defined in the prelude. `\@setfontsize` is what
    /// these route through in LaTeX and it is stubbed empty in `kernel.tex`,
    /// which is why every heading in every book set at the body size.
    ///
    /// Returns whether the command was one of the ten.
    fn lower_size(
        &self,
        name: crate::token::CsId,
        out: &mut Vec<Cmd>,
        size_open: &mut usize,
    ) -> bool {
        let Some(size) = crate::typeset::size_step(name.name(), self.layout.size) else {
            return false;
        };
        // A second size in one group replaces the first -- one size is in
        // force, not a stack of them -- so the previous one closes here.
        self.close_size(out, size_open);
        self.push_text(
            out,
            &format!(
                "{}{};{}{}",
                crate::typeset::SIZE_PUSH,
                size.size,
                size.leading,
                crate::typeset::SIZE_PUSH
            ),
        );
        *size_open += 1;
        true
    }

    /// End a size declaration that is still in force, if there is one.
    fn close_size(&self, out: &mut Vec<Cmd>, size_open: &mut usize) {
        for _ in 0..std::mem::take(size_open) {
            self.push_text(out, &crate::typeset::SIZE_POP.to_string());
        }
    }

    /// An optional `[model]`, as its text.
    fn optional_text(&mut self, lx: &mut Lexer) -> R<Option<String>> {
        Ok(self.eng.read_optional_bracket(lx)?.map(|tokens| {
            tokens
                .iter()
                .map(|t| t.to_text(self.eng.esc()))
                .collect::<String>()
        }))
    }

    /// Lower a group's tokens into the run in progress.
    fn lower_into(&mut self, raw: &[Token], out: &mut Vec<Cmd>) -> R<()> {
        let mut inner = Lexer::new("");
        inner.push_back(raw);
        self.lower_lexer_into(&mut inner, out)
    }

    /// The same, for a source that is characters rather than tokens.
    fn lower_lexer_into(&mut self, lx: &mut Lexer, out: &mut Vec<Cmd>) -> R<()> {
        for cmd in self.block(lx, None)? {
            match (&cmd, out.last_mut()) {
                (Cmd::Text(t), Some(Cmd::Text(prev))) => prev.push_str(t),
                _ => out.push(cmd),
            }
        }
        Ok(())
    }

    /// Lower a code listing, one output line per source line.
    ///
    /// The body arrives raw, so the newlines the author wrote are still in it.
    /// Each line is lowered ON ITS OWN -- `\NormalTok` and its siblings expand
    /// exactly as they did, colour markers and all, because a line is a whole
    /// TeX input -- and is terminated with `LISTING_BREAK`, which is what tells
    /// the breaker the line ends there rather than where the measure runs out.
    ///
    /// A blank source line is a blank code line and is kept as one. Reaching
    /// the lexer it would have been a `\par` instead, which ends the listing
    /// and starts a paragraph -- and a program's blank lines are part of it.
    ///
    /// The block is fenced with a paragraph break either side, so the first
    /// line cannot weld onto the sentence above nor the last onto the one
    /// below.
    fn lower_listing(&mut self, body: &str, out: &mut Vec<Cmd>) -> R<()> {
        let body = without_environment_option(body);
        // The newline that ENDS the `\begin` line is not a code line.
        let body = body.strip_prefix('\n').unwrap_or(body);
        self.push_text(out, "\n\n");
        // A listing is source, and the ligature program must not rewrite it:
        // the `--` in a flag and the `''` in a string literal are what the
        // program says.
        self.listing_depth += 1;
        // fancyvrb's `commandchars=\\\{\}`, which is what pandoc writes:
        //
        //   \DefineVerbatimEnvironment{Highlighting}{Verbatim}{commandchars=\\\{\}}
        //
        // Inside such an environment EVERY character except the three named is
        // catcode 12. texrs honoured the backslash and the braces -- they are
        // special anyway -- and left the rest alone, so a shell prompt opened
        // MATH MODE: `$ awk '/^fn/{f=1}'` lost its `$`, took `^` as a
        // superscript and `_` as a subscript, and set the remains in the
        // roman face at the body size.
        //
        // arb has 597 of these blocks and 397 of them contain a literal `$`,
        // 259 with an odd count so the math span runs to the end of the block.
        const VERBATIM: [char; 7] = ['$', '_', '^', '&', '#', '~', '%'];
        let saved: Vec<_> = VERBATIM
            .iter()
            .map(|c| (*c, self.eng.cats.get(*c)))
            .collect();
        for c in VERBATIM {
            self.eng.cats.set(c, crate::catcode::Cat::Other);
        }
        let lowered = (|| -> R<()> {
            for line in body.lines() {
                let mut lx = Lexer::new(line);
                self.lower_lexer_into(&mut lx, out)?;
                self.push_text(out, &crate::typeset::LISTING_BREAK.to_string());
            }
            Ok(())
        })();
        for (c, cat) in saved {
            self.eng.cats.set(c, cat);
        }
        lowered?;
        self.listing_depth -= 1;
        self.push_text(out, "\n\n");
        Ok(())
    }

    /// A verbatim body: its own lines, in the monospace face.
    ///
    /// It was pushed as one string with no marker of any kind, which cost it
    /// BOTH -- the lines flowed into the surrounding paragraph as prose, and
    /// the characters were set in the body face. `\begin{verbatim}` is how a
    /// document says "this is not prose", and it came out as prose in the body
    /// typeface.
    ///
    /// `--text` was never wrong here, because a verbatim body reaches it with
    /// its newlines intact; only the PDF path flowed them. That is why it went
    /// unnoticed -- the cheaper output was right.
    ///
    /// The lines are marked the way a listing's are and the face is pushed
    /// once around the whole body, rather than lowering the text: a verbatim
    /// body is CHARACTERS, and running it through the lowerer is exactly what
    /// `\begin{verbatim}` exists to prevent.
    fn push_verbatim(&mut self, body: &str, out: &mut Vec<Cmd>) {
        let body = body.strip_prefix('\n').unwrap_or(body);
        self.push_text(out, "\n\n");
        self.push_text(out, &format!("{}m", crate::typeset::FACE_PUSH));
        for line in body.lines() {
            out.push(Cmd::Text(line.to_string()));
            self.push_text(out, &crate::typeset::LISTING_BREAK.to_string());
        }
        self.push_text(out, &crate::typeset::FACE_POP.to_string());
        self.push_text(out, "\n\n");
    }

    /// Append text to the run in progress, looking past line directives.
    ///
    /// Text that opens with a paragraph break ends the paragraph before it, and
    /// that paragraph is broken with the breaker's parameters in force here
    /// (§816), so they go in front of the break when they are not plain.tex's:
    /// see `typeset::BREAK_PARAMS`.
    fn push_text(&self, out: &mut Vec<Cmd>, text: &str) {
        let marked;
        let text = match text.starts_with("\n\n") {
            true => match self.paragraph_end_mark() {
                Some(mark) => {
                    marked = format!("{mark}{text}");
                    marked.as_str()
                }
                None => text,
            },
            false => text,
        };
        let mut at = out.len();
        while at > 0 && matches!(out[at - 1], Cmd::Line(_)) {
            at -= 1;
        }
        match at.checked_sub(1).and_then(|i| out.get_mut(i)) {
            Some(Cmd::Text(t)) => t.push_str(text),
            _ => out.push(Cmd::Text(text.to_string())),
        }
    }

    /// The marker carrying the breaker's parameters as they stand, for a
    /// paragraph ending here -- `None` when they are plain.tex's, which is what
    /// a paragraph without one is broken with, or when no text is kept.
    fn paragraph_end_mark(&self) -> Option<String> {
        let params = crate::linebreak::Params::from_intpars(&self.eng.intpars);
        (self.text_output && params != crate::linebreak::Params::default())
            .then(|| crate::typeset::break_params_span(&params))
    }

    /// The environment name after `\begin`, without consuming it.
    ///
    /// Read as raw characters rather than tokens: the name is needed BEFORE
    /// deciding whether the body is TeX at all, so tokenising it would be
    /// deciding the question by answering it.
    /// Read a `tikzpicture` body raw and emit it as a picture marker.
    ///
    /// The body is NOT expanded, for the reason a verbatim body is not: it is
    /// a different language. Reading it as TeX turns `\draw` into a macro call,
    /// `(0,0)` into three characters of prose and `;` into the delimiter of an
    /// argument that runs to it -- which is precisely what the prelude's
    /// `\def\draw#1;{}` did, and why every picture in every document reached
    /// the page as nothing at all.
    ///
    /// What comes out is one `typeset::PICTURE` span holding the option list
    /// and the body, bracketed by paragraph breaks so the picture owns its own
    /// lines. `typeset::to_pdf` parses it and draws it; `--text` drops it,
    /// because a picture has no words.
    fn picture_environment(&mut self, lx: &mut Lexer, env: &str, out: &mut Vec<Cmd>) -> R<()> {
        // Consume the `{name}` that was only peeked at, or it lands in the
        // output ahead of the picture.
        while let Some(t) = lx.next_token(&self.eng.cats) {
            if matches!(t, Token::Char(_, Cat::EndGroup)) {
                break;
            }
        }
        let end = format!("\\end{{{env}}}");
        let Some(body) = lx.read_raw_until(&end) else {
            return Err(TexError(format!(
                "Runaway argument: \\begin{{{env}}} never ends"
            )));
        };
        if self.text_output {
            // Measuring the picture is parsing it: left in a slot, done on
            // the pool and filled in document order with the formulas.
            let (options, body) = environment_option(&body);
            let slot = self.formulas.picture(options, body);
            self.push_text(out, &crate::typeset::picture_paragraph(&slot));
        }
        Ok(())
    }

    fn peek_environment_name(&self, lx: &Lexer) -> Option<String> {
        let chars = lx.chars();
        let mut i = lx.pos();
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= chars.len() || chars[i] != '{' {
            return None;
        }
        i += 1;
        let start = i;
        while i < chars.len() && chars[i] != '}' {
            i += 1;
        }
        match i < chars.len() {
            true => Some(chars[start..i].iter().collect()),
            false => None,
        }
    }

    /// Recognise `\def\r{BODY \ifnum A<B \r \fi}` -- TeX's loop.
    ///
    /// Returns the body tokens and the condition tokens when `name` has exactly
    /// that shape, and `None` otherwise. Deliberately narrow: the tail call must
    /// be the last thing before the closing `\fi`, the macro must take no
    /// arguments, and it must not name itself anywhere else. Anything less
    /// certain is left to inlining, where the depth bound still catches it --
    /// a recogniser that guesses would silently compile a DIFFERENT program.
    /// Whether a redefined control sequence's macro meaning should win over the
    /// primitive of the same name.
    ///
    /// It normally should: a document means what it last defined. `\end` is the
    /// one that cannot be decided by name alone, because it is two things at
    /// once. The LaTeX prelude defines `\end#1` so `\end{itemize}` runs
    /// `\enditemize`, and TeX's own `\end` is how EVERY document stops --
    /// including every LaTeX one, which stops at `\end` with nothing after it
    /// once `\end{document}` has been read. Letting the macro win outright made
    /// a bare `\end` scan for an argument that is not there and die with
    /// "Paragraph ended before argument was complete".
    ///
    /// So the group decides, which is what the two spellings already differ by:
    /// a `{` following means the environment closer, anything else means the
    /// terminator.
    fn meaning_wins(&mut self, lx: &mut Lexer, name: CsId) -> bool {
        if name.name() != "end" {
            return true;
        }
        let Some(next) = lx.next_token(&self.eng.cats) else {
            return false;
        };
        let is_group = matches!(next, Token::Char(_, Cat::BeginGroup));
        lx.push_back(&[next]);
        is_group
    }

    fn tail_loop(&self, name: CsId) -> Option<TailLoop> {
        let Some(Meaning::Macro(m)) = self.eng.meanings.get(&name) else {
            return None;
        };
        if !m.params.is_empty() {
            return None;
        }
        if let Some(parts) = self.plain_iterate(name, &m.body) {
            return Some(parts);
        }
        let is_self = |t: &Token| matches!(t, Token::Cs(n) if *n == name);
        // The tail call sits at the end, before an optional space and `\fi`.
        let mut end = m.body.len();
        while end > 0 && m.body[end - 1].is_space() {
            end -= 1;
        }
        if end == 0 || !matches!(&m.body[end - 1], Token::Cs(n) if n.name() == "fi") {
            return None;
        }
        end -= 1;
        while end > 0 && m.body[end - 1].is_space() {
            end -= 1;
        }
        if end == 0 || !is_self(&m.body[end - 1]) {
            return None;
        }
        end -= 1;
        // `\ifnum ... \expandafter\loop\fi` is the same loop: the
        // `\expandafter` only closes the conditional (§510) before the call
        // instead of after it returns, which is what keeps tex's input stack
        // from growing, and changes nothing about which passes run.
        if end > 0 && matches!(&m.body[end - 1], Token::Cs(n) if n.name() == "expandafter") {
            end -= 1;
        }
        // Everything before it splits at the `\ifnum` that guards the call.
        let guard = m.body[..end]
            .iter()
            .rposition(|t| matches!(t, Token::Cs(n) if n.name() == "ifnum"))?;
        let body = m.body[..guard].to_vec();
        let cond = m.body[guard + 1..end].to_vec();
        // One self-reference only: another one elsewhere is a different program
        // than a loop, and inlining it is the honest answer.
        if body.iter().chain(cond.iter()).any(is_self) {
            return None;
        }
        // The guard has to be THE conditional the final `\fi` closes, and an
        // ordinary one: a conditional anywhere else in the body or the test
        // (`\ifnum\count1=3 \inner\fi` inside latex.ltx's `\loop`) makes the
        // last `\ifnum` some other one, and an `\unless` in front of the guard
        // inverts it. Either way the shape is not this loop, and expanding
        // the macro is what tex does.
        let conditional = |t: &Token| {
            matches!(t, Token::Cs(n) if Engine::is_conditional(*n)
                || matches!(n.name(), "fi" | "else" | "or" | "unless"))
        };
        if body.iter().chain(cond.iter()).any(conditional) {
            return None;
        }
        Some(TailLoop {
            body,
            cond,
            leaves_relax: None,
        })
    }

    /// Recognise plain.tex's `\iterate` (plain.tex's `\loop...\repeat`):
    ///
    /// ```text
    /// \def\loop#1\repeat{\def\body{#1}\iterate}
    /// \def\iterate{\body \let\next\iterate \else\let\next\relax\fi \next}
    /// ```
    ///
    /// `\body` ends with the OPEN `\ifnum` that `\iterate` closes, so the loop is
    /// `\body`'s text up to that `\ifnum`, then the test, again while it holds:
    /// the do-while `Cmd::Loop` is. Lowered by inlining instead, both `\let`s
    /// ran while lowering and the else arm's won, so every loop ran once.
    ///
    /// The same narrowness as `tail_loop`: exactly this shape, a guard that is
    /// an `\ifnum` with nothing conditional after it, and a body that names
    /// neither macro. The loop leaves `\next` meaning `\relax`, as tex does.
    fn plain_iterate(&self, name: CsId, body: &[Token]) -> Option<TailLoop> {
        let toks: Vec<&Token> = body.iter().filter(|t| !t.is_space()).collect();
        let cs = |i: usize| match toks.get(i) {
            Some(Token::Cs(n)) => Some(*n),
            _ => None,
        };
        if toks.len() != 10 {
            return None;
        }
        let (inner, next) = (cs(0)?, cs(2)?);
        let shape = [(1, "let"), (4, "else"), (5, "let"), (7, "relax"), (8, "fi")];
        if shape
            .iter()
            .any(|(i, w)| cs(*i).map(|n| n.name()) != Some(*w))
            || cs(3)? != name
            || cs(6)? != next
            || cs(9)? != next
        {
            return None;
        }
        let Some(Meaning::Macro(b)) = self.eng.meanings.get(&inner) else {
            return None;
        };
        if !b.params.is_empty() {
            return None;
        }
        let guard = b
            .body
            .iter()
            .rposition(|t| matches!(t, Token::Cs(n) if n.name() == "ifnum"))?;
        let cond = b.body[guard + 1..].to_vec();
        let loop_body = b.body[..guard].to_vec();
        let names_one = |t: &Token| {
            matches!(t, Token::Cs(n) if *n == name || *n == inner || *n == next
                || Engine::is_conditional(*n))
        };
        if cond.iter().any(names_one)
            || loop_body
                .iter()
                .any(|t| matches!(t, Token::Cs(n) if *n == name || *n == inner))
        {
            return None;
        }
        Some(TailLoop {
            body: loop_body,
            cond,
            leaves_relax: Some(next),
        })
    }

    /// Lower a recognised tail loop: the body as a block, the guard as a test.
    fn lower_tail_loop(&mut self, parts: TailLoop) -> R<Vec<Cmd>> {
        let mut body_lx = Lexer::new("");
        body_lx.push_back(&parts.body);
        let body = self.block(&mut body_lx, None)?;

        let mut cond_lx = Lexer::new("");
        cond_lx.push_back(&parts.cond);
        let left = self.number(&mut cond_lx)?;
        let rel = match self.eng.read_relation_file(&mut cond_lx, "ifnum")? {
            '<' => Rel::Less,
            '>' => Rel::Greater,
            _ => Rel::Equal,
        };
        let right = self.number(&mut cond_lx)?;
        // What follows the test inside the true arm, before the tail call:
        // `\loop A \ifnum X<Y B \repeat` runs A, then B and A again while
        // the test holds. Dropping B compiled a different program.
        let mut rest = Vec::new();
        while let Some(t) = cond_lx.next_token(&self.eng.cats) {
            rest.push(t);
        }
        let cmd = match rest.iter().all(Token::is_space) {
            true => vec![Cmd::Loop {
                body,
                left,
                rel,
                right,
            }],
            // A; if X<Y { do { B; A } while X<Y }. A is lowered a second time
            // for the loop's own copy of it.
            false => {
                let mut again_lx = Lexer::new("");
                again_lx.push_back(&rest);
                let mut again = self.block(&mut again_lx, None)?;
                let mut body_lx = Lexer::new("");
                body_lx.push_back(&parts.body);
                again.extend(self.block(&mut body_lx, None)?);
                let mut out = body;
                out.push(Cmd::IfNum {
                    left: left.clone(),
                    rel,
                    right: right.clone(),
                    then_branch: vec![Cmd::Loop {
                        body: again,
                        left,
                        rel,
                        right,
                    }],
                    else_branch: Vec::new(),
                });
                out
            }
        };
        if let Some(next) = parts.leaves_relax {
            self.eng.let_relax(next);
        }
        Ok(cmd)
    }

    /// The arms of a conditional the run decides. What was reported while
    /// lowering an arm is printed at its end, in the arm, so only a run that
    /// takes the arm prints it, as only a tex that takes it scans it. Done in
    /// line rather than in a helper: this recursion is what a mutually
    /// recursive macro pair deepens, and the debug build's stack is what
    /// `tests/recursive_macro.rs` holds it to.
    #[inline(never)]
    fn arms(&mut self, lx: &mut Lexer) -> R<(Vec<Cmd>, Vec<Cmd>)> {
        let negated = self.eng.take_unless();
        // Each arm is read with its conditional open, as tex runs it
        // (`\currentiflevel` and its siblings); `next_arms` says which one.
        let opened_by = self.next_arms.take();
        let enter = |eng: &mut Engine, branch: i64| {
            if let Some(o) = opened_by {
                eng.enter_runtime_arm(o, branch);
            }
        };
        let leave = |eng: &mut Engine| {
            if opened_by.is_some() {
                eng.leave_runtime_arm();
            }
        };
        // Each arm starts from what was known before the conditional; which one
        // runs is the run's, so what they assign is unknown afterwards.
        let known_mark = self.eng.known.mark();
        enter(&mut self.eng, 1);
        let then = self.block(lx, Some(&["else", "fi"]));
        leave(&mut self.eng);
        let mut then_branch = then?;
        self.flush_reports(&mut then_branch);
        let mut else_branch = Vec::new();
        match lx.next_token(&self.eng.cats) {
            Some(Token::Cs(n)) if n.name() == "else" => {
                self.eng.known.undo_to(known_mark, None);
                enter(&mut self.eng, -1);
                let text = self.block(lx, Some(&["fi"]));
                leave(&mut self.eng);
                else_branch = text?;
                self.flush_reports(&mut else_branch);
                // Consume the `\fi`.
                let _ = lx.next_token(&self.eng.cats);
            }
            Some(Token::Cs(n)) if n.name() == "fi" => {}
            other => {
                if let Some(t) = other {
                    lx.push_back(&[t]);
                }
                if self.eng.halted {
                    return Ok((then_branch, else_branch));
                }
                return Err(TexError("Incomplete \\ifnum; missing \\fi".into()));
            }
        }
        self.eng.known.undo_to(known_mark, None);
        // A negated conditional is this one with its arms exchanged.
        match negated {
            true => Ok((else_branch, then_branch)),
            false => Ok((then_branch, else_branch)),
        }
    }

    /// The `\or`-separated cases of a file-level `\ifcase`, the last being
    /// `\else`'s (empty when there is none).
    fn case_arms(&mut self, lx: &mut Lexer) -> R<Vec<Vec<Cmd>>> {
        let known_mark = self.eng.known.mark();
        let mut arms = Vec::new();
        loop {
            let mut arm = self.block(lx, Some(&["or", "else", "fi"]))?;
            self.flush_reports(&mut arm);
            arms.push(arm);
            self.eng.known.undo_to(known_mark, None);
            match lx.next_token(&self.eng.cats) {
                Some(Token::Cs(n)) if n.name() == "or" => continue,
                Some(Token::Cs(n)) if n.name() == "else" => {
                    let mut arm = self.block(lx, Some(&["fi"]))?;
                    self.flush_reports(&mut arm);
                    arms.push(arm);
                    self.eng.known.undo_to(known_mark, None);
                    let _ = lx.next_token(&self.eng.cats);
                    return Ok(arms);
                }
                Some(Token::Cs(n)) if n.name() == "fi" => {
                    arms.push(Vec::new());
                    return Ok(arms);
                }
                other => {
                    if let Some(t) = other {
                        lx.push_back(&[t]);
                    }
                    if self.eng.halted {
                        arms.push(Vec::new());
                        return Ok(arms);
                    }
                    return Err(TexError("Incomplete \\ifcase; missing \\fi".into()));
                }
            }
        }
    }

    /// The base64 body of a `\rustcompile <base64>\endrust`.
    ///
    /// Every character up to the terminating control sequence, spaces skipped:
    /// base64's alphabet is letters, digits, `+`, `/` and `=`, none of which the
    /// mouth can turn into anything but a character token whatever the catcodes
    /// are.
    fn rust_blob(&mut self, lx: &mut Lexer) -> R<String> {
        let mut b64 = String::new();
        loop {
            let Some(t) = lx.next_token(&self.eng.cats) else {
                return Err(TexError("Runaway \\rustcompile: missing \\endrust".into()));
            };
            match &t {
                _ if t.is_space() => continue,
                Token::Char(c, _) => b64.push(*c),
                Token::Cs(n) if n.name() == crate::rust_ffi::END_CS => break,
                Token::Cs(n) => {
                    return Err(TexError(format!(
                        "Unexpected \\{} inside a \\rust block body",
                        n.name()
                    )))
                }
            }
        }
        Ok(b64)
    }

    /// One token, from the file or from the pending list.
    ///
    /// The lowerer reads from two places -- running text (`\rustcall` in a
    /// statement) and an already-expanded token run (`\rustcall` inside a
    /// `\message` body) -- and the FFI call parser is the same either way, so
    /// the difference is a flag rather than two copies of the parser.
    fn take_token(&mut self, lx: &mut Lexer, pending: bool) -> Option<Token> {
        match pending {
            true => lx.pending.pop(),
            false => lx.next_token(&self.eng.cats),
        }
    }

    /// `\rustcall <name> <numbers…>\endrust`.
    ///
    /// The name is characters up to the first space, and the arguments are
    /// numbers up to `\endrust`. Both ends are catcode-independent: a control
    /// sequence terminates the list, and the only category the form depends on
    /// is the escape character.
    fn rust_call(&mut self, lx: &mut Lexer, pending: bool) -> R<Num> {
        let mut name = String::new();
        loop {
            let Some(t) = self.take_token(lx, pending) else {
                return Err(TexError("Runaway \\rustcall: no name".into()));
            };
            match &t {
                _ if t.is_space() && name.is_empty() => continue,
                _ if t.is_space() => break,
                Token::Char(c, _) => name.push(*c),
                // A control sequence ends the name; it is the argument list, or
                // the terminator for a call that takes none.
                Token::Cs(_) => {
                    lx.push_back(&[t]);
                    break;
                }
            }
        }
        if name.is_empty() {
            return Err(TexError("Missing function name after \\rustcall".into()));
        }

        let mut args = Vec::new();
        loop {
            let Some(t) = self.take_token(lx, pending) else {
                return Err(TexError(format!(
                    "Runaway \\rustcall {name}: missing \\endrust"
                )));
            };
            if t.is_space() {
                continue;
            }
            if let Token::Cs(n) = &t {
                if n.name() == crate::rust_ffi::END_CS {
                    break;
                }
            }
            lx.push_back(&[t]);
            args.push(match pending {
                true => self.msg_number(lx)?,
                false => self.number(lx)?,
            });
        }
        Ok(Num::Rust { name, args })
    }

    /// The slot of an INTERNAL QUANTITY standing where an integer is wanted, if
    /// that is what comes next; nothing is consumed otherwise.
    ///
    /// `tex.web` §430 is the coercion: `scan_int` accepts an internal quantity
    /// of any of the four levels and takes its value as a number, so a
    /// dimension reads as its count of scaled points and a glue as the scaled
    /// points of the natural width at the front of it. `\count` needs no
    /// coercion and is here because the four are read the same way.
    ///
    /// Two lines of the standard classes turn on it. size10.clo:135 is the
    /// coercion spelt out -- `\@tempcnta=\@tempdima`, a count register assigned
    /// from a dimension one -- and latex.ltx:10261 is the other half:
    ///
    /// ```text
    /// \def\@settopoint#1{\divide#1\p@\multiply#1\p@}
    /// ```
    ///
    /// where `\p@` is a `\dimendef` name in INTEGER position, so the divisor is
    /// 65536 and the dimension comes back truncated to whole points.
    ///
    /// What comes back is the SLOT and not the value: a register's value
    /// belongs to the run, and only the slot number is known while lowering.
    fn peek_int_register(&mut self, lx: &mut Lexer, pending: bool) -> R<Option<Num>> {
        let mut eaten = Vec::new();
        // §440 reads the signs before an internal integer and negates it for
        // an odd number of minuses: `\count2=-\count1`, `\number-\count1`.
        let mut negative = false;
        loop {
            let Some(t) = self.eng.take_any(lx, pending) else {
                lx.push_back(&eaten);
                return Ok(None);
            };
            if t.is_space() {
                eaten.push(t);
                continue;
            }
            if matches!(t, Token::Char('+' | '-', Cat::Other)) {
                negative ^= matches!(t, Token::Char('-', _));
                eaten.push(t);
                continue;
            }
            // e-TeX's expressions are internal integers too (`last_item`):
            // `\dimexpr` where an integer is wanted is its scaled points.
            if let Token::Cs(n) = &t {
                if let Some(part) = self.glue_part(lx, *n)? {
                    return Ok(Some(match negative {
                        true => step(
                            crate::etex_arith::Op::Sub,
                            crate::etex_arith::INFINITY,
                            vec![Num::Literal(0), part],
                        ),
                        false => part,
                    }));
                }
                if let Some(level) = expr_level_of(self.eng.primitive_meaning(*n).name()) {
                    let e = self.expr(lx, pending, level)?;
                    return Ok(Some(match negative {
                        true => step(
                            crate::etex_arith::Op::Sub,
                            crate::etex_arith::INFINITY,
                            vec![Num::Literal(0), e],
                        ),
                        false => e,
                    }));
                }
            }
            if let Token::Cs(n) = &t {
                if let Some((slot, _)) = self.eng.register_slot(lx, *n, pending)? {
                    return Ok(Some(signed(negative, slot)));
                }
                // `\the` or `\number` of a register is its digits (§465,
                // §470), which `scan_int` reads as its value -- unless a digit
                // follows and §445 goes on reading it into the same number,
                // which no slot holds. Only an integer's digits are its value:
                // `\the\dimen0` writes `1.0pt`, of which the scan keeps `1`.
                if matches!(n.name(), "the" | "number") {
                    lx.push_back(&[t]);
                    if let Some(reg) = self.eng.take_expanded_register(lx, pending)? {
                        let integer = !reg.the || crate::compiler::is_count_slot(reg.slot);
                        if integer && !self.eng.number_continues(lx, pending, false)? {
                            return Ok(Some(signed(negative, reg.slot)));
                        }
                        self.eng.put_back(lx, reg);
                    }
                    lx.push_back(&eaten);
                    return Ok(None);
                }
            }
            // Not a register: everything read goes back in the order it was
            // read, so the scanner that follows sees an untouched stream.
            eaten.push(t);
            lx.push_back(&eaten);
            return Ok(None);
        }
    }

    /// A number operand: a literal, a register read at run time, or a call into
    /// a compiled `\rust{ … }` block.
    /// Whether the next token names a §236 integer parameter, leaving it where
    /// it was.
    fn intpar_follows(&mut self, lx: &mut Lexer) -> bool {
        let Some(t) = lx.next_token(&self.eng.cats) else {
            return false;
        };
        let hit = matches!(t, Token::Cs(cs) if self.eng.intpar_cs(cs).is_some());
        lx.push_back(&[t]);
        hit
    }

    /// Whether the next token is `\attribute` or a `\attributedef` name,
    /// leaving it where it was.
    fn attribute_follows(&mut self, lx: &mut Lexer) -> bool {
        let Some(t) = lx.next_token(&self.eng.cats) else {
            return false;
        };
        let hit = matches!(t, Token::Cs(cs)
            if self.eng.primitive_meaning(cs).name() == "attribute"
                || self.eng.attribute_of(cs).is_some());
        lx.push_back(&[t]);
        hit
    }

    fn number(&mut self, lx: &mut Lexer) -> R<Num> {
        // A register becomes a slot read rather than a constant, because its
        // value is the run's and not the lowerer's.
        if let Some(slot) = self.peek_int_register(lx, false)? {
            return Ok(slot);
        }
        loop {
            let Some(t) = lx.next_token(&self.eng.cats) else {
                return Err(TexError("Missing number, treated as zero".into()));
            };
            if t.is_space() {
                continue;
            }
            match &t {
                Token::Cs(n) if n.name() == crate::rust_ffi::CALL_CS => {
                    return self.rust_call(lx, false);
                }
                _ => {
                    lx.push_back(&[t]);
                    return Ok(Num::Literal(self.eng.scan_number_file(lx)?));
                }
            }
        }
    }

    /// The right-hand side of a glue-register assignment, written into the four
    /// slots at `base`.
    ///
    /// `\skip1=\skip0` copies a register whose value is only known at run time,
    /// so the four writes are slot READS rather than constants; anything else
    /// is a glue the scanner reads now (`tex.web` §461's `scan_glue`).
    /// e-TeX's `\glueexpr` (or `\muexpr`, `mu`), read after the primitive:
    /// luatex's `scan_expr` at `glue_val_level`, as the run evaluates it. A
    /// glue is its four slots' worth of `Num`s -- width, stretch, shrink and
    /// the packed orders -- and each step answers one of the four
    /// (`etex_arith::Op::GlueAdd` and its siblings), folded now when every
    /// operand is known. An overflow is reported once, by the width.
    fn glue_expr(&mut self, lx: &mut Lexer, mu: bool) -> R<[Num; 4]> {
        use crate::etex_arith::Op;
        let g = self.glue_expr_level(lx, mu, false)?;
        let report = self.eng.report_text(lx, "Arithmetic overflow");
        let [w, st, sh, o] = g;
        let end = |n: Num| step(Op::Settle, 0, vec![n]);
        let width = match w {
            Num::Literal(crate::etex_arith::ERROR) => {
                self.eng.report(lx, "Arithmetic overflow");
                Num::Literal(0)
            }
            Num::Literal(v) => Num::Literal(v),
            w => Num::ExprEnd {
                value: Box::new(w),
                report,
            },
        };
        Ok([width, end(st), end(sh), end(o)])
    }

    /// One glue step: `op` on the operands, each component its own `Num`.
    /// e-TeX's `\gluestretch`, `\glueshrink` and the four order primitives
    /// (`last_item` commands over a glue), read after `name`: the component as
    /// a `Num`, or `None` when `name` is none of them. The orders are LuaTeX's
    /// numbering, `fi` counted, for `\gluestretchorder` and `\glueshrinkorder`,
    /// and e-TeX's for the `\eTeX` spellings (measured).
    fn glue_part(&mut self, lx: &mut Lexer, name: CsId) -> R<Option<Num>> {
        use crate::etex_arith::Op;
        let which = match self.eng.primitive_meaning(name).name() {
            "gluestretch" => 1,
            "glueshrink" => 2,
            "eTeXgluestretchorder" => 10,
            "eTeXglueshrinkorder" => 11,
            "gluestretchorder" => 12,
            "glueshrinkorder" => 13,
            _ => return Ok(None),
        };
        let g = self.glue_factor(lx, false)?;
        let [_, stretch, shrink, orders] = g;
        Ok(Some(match which {
            1 => stretch,
            2 => shrink,
            n => step(Op::GlueOrder, n - 10, vec![orders]),
        }))
    }

    fn glue_step(op: crate::etex_arith::Op, operands: &[Num]) -> [Num; 4] {
        std::array::from_fn(|k| step(op, k as i64, operands.to_vec()))
    }

    /// A glue factor: a glue register (negated by its signs), or a glue
    /// written out, whose components may be §453's products of registers.
    fn glue_factor(&mut self, lx: &mut Lexer, mu: bool) -> R<[Num; 4]> {
        // e-TeX's `\gluetomu` and `\mutoglue`: the same four numbers, taken
        // from the other kind of glue.
        let convert = if mu { "gluetomu" } else { "mutoglue" };
        match self.eng.next_x_nonblank(lx, false)? {
            Some(Token::Cs(n)) if self.eng.primitive_meaning(n).name() == convert => {
                return self.glue_factor(lx, !mu);
            }
            Some(t) => lx.back_input(&[t]),
            None => {}
        }
        if let Some((from, negative)) = self.peek_glue_register(lx, mu)? {
            return Ok(std::array::from_fn(|i| {
                signed(negative && i < 3, from + i as i64)
            }));
        }
        let g = match mu {
            true => self.eng.scan_muglue_parts(lx)?,
            false => self.eng.scan_glue_parts(lx)?,
        };
        Ok([
            scaled_num(g.natural),
            scaled_num(g.stretch),
            scaled_num(g.shrink),
            Num::Literal(g.stretch_order * 4 + g.shrink_order),
        ])
    }

    /// One level of the glue `scan_expr`, up to the end or, `nested`, its
    /// `)`; the same machine as [`Self::expr_level`] with glue terms.
    fn glue_expr_level(&mut self, lx: &mut Lexer, mu: bool, nested: bool) -> R<[Num; 4]> {
        use crate::etex_arith::{Op, INFINITY};
        let zero = || {
            [
                Num::Literal(0),
                Num::Literal(0),
                Num::Literal(0),
                Num::Literal(0),
            ]
        };
        let (mut r, mut e) = (None::<Op>, zero());
        let (mut s, mut t, mut n) = (None::<Op>, zero(), Num::Literal(0));
        loop {
            let opened = match self.eng.next_x_nonblank(lx, false)? {
                Some(Token::Char('(', Cat::Other)) => true,
                other => {
                    if let Some(tok) = other {
                        lx.back_input(&[tok]);
                    }
                    false
                }
            };
            // A glue opens a term; a factor after `*` or `/` is an integer.
            let (glue, int) = match (s.is_some(), opened) {
                (false, true) => (Some(self.glue_expr_level(lx, mu, true)?), None),
                (false, false) => (Some(self.glue_factor(lx, mu)?), None),
                (true, true) => (
                    None,
                    Some(self.expr_level(lx, false, ExprLevel::Int, true)?),
                ),
                (true, false) => (None, Some(self.number(lx)?)),
            };
            let next = self.eng.next_x_nonblank(lx, false)?;
            let mut o = match &next {
                Some(Token::Char('+', Cat::Other)) => Some(Op::Add),
                Some(Token::Char('-', Cat::Other)) => Some(Op::Sub),
                Some(Token::Char('*', Cat::Other)) => Some(Op::Mul),
                Some(Token::Char('/', Cat::Other)) => Some(Op::Div),
                _ => None,
            };
            if o.is_none() {
                match next {
                    Some(Token::Char(')', Cat::Other)) if nested => {}
                    Some(tok) if nested => {
                        lx.back_input(&[tok]);
                        self.eng.report(lx, "Missing ) inserted for expression");
                    }
                    Some(tok) if !self.eng.means_relax(&tok) => lx.back_input(&[tok]),
                    _ => {}
                }
            }
            match (s, glue, int) {
                (None, Some(g), _) => {
                    t = Self::glue_step(Op::GlueCheck, &g);
                    // `normalize_glue` on a first factor an operator follows:
                    // adding a zero glue is that and nothing more.
                    if o.is_some() {
                        let mut args = t.to_vec();
                        args.extend(zero());
                        t = Self::glue_step(Op::GlueAdd, &args);
                    }
                }
                (Some(op), _, Some(f)) => {
                    let f = step(Op::Check, INFINITY, vec![f]);
                    match op {
                        Op::Mul if o == Some(Op::Div) => {
                            n = f;
                            o = Some(Op::Scale);
                        }
                        Op::Scale => {
                            let mut args = t.to_vec();
                            args.push(std::mem::replace(&mut n, Num::Literal(0)));
                            args.push(f);
                            t = Self::glue_step(Op::GlueScale, &args);
                        }
                        op => {
                            let glue_op = if op == Op::Mul {
                                Op::GlueMul
                            } else {
                                Op::GlueDiv
                            };
                            let mut args = t.to_vec();
                            args.push(f);
                            t = Self::glue_step(glue_op, &args);
                        }
                    }
                }
                _ => unreachable!("a glue opens a term and an integer follows * or /"),
            }
            match o {
                Some(Op::Mul | Op::Div | Op::Scale) => s = o,
                _ => {
                    s = None;
                    e = match r {
                        None => t.clone(),
                        Some(add_or_sub) => {
                            let glue_op = if add_or_sub == Op::Sub {
                                Op::GlueSub
                            } else {
                                Op::GlueAdd
                            };
                            let mut args = e.to_vec();
                            args.extend(t.iter().cloned());
                            Self::glue_step(glue_op, &args)
                        }
                    };
                    r = o;
                }
            }
            if o.is_none() {
                return Ok(e);
            }
        }
    }

    fn glue_assign(&mut self, lx: &mut Lexer, base: i64) -> R<Vec<Cmd>> {
        // Which glue file the destination is in decides the units the source is
        // read in: `tex.web` §1228 assigns a `mu_val` from a `mu_val` and a
        // `glue_val` from a `glue_val`, and never mixes them.
        let mu = crate::compiler::is_mu_slot(base);
        // `\glueexpr` stands where a glue does and `\muexpr` where a math glue
        // does; the expression is the run's when its operands are registers.
        let want = if mu { "muexpr" } else { "glueexpr" };
        let convert = if mu { "gluetomu" } else { "mutoglue" };
        match self.eng.next_x_nonblank(lx, false)? {
            Some(Token::Cs(n)) if matches!(self.eng.primitive_meaning(n).name(), m if m == want || m == convert) =>
            {
                let g = match self.eng.primitive_meaning(n).name() == want {
                    true => self.glue_expr(lx, mu)?,
                    false => self.glue_factor(lx, !mu)?,
                };
                return Ok(g
                    .into_iter()
                    .enumerate()
                    .map(|(i, part)| Cmd::SetCount(base + i as i64, part))
                    .collect());
            }
            Some(t) => lx.back_input(&[t]),
            None => {}
        }
        if let Some((from, negative)) = self.peek_glue_register(lx, mu)? {
            // Negating a glue negates its three widths and keeps its orders.
            return Ok((0..crate::compiler::SKIP_STRIDE)
                .map(|i| Cmd::SetCount(base + i, signed(negative && i < 3, from + i)))
                .collect());
        }
        self.eng.clear_scaling_sites();
        let g = match mu {
            true => self.eng.scan_muglue_parts(lx)?,
            false => self.eng.scan_glue_parts(lx)?,
        };
        // Any of the three components can be `tex.web` §453's `<factor><internal
        // unit>` -- size10.clo's `\abovedisplayskip 10\p@ \@plus2\p@ \@minus5\p@`
        // is all three at once -- so each slot takes a `Num` rather than a
        // number. The fourth slot packs the two ORDERS, which are always known.
        let parts = [
            scaled_num(g.natural),
            scaled_num(g.stretch),
            scaled_num(g.shrink),
            Num::Literal(g.stretch_order * 4 + g.shrink_order),
        ];
        let mut cmds = Vec::new();
        for (i, part) in parts.into_iter().enumerate() {
            self.note_scaling_site(&mut cmds, lx, &part);
            cmds.push(Cmd::SetCount(base + i as i64, part));
        }
        Ok(cmds)
    }

    /// The base slot of a glue register standing where a glue is wanted, if
    /// that is what comes next, and whether §461's signs before it negate it;
    /// nothing is consumed otherwise.
    fn peek_glue_register(&mut self, lx: &mut Lexer, mu: bool) -> R<Option<(i64, bool)>> {
        let (want, file) = match mu {
            true => ("muskip", crate::compiler::RegFile::MuSkip),
            false => ("skip", crate::compiler::RegFile::Skip),
        };
        let mut eaten = Vec::new();
        let mut negative = false;
        loop {
            let Some(t) = self.eng.take_file(lx) else {
                lx.push_back(&eaten);
                return Ok(None);
            };
            if t.is_space() {
                eaten.push(t);
                continue;
            }
            if matches!(t, Token::Char('+' | '-', Cat::Other)) {
                negative ^= matches!(t, Token::Char('-', _));
                eaten.push(t);
                continue;
            }
            if let Token::Cs(n) = &t {
                if n.name() == want {
                    let slot = self.eng.scan_register_slot(lx, false, file)?;
                    return Ok(Some((slot, negative)));
                }
                if let Some(crate::expand::NumericCs::Register(r)) = self.eng.numeric_cs(*n) {
                    // A `\skipdef` name is a glue register and a `\muskipdef`
                    // name is a math one; each stands only where its own kind
                    // does.
                    let is_mu = crate::compiler::is_mu_slot(r);
                    if crate::compiler::is_glue_slot(r) && is_mu == mu {
                        return Ok(Some((r, negative)));
                    }
                }
                // `\the` of a glue register writes it as §178's `print_spec`
                // does, which §461 reads back as the same glue -- unless what
                // follows goes on with a keyword the written form left open.
                if n.name() == "the" {
                    lx.push_back(&[t]);
                    if let Some(reg) = self.eng.take_expanded_register(lx, false)? {
                        let same_kind = crate::compiler::is_glue_slot(reg.slot)
                            && crate::compiler::is_mu_slot(reg.slot) == mu;
                        if reg.the && same_kind && !self.eng.glue_continues(lx, false)? {
                            return Ok(Some((reg.slot, negative)));
                        }
                        self.eng.put_back(lx, reg);
                    }
                    lx.push_back(&eaten);
                    return Ok(None);
                }
            }
            // Not a glue register: everything read goes back in the order it
            // was read, so the scanner that follows sees an untouched stream.
            eaten.push(t);
            lx.push_back(&eaten);
            return Ok(None);
        }
    }

    /// A DIMENSION operand as the VM will see it: a register read, or a
    /// constant in scaled points.
    ///
    /// A dimension is an integer count of scaled points living in a slot, so
    /// the same `Num` a count register uses carries it and only the scanner
    /// differs — `tex.web` §448's `scan_dimen` rather than §440's `scan_int`.
    /// That is what lets `\ifdim` lower to the same real branch `\ifnum` does.
    ///
    /// Written once for both reading contexts: `\ifdim` occurs in running text
    /// and inside a `\message` body, and the two differ only in where the
    /// tokens come from.
    fn dimen_number(&mut self, lx: &mut Lexer, pending: bool) -> R<Num> {
        loop {
            let Some(t) = self.eng.take_any(lx, pending) else {
                return Err(TexError("Missing number, treated as zero".into()));
            };
            if t.is_space() {
                continue;
            }
            let Token::Cs(n) = &t else {
                lx.push_back(&[t]);
                return Ok(scaled_num(self.eng.scan_dimen_parts(lx, pending)?));
            };
            let n = *n;
            if self.eng.primitive_meaning(n).name() == "dimexpr" {
                return self.expr(lx, pending, ExprLevel::Dimen);
            }
            if matches!(
                self.eng.primitive_meaning(n).name(),
                "gluestretch" | "glueshrink"
            ) {
                return Ok(self.glue_part(lx, n)?.expect("a glue part"));
            }
            match n.name() {
                "dimen" => {
                    let slot =
                        self.eng
                            .scan_register_slot(lx, pending, crate::compiler::RegFile::Dimen)?;
                    return Ok(Num::Count(slot));
                }
                // A glue where a dimension is wanted is its NATURAL width
                // (`tex.web` §430's coercion), which is the first of its slots.
                "skip" => {
                    let slot =
                        self.eng
                            .scan_register_slot(lx, pending, crate::compiler::RegFile::Skip)?;
                    return Ok(Num::Count(slot));
                }
                _ => {}
            }
            // A `\dimendef` or `\skipdef` name is that register, in every
            // position the spelt-out form works.
            if let Some(crate::expand::NumericCs::Register(r)) = self.eng.numeric_cs(n) {
                if !crate::compiler::is_count_slot(r) {
                    return Ok(Num::Count(r));
                }
            }
            lx.push_back(&[t]);
            return Ok(scaled_num(self.eng.scan_dimen_parts(lx, pending)?));
        }
    }

    /// What `\the` stands in front of, read from the file and spelt as a
    /// token list beginning with `\the`, for [`Self::msg_ops`] to render.
    ///
    /// §465 reads the quantity with `get_x_token` and §413's
    /// `scan_something_internal`. A register or code table takes a number,
    /// which is read here and written back as digits; every other quantity is
    /// the one token.
    fn the_operand(&mut self, lx: &mut Lexer) -> R<Vec<Token>> {
        let Some((t, _)) = self.eng.next_unexpandable(lx)? else {
            return Err(TexError("Missing number, treated as zero".into()));
        };
        // §428: reported at the file position, the token spent, zero used.
        if self.eng.cant_use_after_the(lx, &t) {
            return Ok(vec![Token::cs("number"), Token::Char('0', Cat::Other)]);
        }
        // §425 and §578: a font's parameters are the lowerer's font
        // memory, so the value is read here, with its operands.
        if let Token::Cs(n) = t {
            if let Some(text) = self.st_the_font_quantity(n, lx)? {
                return Ok(crate::expand::str_toks(&text));
            }
        }
        let mut spelt = vec![Token::cs("the"), t];
        let takes_number = match t {
            Token::Cs(n) => {
                matches!(
                    n.name(),
                    "count" | "dimen" | "skip" | "muskip" | "toks" | "catcode" | "wd" | "ht" | "dp"
                ) || crate::charcodes::Table::from_name(n.name()).is_some()
            }
            Token::Char(..) => false,
        };
        if takes_number {
            // A register number is §433's eight-bit integer; the code tables
            // take a character code, which is checked where it is read.
            let is_register = matches!(
                t,
                Token::Cs(n) if matches!(n.name(), "count" | "dimen" | "skip" | "muskip" | "toks" | "wd" | "ht" | "dp")
            );
            let boxed = matches!(t, Token::Cs(n) if matches!(n.name(), "wd" | "ht" | "dp"));
            let n = match (is_register, boxed) {
                (true, true) => self.eng.scan_register_file(lx)?,
                (true, false) => self.eng.scan_register_number(lx, false)?,
                (false, _) => self.eng.scan_number_file(lx)?,
            };
            spelt.extend(n.to_string().chars().map(|c| Token::Char(c, Cat::Other)));
            spelt.push(Token::Char(' ', Cat::Space));
        }
        Ok(spelt)
    }

    /// `\the` or `\number` met at the top level; see the arm that calls it. A
    /// function of its own so the dispatch loop's frame, which recursion
    /// multiplies, does not carry its locals.
    #[inline(never)]
    fn expand_the_here(&mut self, lx: &mut Lexer, name: CsId) -> R<()> {
        if self.eng.expand_in_text(lx, name)? {
            return Ok(());
        }
        let tokens = match name.name() {
            "the" => self.the_operand(lx)?,
            _ => self.number_operand(lx)?,
        };
        let mut work = Lexer::new("");
        work.push_back(&tokens);
        let parts = self.msg_ops(&mut work, &[])?;
        let text = self.static_text(&parts);
        lx.push_back(&crate::expand::str_toks(&text));
        Ok(())
    }

    /// What `\number` stands in front of, read from the file and spelt as a
    /// token list beginning with `\number`, for [`Self::msg_ops`] to render.
    fn number_operand(&mut self, lx: &mut Lexer) -> R<Vec<Token>> {
        let digits = |v: i64| -> Vec<Token> {
            v.to_string()
                .chars()
                .map(|c| Token::Char(c, Cat::Other))
                .collect()
        };
        let mut toks = vec![Token::cs("number")];
        match self.number(lx)? {
            Num::Literal(v) => toks.extend(digits(v)),
            Num::Count(slot) => toks.extend(self.register_tokens(slot)?),
            Num::Neg(slot) => {
                toks.push(Token::Char('-', Cat::Other));
                toks.extend(self.register_tokens(slot)?);
            }
            _ => return Err(TexError("Unsupported \\number operand".into())),
        }
        Ok(toks)
    }

    /// The tokens that name the register in `slot`, as a document would spell
    /// it.
    fn register_tokens(&self, slot: i64) -> R<Vec<Token>> {
        use crate::compiler::{DIMEN_BASE, MUSKIP_BASE, SKIP_BASE, SKIP_STRIDE};
        if let Some(name) = crate::params::name_of_slot(slot) {
            return Ok(vec![Token::cs(name)]);
        }
        let pooled = self.eng.pooled_register(slot).map(|(file, n)| (file.name(), n));
        let (word, n) = match slot {
            _ if pooled.is_some() => pooled.expect("just matched"),
            s if s < DIMEN_BASE => ("count", s),
            s if s < SKIP_BASE => ("dimen", s - DIMEN_BASE),
            s if s < MUSKIP_BASE => ("skip", (s - SKIP_BASE) / SKIP_STRIDE),
            s => ("muskip", (s - MUSKIP_BASE) / SKIP_STRIDE),
        };
        let mut toks = vec![Token::cs(word)];
        toks.extend(n.to_string().chars().map(|c| Token::Char(c, Cat::Other)));
        toks.push(Token::Char(' ', Cat::Space));
        Ok(toks)
    }

    /// A message's pieces as the text they stand for NOW, reading each register
    /// as far as the lowerer has followed it. A message prints them at run
    /// time; text that goes back into the mouth cannot wait for that.
    fn static_text(&self, parts: &[MsgOp]) -> String {
        let value = |n: &Num| self.eng.known.num(n).unwrap_or(0);
        let mut out = String::new();
        for part in parts {
            match part {
                MsgOp::Text(t) => out.push_str(t),
                MsgOp::Report(_) | MsgOp::Discard(_) | MsgOp::ShowBox(_) => {}
                MsgOp::Number(n) => out.push_str(&value(n).to_string()),
                MsgOp::Dimen(n) => {
                    out.push_str(&crate::dimen::print_scaled(value(n)));
                    out.push_str("pt");
                }
                MsgOp::Roman(n) => out.push_str(&crate::expand::roman_int(value(n))),
                MsgOp::Glue(s) | MsgOp::MuGlue(s) => {
                    let unit = match part {
                        MsgOp::Glue(_) => "pt",
                        _ => "mu",
                    };
                    let packed = value(&s[3]);
                    out.push_str(&crate::glue::print_glue_in(
                        unit,
                        value(&s[0]),
                        value(&s[1]),
                        packed / 4,
                        value(&s[2]),
                        packed % 4,
                    ));
                }
                MsgOp::If {
                    left,
                    rel,
                    right,
                    then_ops,
                    else_ops,
                } => {
                    let (l, r) = (value(left), value(right));
                    let holds = match rel {
                        Rel::Less => l < r,
                        Rel::Equal => l == r,
                        Rel::Greater => l > r,
                    };
                    let arm = if holds { then_ops } else { else_ops };
                    out.push_str(&self.static_text(arm));
                }
                MsgOp::IfOdd {
                    value: v,
                    then_ops,
                    else_ops,
                } => {
                    let arm = if value(v) % 2 != 0 {
                        then_ops
                    } else {
                        else_ops
                    };
                    out.push_str(&self.static_text(arm));
                }
            }
        }
        out
    }

    /// §1348's `do_extension` for the four extensions of §1344, as LuaTeX's
    /// `extensions.c` reads them: the whatsit is built, and then either done
    /// now (`immediate`, §1375) or kept for the shipout of the page it lands
    /// on (see `deferred_whatsits`).
    fn extension(
        &mut self,
        lx: &mut Lexer,
        which: &str,
        immediate: bool,
        out: &mut Vec<Cmd>,
    ) -> R<()> {
        use crate::write::{Stream, Whatsit};
        let whatsit = match which {
            // `new_write_whatsit(open_node_size, 1)`: the stream is checked
            // against the last of LuaTeX's 128, then §1351 reads the name.
            "openout" => {
                let stream = self.scan_out_stream(lx)?;
                self.eng.skip_equals_file(lx)?;
                let name = self.scan_out_name(lx)?;
                Whatsit::Open { stream, name }
            }
            // §1352: the number, then the text without expanding it.
            "write" => {
                let n = self.eng.scan_number_file(lx)?;
                let tokens = self.eng.read_write_text(lx)?;
                Whatsit::Write {
                    stream: Stream::of(n),
                    tokens,
                }
            }
            "closeout" => Whatsit::Close {
                stream: self.scan_out_stream(lx)?,
            },
            // §1354, with LuaTeX's `shipout` keyword: the text is expanded
            // now as `\xdef` expands, unless it is to wait for the shipout.
            _ => {
                let late = self.eng.scan_keyword(lx, "shipout", false)?;
                let tokens = self.eng.read_write_text(lx)?;
                let tokens = match late {
                    true => tokens,
                    false => self.eng.expand_edef_body(&tokens)?,
                };
                Whatsit::Special { tokens, late }
            }
        };
        match immediate {
            true => self.ship_whatsit(lx, &whatsit, out),
            false => {
                self.deferred_whatsits.push(whatsit);
                Ok(())
            }
        }
    }

    /// LuaTeX's backend extensions (`crate::backend`): what is done now is
    /// done there, and a whatsit a page has to carry waits with the others in
    /// `deferred_whatsits` (see there).
    fn backend_extension(&mut self, lx: &mut Lexer, which: &str, immediate: bool) -> R<()> {
        use crate::write::Whatsit;
        let whatsit = match which {
            "pdfextension" => crate::backend::extension(&mut self.eng, lx, immediate)?,
            "saveimageresource" => {
                crate::backend::save_image(&mut self.eng, lx)?;
                None
            }
            "useimageresource" => Some(crate::backend::use_image(&mut self.eng, lx)?),
            "useboxresource" => Some(crate::backend::use_box(&mut self.eng, lx)?),
            "savepos" => Some(crate::backend::PdfWhatsit::SavePos),
            // `\saveboxresource` takes a box, which only the stomach holds:
            // what is read here is the keywords and the register, and the form
            // is numbered as `scan_pdfxform` numbers it.
            _ => {
                crate::backend::save_box(&mut self.eng, lx)?;
                None
            }
        };
        if let Some(w) = whatsit {
            self.deferred_whatsits.push(Whatsit::Pdf(w));
        }
        Ok(())
    }

    /// `scan_limited_int(last_file_selector)`: a stream for `\openout` or
    /// `\closeout`, reported and read as 0 when it is out of range.
    fn scan_out_stream(&mut self, lx: &mut Lexer) -> R<usize> {
        let n = self.eng.scan_number_file(lx)?;
        if !(0..crate::write::WRITE_STREAMS as i64).contains(&n) {
            self.eng.report(lx, &format!("Bad number ({n})"));
            return Ok(0);
        }
        Ok(n as usize)
    }

    /// §1351's file name, as LuaTeX's `do_extension` reads it: blanks and
    /// `\relax` skipped with expansion, then a name in braces (every character
    /// of the expanded list, spaces included -- `scan_file_name_toks`) or §526's
    /// `scan_file_name`, characters read with `get_x_token` up to a space or
    /// the first token that is not a character, which is put back.
    ///
    /// Expansion is the difference from `\input`'s scanner here: LaTeX opens
    /// its `.aux` with `\openout\@mainaux\jobname.aux`.
    fn scan_out_name(&mut self, lx: &mut Lexer) -> R<String> {
        let first = loop {
            match self.eng.next_unexpandable(lx)? {
                Some((t, _)) if t.is_space() => continue,
                Some((_, Some(p))) if p.name() == "relax" => continue,
                Some((t, _)) => break t,
                None => return Err(TexError("Missing file name".into())),
            }
        };
        self.file_name_from(lx, first)
    }

    /// §1373's `out_what` (`wrapup_leader` in LuaTeX): do what a whatsit asks
    /// for, at the moment it is shipped -- or read, under `\immediate`.
    ///
    /// The node-list stomach calls this for each whatsit on a page it ships,
    /// in the order they sit on the page; `\immediate` calls it at once. A
    /// `\write` is expanded HERE, as §1370 expands it when it is written, so
    /// a macro redefined between the `\write` and the shipout writes its new
    /// meaning.
    pub(crate) fn ship_whatsit(
        &mut self,
        lx: &mut Lexer,
        whatsit: &crate::write::Whatsit,
        out: &mut Vec<Cmd>,
    ) -> R<()> {
        use crate::write::{Stream, Whatsit};
        match whatsit {
            Whatsit::Open { stream, name } => {
                let name = crate::write::out_file_name(name);
                self.written.open(*stream, &name);
                out.push(Cmd::OpenOut {
                    stream: *stream,
                    name,
                });
            }
            Whatsit::Close { stream } => {
                self.written.close(*stream);
                out.push(Cmd::CloseOut(*stream));
            }
            Whatsit::Write { stream, tokens } => {
                self.eng.note_mouth(lx);
                let mut work = Lexer::new("");
                work.push_back(tokens);
                let parts = self.msg_ops(&mut work, &[])?;
                let nl = self.eng.intpars.get(crate::intpar::NEW_LINE_CHAR);
                // What §1370 does when the stream is closed: the log alone
                // for a negative number, and otherwise the terminal, the
                // reports waiting for it in front as `\message` carries them.
                let closed = |this: &mut Self, parts: Vec<MsgOp>| match *stream {
                    Stream::LogOnly => Vec::new(),
                    _ => {
                        let mut ops = this.take_reports();
                        ops.extend(crate::ir::printed_ops(parts, nl));
                        vec![Cmd::Write(ops)]
                    }
                };
                match *stream {
                    Stream::File(n) => {
                        // A stream the lowering has open has its line put into
                        // the lowering's copy of the file, and leaves the
                        // reports waiting for the next thing printed.
                        let otherwise = match self.written.is_open(n) {
                            true => {
                                let line = self.static_text(&parts);
                                self.written
                                    .write(*stream, &crate::write::printed_to_file(&line, nl));
                                vec![Cmd::Write(crate::ir::printed_ops(parts.clone(), nl))]
                            }
                            false => closed(self, parts.clone()),
                        };
                        out.push(Cmd::WriteOut {
                            stream: n,
                            file: crate::write::file_ops(parts, nl),
                            otherwise,
                        });
                    }
                    _ => out.extend(closed(self, parts)),
                }
            }
            // A `\special` is the shipped page's: `pdf_special` in
            // `pdfliteral.c` writes one beginning `pdf:` into the page's
            // content (`crate::pdfpage::PageWriter::special`) at the point it
            // is shipped at, which only a shipout knows. Under `\immediate`
            // there is no page, and LuaTeX's `do_extension` never ships one
            // immediately either.
            Whatsit::Special { .. } => {}
            // A backend whatsit is never done immediately; what `\immediate`
            // does for one is done when it is read (`crate::backend`).
            Whatsit::Pdf(_) => {}
        }
        Ok(())
    }

    /// `\message{...}` lowered to the steps that build it at run time.
    ///
    /// The body is walked as a token list. Macros and `\csname` resolve here --
    /// they depend on the macro table, which is a frontend fact. `\the\count`
    /// and conditionals do not: they read VM slots, so they become a slot read
    /// and a real branch.
    fn message_parts(&mut self, lx: &mut Lexer) -> R<Vec<MsgOp>> {
        let (body, site) = self.eng.read_message_body_sited(lx)?;
        let mut work = Lexer::new("");
        work.push_back(&body);
        work.attach_site(site);
        self.msg_ops(&mut work, &[])
    }

    /// Walk a message token list into build steps, stopping at `stop`.
    fn msg_ops(&mut self, work: &mut Lexer, stop: &[&str]) -> R<Vec<MsgOp>> {
        let mut out: Vec<MsgOp> = Vec::new();
        let mut text = String::new();
        // Conditionals this walk DECIDED (their truth is a frontend fact) and
        // is still inside: §498's `cond_ptr`, for the ones that never become a
        // run-time branch. Their `\else` and `\fi` are met in-stream, as tex
        // meets them, rather than collected up front with both arms lowered --
        // which expanded the arm not taken, so a tail call in it
        // (`\else...\expandafter\l\fi`) recursed forever.
        let mut decided = 0usize;
        macro_rules! flush {
            () => {
                if !text.is_empty() {
                    out.push(MsgOp::Text(std::mem::take(&mut text)));
                }
            };
        }
        // A macro that calls itself never empties `work`: bound the walk the
        // way the expander bounds an `\edef`, so it ends in TeX's capacity
        // error rather than a hang or unbounded memory.
        let mut steps = 0usize;
        while let Some(t) = work.pending.pop() {
            if self.eng.halted {
                break;
            }
            steps += 1;
            if steps > 200_000 {
                return Err(TexError("TeX capacity exceeded".into()));
            }
            // A call that leaves its `\fi` behind each time (`\ifnum..\r\fi`)
            // is a level of tex's input stack per iteration, and tex stops at
            // its `stack_size` of 10000. The bookkeeping for the error context
            // is also linear in the levels, so unbounded it is quadratic.
            if work.input_levels() > 10_000 {
                return Err(TexError(
                    "TeX capacity exceeded, sorry [input stack size=10000]".into(),
                ));
            }
            // Active characters are commands here too: `\message{~}` runs `~`.
            let t = match &t {
                Token::Char(c, Cat::Active) => match self.eng.active_meaning(*c) {
                    Some(id) => Token::Cs(id),
                    None => t,
                },
                _ => t,
            };
            let Token::Cs(n) = &t else {
                // §294: a macro-parameter character prints doubled.
                if let Token::Char(c, Cat::Param) = t {
                    text.push(c);
                }
                text.push_str(&t.to_text(self.eng.esc()));
                continue;
            };
            // A `\let` copy of an expandable primitive is that primitive here,
            // as `cur_cmd` is in tex: the arms below are by name.
            let n = self.eng.expandable_alias(*n);
            // The brace that ended the body: nothing after it is the message's.
            if n.name() == crate::expand::MESSAGE_END {
                break;
            }
            if stop.contains(&n.name()) && !(decided > 0 && matches!(n.name(), "else" | "fi")) {
                work.push_back(&[Token::Cs(n)]);
                break;
            }
            if decided > 0 && matches!(n.name(), "else" | "fi") {
                self.close_decided(work, n.name(), &mut decided)?;
                continue;
            }
            // §510: a `\fi`, `\else` or `\or` that closes no conditional is
            // `Extra`, reported and ignored -- here too, where it is expanded.
            if let Some(kind) = self.eng.stray_conditional_end(n) {
                self.eng.extra_conditional_end(work, kind);
                continue;
            }
            // A `token.set_lua` name without "protected" expands here as the
            // gullet expands it (`lua_expandable_call`).
            if crate::lua::expands_in_gullet(n.name()) {
                self.eng.expand_pending(work, n)?;
                continue;
            }
            match n.name() {
                "the" | "number" => {
                    flush!();
                    if n.name() == "the" {
                        let target = self.the_target(work)?;
                        if let Some(t) = &target {
                            if self.eng.cant_use_after_the(work, t) {
                                text.push('0');
                                continue;
                            }
                        }
                        if let Some(Token::Cs(w)) = &target {
                            if let Some(op) = self.st_the(*w, work)? {
                                out.push(op);
                                continue;
                            }
                        }
                        match target {
                            // `\the\dimen0` is written as a DIMENSION -- 1.0pt,
                            // not 65536 -- which is the whole difference
                            // between the two registers at this level.
                            // `\the\toks0` is the token list as text, and it
                            // is known while lowering because the table is.
                            Some(Token::Cs(w)) if w.name() == "toks" => {
                                let reg = self.eng.scan_register_number(work, true)?;
                                text.push_str(&self.eng.toks_shown(reg));
                                continue;
                            }
                            Some(Token::Cs(w)) if self.eng.toks_cs(w).is_some() => {
                                let reg = self.eng.toks_cs(w).expect("just matched");
                                text.push_str(&self.eng.toks_shown(reg));
                                continue;
                            }
                            Some(Token::Cs(w)) if w.name() == "skip" || w.name() == "muskip" => {
                                let mu = w.name() == "muskip";
                                let file = match mu {
                                    true => crate::compiler::RegFile::MuSkip,
                                    false => crate::compiler::RegFile::Skip,
                                };
                                let base = self.eng.scan_register_slot(work, true, file)?;
                                let slots = [
                                    Num::Count(base),
                                    Num::Count(base + 1),
                                    Num::Count(base + 2),
                                    Num::Count(base + 3),
                                ];
                                out.push(match mu {
                                    true => MsgOp::MuGlue(slots),
                                    false => MsgOp::Glue(slots),
                                });
                                continue;
                            }
                            Some(Token::Cs(w)) if w.name() == "dimen" => {
                                let slot = self.eng.scan_register_slot(
                                    work,
                                    true,
                                    crate::compiler::RegFile::Dimen,
                                )?;
                                out.push(MsgOp::Dimen(Num::Count(slot)));
                                continue;
                            }
                            // e-TeX's `\numexpr` and `\dimexpr`: the value, written
                            // as an integer or as a dimension.
                            Some(Token::Cs(w))
                                if expr_level_of(self.eng.primitive_meaning(w).name())
                                    .is_some() =>
                            {
                                let level = expr_level_of(self.eng.primitive_meaning(w).name())
                                    .expect("just matched");
                                let e = self.expr(work, true, level)?;
                                match (level, e) {
                                    (ExprLevel::Int, Num::Literal(v)) => {
                                        text.push_str(&v.to_string())
                                    }
                                    (ExprLevel::Dimen, Num::Literal(v)) => {
                                        text.push_str(&crate::dimen::print_scaled(v));
                                        text.push_str("pt");
                                    }
                                    (ExprLevel::Int, e) => out.push(MsgOp::Number(e)),
                                    (ExprLevel::Dimen, e) => out.push(MsgOp::Dimen(e)),
                                }
                                continue;
                            }
                            Some(Token::Cs(w))
                                if matches!(
                                    self.eng.primitive_meaning(w).name(),
                                    "gluestretch" | "glueshrink"
                                ) =>
                            {
                                let part = self.glue_part(work, w)?.expect("a glue part");
                                out.push(MsgOp::Dimen(part));
                                continue;
                            }
                            Some(Token::Cs(w))
                                if matches!(
                                    self.eng.primitive_meaning(w).name(),
                                    "gluestretchorder"
                                        | "glueshrinkorder"
                                        | "eTeXgluestretchorder"
                                        | "eTeXglueshrinkorder"
                                ) =>
                            {
                                let part = self.glue_part(work, w)?.expect("a glue order");
                                out.push(MsgOp::Number(part));
                                continue;
                            }
                            Some(Token::Cs(w))
                                if matches!(
                                    self.eng.primitive_meaning(w).name(),
                                    "glueexpr" | "muexpr" | "gluetomu" | "mutoglue"
                                ) =>
                            {
                                let prim = self.eng.primitive_meaning(w).name();
                                let mu = matches!(prim, "muexpr" | "gluetomu");
                                let g = match prim {
                                    "glueexpr" | "muexpr" => self.glue_expr(work, mu)?,
                                    _ => self.glue_factor(work, !mu)?,
                                };
                                out.push(match mu {
                                    true => MsgOp::MuGlue(g),
                                    false => MsgOp::Glue(g),
                                });
                                continue;
                            }
                            Some(Token::Cs(w)) if w.name() == "count" => {}
                            // A LuaTeX attribute register is frontend state.
                            Some(Token::Cs(w)) if w.name() == "attribute" => {
                                let n = self.eng.scan_register_number(work, true)?;
                                text.push_str(&self.eng.attribute_value(n).to_string());
                                continue;
                            }
                            // A §236 parameter is frontend state, so its value
                            // is known now.
                            Some(Token::Cs(w)) if self.eng.intpar_cs(w).is_some() => {
                                let i = self.eng.intpar_cs(w).expect("just matched");
                                text.push_str(&self.eng.intpars.get(i).to_string());
                                continue;
                            }
                            // `\the\catcode`\x` reads the catcode table, which
                            // is frontend state here -- `compile_time_catcode`
                            // writes it -- so the answer is known now, exactly
                            // as the five code tables below are. It is the one
                            // of §413's tables a package is likeliest to ask
                            // about: `graphics.sty:34` opens with five of them.
                            Some(Token::Cs(w)) if w.name() == "catcode" => {
                                let v = self.eng.catcode_value(work)?;
                                text.push_str(&v.to_string());
                                continue;
                            }
                            // `\the\pageno` reads the register the name stands
                            // for, and `\the\active` is the constant itself --
                            // known already, so it is rendered here rather than
                            // asked of the run.
                            // `\the\mathcode`\x` and its siblings read a table
                            // the lowerer owns, so the answer is known now.
                            Some(Token::Cs(w))
                                if crate::charcodes::Table::from_name(w.name()).is_some() =>
                            {
                                let t = crate::charcodes::Table::from_name(w.name())
                                    .expect("just matched");
                                let v = self.eng.charcode_value(work, t)?;
                                text.push_str(&v.to_string());
                                continue;
                            }
                            Some(Token::Cs(w)) => match self.eng.numeric_cs(w) {
                                Some(crate::expand::NumericCs::Register(r)) => {
                                    // A `\dimendef` name is a dimension
                                    // register, so `\the` writes it as one.
                                    let slots = [
                                        Num::Count(r),
                                        Num::Count(r + 1),
                                        Num::Count(r + 2),
                                        Num::Count(r + 3),
                                    ];
                                    out.push(if crate::compiler::is_mu_slot(r) {
                                        MsgOp::MuGlue(slots)
                                    } else if crate::compiler::is_glue_slot(r) {
                                        MsgOp::Glue(slots)
                                    } else if !crate::compiler::is_count_slot(r) {
                                        MsgOp::Dimen(Num::Count(r))
                                    } else {
                                        MsgOp::Number(Num::Count(r))
                                    });
                                    continue;
                                }
                                Some(crate::expand::NumericCs::Value(v)) => {
                                    text.push_str(&v.to_string());
                                    continue;
                                }
                                None => return Err(TexError("Unsupported \\the".into())),
                            },
                            _ => return Err(TexError("Unsupported \\the".into())),
                        }
                        let reg =
                            self.eng
                                .scan_register_slot(work, true, crate::compiler::RegFile::Count)?;
                        out.push(MsgOp::Number(Num::Count(reg)));
                        continue;
                    }
                    // A sign before a register: §440 negates the register's
                    // value, which is the run's, so it is read when the
                    // message runs. A sign before a literal is the scanner's.
                    if matches!(
                        work.pending.last(),
                        Some(Token::Char('+' | '-', Cat::Other))
                    ) {
                        match self.msg_number(work)? {
                            Num::Literal(v) => text.push_str(&v.to_string()),
                            num => out.push(MsgOp::Number(num)),
                        }
                        continue;
                    }
                    // `\number` takes either a register or a literal.
                    // `\number\skip0` gives the natural component only.
                    if matches!(work.pending.last(), Some(Token::Cs(w)) if w.name() == "skip") {
                        let _ = work.pending.pop();
                        let slot =
                            self.eng
                                .scan_register_slot(work, true, crate::compiler::RegFile::Skip)?;
                        out.push(MsgOp::Number(Num::Count(slot)));
                        continue;
                    }
                    if matches!(work.pending.last(), Some(Token::Cs(w)) if w.name() == "muskip") {
                        let _ = work.pending.pop();
                        let slot =
                            self.eng
                                .scan_register_slot(work, true, crate::compiler::RegFile::MuSkip)?;
                        out.push(MsgOp::Number(Num::Count(slot)));
                        continue;
                    }
                    // `\number\dimen0` gives the scaled points, unrendered.
                    if matches!(work.pending.last(), Some(Token::Cs(w)) if w.name() == "dimen") {
                        let _ = work.pending.pop();
                        let slot =
                            self.eng
                                .scan_register_slot(work, true, crate::compiler::RegFile::Dimen)?;
                        out.push(MsgOp::Number(Num::Count(slot)));
                        continue;
                    }
                    // `\count` and every `\countdef` name are read when the
                    // message runs; anything else is a number known now.
                    match self.msg_number(work)? {
                        Num::Literal(v) => text.push_str(&v.to_string()),
                        num => out.push(MsgOp::Number(num)),
                    }
                }
                // An advice marker: it carries depth, not text.
                n if self.eng.advice_marker(n) => {}
                // `\rustcall <name> <numbers…>\endrust` inside a message: the
                // returned value is rendered where the call stands.
                n if n == crate::rust_ffi::CALL_CS => {
                    flush!();
                    let call = self.rust_call(work, true)?;
                    out.push(MsgOp::Number(call));
                }
                // `\unless` negates the conditional that follows it here too.
                "unless" => self.eng.set_unless(true),
                // `\csstring\f` is `\string\f` without the escape character.
                "csstring" => {
                    if let Some(next) = work.pending.pop() {
                        text.push_str(&match &next {
                            Token::Cs(cs) => cs.name().to_string(),
                            other => other.to_text(self.eng.esc()),
                        });
                    }
                }
                // `\Uchar<number>` is the character with that code.
                // `\romannumeral<number>` is expandable: the numeral, as text.
                // A register is read when the message runs, as `\number` reads one.
                "romannumeral" => match self.msg_number(work)? {
                    Num::Literal(n) => text.push_str(&crate::expand::roman_int(n)),
                    n => {
                        flush!();
                        out.push(MsgOp::Roman(n));
                    }
                },
                "Uchar" => {
                    let code = self.eng.scan_number_pending(work)?;
                    match u32::try_from(code).ok().and_then(char::from_u32) {
                        Some(c) => text.push(c),
                        None => return Err(TexError(format!("Bad character code ({code})"))),
                    }
                }
                // `\detokenize{...}` writes the tokens as text -- the same rule
                // `\the\toks` uses, and the same renderer.
                "detokenize" => {
                    let group = self.eng.scan_general_text(work)?;
                    text.push_str(&self.eng.tokens_shown(&group));
                }
                "string" => {
                    if let Some(next) = work.pending.pop() {
                        text.push_str(&match &next {
                            Token::Cs(cs) => self.eng.sprint_cs(*cs),
                            other => other.to_text(self.eng.esc()),
                        });
                    }
                }
                // §367: `\noexpand` makes the next token unexpandable, so a
                // message prints a macro after it as §262's `print_cs` does.
                "noexpand" => {
                    if let Some(next) = work.pending.pop() {
                        match next {
                            // The brace that ends the text is read by the
                            // scanner as it is, and ends it.
                            Token::Cs(m) if m.name() == crate::expand::MESSAGE_END => break,
                            next => text.push_str(&self.eng.tokens_shown(&[next])),
                        }
                    }
                }
                // `\fontname` is expandable too (§472).
                "fontname" => {
                    let name = self.eng.fontname_text(work, true)?;
                    text.push_str(&name);
                }
                // `\jobname` is expandable: the job's name, as text.
                "jobname" => text.push_str(&crate::lua::jobname()),
                // The backend's two expandable commands put their expansion
                // back, where this loop reads it (`crate::backend`).
                "pdffeedback" => crate::backend::feedback(&mut self.eng, work)?,
                "pdfvariable" => crate::backend::variable(&mut self.eng, work)?,
                n if crate::expand::constant_text(n).is_some() => {
                    text.push_str(crate::expand::constant_text(n).expect("just matched"))
                }
                // `\meaning` is expandable, so it reaches a message body as
                // readily as running text: what §296 would print for the next
                // token, as characters.
                "meaning" => {
                    if let Some(next) = work.pending.pop() {
                        text.push_str(&self.eng.meaning_text(&next));
                    }
                }
                // `\expanded{...}` expands its general text as an \edef body is
                // expanded and puts the result back, where this loop expands
                // it again: a `\noexpand` in the text holds for the first
                // pass only. The first pass reads the registers
                // `crate::known` has followed; a text that reads one only the
                // run has goes back as it stands, which differs only in what a
                // `\noexpand` protects.
                "expanded" => {
                    let group = self.eng.scan_general_text(work)?;
                    let mark = self.eng.report_mark();
                    let again = match self.eng.expand_edef_body(&group) {
                        Ok(expanded) => expanded,
                        Err(_) => {
                            self.eng.drain_diagnostics(mark);
                            group
                        }
                    };
                    work.push_back(&again);
                }
                // `\unexpanded{...}` writes the tokens as they stand. In a
                // message that is the same rendering `\detokenize` gives, and
                // the two only part company inside an `\edef`, where these
                // tokens survive as tokens.
                "unexpanded" => {
                    let group = self.eng.scan_general_text(work)?;
                    text.push_str(&self.eng.tokens_shown(&group));
                }
                // `\begincsname` is `\csname` that does not define what it does
                // not find: an unknown name expands to nothing instead of to
                // `\relax`.
                "begincsname" => {
                    let built = self.eng.read_csname_pending(work)?;
                    if let Some(id) = self
                        .eng
                        .named_cs(&built, false)
                        .filter(|_| !built.is_empty())
                    {
                        work.push_back(&[Token::Cs(id)]);
                    }
                }
                "lastnamedcs" => {
                    self.eng.expand_pending(work, n)?;
                }
                // Read again as a file; see the gullet's arm.
                "scantokens" | "scantextokens" => {
                    self.eng.expand_pending(work, n)?;
                }
                "csname" => {
                    // The name is built from text and macros, all compile-time.
                    let built = self.eng.read_csname_pending(work)?;
                    // §372: a name with no meaning is made \relax.
                    let id = self
                        .eng
                        .named_cs(&built, true)
                        .expect("\\csname always makes one");
                    work.push_back(&[Token::Cs(id)]);
                }
                "expandafter" => {
                    let Some(held) = work.pending.pop() else {
                        return Err(TexError("Missing token after \\expandafter".into()));
                    };
                    let Some(next) = work.pending.pop() else {
                        return Err(TexError("Missing token after \\expandafter".into()));
                    };
                    match &next {
                        // The `\else` or `\fi` of a conditional decided here
                        // expands one step like any other: `\expandafter\l\fi`
                        // closes the conditional before `\l` reads its argument.
                        Token::Cs(m) if decided > 0 && matches!(m.name(), "else" | "fi") => {
                            let m = *m;
                            self.close_decided(work, m.name(), &mut decided)?;
                        }
                        Token::Cs(m) if self.eng.is_macro(*m) => {
                            let m = *m;
                            self.eng.expand_macro_pending(work, m)?;
                        }
                        // An active character defined as a macro expands as
                        // a macro does: `\expandafter\string~` is `\string`
                        // over the first token of `~`'s body.
                        Token::Char(c, Cat::Active)
                            if self
                                .eng
                                .active_meaning(*c)
                                .is_some_and(|id| self.eng.is_macro(id)) =>
                        {
                            let id = self.eng.active_meaning(*c).expect("just matched");
                            self.eng.expand_macro_pending(work, id)?;
                        }
                        // `tex.web` §366: `\expandafter\A\B` expands `\B` ONE
                        // step whatever `\B` is, and only a macro was being
                        // expanded here. `\expandafter\wrap\expandafter{\inner}`
                        // is the idiom that breaks without it -- the inner
                        // `\expandafter` was put back unexpanded, so `\wrap`
                        // took the empty brace group as its argument and
                        // `\inner` was left standing outside it.
                        //
                        // The conditionals are the exception, and deliberately:
                        // inside a message body they lower to run-time branches
                        // over registers the VM holds, so letting the expander
                        // decide one now would answer from the frontend's stale
                        // copy of a register instead.
                        Token::Cs(m) if !crate::expand::Engine::is_conditional(*m) => {
                            let m = *m;
                            if !self.eng.expand_pending(work, m)? {
                                work.push_back(&[next]);
                            }
                        }
                        _ => work.push_back(&[next]),
                    }
                    work.push_back(&[held]);
                }
                "iftrue" | "iffalse" => {
                    let taken = n.name() == "iftrue";
                    self.open_decided(work, n, taken, &mut decided)?;
                }
                // LuaTeX's: no `\csname` is reading a name in a message, a
                // name means its primitive or does not, and `\ifcondition`
                // is nothing where it is expanded.
                "ifincsname" => {
                    let inside = self.eng.in_csname();
                    self.open_decided(work, n, inside, &mut decided)?;
                }
                "ifprimitive" => {
                    let truth = match work.pending.pop() {
                        Some(Token::Cs(cs)) => self.eng.means_its_primitive(cs),
                        _ => false,
                    };
                    self.open_decided(work, n, truth, &mut decided)?;
                }
                "ifcondition" => {}
                "ifx" => {
                    let a = work.pending.pop();
                    let b = work.pending.pop();
                    let same = self.eng.meanings_equal_pub(a.as_ref(), b.as_ref());
                    self.open_decided(work, n, same, &mut decided)?;
                }
                // Character codes and categories are frontend facts too, so
                // `\if` and `\ifcat` are decided here as `\ifx` is. They were
                // not named, so a message printed the test and both arms.
                "if" | "ifcat" => {
                    let same = self.eng.if_pending(work, n.name())?;
                    self.open_decided(work, n, same, &mut decided)?;
                }
                // Both ask the macro table a question, which is a FRONTEND
                // fact -- so they are decided here, exactly as `\iftrue` and
                // `\ifx` above are, rather than lowered to a branch over a
                // register the VM holds.
                //
                // `\ifdefined` takes the next token unexpanded (etex.ch's
                // `if_def_code` uses `get_next`, not `get_x_token`);
                // `\ifcsname` reads the characters up to `\endcsname` and looks
                // the name up WITHOUT entering it. Neither reached this loop
                // before: a message body is read as a token list and anything
                // this match did not name was printed as itself, so
                // `\message{[\ifdefined\foo YES\else NO\fi]}` printed
                // `[\ifdefined xYES\else NO\fi]` -- the test, the macro's
                // expansion and BOTH arms.
                // The box conditionals are here for the same reason, one step
                // further on: every box register is void (see `do_conditional`),
                // so what they answer is known while lowering too.
                // §505 over a register the program fills: a real branch on the
                // slot the stomach mirrors its kind in.
                "ifvoid" | "ifhbox" | "ifvbox" if self.st_box_conditionals() => {
                    let reg = self.eng.scan_register_pending(work)?;
                    let (left, right) = Self::st_box_test(n.name(), reg);
                    let (then_ops, else_ops) = self.msg_arms(work)?;
                    flush!();
                    out.push(MsgOp::If {
                        left,
                        rel: Rel::Equal,
                        right,
                        then_ops,
                        else_ops,
                    });
                }
                "ifdefined" | "ifcsname" | "ifvoid" | "ifhbox" | "ifvbox" | "ifeof" => {
                    let truth = match n.name() {
                        // Whether a `\read` stream is closed: the streams are
                        // opened and read while lowering, so this is known now.
                        "ifeof" => {
                            let s = self.eng.scan_stream_number(work, true)?;
                            self.eng.read_stream_closed(s)
                        }
                        "ifcsname" => {
                            let built = self.eng.read_csname_pending(work)?;
                            self.eng.named_cs(&built, false).is_some()
                        }
                        "ifdefined" => {
                            let t = work.pending.pop();
                            self.eng.if_def_truth(t.as_ref())
                        }
                        other => {
                            let _reg = self.eng.scan_number_pending(work)?;
                            other == "ifvoid"
                        }
                    };
                    self.open_decided(work, n, truth, &mut decided)?;
                }
                "ifnum" | "ifabsnum" => {
                    let abs = n.name() == "ifabsnum";
                    self.eng.begin_test();
                    let left = magnitude(abs, self.msg_number(work)?);
                    let rel = match self.eng.read_relation_pending(work, n.name())? {
                        '<' => Rel::Less,
                        '>' => Rel::Greater,
                        _ => Rel::Equal,
                    };
                    let right = magnitude(abs, self.msg_number(work)?);
                    self.eng.end_test();
                    // Two known values are a frontend fact: decide here, as
                    // §498 does, so only the arm taken is lowered. Lowering both
                    // made `\def\r#1{\ifnum#1>0 \r{0}\fi}` unbounded
                    // for a call whose test is false.
                    if let Some(truth) = self.known_truth(&left, rel, &right) {
                        self.open_decided(work, n, truth, &mut decided)?;
                        continue;
                    }
                    let (then_ops, else_ops) = self.msg_arms(work)?;
                    flush!();
                    out.push(MsgOp::If {
                        left,
                        rel,
                        right,
                        then_ops,
                        else_ops,
                    });
                }
                // The same comparison over dimensions; see the `\ifdim` arm in
                // the file-level dispatch.
                "ifdim" | "ifabsdim" => {
                    let abs = n.name() == "ifabsdim";
                    self.eng.begin_test();
                    let left = magnitude(abs, self.dimen_number(work, true)?);
                    let rel = match self.eng.read_relation_pending(work, n.name())? {
                        '<' => Rel::Less,
                        '>' => Rel::Greater,
                        _ => Rel::Equal,
                    };
                    let right = magnitude(abs, self.dimen_number(work, true)?);
                    self.eng.end_test();
                    if let Some(truth) = self.known_truth(&left, rel, &right) {
                        self.open_decided(work, n, truth, &mut decided)?;
                        continue;
                    }
                    let (then_ops, else_ops) = self.msg_arms(work)?;
                    flush!();
                    out.push(MsgOp::If {
                        left,
                        rel,
                        right,
                        then_ops,
                        else_ops,
                    });
                }
                "ifodd" => {
                    self.eng.begin_test();
                    let value = self.msg_number(work);
                    self.eng.end_test();
                    let value = value?;
                    if let Some(v) = self.eng.known.num(&value) {
                        self.open_decided(work, n, v % 2 != 0, &mut decided)?;
                        continue;
                    }
                    let (then_ops, else_ops) = self.msg_arms(work)?;
                    flush!();
                    out.push(MsgOp::IfOdd {
                        value,
                        then_ops,
                        else_ops,
                    });
                }
                "ifcase" => {
                    self.eng.begin_test();
                    let value = self.msg_number(work);
                    self.eng.end_test();
                    let value = value?;
                    let branches = self.msg_case_arms(work)?;
                    flush!();
                    out.push(self.case_chain(value, branches));
                }
                _ if self.eng.is_macro(n) => self.eng.expand_macro_pending(work, n)?,
                // An unexpandable control sequence prints as §262's `print_cs`.
                _ if self.eng.recover && self.eng.is_undefined(n) => {
                    self.eng.report(work, "Undefined control sequence");
                }
                _ => text.push_str(&self.eng.cs_text(n.name())),
            }
        }
        if !text.is_empty() {
            out.push(MsgOp::Text(text));
        }
        Ok(out)
    }

    /// The token `\the` reads in a message body: §465's `get_x_token`, so a
    /// macro, an undefined name (§370, reported) or an expandable primitive
    /// in front of the quantity expands first. A conditional is left alone --
    /// in a message it is a run-time branch, not a decision taken now -- and
    /// so is the brace that ends the body.
    fn the_target(&mut self, work: &mut Lexer) -> R<Option<Token>> {
        loop {
            let Some(t) = work.pending.pop() else {
                return Ok(None);
            };
            let n = match t {
                Token::Char(c, Cat::Active) => match self.eng.active_meaning(c) {
                    Some(id) if self.eng.is_macro(id) => id,
                    _ => return Ok(Some(t)),
                },
                Token::Cs(n) => n,
                Token::Char(..) => return Ok(Some(t)),
            };
            if n.name() == crate::expand::MESSAGE_END
                || crate::expand::Engine::is_conditional(n)
                || matches!(n.name(), "the" | "number")
            {
                return Ok(Some(t));
            }
            if !self.eng.expand_pending(work, n)? {
                return Ok(Some(t));
            }
        }
    }

    /// `\ifcase` becomes a chain of equality branches -- one per `\or` arm, with
    /// the `\else` arm as the tail. The VM has no jump table op, and a chain is
    /// what `\ifcase` means anyway: the nth branch for the value n.
    fn case_chain(&mut self, value: Num, mut branches: Vec<Vec<MsgOp>>) -> MsgOp {
        let default = match branches.len() {
            0 => Vec::new(),
            _ => branches.pop().unwrap_or_default(),
        };
        let mut chain = default;
        for (i, arm) in branches.into_iter().enumerate().rev() {
            chain = vec![MsgOp::If {
                left: value.clone(),
                rel: Rel::Equal,
                right: Num::Literal(i as i64),
                then_ops: arm,
                else_ops: chain,
            }];
        }
        match chain.len() {
            1 => chain.pop().unwrap_or(MsgOp::Text(String::new())),
            _ => MsgOp::Text(String::new()),
        }
    }

    /// The `\or`-separated arms of an `\ifcase`, the last being `\else`'s.
    fn msg_case_arms(&mut self, work: &mut Lexer) -> R<Vec<Vec<MsgOp>>> {
        self.enter_level()?;
        let arms = self.msg_case_arms_inner(work);
        self.depth -= 1;
        arms
    }

    fn msg_case_arms_inner(&mut self, work: &mut Lexer) -> R<Vec<Vec<MsgOp>>> {
        let mut arms = Vec::new();
        loop {
            let arm = self.msg_ops(work, &["or", "else", "fi"])?;
            arms.push(arm);
            match work.pending.pop() {
                Some(Token::Cs(n)) if n.name() == "or" => continue,
                Some(Token::Cs(n)) if n.name() == "else" => {
                    let default = self.msg_ops(work, &["fi"])?;
                    arms.push(default);
                    let _ = work.pending.pop();
                    return Ok(arms);
                }
                Some(Token::Cs(n)) if n.name() == "fi" => {
                    // No `\else`: the default arm is empty.
                    arms.push(Vec::new());
                    return Ok(arms);
                }
                _ if self.eng.halted => return Ok(arms),
                _ => return Err(TexError("Incomplete \\ifcase".into())),
            }
        }
    }

    /// The two arms of a conditional inside a message.
    fn msg_arms(&mut self, work: &mut Lexer) -> R<(Vec<MsgOp>, Vec<MsgOp>)> {
        self.enter_level()?;
        let arms = self.msg_arms_inner(work);
        self.depth -= 1;
        arms
    }

    fn msg_arms_inner(&mut self, work: &mut Lexer) -> R<(Vec<MsgOp>, Vec<MsgOp>)> {
        let negated = self.eng.take_unless();
        let then_ops = self.msg_ops(work, &["else", "fi"])?;
        let mut else_ops = Vec::new();
        match work.pending.pop() {
            Some(Token::Cs(n)) if n.name() == "else" => {
                else_ops = self.msg_ops(work, &["fi"])?;
                let _ = work.pending.pop();
            }
            Some(Token::Cs(n)) if n.name() == "fi" => {}
            _ if self.eng.halted => {}
            _ => return Err(TexError("Incomplete \\if; missing \\fi".into())),
        }
        match negated {
            true => Ok((else_ops, then_ops)),
            false => Ok((then_ops, else_ops)),
        }
    }

    /// Enter a conditional decided while lowering, as §498 does: a true test
    /// runs on into its own text; a false one skips (§494, unexpanded) to the
    /// `\else`, whose text then runs, or past the `\fi`.
    fn open_decided(
        &mut self,
        work: &mut Lexer,
        opener: CsId,
        truth: bool,
        decided: &mut usize,
    ) -> R<()> {
        let negated = self.eng.take_unless();
        let truth = truth != negated;
        if truth || self.eng.skip_to_pending(work, true)? {
            *decided += 1;
            // The conditional is open while its text runs (`\currentif...`).
            let name = crate::expand::conditional_name(opener.name());
            let branch = if truth { 1 } else { -1 };
            self.eng
                .enter_runtime_arm((name, negated, work.line()), branch);
        }
        Ok(())
    }

    /// Meet the `\else` or `\fi` of a decided conditional (§510): `\fi` ends
    /// it, `\else` ends the text that ran and skips the rest to the `\fi`.
    fn close_decided(&mut self, work: &mut Lexer, name: &str, decided: &mut usize) -> R<()> {
        if name == "else" {
            self.eng.skip_to_pending(work, false)?;
        }
        *decided -= 1;
        self.eng.leave_runtime_arm();
        Ok(())
    }

    /// A number operand inside a message body.
    /// e-TeX's `\numexpr` (`level` Int) or `\dimexpr` (Dimen), read after the
    /// primitive: luatex's `scan_expr` (scanning.c), building the expression
    /// as `Num` steps because an operand may be a register whose value only
    /// the run knows. A step whose operands are all known is taken now, so an
    /// expression of constants is a constant, and its overflow is reported
    /// here as the end of `scan_expr` reports it.
    fn expr(&mut self, lx: &mut Lexer, pending: bool, level: ExprLevel) -> R<Num> {
        let e = self.expr_level(lx, pending, level, false)?;
        Ok(match e {
            Num::Literal(crate::etex_arith::ERROR) => {
                self.eng.report(lx, "Arithmetic overflow");
                Num::Literal(0)
            }
            Num::Literal(v) => Num::Literal(v),
            e => Num::ExprEnd {
                value: Box::new(e),
                report: self.eng.report_text(lx, "Arithmetic overflow"),
            },
        })
    }

    /// One level of `scan_expr`: the expression up to the end, or, `nested`,
    /// up to the `)` that closes it. `r` and `e` are the expression so far and
    /// the operator waiting to add the term to it, `s` and `t` the term so far
    /// and the operator waiting to apply the next factor, `n` the numerator of
    /// a multiplication a division follows.
    fn expr_level(
        &mut self,
        lx: &mut Lexer,
        pending: bool,
        level: ExprLevel,
        nested: bool,
    ) -> R<Num> {
        use crate::etex_arith::{Op, INFINITY};
        let max = level.max();
        let (mut r, mut e) = (None::<Op>, Num::Literal(0));
        let (mut s, mut t, mut n) = (None::<Op>, Num::Literal(0), Num::Literal(0));
        loop {
            // A factor after `*` or `/` is an integer, whatever the level.
            let factor_level = match s {
                None => level,
                Some(_) => ExprLevel::Int,
            };
            let f = match self.eng.next_x_nonblank(lx, pending)? {
                Some(Token::Char('(', Cat::Other)) => {
                    self.expr_level(lx, pending, factor_level, true)?
                }
                other => {
                    if let Some(tok) = other {
                        lx.back_input(&[tok]);
                    }
                    match factor_level {
                        ExprLevel::Int => match pending {
                            true => self.msg_number(lx)?,
                            false => self.number(lx)?,
                        },
                        ExprLevel::Dimen => self.dimen_number(lx, pending)?,
                    }
                }
            };
            let next = self.eng.next_x_nonblank(lx, pending)?;
            let mut o = match &next {
                Some(Token::Char('+', Cat::Other)) => Some(Op::Add),
                Some(Token::Char('-', Cat::Other)) => Some(Op::Sub),
                Some(Token::Char('*', Cat::Other)) => Some(Op::Mul),
                Some(Token::Char('/', Cat::Other)) => Some(Op::Div),
                _ => None,
            };
            if o.is_none() {
                match next {
                    Some(Token::Char(')', Cat::Other)) if nested => {}
                    Some(tok) if nested => {
                        // §1381-style `back_error`: the token is read again.
                        lx.back_input(&[tok]);
                        self.eng.report(lx, "Missing ) inserted for expression");
                    }
                    Some(tok) if !self.eng.means_relax(&tok) => lx.back_input(&[tok]),
                    _ => {}
                }
            }
            // Make sure that `f` is in the proper range.
            let bound = match level == ExprLevel::Int || s.is_some() {
                true => INFINITY,
                false => max,
            };
            let f = step(Op::Check, bound, vec![f]);
            // The current term.
            match s {
                None => t = f,
                Some(Op::Mul) if o == Some(Op::Div) => {
                    n = f;
                    o = Some(Op::Scale);
                }
                Some(Op::Mul) => t = step(Op::Mul, max, vec![t, f]),
                Some(Op::Div) => t = step(Op::Div, max, vec![t, f]),
                Some(Op::Scale) => {
                    t = step(
                        Op::Scale,
                        max,
                        vec![t, std::mem::replace(&mut n, Num::Literal(0)), f],
                    )
                }
                Some(_) => unreachable!("a term waits only on *, / or a scale"),
            }
            match o {
                Some(Op::Mul | Op::Div | Op::Scale) => s = o,
                _ => {
                    s = None;
                    e = match r {
                        None => t.clone(),
                        Some(add_or_sub) => step(add_or_sub, max, vec![e, t.clone()]),
                    };
                    r = o;
                }
            }
            if o.is_none() {
                return Ok(e);
            }
        }
    }

    fn msg_number(&mut self, work: &mut Lexer) -> R<Num> {
        if let Some(slot) = self.peek_int_register(work, true)? {
            return Ok(slot);
        }
        loop {
            let Some(t) = work.pending.pop() else {
                return Err(TexError("Missing number, treated as zero".into()));
            };
            if t.is_space() {
                continue;
            }
            match &t {
                Token::Cs(n) if n.name() == crate::rust_ffi::CALL_CS => {
                    return self.rust_call(work, true);
                }
                _ => {
                    work.push_back(&[t]);
                    return Ok(Num::Literal(self.eng.scan_number_pending(work)?));
                }
            }
        }
    }
}

impl Lowerer {
    /// A comparison whose two operands both have values known now -- a
    /// constant, or a register `crate::known` has followed -- decided now.
    fn known_truth(&self, left: &Num, rel: Rel, right: &Num) -> Option<bool> {
        let (l, r) = (self.eng.known.num(left)?, self.eng.known.num(right)?);
        Some(match rel {
            Rel::Less => l < r,
            Rel::Equal => l == r,
            Rel::Greater => l > r,
        })
    }
}

/// A register read, negated when §440's signs before it said so.
fn signed(negative: bool, reg: i64) -> Num {
    match negative {
        true => Num::Neg(reg),
        false => Num::Count(reg),
    }
}
/// The two halves of a recognised tail loop: what runs, and what decides.
struct TailLoop {
    body: Vec<Token>,
    cond: Vec<Token>,
    /// The control sequence the loop leaves meaning `\relax`: plain's `\next`.
    leaves_relax: Option<CsId>,
}

/// The one character a text font joins these two into, if it joins them.
///
/// `tex.web` S1034 runs the font's ligature program over the horizontal list
/// as it is built, and these are the pairs cmr carries and every text font
/// since has kept: two hyphens are an en dash, an en dash and a hyphen an em
/// dash, two backticks a left double quote, two apostrophes a right one.
///
/// A longer run falls out of the pairs rather than needing a rule of its own,
/// which is what TeX does too. `----` is an em dash and a hyphen, because the
/// fourth hyphen finds an em dash to its left and no pair joins those;
/// `-----` is an em dash and an en dash; three backticks are a left double
/// quote and a backtick.
///
/// A LONE backtick or apostrophe is left as it is. That a cmr apostrophe draws
/// as a right single quote is the font's ENCODING rather than its ligature
/// program, and this is the ligature program.
fn ligature(prev: char, c: char) -> Option<char> {
    match (prev, c) {
        ('-', '-') => Some('\u{2013}'),
        ('\u{2013}', '-') => Some('\u{2014}'),
        ('`', '`') => Some('\u{201c}'),
        ('\'', '\'') => Some('\u{201d}'),
        _ => None,
    }
}

/// The environments whose bodies are characters rather than TeX.
///
/// LaTeX's own `verbatim` and `Verbatim`, the fancyvrb and listings families,
/// and `alltt`.
///
/// Pandoc's `Highlighting` and `Shaded` are deliberately NOT here. They look
/// like code environments and are not: Pandoc fills them with `\NormalTok{…}`
/// and friends, which are macros that have to expand for the code to come out
/// as code. Treating them as verbatim emits the markup instead of the program.
const VERBATIM_ENVIRONMENTS: &[&str] = &[
    "verbatim",
    "verbatim*",
    "Verbatim",
    "Verbatim*",
    "BVerbatim",
    "LVerbatim",
    "SaveVerbatim",
    "lstlisting",
    "minted",
    "alltt",
    "filecontents",
    "filecontents*",
];

/// The environments whose body is TeX but whose LINES are the author's.
///
/// Pandoc's `Highlighting` is the whole list: the corpus opens it 68,360 times
/// and puts 220,638 lines of code inside, its body must expand -- the
/// `\NormalTok` family is what colours the code -- and its newlines are the
/// only thing saying where a statement ends. Read as ordinary text they became
/// spaces and the breaker reflowed the program into the prose around it.
///
/// `Shaded` is deliberately not here. It wraps `Highlighting` and holds no code
/// of its own, so reading it raw as well would set the inner `\begin` and
/// `\end` as two blank code lines around every listing in the book.
const LISTING_ENVIRONMENTS: &[&str] = &["Highlighting"];

/// The environments whose body is a PICTURE rather than either.
///
/// `tikzpicture` is what a document writes; `pgfpicture` is the basic layer
/// underneath it, which `\tikzpicture` itself opens and which the same path
/// operators are stated in. Both are read raw and handed to `crate::tikz`.
///
/// `scope` is deliberately not here: it is a picture's own grouping construct
/// and lives INSIDE a body this list already takes whole, so naming it would
/// mean reading the same characters twice.
const PICTURE_ENVIRONMENTS: &[&str] = &["tikzpicture", "pgfpicture"];

/// Drop the `[...]` an environment's `\begin` carries, if it has one.
///
/// `\begin{Highlighting}[]` is how pandoc opens every code block. The stub in
/// the prelude declares the option and eats it, but a listing's body is read
/// raw before any of that runs, so without this the brackets are the first
/// code line of every listing.
fn without_environment_option(body: &str) -> &str {
    environment_option(body).1
}

/// The `[...]` an environment's `\begin` carries, and the body after it.
///
/// A picture needs both halves, not just the second: `\begin{tikzpicture}[x=
/// 0.38pt,y=0.38pt]` states the scale every coordinate in the body is read
/// against, so a body taken without its options is drawn at the wrong size.
fn environment_option(body: &str) -> (&str, &str) {
    let rest = body.trim_start_matches([' ', '\t']);
    let Some(rest) = rest.strip_prefix('[') else {
        return ("", body);
    };
    match rest.find(']') {
        Some(at) => (&rest[..at], &rest[at + 1..]),
        None => ("", body),
    }
}

impl Lowerer {
    /// The file name after `\input`, `\openin` and their kin: §526's
    /// `scan_file_name` as LuaTeX has it (filename.c). Blanks are skipped with
    /// `get_x_token`; a name in braces is its expanded list; otherwise the
    /// name is characters read with `get_x_token` up to a space or the first
    /// token that is not a character, which is put back -- `\input foo\relax`
    /// names `foo`, and `\input\jobname.aux` the job's `.aux`. A `"` quotes a
    /// stretch with spaces in it and is not part of the name (web2c's
    /// `more_name`), which is how latex.ltx asks for a file:
    /// `\openin\@inputcheck"#1" `.
    fn scan_file_name(&mut self, lx: &mut Lexer) -> R<String> {
        let first = loop {
            match self.eng.next_unexpandable(lx)? {
                Some((t, _)) if t.is_space() => continue,
                Some((t, _)) => break t,
                None => return Err(TexError("Missing file name".into())),
            }
        };
        let name = self.file_name_from(lx, first)?;
        match name.is_empty() {
            true => Err(TexError("Missing file name".into())),
            false => Ok(name),
        }
    }

    /// The rest of a file name whose first non-blank token is `first`: the
    /// braced form, or §526's loop with web2c's quotes.
    fn file_name_from(&mut self, lx: &mut Lexer, first: Token) -> R<String> {
        if let Token::Char(_, Cat::BeginGroup) = first {
            lx.push_back(&[first]);
            let body = self.eng.read_balanced_group(lx)?;
            let body = self.eng.expand_edef_body(&body)?;
            let escape = self.eng.esc();
            return Ok(body.iter().map(|t| t.to_text(escape)).collect());
        }
        let mut name = String::new();
        let mut quoted = false;
        let mut next = Some(first);
        while let Some(t) = next {
            match t {
                Token::Char('"', _) => quoted = !quoted,
                Token::Char(_, _) if t.is_space() && !quoted => break,
                Token::Char(c, _) => name.push(c),
                Token::Cs(_) => {
                    lx.push_back(&[t]);
                    break;
                }
            }
            next = self.eng.next_unexpandable(lx)?.map(|(t, _)| t);
        }
        Ok(name)
    }


    /// Find `name`, read it, and say what to print for it.
    ///
    /// TeX resolves a file name through kpathsea. texrs searches the working
    /// directory and then `TEXINPUTS` first, so a document that reads a file
    /// beside itself runs on a machine with no TeX installed at all; only when
    /// both miss does it ask `kpsewhich`, which is already how `\usepackage`
    /// finds a `.sty` (`src/latex/load.rs`). That last step is what makes
    /// `article.cls`'s own last line — `\input{size1\@ptsize.clo}` — reach the
    /// `.clo` in `texmf-dist`, and it costs nothing when the file was found
    /// beside the document. `.tex` is supplied when the name carries no
    /// extension, as tex does.
    fn open_input(&self, name: &str) -> R<(String, String)> {
        // tex's own bound, measured: a file that inputs itself opens 14 more
        // levels above the document's own and refuses the 15th with
        // `! TeX capacity exceeded, sorry [text input levels=15].`. Matching
        // the number AND the wording makes a runaway `\\input` agree with tex
        // rather than merely stopping, and it keeps the lowerer -- which
        // recurses on the host stack for a nested file -- inside a bound it can
        // survive.
        const MAX_LEVELS: usize = 15;
        if self.input_depth + 1 >= MAX_LEVELS {
            return Err(TexError(format!(
                "TeX capacity exceeded, sorry [text input levels={MAX_LEVELS}]"
            )));
        }
        // LuaTeX's `find_read_file` (id 0) and `open_read_file` callbacks,
        // when the Lua side has installed them (`crate::input_hooks`).
        let found = match crate::input_hooks::find_read_file(0, name) {
            Some(Some(file)) => file,
            Some(None) => return Err(TexError(format!("I can't find file `{name}'"))),
            None => name.to_string(),
        };
        let name = found.as_str();
        if let Some(text) = crate::input_hooks::open_read_file(name) {
            let shown = match name.starts_with('/') || name.starts_with("./") {
                true => name.to_string(),
                false => format!("./{name}"),
            };
            return Ok((shown, crate::latex::load::without_trailing_endinput(text)));
        }
        match self
            .locate_written(name)
            .or_else(|| Self::locate_input(name))
        {
            Some((shown, src)) => Ok((shown, crate::latex::load::without_trailing_endinput(src))),
            None => Err(TexError(format!("I can't find file `{name}'"))),
        }
    }

    /// A file this lowering wrote with `\immediate\write`, as it now stands,
    /// found by the name `\input` would look for it under: `.tex` supplied
    /// when the name has no extension, and the working directory written as
    /// `./`, which is where the writer put it.
    fn locate_written(&self, name: &str) -> Option<(String, String)> {
        let file = crate::write::out_file_name(name);
        let text = self.written.text(&file)?;
        let shown = match std::path::Path::new(&file).is_absolute() {
            true => file.clone(),
            false => format!("./{file}"),
        };
        Some((shown, text.to_string()))
    }

    /// The file `name` names, as `open_input` searches for it: the path to
    /// print and the text, or `None` when nothing has it. `\openin` finds a
    /// file the same way (§1275 supplies `.tex` as §537 does).
    fn locate_input(name: &str) -> Option<(String, String)> {
        let candidates = match std::path::Path::new(name).extension().is_some() {
            true => vec![name.to_string()],
            false => vec![format!("{name}.tex"), name.to_string()],
        };
        let mut dirs = vec![std::path::PathBuf::from(".")];
        if let Ok(paths) = std::env::var("TEXINPUTS") {
            dirs.extend(std::env::split_paths(&paths));
        }
        for dir in &dirs {
            for cand in &candidates {
                let full = dir.join(cand);
                if let Ok(src) = std::fs::read_to_string(&full) {
                    // tex prints the path it opened, and writes the working
                    // directory as `./` rather than as nothing.
                    let shown = match full.strip_prefix(".") {
                        Ok(rest) => format!("./{}", rest.display()),
                        Err(_) => full.display().to_string(),
                    };
                    return Some((shown, src));
                }
            }
        }
        // The TeX tree, last. `kpsewhich` is asked once per name per process and
        // the misses are remembered too, so a document that names a file nobody
        // has does not start a process per mention of it.
        for cand in &candidates {
            if let Some(path) = crate::latex::load::locate(cand) {
                if let Ok(src) = std::fs::read_to_string(&path) {
                    return Some((path.display().to_string(), src));
                }
            }
        }
        None
    }

    /// Lower an `\input` file into the stream, sharing the Lowerer's state.
    ///
    /// The same nested pass `preload` runs for the LaTeX prelude, except that
    /// the commands are kept: a preload only wants the definitions, while an
    /// `\input` file can also print.
    fn input_pass(&mut self, src: &str) -> R<Vec<Cmd>> {
        let mut lx = Lexer::new(src);
        // A file the document inputs; one a preloaded format inputs was
        // dumped in luatex, and no callback saw it then.
        if self.reading_document {
            lx.mark_as_file();
        }
        // e-TeX inserts `\everyeof` at the end of every file `\input` reads.
        lx.insert_everyeof(self.eng.everyeof_handle());
        self.block(&mut lx, None)
    }

    /// Drop the spaces that follow, for `\ignorespaces` (`tex.web` §1060).
    fn skip_spaces_in_text(&mut self, lx: &mut Lexer) {
        while let Some(t) = self.eng.take_file(lx) {
            if !t.is_space() {
                lx.push_back(&[t]);
                return;
            }
        }
    }

    /// What a group closing here saves and what it keeps, given where its body
    /// started adding to `globals`: `(saves, keeps)` for `Cmd::Group`.
    ///
    /// A register the body assigns only locally is saved on entry and restored
    /// at the `}`. One it ALSO assigns globally is kept instead: it gets a
    /// scratch slot, and every `Cmd::KeepGlobal` for it in the body -- nested
    /// groups and conditional arms included -- is told to refresh that slot.
    #[allow(clippy::type_complexity)]
    fn group_parts(&mut self, body: &mut [Cmd], mark: usize) -> R<(Vec<i64>, Vec<(i64, u16)>)> {
        let global: std::collections::HashSet<i64> = self.globals[mark.min(self.globals.len())..]
            .iter()
            .copied()
            .collect();
        let mut saves = Vec::new();
        let mut keeps = Vec::new();
        for reg in assigned_counts(body) {
            if !global.contains(&reg) {
                saves.push(reg);
                continue;
            }
            let scratch = self.next_keep_slot;
            self.next_keep_slot = scratch.checked_add(1).ok_or_else(|| {
                TexError(format!(
                    "TeX capacity exceeded, sorry [save size={}]",
                    u16::MAX - crate::compiler::TOTAL_SLOTS
                ))
            })?;
            attach_keep(body, reg, scratch);
            keeps.push((reg, scratch));
        }
        Ok((saves, keeps))
    }

    /// A group has ended: what it assigned locally is what it saved, so those
    /// registers have their old values back, and what it kept through a
    /// `\global` is the value its last global assignment left, which this does
    /// not follow.
    #[inline(never)]
    fn known_after_group(&mut self, mark: usize, saves: &[i64], keeps: &[(i64, u16)]) {
        self.eng.known.undo_to(mark, Some(saves));
        // §283: a register the group assigned `\global`ly is left with what
        // its last global assignment made it, which `crate::known` followed.
        for (reg, _) in keeps {
            let v = self.eng.known.global_value(*reg);
            self.eng.known.set(*reg, v);
        }
    }

    /// Write the `Cmd::KeepGlobal` marks the last command's `\global`
    /// assignment is owed, now that everything it lowered to is in `out`.
    #[inline]
    fn flush_keeps(&mut self, out: &mut Vec<Cmd>) {
        // Called before every token, and owed nothing before nearly all of them.
        if self.pending_keeps.is_empty() {
            return;
        }
        for reg in std::mem::take(&mut self.pending_keeps) {
            out.push(Cmd::KeepGlobal {
                reg,
                into: Vec::new(),
            });
        }
    }

    /// Record `regs` as globally assigned when the assignment about to be
    /// emitted carried `\global`, spending the prefix either way.
    ///
    /// See the `globals` field: a group must not restore a register a `\global`
    /// wrote inside it, and the only place that can be known is here, where the
    /// prefix and the register number are both in hand.
    fn note_global(&mut self, regs: &[i64]) -> bool {
        if !self.eng.take_global_prefix() {
            return false;
        }
        // Pushed every time, not once per register: a group reads only the
        // slice its own body added, so a register a LATER group writes
        // globally must be in that slice even if an earlier one wrote it too.
        self.pending_keeps.extend_from_slice(regs);
        self.globals.extend_from_slice(regs);
        true
    }

    /// luatex's `eq_word_define` and `geq_word_define` under
    /// `\tracingassigns`, for a count or dimension register or parameter:
    /// `{changing \count1=0}{into \count1=5}`, or `reassigning` when a local
    /// assignment leaves the value as it was. Known only where the lowerer
    /// follows the register (`crate::known`); a value only the run has is not
    /// traced.
    fn trace_register(&mut self, slot: i64, value: &Num, global: bool) {
        if self.eng.intpar_named("tracingassigns") <= 0 {
            return;
        }
        let (Some(old), Some(new)) = (self.eng.known.get(slot), self.eng.known.num(value)) else {
            return;
        };
        let shown = |v: i64| match slot >= crate::compiler::DIMEN_BASE {
            true => format!("{}pt", crate::dimen::print_scaled(v)),
            false => v.to_string(),
        };
        let name = self.eng.register_name_pub(slot);
        if !global && old == new {
            self.eng
                .diagnostic(&format!("{{reassigning {name}={}}}", shown(old)));
            return;
        }
        let verb = match global {
            true => "globally changing",
            false => "changing",
        };
        self.eng
            .diagnostic(&format!("{{{verb} {name}={}}}", shown(old)));
        self.eng
            .diagnostic(&format!("{{into {name}={}}}", shown(new)));
    }
}

/// One scanned `<dimen>` as the operand the VM will read.
///
/// `tex.web` §453 has two shapes: a constant the scanner finished, and a factor
/// times an INTERNAL dimension it could not, because the internal dimension is
/// a register whose value the program decides. The second becomes the product
/// itself rather than a number -- which is the whole reason `Num::Scaled`
/// exists, and the only place a `ScannedDimen` turns into one.
fn scaled_num(d: crate::dimen::ScannedDimen) -> Num {
    match d {
        crate::dimen::ScannedDimen::Constant(v) => Num::Literal(v),
        crate::dimen::ScannedDimen::Scaled { int, frac, reg } => Num::Scaled { int, frac, reg },
        crate::dimen::ScannedDimen::ByCount { factor, sign, unit } => Num::ByCount {
            factor,
            sign,
            unit: Box::new(match unit {
                crate::dimen::CountUnit::Register(r) => Num::Count(r),
                crate::dimen::CountUnit::Points(sp) => Num::Literal(sp),
            }),
        },
    }
}

/// The commands out of `cmds` that establish PROGRAM state rather than output,
/// appended to `out`.
///
/// What a preloaded preamble leaves behind is two things, and only one of them
/// used to survive: the macro table, which is the expander's and outlives the
/// lowering by itself, and the REGISTER VALUES, which live in VM slots and
/// therefore only exist as commands. This keeps the second and drops the first
/// kind of everything else -- every `\message` the preamble printed, the `(file`
/// it opened, the text it set -- because those belong to the preamble's own run
/// and the document is what the engine is reporting on.
///
/// A group and a conditional are carried with their bodies filtered the same
/// way, so a register set inside `\if@twocolumn ... \fi` still reaches the run
/// and still depends on the branch. A `Cmd::ErrorSite` is carried with them
/// because it produces nothing on its own: it says where a later arithmetic
/// error would be reported from, and dropping it would leave one unplaced.
/// A `Cmd::KeepGlobal` is carried for the same reason a group is: it is how a
/// group kept in the preamble learns what a `\global` inside it left.
fn state_only(cmds: &[Cmd], out: &mut Vec<Cmd>) {
    fn filtered(cmds: &[Cmd]) -> Vec<Cmd> {
        let mut v = Vec::new();
        state_only(cmds, &mut v);
        v
    }
    for cmd in cmds {
        match cmd {
            // A list action changes what the stomach holds -- a box register, a
            // font, a parameter it was told -- which is state the document reads.
            Cmd::SetCount(..)
            | Cmd::Arith(..)
            | Cmd::ErrorSite(_)
            | Cmd::KeepGlobal { .. }
            | Cmd::List(_)
            | Cmd::OutputRoutine(_) => out.push(cmd.clone()),
            Cmd::Group { saves, keeps, body } => out.push(Cmd::Group {
                saves: saves.clone(),
                keeps: keeps.clone(),
                body: filtered(body),
            }),
            Cmd::IfNum {
                left,
                rel,
                right,
                then_branch,
                else_branch,
            } => out.push(Cmd::IfNum {
                left: left.clone(),
                rel: *rel,
                right: right.clone(),
                then_branch: filtered(then_branch),
                else_branch: filtered(else_branch),
            }),
            Cmd::IfOdd {
                value,
                then_branch,
                else_branch,
            } => out.push(Cmd::IfOdd {
                value: value.clone(),
                then_branch: filtered(then_branch),
                else_branch: filtered(else_branch),
            }),
            Cmd::Loop {
                body,
                left,
                rel,
                right,
            } => out.push(Cmd::Loop {
                body: filtered(body),
                left: left.clone(),
                rel: *rel,
                right: right.clone(),
            }),
            // Output, or the compilation of a `\rust` block, which the document's
            // own pass compiles again where it is used.
            Cmd::Message(_)
            | Cmd::Reports(_)
            | Cmd::Write(_)
            | Cmd::OpenOut { .. }
            | Cmd::WriteOut { .. }
            | Cmd::CloseOut(_)
            | Cmd::FileClose
            | Cmd::Text(_)
            | Cmd::Color { .. }
            | Cmd::TranscriptNotice { .. }
            | Cmd::Line(_)
            | Cmd::RustCompile(_) => {}
        }
    }
}

/// A run-time `\ifcase` as commands: case i runs when the selector equals i,
/// and the last of `branches` (the `\else` text) when it equals none of them --
/// which is §509's answer for a negative selector and for one past the last
/// `\or` alike.
fn case_chain_cmds(value: Num, mut branches: Vec<Vec<Cmd>>) -> Vec<Cmd> {
    let mut chain = branches.pop().unwrap_or_default();
    for (i, arm) in branches.into_iter().enumerate().rev() {
        chain = vec![Cmd::IfNum {
            left: value.clone(),
            rel: Rel::Equal,
            right: Num::Literal(i as i64),
            then_branch: arm,
            else_branch: chain,
        }];
    }
    chain
}

/// `\advance\skip<n> by \skip<m>`: the same §1239 `glue_add`, with an operand
/// whose ORDERS are the run's too. The operand's packed orders are dispatched
/// on first, sixteen ways, so inside each arm they are constants again and the
/// register's own chain is the one `advance_glue` builds.
///
/// The natural width is added first and the operand's components are read
/// from its slots inside the arm, so `\advance\skip1\skip1` doubles: the
/// register's stretch and shrink are read before the arm writes them.
fn advance_glue_by_register(base: i64, from: i64, negative: bool) -> Vec<Cmd> {
    let mut out = vec![Cmd::Arith(Arith::Add, base, signed(negative, from))];
    let mut chain: Vec<Cmd> = Vec::new();
    for state in (0..16i64).rev() {
        let arm = glue_add_orders(
            base,
            (signed(negative, from + 1), state / 4),
            (signed(negative, from + 2), state % 4),
        );
        chain = vec![Cmd::IfNum {
            left: Num::Count(from + 3),
            rel: Rel::Equal,
            right: Num::Literal(state),
            then_branch: arm,
            else_branch: chain,
        }];
    }
    out.extend(chain);
    out
}

/// The stretch and shrink half of `glue_add`, for an operand whose two orders
/// are known while lowering and whose values are `Num`s.
///
/// Adding glue adds the natural widths and combines each infinity ORDER
/// separately: an infinite component beats any finite one however large, a
/// higher infinity beats a lower, and only equal orders add. The operand's
/// orders are known while lowering and the register's are not, so the rule
/// becomes a branch on the register's own orders.
///
/// The two orders live packed in one slot as `stretch*4 + shrink`, sixteen
/// possible values, and the result is a function of that value alone -- so the
/// chain enumerates all sixteen rather than trying to take the packed slot
/// apart with arithmetic the VM has no remainder op for. It is the same shape
/// `\ifcase` lowers to, and for the same reason.
fn glue_add_orders(base: i64, stretch: (Num, i64), shrink: (Num, i64)) -> Vec<Cmd> {
    let packed = base + 3;
    let mut out = Vec::new();
    // Built from the last state backwards, so each arm's `else` is the chain
    // for every state after it.
    let mut chain: Vec<Cmd> = Vec::new();
    for state in (0..16i64).rev() {
        let (was_stretch, was_shrink) = (state / 4, state % 4);
        let mut arm = Vec::new();
        for (slot, was, (add, order)) in [
            (base + 1, was_stretch, &stretch),
            (base + 2, was_shrink, &shrink),
        ] {
            match was.cmp(order) {
                // The register's component is the more infinite: it stands.
                std::cmp::Ordering::Greater => {}
                std::cmp::Ordering::Equal => arm.push(Cmd::Arith(Arith::Add, slot, add.clone())),
                std::cmp::Ordering::Less => arm.push(Cmd::SetCount(slot, add.clone())),
            }
        }
        let after = (was_stretch.max(stretch.1), was_shrink.max(shrink.1));
        let repacked = after.0 * 4 + after.1;
        if repacked != state {
            arm.push(Cmd::SetCount(packed, Num::Literal(repacked)));
        }
        // A state that changes nothing needs no arm of its own; the chain's
        // tail already does nothing.
        if arm.is_empty() && chain.is_empty() {
            continue;
        }
        chain = vec![Cmd::IfNum {
            left: Num::Count(packed),
            rel: Rel::Equal,
            right: Num::Literal(state),
            then_branch: arm,
            else_branch: chain,
        }];
    }
    out.extend(chain);
    out
}

/// Every count register a command block assigns, so a group knows what to save.
pub(crate) fn assigned_counts(cmds: &[Cmd]) -> Vec<i64> {
    let mut regs = Vec::new();
    fn walk(cmds: &[Cmd], regs: &mut Vec<i64>) {
        for c in cmds {
            match c {
                // A line directive, a run of the document's text, a
                // `\rust{ … }` compile, a file's closing paren, an error site,
                // the transcript notice and a global's keep mark all write no
                // register.
                Cmd::KeepGlobal { .. }
                | Cmd::Line(_)
                | Cmd::Text(_)
                | Cmd::RustCompile(_)
                | Cmd::FileClose
                | Cmd::ErrorSite(_)
                | Cmd::TranscriptNotice { .. }
                // The stomach's slots are not saved by a group (see
                // `crate::compiler::is_stomach_slot`).
                | Cmd::List(_)
                | Cmd::OutputRoutine(_) => {}
                Cmd::SetCount(r, _) | Cmd::Arith(_, r, _) => {
                    if !regs.contains(r) {
                        regs.push(*r);
                    }
                }
                Cmd::IfNum {
                    then_branch,
                    else_branch,
                    ..
                }
                | Cmd::IfOdd {
                    then_branch,
                    else_branch,
                    ..
                } => {
                    walk(then_branch, regs);
                    walk(else_branch, regs);
                }
                // A loop's body assigns exactly what its commands assign; a
                // group around one still has to save those registers.
                Cmd::Loop { body, .. } | Cmd::Color { body, .. } | Cmd::Group { body, .. } => {
                    walk(body, regs)
                }
                Cmd::Message(_)
                | Cmd::Reports(_)
                | Cmd::Write(_)
                | Cmd::OpenOut { .. }
                | Cmd::WriteOut { .. }
                | Cmd::CloseOut(_) => {}
            }
        }
    }
    walk(cmds, &mut regs);
    regs
}

/// Tell every `Cmd::KeepGlobal` for `reg` in `cmds` -- however deeply nested in
/// groups, conditional arms, loops and colour runs -- to refresh `scratch` too.
fn attach_keep(cmds: &mut [Cmd], reg: i64, scratch: u16) {
    for c in cmds {
        match c {
            Cmd::KeepGlobal { reg: r, into } if *r == reg => into.push(scratch),
            Cmd::IfNum {
                then_branch,
                else_branch,
                ..
            }
            | Cmd::IfOdd {
                then_branch,
                else_branch,
                ..
            } => {
                attach_keep(then_branch, reg, scratch);
                attach_keep(else_branch, reg, scratch);
            }
            Cmd::Loop { body, .. } | Cmd::Color { body, .. } | Cmd::Group { body, .. } => {
                attach_keep(body, reg, scratch)
            }
            _ => {}
        }
    }
}

impl Default for Lowerer {
    fn default() -> Self {
        Self::new()
    }
}

/// Re-exported so `lower` can build macros without reaching into the engine.
pub type MacroDef = Macro;
/// Kept for the catcode table's benefit.
pub type CatKind = Cat;

/// Whether the mono family is in force after a face code, given whether it was
/// before.
///
/// The lowerer tracks this for one reason: a `--` inside a monospace run is
/// two hyphens the author wrote and must not become an en dash. A face code is
/// a MODIFIER, so `\bfseries` inside a `\ttfamily` group leaves the family
/// alone and the run stays mono -- reading the code as a whole face here is
/// what would silently ligature a flag in bold code.
fn mono_after(code: char, current: Option<bool>) -> bool {
    let current = current.unwrap_or(false);
    match code {
        'm' | 'k' | 'M' | 'K' => true,
        'r' | 's' | 'n' | 'i' | 'b' | 'B' | 'j' | 'S' | 'J' => match code {
            // The series and the shape say nothing about the family.
            'i' | 'b' => current,
            _ => false,
        },
        _ => current,
    }
}

impl Lowerer {
    /// Close one open face declaration.
    fn close_face_one(&mut self, out: &mut Vec<Cmd>, face_open: &mut usize) {
        if *face_open > 0 {
            *face_open -= 1;
            self.faces.pop();
            self.push_text(out, &crate::typeset::FACE_POP.to_string());
        }
    }

    /// Close one open size declaration.
    fn close_size_one(&mut self, out: &mut Vec<Cmd>, size_open: &mut usize) {
        if *size_open > 0 {
            *size_open -= 1;
            self.push_text(out, &crate::typeset::SIZE_POP.to_string());
        }
    }
}

/// What an e-TeX expression computes: `scan_expr`'s `l`, at the two levels the
/// lowerer reads into a `Num`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ExprLevel {
    Int,
    Dimen,
}

impl ExprLevel {
    /// The largest magnitude a value at this level may have.
    fn max(self) -> i64 {
        match self {
            ExprLevel::Int => crate::etex_arith::INFINITY,
            ExprLevel::Dimen => crate::etex_arith::MAX_DIMEN,
        }
    }
}

/// One `scan_expr` step, taken now when every operand is known.
fn step(op: crate::etex_arith::Op, max: i64, args: Vec<Num>) -> Num {
    let known: Option<Vec<i64>> = args
        .iter()
        .map(|a| match a {
            Num::Literal(v) => Some(*v),
            _ => None,
        })
        .collect();
    match known {
        Some(values) => Num::Literal(crate::etex_arith::apply(op, max, &values)),
        None => Num::Expr { op, max, args },
    }
}

/// The expression a primitive opens, if it is `\numexpr` or `\dimexpr`.
fn expr_level_of(primitive: &str) -> Option<ExprLevel> {
    match primitive {
        "numexpr" => Some(ExprLevel::Int),
        "dimexpr" => Some(ExprLevel::Dimen),
        _ => None,
    }
}

/// `n`, or its magnitude for `\ifabsnum` and `\ifabsdim`.
fn magnitude(abs: bool, n: Num) -> Num {
    match abs {
        true => step(crate::etex_arith::Op::Abs, 0, vec![n]),
        false => n,
    }
}
