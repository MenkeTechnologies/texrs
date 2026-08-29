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
- `\csname`/`\endcsname`, `\string`, `\the`.
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

The corpus is small and deliberately awkward — `##` inside a nested `\def`,
catcode changes mid-file, `\csname` built from a macro, control-word space
swallowing. 16 of 27 cases are in parity. The engine was rebuilt to lower onto fusevm rather
than interpret, and the lowering pass is narrower than the interpreter it
replaced; `BUGS.md` lists every remaining divergence by class, the largest being
conditionals inside a `\message` body.

## Licence

MIT.
