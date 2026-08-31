//! The primitive-reference corpus: the `(name, chapter, doc, syntax + example)`
//! table behind editor completion, editor hover (`src/lsp.rs`) and the generated
//! `docs/reference.html` (`src/bin/gen_docs.rs`).
//!
//! One entry per primitive the engine actually resolves. Every entry mirrors
//! something in the implementation:
//!
//!   * "Category codes"    → `catcode.rs` and `expand::do_catcode`.
//!   * "Macro definition"  → `expand::do_def` / `do_let` and
//!     `lower::compile_time_def`.
//!   * "Expansion"         → `expand::expand_one` and the `lower` message path.
//!   * "Conditionals"      → `expand::CONDITIONALS`, `expand::do_conditional`,
//!     and the branch lowering in `lower.rs`.
//!   * "Registers"         → `compiler::COUNT_SLOTS` and `expand::do_arith`.
//!   * "Grouping"          → `expand::begin_group` / `end_group` and the
//!     save/restore wrapper `lower.rs` emits around a group body.
//!   * "LaTeX"             → `latex.rs`, `expand::do_newcommand` and
//!     `expand::compile_time_preamble_directive`.
//!
//! A primitive is documented here only if texrs resolves it, so the language
//! server and the static reference never drift from what the engine can run.
//! Where the behaviour diverges from real tex, the entry says so rather than
//! describing tex — `BUGS.md` carries the same divergence with its reason.
//!
//! The fourth field is a syntax line followed by a usage example, both rendered
//! in one TeX code block.

/// A reference entry: `(name, chapter, one-line doc, syntax + example)`.
pub type Entry = (&'static str, &'static str, &'static str, &'static str);

/// The chapters, in the order the reference presents them.
pub const CHAPTERS: &[&str] = &[
    "Category codes",
    "Macro definition",
    "Expansion",
    "Conditionals",
    "Registers",
    "Grouping",
    "Intercepts",
    "Inline Rust",
    "Files",
    "LaTeX",
];

/// The reference corpus, in chapter order.
pub const CORPUS: &[Entry] = &[
    // ══ Category codes — catcode.rs, expand::do_catcode ════════════════════
    (
        "\\catcode",
        "Category codes",
        "Set or read a character's category code. A character's category is a mutable table entry, not a property of the language: texrs starts from INITEX's sparse defaults, where `{` is an ordinary character until something makes it a group opener. Takes effect at COMPILE time, because it changes how the rest of the file reads.",
        "\\catcode`\\X=N\n\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6\n\\message{now braces group}",
    ),
    (
        "^^X",
        "Category codes",
        "Control-character notation (tex.web \u{a7}352), in two forms decided by what follows: `^^` and two LOWERCASE hex digits is that hex code, while `^^` and anything else is one character shifted by 64. So `^^41` is `A` and `^^4a` is `J`, but `^^4A` is `tA` \u{2014} `A` is not a lowercase hex digit, so the shift applies to the `4` alone. The substitution belongs to the input processor (\u{a7}353), which runs before anything is classified, so it applies inside a control sequence name too: plain.tex writes `\\catcode`\\^^K=7`. This is also how a line end is written inside a macro body.",
        "^^M   % carriage return\n^^I   % tab\n^^41  % the hex form: A",
    ),
    (
        "`",
        "Category codes",
        "A character code as a number, for a register assignment or a comparison: a backtick followed by a character is that character's code, and the backslash before the character is optional unless it is a control sequence.",
        "\\count1=`\\A\n\\message{\\the\\count1}   % => 65",
    ),
    // ══ Macro definition — expand::do_def / do_let ═════════════════════════
    (
        "\\def",
        "Macro definition",
        "Define a macro. Parameters may be undelimited (`#1`) or delimited (`\\def\\pair#1,#2.{...}`, where the argument is whatever precedes the delimiter), and `##` is a literal parameter character. The parameter text is validated as tex.web \u{a7}476 validates it: every `#` must be followed by a digit, and the digits must run consecutively. Takes effect at COMPILE time. The name may be an ACTIVE CHARACTER as well as a control sequence (`tex.web` \u{a7}1215): `\\catcode`\\~=13 \\def~{...}` defines `~` itself, and an active `~` and the control sequence `\\~` stay different things.",
        "\\def\\name<parameter text>{<body>}\n\\def\\greet#1{HELLO-#1}\n\\def\\pair#1,#2.{[#1|#2]}\n\\message{\\greet{WORLD}}   % => HELLO-WORLD\n\\message{\\pair 1,2.}      % => [1|2]",
    ),
    (
        "\\gdef",
        "Macro definition",
        "Define globally: the definition survives the enclosing group, where a `\\def` inside braces is undone at the closing brace.",
        "\\gdef\\name{<body>}\n{\\gdef\\v{IN}}\n\\message{\\v}   % => IN",
    ),
    (
        "\\edef",
        "Macro definition",
        "Define with the body expanded NOW, so a register read is frozen at definition time and a later assignment cannot move it. texrs freezes a `\\the\\count` read into a scratch register (taken from the top of the count range). It does NOT yet decide a conditional in the body at definition time \u{2014} see BUGS.md.",
        "\\edef\\name{<body>}\n\\count1=1\n\\edef\\frozen{\\the\\count1}\n\\count1=2\n\\message{\\frozen}   % => 1",
    ),
    (
        "\\xdef",
        "Macro definition",
        "A global `\\edef`: the body is expanded now and the definition survives the enclosing group.",
        "\\xdef\\name{<body>}\n{\\xdef\\frozen{\\the\\count1}}",
    ),
    (
        "\\let",
        "Macro definition",
        "Give a control sequence another's CURRENT meaning, not a reference to it: redefining the source afterwards does not change the alias.",
        "\\let\\alias=\\source\n\\def\\v{ONE}\n\\let\\w=\\v\n\\def\\v{TWO}\n\\message{\\w}   % => ONE",
    ),
    (
        "\\futurelet",
        "Macro definition",
        "Look one token past the next without eating either: `\\futurelet\\a\\b\\c` gives `\\a` the meaning of `\\c` and then puts `\\b` and `\\c` back, so the stream is exactly as it was (tex.web \u{a7}1221). That non-destructive peek is what LaTeX's `\\@ifnextchar` is built from, and so is every optional argument in the language. A control sequence let to a character MEANS that character, so `\\ifx\\next[` compares true \u{2014} which is the comparison the whole idiom rests on.",
        "\\futurelet\\next<token><token>\n\\def\\peek{\\futurelet\\next\\decide}\n\\def\\decide{\\ifx\\next[ OPTIONAL\\else PLAIN\\fi}",
    ),
    (
        "\\global",
        "Macro definition",
        "Prefix making the following assignment global, so it survives the enclosing group.",
        "\\global\\count1=5\n\\global\\def\\v{OUT}",
    ),
    // ══ Expansion — expand::expand_one, lower's message path ═══════════════
    (
        "\\csname",
        "Expansion",
        "Build a control sequence out of the characters up to `\\endcsname`, so a macro can name another macro. A name with no meaning becomes `\\relax`, as in tex.",
        "\\csname <characters>\\endcsname\n\\def\\greeting{HI}\n\\def\\name{greeting}\n\\message{\\csname \\name\\endcsname}   % => HI",
    ),
    (
        "\\endcsname",
        "Expansion",
        "Terminate a `\\csname`. It is an error to reach one without an open `\\csname`.",
        "\\csname foo\\endcsname",
    ),
    (
        "\\string",
        "Expansion",
        "Print a control sequence as text, escape character included \u{2014} the inverse of `\\csname`.",
        "\\string\\cs\n\\message{\\string\\undefined}   % => \\undefined",
    ),
    (
        "\\the",
        "Expansion",
        "The value of a register, as characters. The read happens at RUN time on the VM, which is why a `\\message` containing `\\the\\count0` follows a later assignment.",
        "\\the\\count<N>\n\\count1=12\n\\message{count=\\the\\count1}   % => count=12",
    ),
    (
        "\\number",
        "Expansion",
        "A scanned number, as characters, with no leading zeros or plus sign. The scan keeps reading (and expanding) until something that cannot be part of a number stops it, so a following letter needs a terminating space.",
        "\\number<number>\n\\message{\\number0042}   % => 42",
    ),
    (
        "\\expandafter",
        "Expansion",
        "One token of lookahead: hold the next token back, expand what follows it once, then put the held token in front of the result.",
        "\\expandafter<token><token>\n\\def\\a{\\b}\n\\def\\b{DEEP}\n\\message{\\expandafter\\string\\a}   % => \\b",
    ),
    (
        "\\noexpand",
        "Expansion",
        "Suppress expansion of the next token for one step \u{2014} the token is used for its own sake rather than its meaning.",
        "\\noexpand<token>\n\\edef\\keep{\\noexpand\\later}",
    ),
    (
        "\\message",
        "Expansion",
        "Write to the terminal. This is texrs's parity contract for the milestone: two host builtins, one appending a rendered piece and one flushing the assembled string, so a message reads its registers at run time.",
        "\\message{<text>}\n\\message{HELLO-WORLD}   % => (./file.tex HELLO-WORLD )",
    ),
    (
        "\\relax",
        "Expansion",
        "Do nothing. Accepted so a document can stop a number scan, or fill a slot that needs a token but no action \u{2014} which is also what `\\csname` makes of a name with no meaning.",
        "\\relax",
    ),
    (
        "\\par",
        "Expansion",
        "End a paragraph. Accepted and ignored: paragraphs belong to the stomach, which texrs does not have. A blank line produces one, which is why the line scanner tracks its state.",
        "\\par",
    ),
    (
        "\\ignorespaces",
        "Expansion",
        "Skip the spaces that follow. Accepted and ignored in this milestone, because spacing only matters once there is a stomach to typeset it.",
        "\\ignorespaces",
    ),
    (
        "\\end",
        "Expansion",
        "Stop the run. It does not ship a page: there is no stomach, which is why real tex reports `No pages of output.` for every case in the corpus.",
        "\\end",
    ),
    // ══ Conditionals — expand::do_conditional, lower's branch emission ═════
    (
        "\\ifnum",
        "Conditionals",
        "Compare two numbers with `<`, `=` or `>`. It tests run-time state, so it lowers to a real branch \u{2014} a comparison plus a jump \u{2014} rather than a decision taken while walking a tree.",
        "\\ifnum<number><rel><number> <true>\\else <false>\\fi\n\\count1=5\n\\message{\\ifnum\\count1>3 BIG\\else SMALL\\fi}   % => BIG",
    ),
    (
        "\\ifodd",
        "Conditionals",
        "True when a number is odd. Reads run-time state, so it lowers to a real branch rather than folding. Negative odd numbers are odd: the test is on the remainder being non-zero, not on its sign.",
        "\\ifodd<number> <true>\\else <false>\\fi\n\\message{\\ifodd\\count1 ODD\\else EVEN\\fi}",
    ),
    (
        "\\ifcase",
        "Conditionals",
        "Switch on a number: the first case is 0, each `\\or` starts the next, and `\\else` catches everything past the last. Lowers to a real branch. A selector past the last `\\or` with no `\\else` selects nothing and the run continues. DIVERGENCE: a NEGATIVE selector takes case 0 here where tex takes `\\else` \u{2014} `\\ifcase -1 ZERO\\else DEFAULT\\fi` prints ZERO rather than DEFAULT. Pinned by `tests/cases/cond_ifcase_negative.tex`.",
        "\\ifcase<number> <0>\\or <1>\\or <2>\\else <other>\\fi\n\\count1=2\n\\message{\\ifcase\\count1 ZERO\\or ONE\\or TWO\\else MANY\\fi}   % => TWO",
    ),
    (
        "\\iftrue",
        "Conditionals",
        "Constant truth. Decidable without the VM, so it is FOLDED while lowering and the untaken arm is never emitted \u{2014} it does not merely jump over that code, it never emits it. This is why a `\\def` inside the untaken arm of an `\\iftrue` is safe where the same `\\def` inside a run-time conditional is not; see `def_in_conditional_arm.tex`., so it is FOLDED while lowering and the untaken arm is never emitted.",
        "\\iftrue <true>\\else <false>\\fi",
    ),
    (
        "\\iffalse",
        "Conditionals",
        "Constant falsity. Folded while lowering, like `\\iftrue`, so the taken arm is the only code that reaches the chunk.",
        "\\iffalse <true>\\else <false>\\fi",
    ),
    (
        "\\ifx",
        "Conditionals",
        "Compare two meanings: two macros are equal when their parameter texts and bodies match, and two primitives when they are the same primitive. Decidable from the macro table alone, so it is folded while lowering.",
        "\\ifx<token><token> <true>\\else <false>\\fi\n\\def\\a{X}\\def\\b{X}\n\\message{\\ifx\\a\\b SAME\\else DIFF\\fi}   % => SAME",
    ),
    (
        "\\if",
        "Conditionals",
        "Compare two character codes after expansion. A control sequence compares equal to any OTHER control sequence, because neither has a character code to compare \u{2014} that is `tex.web` \u{a7}506's rule, not a shortcut. Both sides are expanded first, so `\\if\\a\\b` tests what the macros produce, not their names.",
        "\\if<token><token> <true>\\else <false>\\fi",
    ),
    (
        "\\ifdefined",
        "Conditionals",
        "True when a control sequence has a meaning. Decidable from the macro table alone, so it is FOLDED while lowering and only the taken arm is emitted. An undefined name is false rather than an error, which is the one place an undefined control sequence is not a divergence here.",
        "\\ifdefined<token> <true>\\else <false>\\fi",
    ),
    (
        "\\else",
        "Conditionals",
        "Start the false arm of a conditional. Both arms are collected as token runs and lowered, so the untaken one is real code the VM jumps over rather than a subtree nobody walked.",
        "\\else",
    ),
    (
        "\\or",
        "Conditionals",
        "Start the next case of an `\\ifcase`: the case before the first `\\or` is 0, and each `\\or` moves to the next.",
        "\\or",
    ),
    (
        "\\fi",
        "Conditionals",
        "End a conditional. A control word swallows the space after it, so `\\fi X` prints `X` with no leading space.",
        "\\fi",
    ),
    (
        "\\ifcat",
        "Conditionals",
        "RECOGNISED BUT NOT EVALUATED. Compares category codes; texrs skips the construct correctly so an unbalanced branch cannot confuse the scanner, but cannot decide it. Reaching one stops the run with `! Unsupported conditional \\NAME.` and exit status 1; SKIPPING one inside an untaken branch is correct, because the skipper counts it for nesting.",
        "\\ifcat<token><token> <true>\\else <false>\\fi",
    ),
    (
        "\\ifdim",
        "Conditionals",
        "RECOGNISED BUT NOT EVALUATED: there are no dimen registers yet. Reaching one stops the run with `! Unsupported conditional \\NAME.` and exit status 1; SKIPPING one inside an untaken branch is correct, because the skipper counts it for nesting.",
        "\\ifdim<dimen><rel><dimen> <true>\\else <false>\\fi",
    ),
    (
        "\\ifvoid",
        "Conditionals",
        "RECOGNISED BUT NOT EVALUATED: there are no box registers yet. Reaching one stops the run with `! Unsupported conditional \\NAME.` and exit status 1; SKIPPING one inside an untaken branch is correct, because the skipper counts it for nesting.",
        "\\ifvoid<N> <true>\\else <false>\\fi",
    ),
    (
        "\\ifhbox",
        "Conditionals",
        "RECOGNISED BUT NOT EVALUATED: there are no box registers yet. Reaching one stops the run with `! Unsupported conditional \\NAME.` and exit status 1; SKIPPING one inside an untaken branch is correct, because the skipper counts it for nesting.",
        "\\ifhbox<N> <true>\\else <false>\\fi",
    ),
    (
        "\\ifvbox",
        "Conditionals",
        "RECOGNISED BUT NOT EVALUATED: there are no box registers yet. Reaching one stops the run with `! Unsupported conditional \\NAME.` and exit status 1; SKIPPING one inside an untaken branch is correct, because the skipper counts it for nesting.",
        "\\ifvbox<N> <true>\\else <false>\\fi",
    ),
    (
        "\\ifvmode",
        "Conditionals",
        "RECOGNISED BUT NOT EVALUATED: modes belong to the stomach. Reaching one stops the run with `! Unsupported conditional \\NAME.` and exit status 1; SKIPPING one inside an untaken branch is correct, because the skipper counts it for nesting.",
        "\\ifvmode <true>\\else <false>\\fi",
    ),
    (
        "\\ifhmode",
        "Conditionals",
        "RECOGNISED BUT NOT EVALUATED: modes belong to the stomach. Reaching one stops the run with `! Unsupported conditional \\NAME.` and exit status 1; SKIPPING one inside an untaken branch is correct, because the skipper counts it for nesting.",
        "\\ifhmode <true>\\else <false>\\fi",
    ),
    (
        "\\ifmmode",
        "Conditionals",
        "RECOGNISED BUT NOT EVALUATED: modes belong to the stomach. Reaching one stops the run with `! Unsupported conditional \\NAME.` and exit status 1; SKIPPING one inside an untaken branch is correct, because the skipper counts it for nesting.",
        "\\ifmmode <true>\\else <false>\\fi",
    ),
    (
        "\\ifinner",
        "Conditionals",
        "RECOGNISED BUT NOT EVALUATED: modes belong to the stomach. Reaching one stops the run with `! Unsupported conditional \\NAME.` and exit status 1; SKIPPING one inside an untaken branch is correct, because the skipper counts it for nesting.",
        "\\ifinner <true>\\else <false>\\fi",
    ),
    (
        "\\ifeof",
        "Conditionals",
        "RECOGNISED BUT NOT EVALUATED: there is no file I/O yet. Reaching one stops the run with `! Unsupported conditional \\NAME.` and exit status 1; SKIPPING one inside an untaken branch is correct, because the skipper counts it for nesting.",
        "\\ifeof<N> <true>\\else <false>\\fi",
    ),
    (
        "\\ifcsname",
        "Conditionals",
        "RECOGNISED BUT NOT EVALUATED. Use `\\ifdefined` on a `\\csname`-built token instead. Reaching one stops the run with `! Unsupported conditional \\NAME.` and exit status 1; SKIPPING one inside an untaken branch is correct, because the skipper counts it for nesting.",
        "\\ifcsname <characters>\\endcsname <true>\\else <false>\\fi",
    ),
    // ══ Registers — compiler::COUNT_SLOTS, expand::do_arith ════════════════
    (
        "\\count",
        "Registers",
        "A count register. There are exactly 256 of them (tex.web \u{a7}236) and they map onto VM slots 0..255, so a read is an array index rather than a hash lookup. texrs loads no format, so every register starts at zero as INITEX leaves them \u{2014} where the reference `tex` has plain's values, including `\\count0` = the page number.",
        "\\count<N>=<number>\n\\count1=7\n\\message{\\the\\count1}   % => 7",
    ),
    (
        "\\advance",
        "Registers",
        "Add to a register. Lowers to `GetSlot / LoadInt / Add / SetSlot` \u{2014} native ops the JIT can compile.",
        "\\advance\\count<N> by <number>\n\\count1=7 \\advance\\count1 by 5\n\\message{\\the\\count1}   % => 12",
    ),
    (
        "\\multiply",
        "Registers",
        "Multiply a register. Like the other arithmetic, it lowers to native fusevm ops on the register's slot rather than a call.",
        "\\multiply\\count<N> by <number>\n\\count1=7 \\multiply\\count1 by 3   % => 21",
    ),
    (
        "\\divide",
        "Registers",
        "Divide a register, truncating toward zero as TeX's does.",
        "\\divide\\count<N> by <number>\n\\count1=7 \\divide\\count1 by 2   % => 3",
    ),
    // ══ Intercepts — intercepts.rs, expand::weave_advice ══════════════════
    (
        "\\intercept",
        "Intercepts",
        "Register advice on macro expansion: `before` puts the handler's body in front of the expansion, `after` puts it behind, and `around` replaces it with `\\proceed` standing for what the macro would have expanded to. The pattern is a GLOB over macro names, so advice registered now catches macros a package defines later. The handler is a macro that takes no parameters. Like `\\def`, this takes effect at compile time and is undone by the group it was registered in.",
        "\\intercept{before|after|around}{<glob>}{\\handler}\n\\def\\greet#1{HELLO-#1}\n\\def\\trace{[in]}\n\\intercept{before}{greet}{\\trace}\n\\message{\\greet{WORLD}}   % => [in]HELLO-WORLD",
    ),
    (
        "\\proceed",
        "Intercepts",
        "Inside an `around` handler, what the intercepted macro would have expanded to. A handler with no `\\proceed` replaces the call outright, which is how advice suppresses one. Outside an `around` handler it means nothing.",
        "\\def\\loud{<<\\proceed>>}\n\\intercept{around}{greet}{\\loud}\n\\message{\\greet{WORLD}}   % => <<HELLO-WORLD>>",
    ),
    (
        "\\chardef",
        "Registers",
        "Define a control sequence that IS a number: `\\chardef\\active=13` makes `\\active` usable wherever a number is scanned, which is how plain.tex writes `\\catcode`\\~=\\active`. The code is 0..255 and a wider one is `! Bad character code (N).`. The value is fixed when it is defined, so it folds while lowering rather than being read at run time.",
        "\\chardef\\active=13\n\\catcode`\\~=\\active",
    ),
    (
        "\\countdef",
        "Registers",
        "Give a count register a name, usable in every position the register itself is: assignment, `\\advance` and `\\the` all reach the same register through either spelling. plain.tex's `\\pageno` is `\\countdef\\pageno=0`. The register number is 0..255 and a wider one is `! Bad register code (N).`.",
        "\\countdef\\pageno=0\n\\pageno=7\n\\advance\\pageno by 1",
    ),
    (
        "\\long",
        "Macro definition",
        "A definition prefix: the macro's arguments may contain `\\par`. Without it a paragraph break inside an argument is a runaway, which is TeX's guard against a missing closing brace swallowing the rest of a document. texrs records the prefix and does not yet enforce the restriction it lifts.",
        "\\long\\def\\note#1{[#1]}",
    ),
    (
        "\\outer",
        "Macro definition",
        "A definition prefix: the macro may not then appear in an argument, in a group being scanned as text, or in skipped conditional text. It is an error-detection feature -- plain.tex marks its sectioning macros `\\outer` so a missing brace is caught at the next section rather than at the end of the file. texrs records the prefix and does not police the restriction, so a use tex forbids is accepted here; `tests/cases/outer_forbidden_use.tex` pins the difference. Prefixes are part of the meaning, so `\\ifx` tells a prefixed definition from a bare one.",
        "\\outer\\def\\chapter{...}",
    ),
    (
        "\\mathcode",
        "Registers",
        "How a character is set in math mode, as a 15-bit code: class, family and position. INITEX gives a letter \u{a7}7100+c, a digit \u{a7}7000+c and everything else its own code, so `\\mathcode`\\A` is \"7141 and `\\mathcode`\\+` is 43. Written and read like `\\catcode`, and restored at the end of a group the same way.",
        "\\mathcode`\\x=\"2201",
    ),
    (
        "\\lccode",
        "Registers",
        "A character's lowercase form, which is what `\\lowercase` consults. INITEX sets it for the letters and leaves it 0 for everything else -- a character with no case is not lowercased to a null, it is left alone.",
        "\\lccode`\\A=`\\a",
    ),
    (
        "\\uccode",
        "Registers",
        "The same for uppercase, consulted by `\\uppercase`. Also 0 for a character with no case.",
        "\\uccode`\\a=`\\A",
    ),
    (
        "\\sfcode",
        "Registers",
        "The space factor a character leaves behind, which stretches the space after it. INITEX gives every character 1000 except an uppercase letter, which gets 999 -- that is what stops a sentence appearing to end at the full stops in \"N.A.S.A.\".",
        "\\sfcode`\\A=999",
    ),
    (
        "\\delcode",
        "Registers",
        "A character's meaning as a delimiter, as a 24-bit code naming a small and a large variant. INITEX gives -1 everywhere, meaning \"not a delimiter\", except the period, whose code is 0.",
        "\\delcode`\\(=\"161361",
    ),
    (
        "\\mathchardef",
        "Registers",
        "Define a control sequence standing for a math code, as `\\chardef` does for a character code. The range runs to \"7FFF and a wider value is `! Bad mathchar (N).`.",
        "\\mathchardef\\half=\"2201\n\\mathcode`\\y=\\half",
    ),
    (
        "\"",
        "Registers",
        "A hexadecimal constant, and `'` an octal one (tex.web \\u{a7}445). The hex digits are UPPERCASE: `\"FF` is 255 and `\"ff` is an error -- the opposite of `^^` notation, which takes lowercase. plain.tex writes every `\\mathcode` as a hexadecimal constant.",
        "\\count1=\"FF   % 255\n\\count2='777  % 511",
    ),
    (
        "\\dimen",
        "Registers",
        "A dimension register. A dimension is an integer count of scaled points, 65536 to the printer's point, and a unit is an exact integer ratio to a point (tex.web \u{a7}458) rather than a float -- which is why `1in` is 72.26999pt. The units are pt, in, pc, cm, mm, bp, dd, cc and sp. `\\the` writes one back by Knuth's print_scaled (\u{a7}103), the fewest digits that read back as the same integer, and `\\number` gives the scaled points instead.",
        "\\dimen0=1in\n\\message{\\the\\dimen0}   % => 72.26999pt",
    ),
    (
        "\\dimendef",
        "Registers",
        "Give a dimension register a name, as `\\countdef` does for a count. The name behaves as the register does on both sides: an assignment through it reads a dimension, and `\\the` through it writes one.",
        "\\dimendef\\dimen@=0\n\\dimen@=2pt",
    ),
    (
        "\\skip",
        "Registers",
        "A glue register: a natural dimension that can stretch and shrink. The stretch and shrink may be infinite -- `fil`, `fill`, `filll` -- and an infinite component beats any finite one however large, which is what `\\hfil` is made of. `\\the` writes the components back with `plus` and `minus`, omitting a zero one, and `\\number` gives the natural component alone.",
        "\\skip0=1pt plus 2pt minus 3pt\n\\skip1=0pt plus 1fil",
    ),
    (
        "\\skipdef",
        "Registers",
        "Give a glue register a name, as `\\countdef` and `\\dimendef` do for theirs. An assignment through the name reads a whole glue, not the dimension at the front of one.",
        "\\skipdef\\skip@=0\n\\skip@=1pt plus 2pt",
    ),
    (
        "\\toks",
        "Registers",
        "A token register: a token list stored VERBATIM, since nothing inside the braces expands -- which is the difference between it and a macro, and why `\\toks0={\\x}` reads back as `\\x` whatever `\\x` means. `\\toks1=\\toks0` copies one register to another. `\\the` writes the list back by the token-list rule rather than `\\string`'s: a control word carries a trailing space however short, a one-character control sequence does not.",
        "\\toks0={a\\b c}\n\\message{\\the\\toks0}   % => a\\b c",
    ),
    (
        "\\toksdef",
        "Registers",
        "Give a token register a name, as `\\countdef` does for a count. The list is frontend state like a macro body rather than a number in a slot, so the name stands for the register itself.",
        "\\toksdef\\toks@=0\n\\toks@={...}",
    ),
    // ══ eTeX extensions — expand::scan_expr ═══════════════════════════════
    (
        "\\numexpr",
        "Registers",
        "An integer expression, closed by an optional `\\relax`: `+`, `-`, `*`, `/` with ordinary precedence and parentheses. Division ROUNDS, half away from zero, so `\\numexpr 7/2` is 4 where `\\divide` gives 3 -- the two are different operations and texrs keeps them apart. An eTeX primitive, so the oracle for it is LuaTeX rather than tex; `tests/etex.rs` holds the comparison.",
        "\\count0=\\numexpr (2+3)*4\\relax   % => 20",
    ),
    (
        "\\dimexpr",
        "Registers",
        "The same for dimensions: the operands are lengths and the multiplier and divisor are integers, so `\\dimexpr 1pt*3` is three points. The arithmetic happens in scaled points, which is the only form a dimension has.",
        "\\dimen0=\\dimexpr 1pt+2pt\\relax   % => 3.0pt",
    ),
    (
        "\\unless",
        "Conditionals",
        "Negate the conditional that follows: `\\unless\\ifnum 1>2 A\\else B\\fi` runs A. An eTeX primitive. A negated conditional is the same conditional with its arms exchanged, which is how it is lowered.",
        "\\unless\\ifnum \\count0>2 few\\else many\\fi",
    ),
    (
        "\\protected",
        "Macro definition",
        "A definition prefix: the macro does not expand inside an `\\edef`, it survives as itself and runs when the result does. Redefining it afterwards therefore changes what the `\\edef`'d macro produces, which is the observable difference. An eTeX primitive, and the one LaTeX leans on to keep a fragile command safe in a moving argument.",
        "\\protected\\def\\note#1{[#1]}",
    ),
    (
        "\\detokenize",
        "Expansion",
        "The tokens of `{...}` written as text, by the token-list rule: a control word carries a trailing space, a one-character control sequence does not. Nothing in the group expands. An eTeX primitive.",
        "\\message{\\detokenize{\\a b}}   % => \\a b",
    ),
    (
        "\\csstring",
        "Expansion",
        "`\\string` without the escape character: `\\csstring\\foo` is `foo` where `\\string\\foo` is `\\foo`. A LuaTeX primitive.",
        "\\message{\\csstring\\foo}   % => foo",
    ),
    (
        "\\Uchar",
        "Expansion",
        "The character with the given code: `\\Uchar65` is `A`. A LuaTeX primitive, and the one that reaches past 255 -- texrs reads characters rather than bytes, so it carries the whole range.",
        "\\message{\\Uchar65\\Uchar97}   % => Aa",
    ),
    (
        "\\expanded",
        "Expansion",
        "Expand the group's contents completely, here and now, and put the result back. Inside an `\\edef` it is the wrapper coming off, since everything there expands anyway; in running text it forces the expansion a macro would otherwise have deferred. An eTeX primitive.",
        "\\message{\\expanded{\\body}}",
    ),
    (
        "\\unexpanded",
        "Expansion",
        "The opposite: the group's tokens are used as they stand. Inside an `\\edef` they survive as TOKENS, so a macro among them is called when the body runs rather than when it is defined -- which is where this and `\\expanded` part company. In a message the two render alike, by the token-list rule. An eTeX primitive.",
        "\\edef\\keep{\\unexpanded{\\later}}",
    ),
    (
        "\\begincsname",
        "Expansion",
        "`\\csname` that does not define what it does not find: an unknown name expands to nothing, where `\\csname` would make it `\\relax` and leave it defined ever after. A LuaTeX primitive, and the one that makes \"is this defined?\" answerable without changing the answer.",
        "\\begincsname maybe\\endcsname",
    ),
    // ══ Files — lower::open_input ═════════════════════════════════════════
    (
        "\\input",
        "Files",
        "Read another file here, sharing every piece of state with it: a macro it defines is defined afterwards, and a `\\catcode` it sets stays set. The name runs to the first space or end of line (tex.web \u{a7}537) and `.tex` is supplied when it carries no extension. texrs searches the working directory and then `TEXINPUTS`, and does not shell out to `kpsewhich`, so running a document never depends on a TeX Live installation being present. Fifteen text input levels are allowed, counting the document's own, which is tex's limit and tex's wording when it is passed.",
        "\\input macros\n\\input chapters/one.tex",
    ),
    // ══ Inline Rust — rust_ffi.rs, fusevm::ffi ════════════════════════════
    (
        "\\rust",
        "Inline Rust",
        "Open a block of Rust compiled and loaded at run time. The body is Rust, not TeX \u{2014} it is lifted out of the file BEFORE the mouth reads it, because `#`, `{`, `}` and `&` are category codes the mouth would act on. Every `#[no_mangle] pub extern \"C\"` function the block exports becomes callable with `\\rustcall`. Needs `rustc` on PATH; the compiled library is cached by body hash, so a second run does not compile it again.",
        "\\rust{ <rust source> }\n\\rust{\n    #[no_mangle]\n    pub extern \"C\" fn twice(n: i64) -> i64 { n * 2 }\n}",
    ),
    (
        "\\rustcall",
        "Inline Rust",
        "Call a function a `\\rust` block exported. The name runs to the first space, the arguments are numbers, and `\\endrust` ends the list. It is a NUMBER wherever TeX reads one \u{2014} a register assignment, an arithmetic operand, a conditional, or a `\\message` body \u{2014} and in running text it is called for its effect with the value dropped.",
        "\\rustcall <name> <numbers…>\\endrust\n\\count1=21\n\\message{\\rustcall twice \\count1 \\endrust}   % => 42\n\\count2=\\rustcall add \\count1 22 \\endrust",
    ),
    (
        "\\rustcompile",
        "Inline Rust",
        "What a `\\rust{ … }` block becomes: compile and register the block whose base64 body follows, up to `\\endrust`. Written by the desugarer rather than by hand, and carried in a brace-free form so it reads correctly whatever the category codes are where the block appeared.",
        "\\rustcompile <base64>\\endrust",
    ),
    (
        "\\endrust",
        "Inline Rust",
        "Terminate a `\\rustcompile` body or a `\\rustcall` argument list. A control sequence rather than a brace, so neither form depends on a category code the document may not have set yet.",
        "\\rustcall twice 21\\endrust",
    ),
    // ══ Grouping — expand::begin_group / end_group, lower's save/restore ═══
    (
        "{",
        "Grouping",
        "Open a group (category code 1), which scopes the macro table AND the count registers written inside it: the lowered body is wrapped in save/restore for exactly the registers it assigns. It also delimits a macro argument.",
        "{<body>}\n\\def\\v{OUT}\n{\\def\\v{IN}\\message{\\v}}   % => IN\n\\message{\\v}             % => OUT",
    ),
    (
        "}",
        "Grouping",
        "Close a group (category code 2), undoing every non-global assignment made inside it.",
        "{<body>}",
    ),
    (
        "\\begingroup",
        "Grouping",
        "Open a group without braces, scoping the macro table and the registers written inside it. It must be closed by `\\endgroup`, not by `}`.",
        "\\begingroup <body>\\endgroup",
    ),
    (
        "\\endgroup",
        "Grouping",
        "Close a `\\begingroup`, undoing every non-global assignment made since it. A `}` will not close one, and neither will the end of the file.",
        "\\begingroup <body>\\endgroup",
    ),
    // ══ LaTeX — latex.rs, expand::do_newcommand ════════════════════════════
    (
        "\\newcommand",
        "LaTeX",
        "Define a macro with `n` positional parameters: `\\newcommand{\\x}[2]{#1 and #2}`. LaTeX writes this as a chain of `\\ifnum...\\def`, which cannot run here because lowering emits both arms of a conditional, so texrs dispatches on the argument count natively instead. Two divergences from latex.ltx: redefining an existing name is allowed rather than an error, and the `[default]` form's default is recorded but not yet substituted at a call that omits the bracket.",
        "\\newcommand{\\NAME}[ARGC][DEFAULT]{BODY}\n\\newcommand{\\greet}[1]{hello #1}\n\\message{\\greet{world}}   % => hello world",
    ),
    (
        "\\renewcommand",
        "LaTeX",
        "Redefine a macro. Identical to `\\newcommand` here, because texrs does not check whether the name already exists in either direction.",
        "\\renewcommand{\\NAME}[ARGC]{BODY}\n\\newcommand{\\x}{one}\n\\renewcommand{\\x}{two}\n\\message{\\x}   % => two",
    ),
    (
        "\\providecommand",
        "LaTeX",
        "Define a macro only if the name is free. An existing definition is kept and the new body is consumed rather than left in the document as text.",
        "\\providecommand{\\NAME}[ARGC]{BODY}\n\\newcommand{\\x}{first}\n\\providecommand{\\x}{second}\n\\message{\\x}   % => first",
    ),
    (
        "\\DeclareRobustCommand",
        "LaTeX",
        "Define a macro. Robustness is a property of LaTeX's expansion-in-moving-arguments machinery, which has no counterpart here, so this behaves exactly as `\\newcommand`.",
        "\\DeclareRobustCommand{\\NAME}[ARGC]{BODY}\n\\DeclareRobustCommand{\\x}{text}\n\\message{\\x}   % => text",
    ),
    (
        "\\documentclass",
        "LaTeX",
        "Consumed, with its optional arguments, and produces nothing. A class is TeX that builds boxes and there is no stomach to build them in, so the directive is read and dropped — which is what lets the REST of the document be read instead of the run failing at line one.",
        "\\documentclass[OPTIONS]{CLASS}\n\\documentclass[12pt]{article}\n\\message{the body still runs}",
    ),
    (
        "\\usepackage",
        "LaTeX",
        "Consumed with its optional arguments, producing nothing, for the same reason as `\\documentclass`: the package cannot be loaded, and dropping it reads the document minus whatever the package would have drawn.",
        "\\usepackage[OPTIONS]{PACKAGE}\n\\usepackage[utf8]{inputenc}",
    ),
    (
        "\\RequirePackage",
        "LaTeX",
        "Consumed with its optional arguments, producing nothing. Same treatment as `\\usepackage`; the difference between them is where LaTeX allows each, which does not matter to an engine that loads neither.",
        "\\RequirePackage[OPTIONS]{PACKAGE}\n\\RequirePackage{amsmath}",
    ),
    (
        "\\PassOptionsToPackage",
        "LaTeX",
        "Consumed with both of its arguments, producing nothing, because the package it would have carried options to is never loaded.",
        "\\PassOptionsToPackage{OPTIONS}{PACKAGE}\n\\PassOptionsToPackage{dvipsnames}{xcolor}",
    ),
    (
        "\\PassOptionsToClass",
        "LaTeX",
        "Consumed with both of its arguments, producing nothing. The class counterpart of `\\PassOptionsToPackage`.",
        "\\PassOptionsToClass{OPTIONS}{CLASS}\n\\PassOptionsToClass{a4paper}{article}",
    ),
    (
        "\\begin",
        "LaTeX",
        "Open an environment. Most environments are the prelude's business, but a VERBATIM one is the lowerer's: it is caught here, before `\\begin` expands, because expanding is exactly what must not happen to the body. A code listing is full of backslashes that are not control sequences, and reading them as control sequences is why a book of code samples could not be read at all. The body is taken as raw characters up to the matching `\\end`.",
        "\\begin{ENVIRONMENT}\n\\begin{verbatim}\n\\this is text, not a command\n\\end{verbatim}",
    ),
    // ── Page structure — lower::lower_page_break ─────────────────────────
    (
        "\\newpage",
        "LaTeX",
        "Start a new page. The prelude defined this to expand to nothing, so a book's title page, copyright page and first chapter ran together as one stream of prose and the page count came out at roughly half what the document asks for. Carried through the text as a form feed, which is what the character means, and split out before words are because Rust counts it as whitespace.",
        "\\newpage\nfirst page\n\\newpage\nsecond page",
    ),
    (
        "\\clearpage",
        "LaTeX",
        "Start a new page, after placing any pending floats. There are no floats here, so it is `\\newpage`. Two breaks in a row are one break: `\\clearpage` straight after `\\newpage` does not leave a blank sheet between them.",
        "\\clearpage",
    ),
    (
        "\\cleardoublepage",
        "LaTeX",
        "Start a new page, and in a two-sided document a new RIGHT-hand one. The blank verso a two-sided run would insert is not written, so this is `\\clearpage` here.",
        "\\cleardoublepage",
    ),
    (
        "\\pagebreak",
        "LaTeX",
        "Break the page. `\\pagebreak[0-4]` takes an optional strength, which is advice about how badly the break is wanted rather than a break itself; the break is taken either way, because the document asked for one.",
        "\\pagebreak\n\\pagebreak[4]",
    ),
    (
        "\\chapter",
        "LaTeX",
        "Begin a chapter, on a new page. The prelude defined this as its own argument -- the heading text and nothing else -- so no chapter began a page. `\\chapter*{...}` is the unnumbered form and `\\chapter[short]{long}` carries a running-head title; both start a page and both set the long title. The heading is not yet set in a larger face, and the number is not printed.",
        "\\chapter{TITLE}\n\\chapter*{Preface}\n\\chapter[Short]{A Longer Title}",
    ),
    // ── Colour — lower::lower_colour, colour.rs ──────────────────────────
    (
        "\\definecolor",
        "LaTeX",
        "Name a colour: `\\definecolor{neonCyan}{HTML}{05D9E8}`. The models read are `HTML` (six hex digits), `rgb` (three components in 0..=1), `RGB` (the same in 0..=255), `gray` and `cmyk`. A model that is not one of those defines nothing rather than guessing, because a colour read in the wrong model is wrong on every page it reaches. Documents define their palette once in the preamble and refer to it by name afterwards, so without this every later `\\color` names something unknown and the page comes out black.",
        "\\definecolor{NAME}{MODEL}{SPEC}\n\\definecolor{neonCyan}{HTML}{05D9E8}",
    ),
    (
        "\\providecolor",
        "LaTeX",
        "Define a colour only if that name is not already defined. Otherwise `\\definecolor`.",
        "\\providecolor{NAME}{MODEL}{SPEC}\n\\providecolor{link}{HTML}{05D9E8}",
    ),
    (
        "\\colorlet",
        "LaTeX",
        "Give an existing colour another name. A name nothing has defined yet defines nothing.",
        "\\colorlet{NEW}{OLD}\n\\definecolor{brand}{HTML}{FF2A6D}\n\\colorlet{heading}{brand}",
    ),
    (
        "\\color",
        "LaTeX",
        "Switch the colour of everything that follows, until the group holding it closes -- a switch rather than a wrapper, which is why `{\\color{red}...}` colours only what is inside the braces. Takes a defined name, or a model and a spec: `\\color[rgb]{1,0,0}`. A second `\\color` in one group replaces the first rather than nesting, because TeX has one current colour and not a stack. A name nothing defined leaves the text in the colour it already had.",
        "\\color{NAME}\n\\color[MODEL]{SPEC}\n{\\color{neonCyan}cyan words}",
    ),
    (
        "\\textcolor",
        "LaTeX",
        "Colour exactly one argument: `\\textcolor{neonCyan}{words}`. Takes the same two forms as `\\color`. Unlike `\\color` it puts the previous colour back afterwards, so it never leaks into the text beside it.",
        "\\textcolor{NAME}{TEXT}\n\\textcolor[MODEL]{SPEC}{TEXT}\n\\textcolor{red}{warning}",
    ),
    (
        "\\pagecolor",
        "LaTeX",
        "Paint the page. Drawn under everything else, which is the only order that leaves the words on top of it -- and it has to be honoured, because a document that sets a dark page also sets light text to go on it, and doing one without the other leaves white on white.",
        "\\pagecolor{NAME}\n\\definecolor{bgPrimary}{HTML}{05050A}\n\\pagecolor{bgPrimary}",
    ),
    (
        "\\setmainfont",
        "LaTeX",
        "Record the document's body typeface, and keep the name rather than dropping it. The PDF backend looks the family up on the system and EMBEDS it when it finds a TrueType-flavoured one, so the page is set in that face and measured with its own widths; failing that it falls back to one of the fourteen faces every reader has, chosen by metrics — Arimo, Liberation Sans and Arial all set at Helvetica's widths, and a name nothing is known about falls to Helvetica too. The DVI backend names `.tfm` fonts and cannot carry an OpenType one, so it still sets in Computer Modern. An optional bracket on either side of the family is consumed.",
        "\\setmainfont[OPTIONS]{FAMILY}[OPTIONS]\n\\setmainfont{Arimo}   % embedded if Arimo is installed, else Helvetica's metrics",
    ),
    (
        "\\setromanfont",
        "LaTeX",
        "The older fontspec spelling of `\\setmainfont`, and the same thing here: it fills the same slot, so whichever of the two the preamble writes last is the family the PDF backend embeds or maps.",
        "\\setromanfont[OPTIONS]{FAMILY}\n\\setromanfont{Arimo}   % identical in effect to \\setmainfont{Arimo}",
    ),
    (
        "\\setsansfont",
        "LaTeX",
        "Record the document's sans-serif family. The name is read and kept and its bracketed options are consumed, but no backend selects it yet: the PDF page is set in the main family throughout, so this records an intention rather than changing the output. Documented because a preamble writes it and the engine resolves it.",
        "\\setsansfont[OPTIONS]{FAMILY}[OPTIONS]\n\\setsansfont{Arimo}",
    ),
    (
        "\\setmonofont",
        "LaTeX",
        "Record the document's monospace family, the counterpart of `\\setsansfont`. Read and kept, its options consumed, and likewise not yet selected by either backend — the PDF page is set in the main family throughout.",
        "\\setmonofont[OPTIONS]{FAMILY}[OPTIONS]\n\\setmonofont{Cousine}",
    ),
    (
        "\\makeatletter",
        "LaTeX",
        "Make `@` a letter (category code 11), so LaTeX's internal names like `\\@ifnextchar` become spellable as single control sequences. This is a catcode change, so it takes effect at COMPILE time, exactly as `\\catcode`\\@=11` does.",
        "\\makeatletter <internal names> \\makeatother",
    ),
    (
        "\\makeatother",
        "LaTeX",
        "Make `@` an ordinary character again (category code 12), closing a `\\makeatletter` region. After it, `\\@x` is the control sequence `\\` followed by the characters `@x` rather than one name.",
        "\\makeatletter <internal names> \\makeatother",
    ),
];

/// The entry for `name`, if the corpus documents it.
pub fn lookup(name: &str) -> Option<&'static Entry> {
    CORPUS.iter().find(|(n, ..)| *n == name)
}
