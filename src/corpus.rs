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
        "Control-character notation (tex.web \u{a7}352): a superscript character twice, then one character, denotes the control character 64 above or below it. This is how a line end is written inside a macro body.",
        "^^M   % carriage return\n^^I   % tab",
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
        "Define a macro. Parameters may be undelimited (`#1`) or delimited (`\\def\\pair#1,#2.{...}`, where the argument is whatever precedes the delimiter), and `##` is a literal parameter character. The parameter text is validated as tex.web \u{a7}476 validates it: every `#` must be followed by a digit, and the digits must run consecutively. Takes effect at COMPILE time.",
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
        "True when a number is odd. Lowers to a real branch.",
        "\\ifodd<number> <true>\\else <false>\\fi\n\\message{\\ifodd\\count1 ODD\\else EVEN\\fi}",
    ),
    (
        "\\ifcase",
        "Conditionals",
        "Switch on a number: the first case is 0, each `\\or` starts the next, and `\\else` catches everything past the last. Lowers to a real branch.",
        "\\ifcase<number> <0>\\or <1>\\or <2>\\else <other>\\fi\n\\count1=2\n\\message{\\ifcase\\count1 ZERO\\or ONE\\or TWO\\else MANY\\fi}   % => TWO",
    ),
    (
        "\\iftrue",
        "Conditionals",
        "Constant truth. Decidable without the VM, so it is FOLDED while lowering and the untaken arm is never emitted.",
        "\\iftrue <true>\\else <false>\\fi",
    ),
    (
        "\\iffalse",
        "Conditionals",
        "Constant falsity. Folded while lowering, like `\\iftrue`.",
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
        "Compare two character codes after expansion.",
        "\\if<token><token> <true>\\else <false>\\fi",
    ),
    (
        "\\ifdefined",
        "Conditionals",
        "True when a control sequence has a meaning.",
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
        "RECOGNISED BUT NOT EVALUATED. Compares category codes; texrs skips the construct correctly so an unbalanced branch cannot confuse the scanner, but cannot decide it.",
        "\\ifcat<token><token> <true>\\else <false>\\fi",
    ),
    (
        "\\ifdim",
        "Conditionals",
        "RECOGNISED BUT NOT EVALUATED: there are no dimen registers yet.",
        "\\ifdim<dimen><rel><dimen> <true>\\else <false>\\fi",
    ),
    (
        "\\ifvoid",
        "Conditionals",
        "RECOGNISED BUT NOT EVALUATED: there are no box registers yet.",
        "\\ifvoid<N> <true>\\else <false>\\fi",
    ),
    (
        "\\ifhbox",
        "Conditionals",
        "RECOGNISED BUT NOT EVALUATED: there are no box registers yet.",
        "\\ifhbox<N> <true>\\else <false>\\fi",
    ),
    (
        "\\ifvbox",
        "Conditionals",
        "RECOGNISED BUT NOT EVALUATED: there are no box registers yet.",
        "\\ifvbox<N> <true>\\else <false>\\fi",
    ),
    (
        "\\ifvmode",
        "Conditionals",
        "RECOGNISED BUT NOT EVALUATED: modes belong to the stomach.",
        "\\ifvmode <true>\\else <false>\\fi",
    ),
    (
        "\\ifhmode",
        "Conditionals",
        "RECOGNISED BUT NOT EVALUATED: modes belong to the stomach.",
        "\\ifhmode <true>\\else <false>\\fi",
    ),
    (
        "\\ifmmode",
        "Conditionals",
        "RECOGNISED BUT NOT EVALUATED: modes belong to the stomach.",
        "\\ifmmode <true>\\else <false>\\fi",
    ),
    (
        "\\ifinner",
        "Conditionals",
        "RECOGNISED BUT NOT EVALUATED: modes belong to the stomach.",
        "\\ifinner <true>\\else <false>\\fi",
    ),
    (
        "\\ifeof",
        "Conditionals",
        "RECOGNISED BUT NOT EVALUATED: there is no file I/O yet.",
        "\\ifeof<N> <true>\\else <false>\\fi",
    ),
    (
        "\\ifcsname",
        "Conditionals",
        "RECOGNISED BUT NOT EVALUATED. Use `\\ifdefined` on a `\\csname`-built token instead.",
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
