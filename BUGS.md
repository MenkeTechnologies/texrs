# Known gaps

texrs implements TeX's mouth and expander, and enough of a stomach to ship a
page. What follows is what is deliberately not done, and what is done
differently. Everything here was
measured against **tex 3.141592653** (TeX Live 2026).

That version string is not decoration: `src/parity.rs` reads it out of this file
and refuses to run any parity harness against a different engine. A
mismatched oracle does not fail loudly — it reports a different set of
divergences, which reads exactly like a regression in texrs. `TEX_VERSION_EXPECT`
overrides it for a deliberate cross-version run.

## The corpus

Every committed case in `tests/cases` either matches the reference engine or is
listed in `tests/known_gaps.txt` with the reason it does not. The gate fails on
any divergence that is not listed there AND on a listed case that has started
passing, so the list is a claim the harness enforces rather than a note.

## Not implemented

- **`tex.web`'s stomach.** `--dvi` measures the text in a real `.tfm`, breaks it
  into lines with the first break that fits, stacks them at a fixed leading and
  ships DVI (`src/typeset.rs`). That is the whole of it. TeX chooses breakpoints
  by minimising total badness over every feasible sequence (§813-§890); there is
  no hyphenation, no glue stretching or shrinking, no page breaking by
  penalties, no maths, and no boxes a document can nest. So a paragraph set here
  and the same paragraph set by `tex` do not agree line for line, and the
  milestone's real parity bar — byte-identical DVI — is not approached.

  Without `--dvi`, `\end` stops the run and ships nothing. `tex` prints
  `No pages of output.` for the corpus here, which is why the parity contract
  for the committed cases is still the `\message` stream rather than the page.
- **Registers other than `\count`.** No `\dimen`, `\skip`, `\muskip`, `\toks`,
  `\box`, so `\ifdim`, `\ifvoid`, `\ifhbox` and `\ifvbox` are recognised as
  conditionals for skipping purposes but cannot be evaluated.
- **Mode and file conditionals.** `\ifvmode`, `\ifhmode`, `\ifmmode`,
  `\ifinner`, `\ifeof` — all of them test state that belongs to the stomach or
  to file I/O, neither of which exists yet.
- **`\aftergroup`, `\afterassignment`, `\uppercase`/`\lowercase`, `\meaning`,
  `\jobname`.** Each stops the run with `! Undefined control sequence`.
  `\futurelet` was on this list and is no longer missing: it is in
  `src/expand.rs`, documented in the corpus, and pinned by `tests/futurelet.rs`.
  So was `\input`, which is now in `src/lower.rs` and pinned differentially by
  `tests/input.rs` — see "Finding files" below for what it does differently.
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

## Finding files

`\input` reads a file, and TeX finds one through kpathsea: a search path built
from `texmf.cnf`, the TEXMF trees and `TEXINPUTS`. texrs searches the working
directory and then `TEXINPUTS`, and deliberately does NOT shell out to
`kpsewhich` — a document that runs today would otherwise stop running on a
machine with no TeX Live installed, which is the opposite of what a
self-contained engine is for.

The consequence to know: `\input plain` finds nothing unless the TeX tree is on
`TEXINPUTS`, while `\input chapter1` beside the document always works. Fifteen
text input levels are allowed counting the document's own, which is tex's own
limit, reported in tex's own words (`! TeX capacity exceeded, sorry [text input
levels=15].`) — measured, not assumed.

A file that cannot be found stops the run naming it, where tex prompts for a
replacement. That is the error model recorded under "Not implemented" rather
than anything specific to files.

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

## Arithmetic

A `\count` register is a 32-bit word (`tex.web` §236), and texrs now holds it as
one. Measured against tex 3.141592653:

- `\advance` past the range WRAPS, silently, as tex does: `2147483647 + 1` is
  `-2147483648`. It is not an error in either engine.
- `\multiply` and `\divide` CHECK (`tex.web` §1236) and raise
  `Arithmetic overflow`, leaving the register alone. So does a division by zero.

The same 32-bit limit binds a constant the scanner READS, not just one it
computes: above 2147483647 tex reports `! Number too big.` and clamps to it
(`tex.web` §445), and the magnitude is tested before the sign, which is why a
too-big negative gives -2147483647 rather than -2147483648. texrs checked only
for host `i64` overflow until this was measured, so `\count1=99999999999` was
accepted outright and printed back.

The one divergence left is the recovery, not the arithmetic: tex reports and
carries on, texrs stops, which is the error model recorded under
"Not implemented". `tests/cases/multiply_overflow.tex` and
`tests/cases/number_too_big.tex` pin it.

## The JIT

The tracing JIT is enabled for an ordinary run, and NOT for `--dap`: a compiled
loop does not call the debugger's line marker, so a debug run stays interpreted.
`texrs --tiers FILE` reports what actually happened rather than what was enabled
— `traced=true` is a trace installed, `block-JIT compiled` is the whole chunk in
one piece — and `src/tiers.rs` pins that TeX's loop idiom reaches native code, so
the recogniser, the rotated lowering and the switch cannot silently be lost.

The JIT is switched on only for a chunk that HAS a loop, which is the shape a
tracing JIT is for. That is not only an optimisation: under fusevm 0.26.0,
switching it on for every run makes the third of three GROWING loop-free
documents fault in JIT-compiled code -- `EXC_BAD_ACCESS` at address 0, a null
base register in native code with no Rust frame on the stack. A REPL session is
exactly that shape, because every prompt re-runs the whole accumulated source,
so `texrs --repl` crashed on the third line that opened a group. Reusing one VM
per thread through `VM::reset` does NOT avoid it, so the fault is in the trace
tier rather than in stale slot buffers. `tests/jit_reentry.rs` holds the
reproducer through the public API. The gate costs nothing measurable -- a
program with no loop had nothing to gain -- and the 40-million-iteration loop
still runs in 0.07s.

fusevm's strict numeric mode (`set_numeric_hook` + `set_fixnum_range`) is NOT
used, though it describes a 32-bit integer type exactly and seven sibling
frontends use it. It was tried and measured: on a 40-million-iteration
`\advance` loop, strict mode ran in 0.14s against 0.11s for the wrapping that
`Compiler::wrap_to_32_bits` emits inline, because strict-mode native code
carries an overflow trap per arithmetic op. Both reach the tracing tier; the
block tier is out either way, on the message builtins rather than on the numeric
policy. The inline wrap is also the only one of the two that needs no host
callback, so it holds under `--aot`. Do not re-litigate without a new
measurement.

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
