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
- Man pages (`texrs(1)`, `texrsall(1)`), a zsh completion, and the docs site
  (`docs/index.html`, `docs/report.html`, `docs/reference.html`).
- A JetBrains plugin under `editors/intellij`, ported from the sibling engines':
  highlighting by category code (the sixteen classes of `tex.web` §207, with the
  primitives texrs implements told apart from the control sequences a document
  defines), run configurations over the CLI including `--dump-tokens`,
  `--disasm` and `--no-cache`, `%` comments, brace matching, spell-checking that
  reads prose and skips markup, new-file templates that set the category codes
  INITEX leaves ordinary, and a colour settings page. No LSP client and no
  debugger, since texrs ships neither server — which is also why this plugin
  runs on Community editions where its siblings need a paid IDE.

### Fixed

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
