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
use crate::expand::{Engine, Macro, Meaning, TexError};
use crate::ir::{Arith, Cmd, MsgOp, Num, Rel};
use crate::lexer::Lexer;
use crate::token::{CsId, Token};

type R<T> = Result<T, TexError>;

pub struct Lowerer {
    pub eng: Engine,
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
    /// Every register the program has ASSIGNED so far, in any way.
    ///
    /// `tex.web` §366 expands an `\edef` body while READING it, so a
    /// conditional in the body is decided at DEFINITION time and `\the`
    /// freezes to characters. An interpreter can do that because it holds the
    /// registers; a compiler cannot, because they are VM slots -- except for
    /// one case it can prove outright, and that case is the common one. A
    /// register nothing has written is at INITEX's zero, which is exactly what
    /// `Compiler::compile` writes into every slot before the first command
    /// runs. So this records what has been written, and an `\edef` whose body
    /// reads only registers ABSENT from it is frozen the way tex freezes it.
    ///
    /// Conservative in the one direction that matters: a register is entered
    /// on the first assignment of any kind, whether or not that assignment is
    /// reached at run time. An `\edef` over a register some branch might have
    /// written therefore falls back to carrying the conditional, which is
    /// wrong in the way BUGS.md records rather than wrong in a new way.
    assigned: std::collections::HashSet<i64>,
    /// Every register a `\global` assignment has written, in the order they
    /// were lowered.
    ///
    /// A group must not restore one. `tex.web` §283's `geq_word_define` sets
    /// `xeq_level` to `level_one`, and §282's `unsave` DESTROYS a saved value
    /// whose level no longer matches -- so a register assigned globally inside
    /// a group keeps its new value at the `}`. A `Cmd::Group` expresses
    /// save-and-restore as one pair around the whole body, so "do not restore"
    /// has to mean "do not save", and the enclosing group finds out which
    /// registers those are by the slice of this list its body added.
    ///
    /// The one shape this does not reproduce is a register assigned BOTH ways
    /// in the same group with the local assignment last: tex restores the
    /// globally-set value there and this keeps the local one. A per-assignment
    /// save stack is what that would take.
    globals: Vec<i64>,
    /// Errors the expander REPORTED rather than raised, waiting to be written
    /// into the message stream in front of whatever is printed next.
    ///
    /// tex prints an error on the terminal where it happens, and `print_ln`
    /// ends that display with no character of its own — so the `\message` that
    /// follows begins at column zero and gets none of the separating space
    /// §1279 would otherwise put in front of it. Prepending the report to that
    /// message's pieces is that arrangement: the report and the message are one
    /// terminal line, not two.
    reports: Vec<String>,
    /// Whether anything was reported at all, for the line tex writes at the end
    /// of a run that had errors.
    reported: bool,
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
const MAX_LOWER_DEPTH: usize = 64;

impl Lowerer {
    pub fn new() -> Self {
        Self {
            assigned: std::collections::HashSet::new(),
            input_depth: 0,
            eng: Engine::new(),
            ended: false,
            next_scratch: 255,
            globals: Vec::new(),
            reports: Vec::new(),
            reported: false,
            fonts: crate::typeset::Families::default(),
            colours: crate::colour::Colours::new(),
            page_colour: None,
            layout: crate::typeset::Layout::default(),
            text_output: false,
            depth: 0,
            lists: Vec::new(),
            table_depth: 0,
            title_formats: std::collections::HashMap::new(),
            faces: Vec::new(),
            listing_depth: 0,
            lig: None,
            toc_depth: 2,
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
    pub fn with_text_output(mut self) -> Self {
        self.text_output = true;
        self
    }

    /// Compile a whole source to a command stream.
    pub fn lower(&mut self, src: &str) -> R<Vec<Cmd>> {
        self.lower_located(src).map_err(|(e, _line)| e)
    }

    /// Run the LaTeX prelude through this lowerer, keeping its definitions and
    /// discarding whatever commands it emitted.
    ///
    /// The prelude is all definitions, so there is nothing to keep: what
    /// matters is the macro table it leaves behind on `self.eng`, which the
    /// document is then lowered against.
    pub fn preload(&mut self, src: &str) -> R<()> {
        let out = self.lower_located(src).map(|_| ()).map_err(|(e, _)| e);
        // The prelude's own diagnostics belong to the prelude, and the document
        // is what tex is reporting on: an error the prelude recovered from must
        // not put `(see the transcript file …)` on the end of a clean run.
        self.reports.clear();
        self.reported = false;
        out
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
        match self.block(&mut lx, None) {
            Ok(mut cmds) => {
                self.close_reports(&mut cmds);
                Ok(cmds)
            }
            Err(e) => Err((e, lx.line())),
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
        if !self.reported {
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
            if !cmds.is_empty() {
                cmds.push(Cmd::TranscriptNotice);
            }
            return;
        }
        if !held.is_empty() {
            cmds.push(Cmd::Message(held));
        }
        cmds.push(Cmd::Message(vec![MsgOp::Text(
            ")(see the transcript file for additional information".into(),
        )]));
    }

    /// The reports waiting to be printed, as message pieces.
    ///
    /// Draining the expander's own list here is what puts a report in the right
    /// place: everything reported since the last message goes in front of the
    /// next one.
    fn take_reports(&mut self) -> Vec<MsgOp> {
        let fresh = self.eng.take_errors();
        self.reported = self.reported || !fresh.is_empty() || !self.reports.is_empty();
        self.reports.extend(fresh);
        std::mem::take(&mut self.reports)
            .into_iter()
            .map(MsgOp::Text)
            .collect()
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
    fn push_text_char(&mut self, out: &mut Vec<Cmd>, c: char) {
        let mut at = out.len();
        while at > 0 && matches!(out[at - 1], Cmd::Line(_)) {
            at -= 1;
        }
        let tail = at.checked_sub(1).and_then(|i| match &out[i] {
            Cmd::Text(t) => Some(t.as_str()),
            _ => None,
        });
        // A text command's face arrives as characters, so it is seen here: the
        // marker, the one character naming the face, and the closing marker.
        if c == crate::typeset::FACE_POP {
            self.faces.pop();
        } else if tail.is_some_and(|t| t.ends_with(crate::typeset::FACE_PUSH)) {
            self.faces.push(c == crate::typeset::Face::Mono.code());
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
                .filter(|p| tail.is_some_and(|t| t.ends_with(*p)))
                .and_then(|p| ligature(p, c)),
        };
        let mut c = c;
        if let Some(j) = joined {
            if let Some(Cmd::Text(t)) = at.checked_sub(1).and_then(|i| out.get_mut(i)) {
                t.pop();
            }
            c = j;
        }
        self.lig = (!code).then_some(c);
        match at.checked_sub(1).and_then(|i| out.get_mut(i)) {
            Some(Cmd::Text(t)) => t.push(c),
            _ => out.push(Cmd::Text(c.to_string())),
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

    fn block(&mut self, lx: &mut Lexer, stop: Option<&[&str]>) -> R<Vec<Cmd>> {
        self.depth += 1;
        if self.depth > MAX_LOWER_DEPTH {
            self.depth -= 1;
            return Err(TexError(
                "TeX capacity exceeded, sorry [input stack size=100]".into(),
            ));
        }
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
        let mut face_open = false;
        // A size declaration is scoped to its group exactly as a face is, and
        // is closed everywhere a face is closed.
        let mut size_open = false;
        // The line the last directive named, so one is emitted per line rather
        // than per command: a `\count` assignment and the `\message` beside it
        // share a line and need only one.
        let mut marked = 0u32;
        while let Some(tok) = lx.next_token(&self.eng.cats) {
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
                if crate::math::lower_math(&mut self.eng, size, measure, lx, &tok, &mut out)? {
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
                        self.eng.compile_time_begin_group();
                        let mark = self.globals.len();
                        let body = self.block(lx, Some(&["\u{0}endgroup"]))?;
                        self.eng.compile_time_end_group()?;
                        // Whatever `\aftergroup` held for this group is read
                        // next, which is what "after the group" means.
                        let after = self.eng.take_after_group();
                        lx.push_back(&after);
                        let saves = self.saved_by_group(&body, mark);
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
                        if saves.is_empty() && only_text {
                            for cmd in body {
                                match (&cmd, out.last_mut()) {
                                    (Cmd::Text(t), Some(Cmd::Text(prev))) => prev.push_str(t),
                                    _ => out.push(cmd),
                                }
                            }
                        } else {
                            out.push(Cmd::Group { saves, body });
                        }
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
                                false => out.push(Cmd::Text(body)),
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
            // Only the size is scoped here. The face has always leaked the
            // same way and is left alone: with no `\setmonofont` the mono face
            // IS the main face, so nothing shows it, and changing what every
            // `\ttfamily` in the corpus reaches is not a thing to do in the
            // same commit as this.
            if self.text_output && name.name() == "end" {
                self.close_size(&mut out, &mut size_open);
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
                continue;
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
                    out.push(self.lower_tail_loop(parts)?);
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
            match name.name() {
                "end" => {
                    self.ended = true;
                    break;
                }
                // Compile-time: these change how the REST of the file reads.
                "def" | "gdef" => self.eng.compile_time_def(lx, name.name())?,
                "catcode" => self.eng.compile_time_catcode(lx)?,
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
                "count" => {
                    let reg = self.eng.scan_number_file(lx)?;
                    self.eng.skip_equals_file(lx)?;
                    let v = self.number(lx)?;
                    self.note_global(&[reg]);
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
                    let reg = self.eng.scan_number_file(lx)?;
                    self.eng.do_toks_assign(lx, reg)?;
                }
                // `\skip0=1pt plus 2pt minus 3pt`. Four slots, written
                // together, so the whole glue is one assignment.
                // `\muskip0=3mu plus 1mu`. The same four slots in the second
                // glue file: what differs is the unit the right-hand side is
                // read in, and `glue_assign` reads that off the base.
                "skip" | "muskip" => {
                    let file = match name.name() {
                        "muskip" => crate::compiler::MUSKIP_BASE,
                        _ => crate::compiler::SKIP_BASE,
                    };
                    let reg = self.eng.scan_number_file(lx)?;
                    self.eng.skip_equals_file(lx)?;
                    let base = file + reg * crate::compiler::SKIP_STRIDE;
                    self.note_global(&[base, base + 1, base + 2, base + 3]);
                    out.extend(self.glue_assign(lx, base)?);
                }
                "dimen" => {
                    let reg = self.eng.scan_number_file(lx)?;
                    self.eng.skip_equals_file(lx)?;
                    // `\dimen1=\dimen0` copies a register whose value is only
                    // known at run time, and `\dimen1=\skip0` takes a glue's
                    // natural width (§430's coercion); reading a constant here
                    // refused both.
                    let v = self.dimen_number(lx, false)?;
                    let slot = crate::compiler::DIMEN_BASE + reg;
                    self.note_global(&[slot]);
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
                    if reg >= crate::compiler::SKIP_BASE {
                        self.note_global(&[reg, reg + 1, reg + 2, reg + 3]);
                        let cmds = self.glue_assign(lx, reg)?;
                        out.extend(cmds);
                    } else {
                        let v = match reg >= crate::compiler::DIMEN_BASE {
                            true => self.dimen_number(lx, false)?,
                            false => self.number(lx)?,
                        };
                        self.note_global(&[reg]);
                        out.push(Cmd::SetCount(reg, v));
                    }
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
                        _ => match what.name() {
                            "count" => self.eng.scan_number_file(lx)?,
                            "dimen" => {
                                crate::compiler::DIMEN_BASE + self.eng.scan_number_file(lx)?
                            }
                            "skip" => {
                                crate::compiler::SKIP_BASE
                                    + self.eng.scan_number_file(lx)? * crate::compiler::SKIP_STRIDE
                            }
                            "muskip" => {
                                crate::compiler::MUSKIP_BASE
                                    + self.eng.scan_number_file(lx)? * crate::compiler::SKIP_STRIDE
                            }
                            other => {
                                return Err(TexError(format!("Unsupported register \\{other}")))
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
                    let glue = reg >= crate::compiler::SKIP_BASE;
                    let dimen = !glue && reg >= crate::compiler::DIMEN_BASE;
                    if glue {
                        self.note_global(&[reg, reg + 1, reg + 2, reg + 3]);
                    } else {
                        self.note_global(&[reg]);
                    }
                    match (glue, matches!(op, Arith::Add)) {
                        (true, true) => {
                            let (nat, st, sto, sh, sho) = match reg >= crate::compiler::MUSKIP_BASE
                            {
                                true => self.eng.scan_muglue(lx)?,
                                false => self.eng.scan_glue(lx)?,
                            };
                            out.extend(advance_glue(
                                reg,
                                crate::glue::Glue {
                                    natural: nat,
                                    stretch: st,
                                    stretch_order: sto,
                                    shrink: sh,
                                    shrink_order: sho,
                                },
                            ));
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
                            let v = match dimen && matches!(op, Arith::Add) {
                                true => self.dimen_number(lx, false)?,
                                false => self.number(lx)?,
                            };
                            self.note_error_site(&mut out, lx, op);
                            out.push(Cmd::Arith(op, reg, v));
                        }
                    }
                }
                "message" => {
                    let parts = self.message_parts(lx)?;
                    // Anything reported so far is printed HERE, in front of the
                    // message and with nothing between them: see `reports`.
                    let mut ops = self.take_reports();
                    ops.extend(parts);
                    out.push(Cmd::Message(ops));
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
                "ifnum" => {
                    let left = self.number(lx)?;
                    let rel = match self.eng.read_relation_file(lx)? {
                        '<' => Rel::Less,
                        '>' => Rel::Greater,
                        _ => Rel::Equal,
                    };
                    let right = self.number(lx)?;
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
                "ifdim" => {
                    let left = self.dimen_number(lx, false)?;
                    let rel = match self.eng.read_relation_file(lx)? {
                        '<' => Rel::Less,
                        '>' => Rel::Greater,
                        _ => Rel::Equal,
                    };
                    let right = self.dimen_number(lx, false)?;
                    let (then_branch, else_branch) = self.arms(lx)?;
                    out.push(Cmd::IfNum {
                        left,
                        rel,
                        right,
                        then_branch,
                        else_branch,
                    });
                }
                "ifodd" => {
                    let value = self.number(lx)?;
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
                | "toksdef" => self.eng.compile_time_numeric_def(lx, name.name())?,
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
                    if let Some(cmd) = self.edef_snapshot(lx)? {
                        out.push(cmd);
                    }
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
                    self.eng.compile_time_begin_group();
                    let mark = self.globals.len();
                    let body = self.block(lx, Some(&["endgroup"]))?;
                    // The `\endgroup` that stopped the block was pushed back
                    // for this arm to consume; at end of input there is none.
                    if !self.ended {
                        let _ = lx.next_token(&self.eng.cats);
                    }
                    self.eng.compile_time_end_group()?;
                    let after = self.eng.take_after_group();
                    lx.push_back(&after);
                    let saves = self.saved_by_group(&body, mark);
                    // No register written means nothing is left for run time --
                    // the macro table was already scoped above -- so the body
                    // is spliced in rather than wrapped, which also keeps a
                    // stretch of text one constant instead of two.
                    if saves.is_empty() {
                        for cmd in body {
                            match (&cmd, out.last_mut()) {
                                (Cmd::Text(t), Some(Cmd::Text(prev))) => prev.push_str(t),
                                _ => out.push(cmd),
                            }
                        }
                    } else {
                        out.push(Cmd::Group { saves, body });
                    }
                    // `\end` inside the group stops the whole run, as it does
                    // inside an `\input` file.
                    if self.ended {
                        break;
                    }
                }
                "endgroup" => self.eng.compile_time_end_group()?,
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
                    self.eng.after_group(lx, t);
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
                "global" => self.eng.set_global_prefix(true),
                // The other two definition prefixes. Like `\global` they set a
                // flag the definition that follows reads and spends.
                "long" => self.eng.set_long_prefix(true),
                "outer" => self.eng.set_outer_prefix(true),
                "protected" => self.eng.set_protected_prefix(true),
                // `\unless\ifnum ...` runs the ELSE arm when the test holds.
                "unless" => self.eng.set_unless(true),
                "relax" => {}
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
                other => {
                    // TeX's loop idiom -- a macro whose last act is to call
                    // itself under a test -- becomes a real loop rather than an
                    // inline copy. Inlining it cannot terminate: the copy holds
                    // the call that gets copied.
                    if let Some(parts) = self.tail_loop(name) {
                        let cmd = self.lower_tail_loop(parts)?;
                        out.push(cmd);
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
                    return Err(TexError(format!("Undefined control sequence \\{other}")));
                }
            }
            // `tex.web` §1269: a token `\afterassignment` held is put back once
            // the ASSIGNMENT that followed has finished -- at the end of
            // `prefixed_command` and nowhere else, which is why this is here
            // rather than at the top of the loop.
            let assigned = crate::expand::Engine::is_assignment(name.name())
                || matches!(
                    self.eng.numeric_cs(name),
                    Some(crate::expand::NumericCs::Register(_))
                )
                || self.eng.toks_cs(name).is_some();
            if let Some(t) = self.eng.take_after_assignment(assigned) {
                lx.push_back(&[t]);
            }
        }
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
        if let Some(site) = lx.context() {
            out.push(Cmd::ErrorSite(site));
        }
    }

    /// Whether every register `body` READS is one nothing has assigned, so its
    /// value is provably INITEX's zero.
    ///
    /// The scan is over the body AFTER macro expansion, so a register a macro
    /// reads is visible here as `\count<n>`. Anything it cannot prove -- a
    /// register number that is not a constant, a name it cannot resolve, a
    /// scan that fails -- answers NO, which costs only the older behaviour.
    fn reads_only_untouched_registers(&mut self, body: &[Token]) -> bool {
        let mut work = Lexer::new("");
        work.push_back(body);
        while let Some(t) = work.pending.pop() {
            let Token::Cs(n) = &t else { continue };
            let n = *n;
            // A name `\countdef` and friends gave a register IS that register.
            if let Some(crate::expand::NumericCs::Register(r)) = self.eng.numeric_cs(n) {
                if self.assigned.contains(&r) {
                    return false;
                }
                continue;
            }
            let file = match n.name() {
                "count" => 0,
                "dimen" => crate::compiler::DIMEN_BASE,
                "skip" => crate::compiler::SKIP_BASE,
                "muskip" => crate::compiler::MUSKIP_BASE,
                _ => continue,
            };
            let stride = match n.name() {
                "skip" | "muskip" => crate::compiler::SKIP_STRIDE,
                _ => 1,
            };
            // Only a CONSTANT register number can be proved: `\count\count0`
            // names a register the run picks.
            let next = work.pending.iter().rev().find(|t| !t.is_space());
            if !matches!(next, Some(Token::Char(c, _)) if c.is_ascii_digit()) {
                return false;
            }
            let Ok(reg) = self.eng.scan_number_pending(&mut work) else {
                return false;
            };
            let base = file + reg * stride;
            if (0..stride).any(|i| self.assigned.contains(&(base + i))) {
                return false;
            }
        }
        true
    }

    /// `\edef\x{...\the\count<n>...}` freezes the register's CURRENT value.
    ///
    /// The value lives in a VM slot, so "now" is run time. The snapshot is
    /// written to a scratch register at this point in the program and the macro
    /// is defined to read THAT, which is what makes a later use see the frozen
    /// value rather than the live one.
    fn edef_snapshot(&mut self, lx: &mut Lexer) -> R<Option<Cmd>> {
        let Some(Token::Cs(name)) = lx.next_token(&self.eng.cats) else {
            return Err(TexError("Missing control sequence inserted".into()));
        };
        // The parameter text, exactly as `\def` reads it. `\edef` differs from
        // `\def` only in WHEN the body is expanded; dropping the parameters
        // here left `\edef\pair#1,#2.{…}` matching nothing and its delimiters
        // landing in the output. Found by `parity-fuzz`.
        let mut params: Vec<Token> = Vec::new();
        loop {
            let Some(t) = lx.next_token(&self.eng.cats) else {
                return Err(TexError("Runaway definition".into()));
            };
            if matches!(t, Token::Char(_, Cat::BeginGroup)) {
                break;
            }
            params.push(t);
        }
        let raw = self.eng.read_balanced_pub(lx)?;
        // `\edef` expands its body NOW, which is the whole difference from
        // `\def`. Only the macro calls: `\the\count<n>` has to reach the walk
        // below as tokens so it can be snapshotted into a scratch register.
        let body = self.eng.expand_macros_only(&raw)?;
        // `tex.web` §366 expands the body the WHOLE way while reading it, so a
        // conditional in it is decided now and `\the` becomes §478's
        // characters. That needs the registers, and the lowerer has them for
        // exactly one case: a register nothing has assigned is still at
        // INITEX's zero. When the body reads only those, freeze it as tex does.
        //
        // On any error the older path runs instead, so this can only ever
        // freeze MORE bodies correctly -- never refuse one that used to work.
        // The PROOF is read off the macro-expanded body, where a register a
        // macro reads is visible; the freezing runs on the body as written, so
        // that `\unexpanded` still stops it.
        if self.reads_only_untouched_registers(&body) {
            if let Ok(frozen) = self.eng.expand_edef_body(&raw) {
                self.eng.define_macro_with_params(name, params, frozen)?;
                return Ok(None);
            }
        }
        // Find `\the\count<n>` in the body; anything else stays literal.
        let mut work = Lexer::new("");
        work.push_back(&body);
        let mut new_body: Vec<Token> = Vec::new();
        let mut cmd = None;
        while let Some(t) = work.pending.pop() {
            match &t {
                Token::Cs(n) if n.name() == "the" => {
                    match work.pending.pop() {
                        Some(Token::Cs(w)) if w.name() == "count" => {}
                        _ => return Err(TexError("Unsupported \\edef body".into())),
                    }
                    let reg = self.eng.scan_number_pending(&mut work)?;
                    let scratch = self.next_scratch;
                    self.next_scratch -= 1;
                    self.assigned.insert(scratch);
                    cmd = Some(Cmd::SetCount(scratch, Num::Count(reg)));
                    new_body.push(Token::cs("the"));
                    new_body.push(Token::cs("count"));
                    for ch in scratch.to_string().chars() {
                        new_body.push(Token::Char(ch, Cat::Other));
                    }
                }
                other => new_body.push(*other),
            }
        }
        self.eng.define_macro_with_params(name, params, new_body)?;
        Ok(cmd)
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
                        let bold = crate::typeset::Face::Bold;
                        self.push_text(
                            out,
                            &format!("{}{}", crate::typeset::FACE_PUSH, bold.code()),
                        );
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
        face_open: &mut bool,
    ) -> bool {
        use crate::typeset::Face;
        let face = match name.name() {
            "ttfamily" => Face::Mono,
            "bfseries" => Face::Bold,
            "itshape" => Face::Italic,
            // Back to the body face. `\normalfont` is the one a heading writes
            // to undo everything, and the two family switches are what a
            // document says to leave the mono face inside a group that set it.
            "sffamily" => Face::Sans,
            // Back to the body face. `\normalfont` is the one a heading writes
            // to undo everything, and `\rmfamily` is what a document says to
            // leave a display family inside a group that set it.
            "rmfamily" | "normalfont" => Face::Main,
            _ => return false,
        };
        // A second switch in one group REPLACES the first, as a second `\color`
        // does: there is one face in force, not a stack of them per group.
        self.close_face(out, face_open);
        self.faces.push(matches!(face, Face::Mono));
        self.push_text(
            out,
            &format!("{}{}", crate::typeset::FACE_PUSH, face.code()),
        );
        *face_open = true;
        true
    }

    /// End a face declaration that is still in force, if there is one.
    fn close_face(&mut self, out: &mut Vec<Cmd>, face_open: &mut bool) {
        if std::mem::take(face_open) {
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
        size_open: &mut bool,
    ) -> bool {
        let Some(size) = crate::typeset::size_step(name.name(), self.layout.size) else {
            return false;
        };
        // A second size in one group REPLACES the first, as a second face
        // does: there is one size in force, not a stack of them per group.
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
        *size_open = true;
        true
    }

    /// End a size declaration that is still in force, if there is one.
    fn close_size(&self, out: &mut Vec<Cmd>, size_open: &mut bool) {
        if std::mem::take(size_open) {
            self.push_text(out, &crate::typeset::SIZE_POP.to_string());
        }
    }

    /// An optional `[model]`, as its text.
    fn optional_text(&mut self, lx: &mut Lexer) -> R<Option<String>> {
        Ok(self.eng.read_optional_bracket(lx)?.map(|tokens| {
            tokens
                .iter()
                .map(|t| t.to_text(self.eng.escape))
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
        for line in body.lines() {
            let mut lx = Lexer::new(line);
            self.lower_lexer_into(&mut lx, out)?;
            self.push_text(out, &crate::typeset::LISTING_BREAK.to_string());
        }
        self.listing_depth -= 1;
        self.push_text(out, "\n\n");
        Ok(())
    }

    /// Append text to the run in progress, looking past line directives.
    fn push_text(&self, out: &mut Vec<Cmd>, text: &str) {
        let mut at = out.len();
        while at > 0 && matches!(out[at - 1], Cmd::Line(_)) {
            at -= 1;
        }
        match at.checked_sub(1).and_then(|i| out.get_mut(i)) {
            Some(Cmd::Text(t)) => t.push_str(text),
            _ => out.push(Cmd::Text(text.to_string())),
        }
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
            let (options, body) = environment_option(&body);
            self.push_text(out, &crate::typeset::picture_mark(options, body));
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
        Some(TailLoop { body, cond })
    }

    /// Lower a recognised tail loop: the body as a block, the guard as a test.
    fn lower_tail_loop(&mut self, parts: TailLoop) -> R<Cmd> {
        let mut body_lx = Lexer::new("");
        body_lx.push_back(&parts.body);
        let body = self.block(&mut body_lx, None)?;

        let mut cond_lx = Lexer::new("");
        cond_lx.push_back(&parts.cond);
        let left = self.number(&mut cond_lx)?;
        let rel = match self.eng.read_relation_file(&mut cond_lx)? {
            '<' => Rel::Less,
            '>' => Rel::Greater,
            _ => Rel::Equal,
        };
        let right = self.number(&mut cond_lx)?;
        Ok(Cmd::Loop {
            body,
            left,
            rel,
            right,
        })
    }

    fn arms(&mut self, lx: &mut Lexer) -> R<(Vec<Cmd>, Vec<Cmd>)> {
        let negated = self.eng.take_unless();
        let then_branch = self.block(lx, Some(&["else", "fi"]))?;
        let mut else_branch = Vec::new();
        match lx.next_token(&self.eng.cats) {
            Some(Token::Cs(n)) if n.name() == "else" => {
                else_branch = self.block(lx, Some(&["fi"]))?;
                // Consume the `\fi`.
                let _ = lx.next_token(&self.eng.cats);
            }
            Some(Token::Cs(n)) if n.name() == "fi" => {}
            other => {
                if let Some(t) = other {
                    lx.push_back(&[t]);
                }
                return Err(TexError("Incomplete \\ifnum; missing \\fi".into()));
            }
        }
        // A negated conditional is this one with its arms exchanged.
        match negated {
            true => Ok((else_branch, then_branch)),
            false => Ok((then_branch, else_branch)),
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

    /// A number operand: a literal, `\count<n>` read at run time, or a call into
    /// a compiled `\rust{ … }` block.
    fn number(&mut self, lx: &mut Lexer) -> R<Num> {
        // Peek for `\count`, which becomes a slot read rather than a constant.
        loop {
            let Some(t) = lx.next_token(&self.eng.cats) else {
                return Err(TexError("Missing number, treated as zero".into()));
            };
            if t.is_space() {
                continue;
            }
            match &t {
                Token::Cs(n) if n.name() == "count" => {
                    let reg = self.eng.scan_number_file(lx)?;
                    return Ok(Num::Count(reg));
                }
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
    fn glue_assign(&mut self, lx: &mut Lexer, base: i64) -> R<Vec<Cmd>> {
        // Which glue file the destination is in decides the units the source is
        // read in: `tex.web` §1228 assigns a `mu_val` from a `mu_val` and a
        // `glue_val` from a `glue_val`, and never mixes them.
        let mu = base >= crate::compiler::MUSKIP_BASE;
        if let Some(from) = self.peek_glue_register(lx, mu)? {
            return Ok((0..crate::compiler::SKIP_STRIDE)
                .map(|i| Cmd::SetCount(base + i, Num::Count(from + i)))
                .collect());
        }
        let (nat, st, sto, sh, sho) = match mu {
            true => self.eng.scan_muglue(lx)?,
            false => self.eng.scan_glue(lx)?,
        };
        Ok([nat, st, sh, sto * 4 + sho]
            .into_iter()
            .enumerate()
            .map(|(i, v)| Cmd::SetCount(base + i as i64, Num::Literal(v)))
            .collect())
    }

    /// The base slot of a glue register standing where a glue is wanted, if
    /// that is what comes next; nothing is consumed otherwise.
    fn peek_glue_register(&mut self, lx: &mut Lexer, mu: bool) -> R<Option<i64>> {
        let (want, file) = match mu {
            true => ("muskip", crate::compiler::MUSKIP_BASE),
            false => ("skip", crate::compiler::SKIP_BASE),
        };
        let mut eaten = Vec::new();
        loop {
            let Some(t) = self.eng.take_file(lx) else {
                lx.push_back(&eaten);
                return Ok(None);
            };
            if t.is_space() {
                eaten.push(t);
                continue;
            }
            if let Token::Cs(n) = &t {
                if n.name() == want {
                    return Ok(Some(
                        file + self.eng.scan_number_file(lx)? * crate::compiler::SKIP_STRIDE,
                    ));
                }
                if let Some(crate::expand::NumericCs::Register(r)) = self.eng.numeric_cs(*n) {
                    // A `\skipdef` name is a glue register and a `\muskipdef`
                    // name is a math one; each stands only where its own kind
                    // does.
                    let is_mu = r >= crate::compiler::MUSKIP_BASE;
                    if r >= crate::compiler::SKIP_BASE && is_mu == mu {
                        return Ok(Some(r));
                    }
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
                return Ok(Num::Literal(self.eng.scan_dimen_any(lx, pending)?));
            };
            let n = *n;
            match n.name() {
                "dimen" => {
                    let reg = self.eng.scan_number_any(lx, pending)?;
                    return Ok(Num::Count(crate::compiler::DIMEN_BASE + reg));
                }
                // A glue where a dimension is wanted is its NATURAL width
                // (`tex.web` §430's coercion), which is the first of its slots.
                "skip" => {
                    let reg = self.eng.scan_number_any(lx, pending)?;
                    return Ok(Num::Count(
                        crate::compiler::SKIP_BASE + reg * crate::compiler::SKIP_STRIDE,
                    ));
                }
                _ => {}
            }
            // A `\dimendef` or `\skipdef` name is that register, in every
            // position the spelt-out form works.
            if let Some(crate::expand::NumericCs::Register(r)) = self.eng.numeric_cs(n) {
                if r >= crate::compiler::DIMEN_BASE {
                    return Ok(Num::Count(r));
                }
            }
            lx.push_back(&[t]);
            return Ok(Num::Literal(self.eng.scan_dimen_any(lx, pending)?));
        }
    }

    /// `\message{...}` lowered to the steps that build it at run time.
    ///
    /// The body is walked as a token list. Macros and `\csname` resolve here --
    /// they depend on the macro table, which is a frontend fact. `\the\count`
    /// and conditionals do not: they read VM slots, so they become a slot read
    /// and a real branch.
    fn message_parts(&mut self, lx: &mut Lexer) -> R<Vec<MsgOp>> {
        let body = self.eng.read_message_body(lx)?;
        let mut work = Lexer::new("");
        work.push_back(&body);
        self.msg_ops(&mut work, &[])
    }

    /// Walk a message token list into build steps, stopping at `stop`.
    fn msg_ops(&mut self, work: &mut Lexer, stop: &[&str]) -> R<Vec<MsgOp>> {
        let mut out: Vec<MsgOp> = Vec::new();
        let mut text = String::new();
        macro_rules! flush {
            () => {
                if !text.is_empty() {
                    out.push(MsgOp::Text(std::mem::take(&mut text)));
                }
            };
        }
        while let Some(t) = work.pending.pop() {
            // Active characters are commands here too: `\message{~}` runs `~`.
            let t = match &t {
                Token::Char(c, Cat::Active) => match self.eng.active_meaning(*c) {
                    Some(id) => Token::Cs(id),
                    None => t,
                },
                _ => t,
            };
            let Token::Cs(n) = &t else {
                text.push_str(&t.to_text(self.eng.escape));
                continue;
            };
            let n = *n;
            if stop.contains(&n.name()) {
                work.push_back(&[Token::Cs(n)]);
                break;
            }
            match n.name() {
                "the" | "number" => {
                    flush!();
                    if n.name() == "the" {
                        match work.pending.pop() {
                            // `\the\dimen0` is written as a DIMENSION -- 1.0pt,
                            // not 65536 -- which is the whole difference
                            // between the two registers at this level.
                            // `\the\toks0` is the token list as text, and it
                            // is known while lowering because the table is.
                            Some(Token::Cs(w)) if w.name() == "toks" => {
                                let reg = self.eng.scan_number_pending(work)?;
                                text.push_str(&self.eng.toks_text(reg));
                                continue;
                            }
                            Some(Token::Cs(w)) if self.eng.toks_cs(w).is_some() => {
                                let reg = self.eng.toks_cs(w).expect("just matched");
                                text.push_str(&self.eng.toks_text(reg));
                                continue;
                            }
                            Some(Token::Cs(w)) if w.name() == "skip" || w.name() == "muskip" => {
                                let mu = w.name() == "muskip";
                                let file = match mu {
                                    true => crate::compiler::MUSKIP_BASE,
                                    false => crate::compiler::SKIP_BASE,
                                };
                                let reg = self.eng.scan_number_pending(work)?;
                                let base = file + reg * crate::compiler::SKIP_STRIDE;
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
                                let reg = self.eng.scan_number_pending(work)?;
                                out.push(MsgOp::Dimen(Num::Count(
                                    crate::compiler::DIMEN_BASE + reg,
                                )));
                                continue;
                            }
                            Some(Token::Cs(w)) if w.name() == "count" => {}
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
                                    out.push(if r >= crate::compiler::MUSKIP_BASE {
                                        MsgOp::MuGlue(slots)
                                    } else if r >= crate::compiler::SKIP_BASE {
                                        MsgOp::Glue(slots)
                                    } else if r >= crate::compiler::DIMEN_BASE {
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
                        let reg = self.eng.scan_number_pending(work)?;
                        out.push(MsgOp::Number(Num::Count(reg)));
                        continue;
                    }
                    // `\number` takes either a register or a literal.
                    // `\number\skip0` gives the natural component only.
                    if matches!(work.pending.last(), Some(Token::Cs(w)) if w.name() == "skip") {
                        let _ = work.pending.pop();
                        let reg = self.eng.scan_number_pending(work)?;
                        out.push(MsgOp::Number(Num::Count(
                            crate::compiler::SKIP_BASE + reg * crate::compiler::SKIP_STRIDE,
                        )));
                        continue;
                    }
                    if matches!(work.pending.last(), Some(Token::Cs(w)) if w.name() == "muskip") {
                        let _ = work.pending.pop();
                        let reg = self.eng.scan_number_pending(work)?;
                        out.push(MsgOp::Number(Num::Count(
                            crate::compiler::MUSKIP_BASE + reg * crate::compiler::SKIP_STRIDE,
                        )));
                        continue;
                    }
                    // `\number\dimen0` gives the scaled points, unrendered.
                    if matches!(work.pending.last(), Some(Token::Cs(w)) if w.name() == "dimen") {
                        let _ = work.pending.pop();
                        let reg = self.eng.scan_number_pending(work)?;
                        out.push(MsgOp::Number(Num::Count(crate::compiler::DIMEN_BASE + reg)));
                        continue;
                    }
                    let is_reg =
                        matches!(work.pending.last(), Some(Token::Cs(w)) if w.name() == "count");
                    if is_reg {
                        let _ = work.pending.pop();
                        let reg = self.eng.scan_number_pending(work)?;
                        out.push(MsgOp::Number(Num::Count(reg)));
                    } else {
                        let v = self.eng.scan_number_pending(work)?;
                        text.push_str(&v.to_string());
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
                            other => other.to_text(self.eng.escape),
                        });
                    }
                }
                // `\Uchar<number>` is the character with that code.
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
                    let group = self.eng.read_group_tokens(work)?;
                    text.push_str(&self.eng.tokens_text(&group));
                }
                "string" => {
                    if let Some(next) = work.pending.pop() {
                        text.push_str(&match &next {
                            Token::Cs(cs) => format!("{}{}", self.eng.escape, cs.name()),
                            other => other.to_text(self.eng.escape),
                        });
                    }
                }
                // `\meaning` is expandable, so it reaches a message body as
                // readily as running text: what §296 would print for the next
                // token, as characters.
                "meaning" => {
                    if let Some(next) = work.pending.pop() {
                        text.push_str(&self.eng.meaning_text(&next));
                    }
                }
                // `\expanded{...}` puts the group back to be expanded, which
                // is what this loop does to everything it meets anyway -- so
                // the primitive is the wrapper coming off.
                "expanded" => {
                    let group = self.eng.read_group_tokens(work)?;
                    work.push_back(&group);
                }
                // `\unexpanded{...}` writes the tokens as they stand. In a
                // message that is the same rendering `\detokenize` gives, and
                // the two only part company inside an `\edef`, where these
                // tokens survive as tokens.
                "unexpanded" => {
                    let group = self.eng.read_group_tokens(work)?;
                    text.push_str(&self.eng.tokens_text(&group));
                }
                // `\begincsname` is `\csname` that does not define what it does
                // not find: an unknown name expands to nothing instead of to
                // `\relax`.
                "begincsname" => {
                    let built = self.eng.read_csname_pending(work)?;
                    let id = crate::token::CsId::intern(&built);
                    if self.eng.meanings.contains_key(&id) {
                        work.push_back(&[Token::Cs(id)]);
                    }
                }
                "csname" => {
                    // The name is built from text and macros, all compile-time.
                    let built = self.eng.read_csname_pending(work)?;
                    work.push_back(&[Token::cs(&built)]);
                }
                "expandafter" => {
                    let Some(held) = work.pending.pop() else {
                        return Err(TexError("Missing token after \\expandafter".into()));
                    };
                    let Some(next) = work.pending.pop() else {
                        return Err(TexError("Missing token after \\expandafter".into()));
                    };
                    match &next {
                        Token::Cs(m) if self.eng.is_macro(*m) => {
                            let m = *m;
                            self.eng.expand_macro_pending(work, m)?;
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
                    let (t_ops, e_ops) = self.msg_arms(work)?;
                    flush!();
                    out.extend(if taken { t_ops } else { e_ops });
                }
                "ifx" => {
                    let a = work.pending.pop();
                    let b = work.pending.pop();
                    let same = self.eng.meanings_equal_pub(a.as_ref(), b.as_ref());
                    let (t_ops, e_ops) = self.msg_arms(work)?;
                    flush!();
                    out.extend(if same { t_ops } else { e_ops });
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
                "ifdefined" | "ifcsname" | "ifvoid" | "ifhbox" | "ifvbox" => {
                    let truth = match n.name() {
                        "ifcsname" => {
                            let built = self.eng.read_csname_pending(work)?;
                            let id = crate::token::CsId::intern(&built);
                            self.eng.meanings.contains_key(&id)
                        }
                        "ifdefined" => match work.pending.pop() {
                            Some(Token::Cs(cs)) => self.eng.meanings.contains_key(&cs),
                            _ => false,
                        },
                        other => {
                            let _reg = self.eng.scan_number_pending(work)?;
                            other == "ifvoid"
                        }
                    };
                    let (t_ops, e_ops) = self.msg_arms(work)?;
                    flush!();
                    out.extend(if truth { t_ops } else { e_ops });
                }
                "ifnum" => {
                    let left = self.msg_number(work)?;
                    let rel = match self.eng.read_relation_pending(work)? {
                        '<' => Rel::Less,
                        '>' => Rel::Greater,
                        _ => Rel::Equal,
                    };
                    let right = self.msg_number(work)?;
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
                "ifdim" => {
                    let left = self.dimen_number(work, true)?;
                    let rel = match self.eng.read_relation_pending(work)? {
                        '<' => Rel::Less,
                        '>' => Rel::Greater,
                        _ => Rel::Equal,
                    };
                    let right = self.dimen_number(work, true)?;
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
                    let value = self.msg_number(work)?;
                    let (then_ops, else_ops) = self.msg_arms(work)?;
                    flush!();
                    out.push(MsgOp::IfOdd {
                        value,
                        then_ops,
                        else_ops,
                    });
                }
                "ifcase" => {
                    let value = self.msg_number(work)?;
                    let branches = self.msg_case_arms(work)?;
                    flush!();
                    out.push(self.case_chain(value, branches));
                }
                _ if self.eng.is_macro(n) => self.eng.expand_macro_pending(work, n)?,
                _ => text.push_str(&t.to_text(self.eng.escape)),
            }
        }
        if !text.is_empty() {
            out.push(MsgOp::Text(text));
        }
        Ok(out)
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
                _ => return Err(TexError("Incomplete \\ifcase".into())),
            }
        }
    }

    /// The two arms of a conditional inside a message.
    fn msg_arms(&mut self, work: &mut Lexer) -> R<(Vec<MsgOp>, Vec<MsgOp>)> {
        let negated = self.eng.take_unless();
        let then_ops = self.msg_ops(work, &["else", "fi"])?;
        let mut else_ops = Vec::new();
        match work.pending.pop() {
            Some(Token::Cs(n)) if n.name() == "else" => {
                else_ops = self.msg_ops(work, &["fi"])?;
                let _ = work.pending.pop();
            }
            Some(Token::Cs(n)) if n.name() == "fi" => {}
            _ => return Err(TexError("Incomplete \\if; missing \\fi".into())),
        }
        match negated {
            true => Ok((else_ops, then_ops)),
            false => Ok((then_ops, else_ops)),
        }
    }

    /// A number operand inside a message body.
    fn msg_number(&mut self, work: &mut Lexer) -> R<Num> {
        loop {
            let Some(t) = work.pending.pop() else {
                return Err(TexError("Missing number, treated as zero".into()));
            };
            if t.is_space() {
                continue;
            }
            match &t {
                Token::Cs(n) if n.name() == "count" => {
                    let reg = self.eng.scan_number_pending(work)?;
                    return Ok(Num::Count(reg));
                }
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

/// The two halves of a recognised tail loop: what runs, and what decides.
struct TailLoop {
    body: Vec<Token>,
    cond: Vec<Token>,
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
    /// The file name after `\input`, per `tex.web` §537: leading spaces are
    /// skipped and the name runs to the first space or end of line.
    ///
    /// A control sequence ends it too and is put back — `\input foo\relax` names
    /// `foo`.
    fn scan_file_name(&mut self, lx: &mut Lexer) -> R<String> {
        let mut name = String::new();
        while let Some(t) = lx.next_token(&self.eng.cats) {
            match &t {
                t if t.is_space() => {
                    if name.is_empty() {
                        continue;
                    }
                    break;
                }
                // LaTeX writes `\input{NAME}`, and the name inside the braces
                // may be BUILT: `article.cls` ends on
                // `\input{size1\@ptsize.clo}`, where `\@ptsize` is the macro
                // the class options set to `0`, `1` or `2`. §537 reads a file
                // name with `get_x_token`, so expansion is not an extra here --
                // it is what tex does. The group is read whole and its macros
                // expanded; `\the`, the conditionals and the primitives are
                // left alone by `expand_macros_only`, and none of them belongs
                // in a file name.
                Token::Char(_, Cat::BeginGroup) if name.is_empty() => {
                    lx.push_back(std::slice::from_ref(&t));
                    let body = self.eng.read_balanced_group(lx)?;
                    let body = self.eng.expand_macros_only(&body)?;
                    let escape = self.eng.escape;
                    name = body.iter().map(|t| t.to_text(escape)).collect();
                    // A control word's `to_text` writes the space that ends its
                    // name, and a file name has no spaces in it.
                    name.retain(|c| !c.is_whitespace());
                    break;
                }
                Token::Char(c, _) => name.push(*c),
                Token::Cs(_) => {
                    lx.push_back(std::slice::from_ref(&t));
                    break;
                }
            }
        }
        match name.is_empty() {
            true => Err(TexError("Missing file name".into())),
            false => Ok(name),
        }
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
                    return Ok((shown, crate::latex::load::without_trailing_endinput(src)));
                }
            }
        }
        // The TeX tree, last. `kpsewhich` is asked once per name per process and
        // the misses are remembered too, so a document that names a file nobody
        // has does not start a process per mention of it.
        for cand in &candidates {
            if let Some(path) = crate::latex::load::locate(cand) {
                if let Ok(src) = std::fs::read_to_string(&path) {
                    return Ok((
                        path.display().to_string(),
                        crate::latex::load::without_trailing_endinput(src),
                    ));
                }
            }
        }
        Err(TexError(format!("I can't find file `{name}'")))
    }

    /// Lower an `\input` file into the stream, sharing the Lowerer's state.
    ///
    /// The same nested pass `preload` runs for the LaTeX prelude, except that
    /// the commands are kept: a preload only wants the definitions, while an
    /// `\input` file can also print.
    fn input_pass(&mut self, src: &str) -> R<Vec<Cmd>> {
        let mut lx = Lexer::new(src);
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

    /// The registers a group closing here must save, given where its body
    /// started adding to `globals`.
    ///
    /// Everything the body assigns, less everything it assigned GLOBALLY: a
    /// global write is not undone by the group it sits in, and a `Cmd::Group`
    /// has one save/restore pair for the whole body, so not restoring means not
    /// saving.
    fn saved_by_group(&self, body: &[Cmd], mark: usize) -> Vec<i64> {
        let global = &self.globals[mark.min(self.globals.len())..];
        assigned_counts(body)
            .into_iter()
            .filter(|r| !global.contains(r))
            .collect()
    }

    /// Record `regs` as globally assigned when the assignment about to be
    /// emitted carried `\global`, spending the prefix either way.
    ///
    /// See the `globals` field: a group must not restore a register a `\global`
    /// wrote inside it, and the only place that can be known is here, where the
    /// prefix and the register number are both in hand.
    fn note_global(&mut self, regs: &[i64]) {
        // Every register assignment the lowerer makes passes through here,
        // which is what makes it the one place to record that the register has
        // left INITEX's zero. See `assigned` for who reads it.
        self.assigned.extend(regs.iter().copied());
        if !self.eng.take_global_prefix() {
            return;
        }
        for reg in regs {
            if !self.globals.contains(reg) {
                self.globals.push(*reg);
            }
        }
    }
}

/// `\advance\skip<n> by <glue>`, as commands (`tex.web` §1240 through §1239's
/// `glue_add`).
///
/// Adding glue adds the natural widths and combines each infinity ORDER
/// separately: an infinite component beats any finite one however large, a
/// higher infinity beats a lower, and only equal orders add. The operand is
/// known while lowering and the register is not, so the rule becomes a branch
/// on the register's own order.
///
/// The two orders live packed in one slot as `stretch*4 + shrink`, sixteen
/// possible values, and the result is a function of that value alone -- so the
/// chain enumerates all sixteen rather than trying to take the packed slot
/// apart with arithmetic the VM has no remainder op for. It is the same shape
/// `\ifcase` lowers to, and for the same reason.
fn advance_glue(base: i64, op: crate::glue::Glue) -> Vec<Cmd> {
    let packed = base + 3;
    let mut out = vec![Cmd::Arith(Arith::Add, base, Num::Literal(op.natural))];
    // Built from the last state backwards, so each arm's `else` is the chain
    // for every state after it.
    let mut chain: Vec<Cmd> = Vec::new();
    for state in (0..16i64).rev() {
        let (was_stretch, was_shrink) = (state / 4, state % 4);
        let mut arm = Vec::new();
        for (slot, was, add, order) in [
            (base + 1, was_stretch, op.stretch, op.stretch_order),
            (base + 2, was_shrink, op.shrink, op.shrink_order),
        ] {
            match was.cmp(&order) {
                // The register's component is the more infinite: it stands.
                std::cmp::Ordering::Greater => {}
                std::cmp::Ordering::Equal => {
                    arm.push(Cmd::Arith(Arith::Add, slot, Num::Literal(add)))
                }
                std::cmp::Ordering::Less => arm.push(Cmd::SetCount(slot, Num::Literal(add))),
            }
        }
        let after = (
            was_stretch.max(op.stretch_order),
            was_shrink.max(op.shrink_order),
        );
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
fn assigned_counts(cmds: &[Cmd]) -> Vec<i64> {
    let mut regs = Vec::new();
    fn walk(cmds: &[Cmd], regs: &mut Vec<i64>) {
        for c in cmds {
            match c {
                // A line directive, a run of the document's text, a
                // `\rust{ … }` compile, a file's closing paren, an error site
                // and the transcript notice all write no register.
                Cmd::Line(_)
                | Cmd::Text(_)
                | Cmd::RustCompile(_)
                | Cmd::FileClose
                | Cmd::ErrorSite(_)
                | Cmd::TranscriptNotice => {}
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
                Cmd::Message(_) => {}
            }
        }
    }
    walk(cmds, &mut regs);
    regs
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
