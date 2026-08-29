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
