# Known gaps

texrs implements TeX's mouth and expander. What follows is what is deliberately
not done, and what is done differently — measured against `tex` 3.141592653
(TeX Live 2026).

## The corpus: 16 of 27 in parity

The engine was rebuilt to lower onto fusevm rather than interpret, and the
lowering pass is narrower than the interpreter it replaced. What still diverges,
by class:

- **Conditionals inside a `\message` body** (6 cases). `\ifnum` in running text
  lowers to `NumLt`/`JumpIfFalse`; inside a message it does not, because the
  message is built from parts and a branch there has to select between STRING
  pieces at run time. Needs runtime string assembly, not more parsing.
- **`\csname` inside a message** (2 cases). Same reason.
- **A `\count` assignment inside a group is not restored** (1 case). Grouping is
  compile-time here and restores the macro table, but a count register is a VM
  SLOT and restoring it needs a runtime save/restore around the group.
- **`\edef` capturing a register** (1 case). `\edef\x{\the\count0}` must
  freeze the value at definition time; the value lives in a slot, so this is a
  run-time operation that the compile-time `\edef` cannot see.
- **`\expandafter` in running text** (1 case). Handled in the old expander,
  not yet in the lowering pass.

## Not implemented

- **The stomach.** No boxes, glue, paragraphs, fonts or DVI. `\end` stops the
  run; it does not ship a page. `tex` prints `No pages of output.` for the
  corpus here, which is why the parity contract is the `\message` stream.
- **Conditionals.** `\if`, `\ifnum`, `\ifx`, `\else`, `\fi` are not implemented.
  A document using them will fail with `Undefined control sequence`, which is
  honest but useless — this is the next thing worth doing (milestone 3).
- **Groups.** `{...}` delimits macro arguments but does not open a save-stack
  group, so an assignment inside braces is not undone at the closing brace.
- **Registers other than `\count`.** No `\dimen`, `\skip`, `\muskip`, `\toks`,
  `\box`.
- **`\edef`, `\gdef`, `\xdef`, `\let`, `\futurelet`, `\noexpand`,
  `\expandafter`.** `\expandafter` in particular is load-bearing in real macro
  packages; its absence caps what can be run far more than the missing stomach.

## Divergences from tex

- **Characters above U+00FF are one Letter token.** TeX82 reads BYTES, so `é` in
  a UTF-8 file is two `Other` tokens there and one `Letter` here. That changes
  what `\string` prints and how a delimited argument matches. Deliberate for now
  — matching TeX means reading bytes, which is the right call but has to be made
  once, everywhere, rather than piecemeal.
