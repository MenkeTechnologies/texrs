# Changelog

All notable changes to texrs are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Bytecode cache: what a document compiled to is kept in an rkyv shard keyed by
  path, valid while the source's mtime matches to the nanosecond, so a second
  run skips the mouth, the expander and the lowerer. `--no-cache`,
  `--cache-stats`, `--cache-clear`, and `TEXRS_CACHE=0`.
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
- Differential fuzzing: `scripts/fuzz_parity.sh` generates seeded random
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
