```
████████╗███████╗██╗  ██╗██████╗ ███████╗
╚══██╔══╝██╔════╝╚██╗██╔╝██╔══██╗██╔════╝
   ██║   █████╗   ╚███╔╝ ██████╔╝███████╗
   ██║   ██╔══╝   ██╔██╗ ██╔══██╗╚════██║
   ██║   ███████╗██╔╝ ██╗██║  ██║███████║
   ╚═╝   ╚══════╝╚═╝  ╚═╝╚═╝  ╚═╝╚══════╝
                                         
```

A TeX engine in Rust: Knuth's **mouth** and **expander**, built to be lowered
onto a bytecode VM.

## What it is

TeX is two machines. The *mouth* turns bytes into tokens under a mutable
category-code table; the *expander* turns tokens into other tokens — `\def`,
`\csname`, `\the`, the conditionals. Only after that does the *stomach* build
boxes and ship DVI.

texrs implements the first two. That is the half a macro-heavy document spends
its time in, and the half where a compiled implementation has something to prove:
every mainstream engine (pdfTeX, XeTeX, LuaTeX) descends from `tex.web` through
web2c and *interprets* the expander.

## What works

- Category codes, `\catcode`, and INITEX's sparse defaults — `{` is not a group
  character until something makes it one, exactly as a bare `tex` run behaves.
- The three-state line scanner: blank line to `\par`, spaces collapsed, the
  space after a control word swallowed and after a control symbol kept.
- `^^X` notation.
- `\def` with undelimited *and* delimited parameters (`\def\pair#1,#2.{...}`),
  `##`, and nested definitions.
- `\csname`/`\endcsname`, `\string`, `\the`, `\number`, `\expandafter`.
- `\let`, `\edef`/`\xdef`, `\gdef`, `\global`, `\begingroup`/`\endgroup`.
- Conditionals: `\iftrue`, `\iffalse`, `\ifnum`, `\ifodd`, `\ifx`, `\ifcase`
  with `\or`, `\else`, `\fi` — nested, and inside a `\message` body.
- Groups, which scope the macro table AND the count registers they write.
- `\count` registers, `` `x `` character codes, `\advance`/`\multiply`/`\divide`.
- `\message`.

## What does not

No boxes, no glue, no paragraph breaking, no fonts, no DVI. This is not a
typesetter yet — see `docs/ROADMAP.md`.

## How it runs

texrs is a **fusevm frontend**, not an interpreter: mouth → expander → command
stream → fusevm bytecode → the VM runs it. A count register is a VM **slot**, so
`\advance\count0 by 5` is `GetSlot / LoadInt / Add / SetSlot` — native ops the
JIT can compile. `\ifnum` is `NumGt` + `JumpIfFalse`, a real branch.

A conditional whose truth depends only on the macro table (`\iftrue`, `\ifx`)
is folded while lowering instead, because there is nothing for the VM to test.

`tests/lowering.rs` asserts the emitted bytecode rather than the printed output —
output parity alone would not distinguish a frontend from a tree-walker.

## Parity

The contract is the `\message` stream, compared byte-for-byte against the real
`tex` binary. No expectation is written by hand:

```sh
bash scripts/parity.sh          # the committed corpus
bash scripts/parity.sh case.tex # one ad-hoc case
cargo test                      # the same comparison, as a gate
```

Both harnesses read the engine version they were measured against out of
`BUGS.md` and refuse to run against a different `tex`: a mismatched oracle does
not fail loudly, it reports a different set of divergences, which reads exactly
like a regression.

The corpus is small and deliberately awkward — `##` inside a nested `\def`,
catcode changes mid-file, `\csname` built from a macro, control-word space
swallowing, conditionals nested inside a `\message` body, `\ifcase`,
`\expandafter`, `\edef` freezing a register, a `\count` assignment scoped by a
group. Every case is in parity except the ones `tests/known_gaps.txt` names,
and the gate fails both on an unlisted divergence and on a listed case that has
started passing, so the list cannot go stale.

## Fuzzing

Hand-written cases only cover what someone thought to write down.

```sh
bash scripts/fuzz_parity.sh -n 500        # random programs, both engines, diffed
bash scripts/fuzz_parity.sh -1 case.tex   # one file, verbose
cargo +nightly fuzz run lower -- -timeout=10
```

`scripts/fuzz_parity.sh` generates seeded random programs confined to the
implemented subset, runs them under both engines in parallel, and REDUCES
whatever diverges — dropping statements for as long as the divergence survives,
and refusing a reduction that changes the divergence into a different one — so
what it hands back is small enough to commit to `tests/cases`. The same seed
generates the same corpus on any machine.

`fuzz/` is a cargo-fuzz crate (targets `lex`, `lower`, `run`) looking for panics
rather than divergences. `tests/fuzz_smoke.rs` replays each target on its seed
corpus under stable Rust, and `tests/fuzz_mass_replay.rs` points the mouth at
generated mutations of every `.tex` in the tree, so `cargo test` still exercises
the harness on a machine with no nightly toolchain.

## Licence

MIT.
