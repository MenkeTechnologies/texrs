# Known gaps

texrs implements TeX's mouth and expander. What follows is what is deliberately
not done, and what is done differently — measured against `tex` 3.141592653
(TeX Live 2026).

## The corpus: 27 of 27 in parity

Every committed case matches `tex` 3.141592653 (TeX Live 2026), and
`tests/known_gaps.txt` is empty. The gate fails on any divergence not listed
there AND on a listed case that starts passing, so an empty list is a claim the
harness enforces rather than a note.

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
