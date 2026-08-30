# Known gaps

texrs implements TeX's mouth and expander. What follows is what is deliberately
not done, and what is done differently. Everything here was
measured against **tex 3.141592653** (TeX Live 2026).

That version string is not decoration: `src/parity.rs` reads it out of this file
and refuses to run any parity harness against a different engine. A
mismatched oracle does not fail loudly — it reports a different set of
divergences, which reads exactly like a regression in texrs. `TEX_VERSION_EXPECT`
overrides it for a deliberate cross-version run.

## The corpus

Every committed case in `tests/cases` matches the reference engine, and
`tests/known_gaps.txt` is empty. The gate fails on any divergence not listed
there AND on a listed case that has started passing, so an empty list is a claim
the harness enforces rather than a note.

## Not implemented

- **The stomach.** No boxes, glue, paragraphs, fonts or DVI. `\end` stops the
  run; it does not ship a page. `tex` prints `No pages of output.` for the
  corpus here, which is why the parity contract is the `\message` stream.
- **Registers other than `\count`.** No `\dimen`, `\skip`, `\muskip`, `\toks`,
  `\box`, so `\ifdim`, `\ifvoid`, `\ifhbox` and `\ifvbox` are recognised as
  conditionals for skipping purposes but cannot be evaluated.
- **Mode and file conditionals.** `\ifvmode`, `\ifhmode`, `\ifmmode`,
  `\ifinner`, `\ifeof` — all of them test state that belongs to the stomach or
  to file I/O, neither of which exists yet.
- **`\aftergroup`, `\afterassignment`, `\uppercase`/`\lowercase`, `\meaning`,
  `\jobname`, `\input`.** Each stops the run with `! Undefined control
  sequence`. `\futurelet` was on this list and is no longer missing: it is in
  `src/expand.rs`, documented in the corpus, and pinned by `tests/futurelet.rs`.
- **`#{` parameter text.** A parameter delimited by the left brace, which tex
  then puts back: `\def\a#{[X]}` called as `\a{Y}` prints `[X]{Y}`. texrs
  refuses the definition. Until `cargo fuzz run lower` found it, the argument
  reader indexed past the end of the parameter list on the trailing `#` and
  PANICKED; `src/expand.rs` now validates the parameter text at definition time,
  as `tex.web` §476 does, and `fuzz/corpus/lower/crash_param_brace.tex` keeps the
  crashing input.
- **`\edef` does not freeze a conditional.** tex decides `\ifcase`/`\ifodd`
  inside an `\edef` body while READING it, so the body becomes the token run
  the branch produced and a later register change cannot move it. texrs keeps
  the conditional in the body and decides it at use time, which also makes two
  bodies tex froze to the same tokens compare unequal under `\ifx`.
  `tests/cases/edef_freezes_conditional.tex` and
  `tests/cases/ifx_after_edef_conditional.tex` pin both faces of it; found by
  `the parity-fuzz binary` at seed 77.
- **Errors.** An undefined control sequence is not an error: texrs prints its
  name into the message stream and exits 0, where tex reports `! Undefined
  control sequence.` and expands it to nothing. `tests/cases/undefined_cs.tex`
  pins it. Every other error path is the same shape -- texrs either handles the
  construct or stops with one `TexError`, and does not have tex's recover-and-
  continue behaviour.
- **No expansion budget.** `\def\x{\x}\x` expands forever, exactly as it does in
  real tex — neither engine has a step limit, so this is parity rather than a
  bug. It is why the fuzz targets are run under a timeout (see below).

## Divergences from tex

- **Characters above U+00FF are one Letter token.** TeX82 reads BYTES, so `é` in
  a UTF-8 file is two `Other` tokens there and one `Letter` here. That changes
  what `\string` prints and how a delimited argument matches. Deliberate for now
  — matching TeX means reading bytes, which is the right call but has to be made
  once, everywhere, rather than piecemeal.
- **No format is preloaded.** The oracle is `tex`, which loads `plain.tex`, so
  `\count0` already holds the page number 1 and `\count255` holds plain's own
  scratch value. texrs starts every register at zero, as INITEX does.
  `tests/cases/plain_count0.tex` pins that, and `scripts/fuzz/gen.pl` generates
  against `\count1`..`\count9`, the window where both engines start equal.
- **A negative `\ifcase` selector takes case 0.** `tex.web` §509 skips n cases
  and takes the (n+1)th, so a selector below zero — or past the last `\or` with
  no `\else` — matches nothing and the `\else` branch runs. `do_ifcase` counts
  down with `while remaining > 0`, which a negative n never enters, so it falls
  through to case 0: `\ifcase -1 ZERO\else DEFAULT\fi` prints `ZERO` where tex
  prints `DEFAULT`. Like `def_in_conditional_arm.tex` this is a wrong answer
  rather than a refusal — nothing errors. Pinned by
  `tests/cases/cond_ifcase_negative.tex`. The fix looks like one branch in
  `do_ifcase`, but it is a semantics change and is recorded here first, as the
  roadmap's rule requires.
- **`\edef` scratch registers.** Freezing `\the\count0` into a macro body needs
  somewhere to put the value now, and the count registers are the only run-time
  store this milestone has. texrs takes them from the top (255 downward), so a
  document that both uses `\edef` and reads a high register can see a value real
  tex would not put there. Low registers are untouched.

## Inline Rust

- **Two runs compiling the same block at once can collide.** fusevm keys its FFI
  cache by the body's hash under one shared directory, and two `rustc`
  invocations landing there together trample each other's intermediate object
  files (`rust-lld: cannot open …rcgu.o`). One run is fine, and the second run of
  a document is a cache hit that compiles nothing; a build that starts several
  texrs processes on documents sharing a block is what to avoid.
  `tests/ffi.rs` serializes for the same reason.
- **A block needs `rustc` on PATH.** `RUSTC` overrides it. A block that does not
  compile stops the run with rustc's own diagnostic rather than failing later at
  the call.

## How gaps get found

Four harnesses, in increasing order of how much they cost to run:

| harness | what it does | how to run |
| --- | --- | --- |
| `tests/differential.rs` | every committed case against real `tex`, no hand-written expectations | `cargo test` (skips where there is no tex) |
| `tests/parity.rs` | the same corpus against the outputs `--freeze` recorded, so CI verifies it with no TeX installed | `cargo test` |
| `tests/examples.rs` | every example, live against tex where there is one and against the frozen output everywhere | `cargo test` |
| `tests/eval.rs` | one engine rule per test, so a break says which rule rather than which document | `cargo test` |
| `tests/fuzz_smoke.rs`, `tests/fuzz_mass_replay.rs` | replays the fuzz targets on the seed corpus and on generated mutations of every `.tex` in the tree | `cargo test` |
| `the parity-fuzz binary` | generates random programs in the implemented subset and diffs both engines, reducing whatever diverges | `cargo run --bin parity-fuzz -- --programs 200` |
| `fuzz/` (cargo-fuzz) | coverage-guided, looking for panics rather than divergences | `cargo +nightly fuzz run lower -- -timeout=10` |

A divergence the fuzzer finds is reduced to a minimal case and committed to
`tests/cases`, where the differential gate keeps it from coming back.
