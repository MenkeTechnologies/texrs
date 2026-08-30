# Changelog

All notable changes to texrs are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `.tfm` reading, ported from xetex's `read_font_info` (`tex.web` §539-§576):
  character widths, heights, depths and italic corrections, the ligature and
  kern program, and the `FONTDIMEN` parameters. `-X tfm FILE.tfm [CHAR]` prints
  a font's own description or one character's, and takes a font name as well as
  a path. Held against `tftopl`, Knuth's own reader, for every character of
  cmr10, cmmi10, cmsy10, cmex10 and cmtt10, and for cmr10's whole lig/kern
  program. This is the piece nothing that sets type can be right without, and it
  is what `width$` in a `.bst` was waiting on.

## [0.4.0] - 2026-08-30

Versions 0.2.0, 0.3.0 and 0.3.1 were released without cutting a section here;
everything below had accumulated under Unreleased and is filed under 0.4.0,
which is the release that carries it to crates.io and the Homebrew tap.

### Added

- A LaTeX layer for the half of LaTeX that lives in the mouth and the expander.
  `src/latex/prelude.tex` is a file of `\newcommand`s compiled into the binary,
  and a document naming `\documentclass`, `\usepackage`, `\PassOptionsToPackage`
  or `\RequirePackage` is recognised as LaTeX and lowered against it.
  `\newcommand`, `\renewcommand`, `\providecommand` and
  `\DeclareRobustCommand` are dispatched natively rather than through
  latex.ltx's `\ifnum` chain, which cannot run here because lowering emits both
  arms of a conditional. The preamble directives are consumed and produce
  nothing; `\makeatletter` and `\makeatother` are the catcode change they are.
  Against a 167-file LaTeX/LuaLaTeX corpus this moves the compile count from 0
  to 2 and moves the remaining failures out of the preamble directives and into
  the packages themselves — 122 now stop at `\defaultfontfeatures`, 41 at
  `\directlua`.
- The eleven LaTeX control sequences are in `src/corpus.rs` under a new "LaTeX"
  chapter, so they reach `docs/reference.html`, editor completion and hover the
  same way every other primitive does.
- `-X bst FILE.bst` reads a BibTeX style: the fields an entry may carry, the
  `MACRO` abbreviations, the functions, and the `READ`/`SORT`/`ITERATE` that say
  what it does. It also names every function the style calls that nothing
  defines. bibtex reports those one per run, at run time, so a style with three
  gaps costs three builds to find; this asks once and exits non-zero. Read
  against the styles TeX Live ships (`plain`, `unsrt`, `abbrv`, `alpha`), which
  parse whole with nothing undefined. The interpreter is deliberately not here:
  it needs `width$`, which measures a string in a font and so needs a `.tfm`.
- `-X build --help`, and the same for `-X watch` and `-X dump`, print the usage
  instead of reporting `--help` as an unknown argument.
- `tests/cli.rs` now holds the `-X` commands to the same drift guard the options
  have: every command the usage text lists must be offered by the zsh completion
  and documented in the man page, and the completion may offer none the binary
  does not list.
- Bytecode cache: what a document compiled to is kept in an rkyv shard keyed by
  path, valid while the source's mtime matches to the nanosecond, so a second
  run skips the mouth, the expander and the lowerer. `--no-cache`,
  `--cache-stats`, `--cache-clear`, and `TEXRS_CACHE=0`.
- `--build` compiles a document into the bytecode cache and stops. A build step
  that runs it leaves the run that follows starting from bytecode: on a hit the
  mouth, the expander and the lowerer are skipped entirely. Nothing the document
  does at RUN time happens, because none of that is compilation.
- `parity-fuzz`, the differential fuzzer as a binary, ported in shape from the
  sibling frontends' — and for their reason. The oracle is the expensive part:
  a `tex` invocation costs ~0.5s of process start and format load, so a fuzzer
  running one construct per invocation spends its budget on startup. Each
  program now packs 40 independent probes, and a divergence is minimized to the
  single probe responsible before it is reported. 1600 probes cost 80 tex
  invocations rather than 1600. It replaces `scripts/fuzz_parity.sh` and
  `scripts/fuzz/gen.pl`: one implementation of the harness, in the language the
  engine is written in, with no bash or perl in the loop.
- `tests/opcodes.rs` guards the builtin id space. The ids are a wire format,
  not an internal detail: a cached chunk and an `--aot` object both call
  builtins by number, so renumbering one does not fail to compile — it makes
  every artefact written before the change call the wrong function. The gate
  fails on a duplicate id, a declared-but-unregistered op, a double
  registration, a call written as a bare number, and any movement of the five
  ids already on disk.
- `tests/embed.rs` pins the library API from an embedder's side: output comes
  back as values rather than going to the terminal, a failure is a `Result`
  rather than a panic or an exit, and one run leaves nothing behind for the
  next — no macro, no register, no catcode, no message buffer, no fault. The
  binary runs one document and exits, so a leak between runs is invisible to
  it; these are what say so out loud.
- The command line is `tex`'s. A bare name gets `.tex`; a first argument
  beginning with `\` makes the whole list a line of input with no file; further
  arguments after a file are input read after it — unless the file's own `\end`
  stopped the run, which `tex` also honours; `-interaction=batchmode` writes
  nothing to the terminal; and `-jobname`, `-output-directory`, `-progname`,
  `-fmt`, `-ini`, `-halt-on-error`, `-file-line-error`, `-recorder` and `-8bit`
  are accepted in tex's own spelling, so an invocation written for `tex` drives
  texrs unchanged. `tests/cli_tex.rs` compares each form against the real binary,
  including which side of the closing paren a message lands on.
- `texrs` with no arguments opens the prompt rather than printing its usage.
  `tex` prompts for input here too: an engine given nothing to do should ask.
- `--help` is painted in the house palette — yellow `USAGE:`, bold program name
  and flags, cyan section rules, green `//` — and only when stdout is a
  terminal. A pipe gets plain bytes, which is what keeps an escape out of a file
  someone is grepping and what `tests/cli.rs` reads the flag list out of. The
  banner follows the same rule.
- The banner is the fleet's — logo, live-stats box, tagline — shared by the
  prompt, `--help` and `--version`, with every count read from the tables at
  call time so it cannot go stale. `--help` prints it above a sectioned option
  list in house style.
- Intercepts: `\intercept{before|after|around}{<glob>}{\handler}` weaves advice
  into macro expansion, with `\proceed` standing for the original expansion
  inside an `around` handler. The pattern is a glob over macro names, so advice
  registered now catches macros a package defines later — a registry keyed by
  exact name would need the document to know every name up front, which is the
  thing a macro package makes impossible. Expansion is a compile-time act here,
  so advice is woven into the token stream and undone by the group that
  registered it. A call inside advice is not advised, which is what keeps a
  handler that calls the macro it advises from weaving itself forever; the depth
  travels in the token stream on two markers the mouth cannot produce.
- Inline Rust: a `\rust{ … }` block is compiled by `rustc`, loaded, and its
  exported functions are callable as `\rustcall <name> <numbers…>\endrust` —
  a number wherever TeX reads one. The block is lifted out of the file before
  the mouth reads it, because its body is Rust and `#`, `{`, `}` and `&` are
  category codes the mouth would act on. The replacement carries no braces, so
  it reads correctly whatever the catcodes are where the block appeared. A block
  that does not compile stops the run with rustc's own diagnostic.
- `--aot`: compile a document to a standalone native executable. The chunk is
  emitted as a relocatable object through fusevm's ahead-of-time compiler and
  linked against the texrs runtime staticlib, so the result runs with no
  interpreter dispatch loop and needs no texrs on the machine. It prints exactly
  what an ordinary run prints, which `tests/aot.rs` checks byte for byte —
  "compiled" that behaves differently is not a compiler. Short compared with the
  sibling frontends' AOT paths for a structural reason: their closures live in a
  host table outside the bytecode and have to be smuggled through the chunk and
  rebuilt, while texrs has nothing outside the chunk. A macro is gone by the
  time the VM starts.
- `--repl`: an interactive prompt. A line is read with every line before it
  still in effect — the session re-lowers and re-runs the document it has built,
  so a `\catcode` changes how the next line reads and a register assignment
  survives, because it IS the same program with one more line. A line that fails
  is rolled back rather than left in the document, `\end` ends the line rather
  than the session, and with stdin on a pipe the line editor is skipped so
  `texrs --repl < doc.tex` works.
- `src/banner.rs`: the version line names the TeX level first and the engine
  second, so nothing is misrepresented as TeX Live.
- `--dap`: a debug adapter over stdio — source-line breakpoints, single
  stepping, a stack frame, and the `\count` registers as the variables scope.
  What is debuggable is what survives lowering: macros expand at compile time,
  so a breakpoint stops on lines that left run-time work behind, and one set on
  a line that did not is reported unverified rather than silently never firing.
- Source lines on every op. The lowerer emits a line directive when the line
  changes and the code generator stamps it onto each op, so `--disasm` reads
  against the document and the debugger has something to map. Before this every
  op reported line 0.
- `--tiers`: run a document, then report what fusevm's tiers did with its
  bytecode — block-tier eligibility, the largest eligible op region, every loop
  header and whether the tracing tier compiled it, and the ops the block tier
  refuses with their counts. The answers come from fusevm's own predicates,
  because enabling the JIT is not the same as being compiled by it.
- `--lsp`: a language server over stdio. Completion and hover answer from
  `src/corpus.rs` — the same table that generates `docs/reference.html`, so the
  editor and the site cannot disagree — and diagnostics come from the engine's
  own lowerer, landing on the line the mouth had reached.
- `src/corpus.rs`, the primitive reference table, with `tests/corpus_coverage.rs`
  holding it against the engine's dispatch in both directions: a primitive the
  engine gained and the corpus never heard of fails, and so does an entry naming
  a control sequence the engine no longer dispatches.
- `cargo run --bin gen-docs` regenerates `docs/reference.html` from the corpus,
  and `tests/docs_generated.rs` fails when the committed page and the generator
  disagree.
- `--dump-tokens` and `--disasm`: the mouth's token stream, and the lowered
  fusevm bytecode.
- Differential fuzzing: `the parity-fuzz binary` generates seeded random
  programs confined to the implemented subset, runs both engines in parallel,
  and reduces whatever diverges to a minimal case.
- cargo-fuzz targets `lex`, `lower` and `run`, with `tests/fuzz_smoke.rs`
  replaying them on stable and `tests/fuzz_mass_replay.rs` pointing the mouth at
  generated mutations of every `.tex` in the tree.
- `examples/`, held in parity with real tex by `tests/examples.rs` with no
  known-gap escape hatch.
- `editors/` configuration for coc.nvim, Helix, Vim/Neovim (native LSP) and
  VS Code, pointing at `texrs --lsp` and `texrs --dap`.
- `docs/_config.yml`, excluding Markdown from the Pages build: GitHub Pages
  ships Jekyll 3.10, whose Liquid parser aborts the whole site on a TeX brace.
- Man pages (`texrs(1)`, `texrsall(1)`), a zsh completion, and the docs site
  (`docs/index.html`, `docs/report.html`, `docs/reference.html`).
- A JetBrains plugin under `editors/intellij`, ported from the sibling engines':
  highlighting by category code (the sixteen classes of `tex.web` §207, with the
  primitives texrs implements told apart from the control sequences a document
  defines), run configurations over the CLI including `--dump-tokens`,
  `--disasm` and `--no-cache`, `%` comments, brace matching, spell-checking that
  reads prose and skips markup, new-file templates that set the category codes
  INITEX leaves ordinary, and a colour settings page. Since the engine grew
  `--lsp` and `--dap`, the plugin drives both: completion over the primitives
  the engine implements, hover and diagnostics from the real mouth and expander,
  and a debugger with line breakpoints, stepping, frames, scopes, variables,
  evaluation and run-to-cursor. A paid IDE is required, as for the sibling
  plugins, because the platform LSP API is not in the Community editions.

- Benchmarks. `cargo bench` measures the pipeline against itself — the mouth
  alone, the frontend, the VM alone, and the whole run — so a slow expander and
  a slow VM cannot hide behind one end-to-end number, plus a size sweep that
  shows how the cost scales. `bench/compare.sh` measures the only comparison
  that says whether the engine is fast: the same documents through texrs and
  real `tex`, end to end, with the two caveats printed beside the numbers.

### Fixed

- `\edef` dropped its parameter text. `\edef\pair#1,#2.{…}` matched nothing and
  left its delimiters in the output where tex prints the arguments; `\edef`
  differs from `\def` only in WHEN the body is expanded, not in whether it takes
  parameters. Found by `parity-fuzz` on its third program, and pinned by
  `tests/cases/edef_with_parameters.tex`.

- `Lexer::line` counted newlines from the start of the file on every call, which
  is O(n) per token and O(n²) per document: the scaling benchmark caught it on
  its first run, where quadrupling the input cost 27x. It now counts only what
  has been consumed since the last answer. The 800-statement document went from
  248 ms to 11.7 ms; the sweep is close to linear rather than quadratic.

- A panic in the argument reader on TeX's `#{` parameter form, found by
  `cargo fuzz run lower`. The parameter text is now validated at definition
  time as `tex.web` §476 validates it.
- The parity harnesses misread any tex output over 79 columns: tex wraps its
  terminal output at `max_print_line` and the break can land right after the
  filename. `max_print_line` is now pinned high and continuation lines are
  joined regardless. The extractor also cut at the first close paren, truncating
  any message that printed one.

## [0.1.0] - 2026-08-29

### Added

- TeX's mouth and expander, lowered onto fusevm bytecode: category codes, the
  three-state line scanner, `^^X`, `\def` with delimited parameters, `\let`,
  `\edef`/`\xdef`/`\gdef`/`\global`, `\csname`, `\string`, `\the`, `\number`,
  `\expandafter`, `\noexpand`, the conditionals, groups scoping both the macro
  table and the registers written inside them, the `\count` registers with
  `\advance`/`\multiply`/`\divide`, and `\message`.
- The differential corpus in `tests/cases`, compared against the real `tex`
  binary with no hand-written expectations, and `tests/known_gaps.txt` recording
  every case that does not match yet with its reason.
- `tests/lowering.rs`, which asserts the emitted bytecode rather than the
  printed output — output parity alone would not distinguish a frontend from a
  tree-walker.

[Unreleased]: https://github.com/MenkeTechnologies/texrs/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/MenkeTechnologies/texrs/releases/tag/v0.1.0
