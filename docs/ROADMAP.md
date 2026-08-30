# Roadmap

## Milestone 1 — the mouth and the expander (done)

Category codes, the line scanner, `\def` with delimited parameters, `\csname`,
`\the`, count registers and arithmetic, `\message`. Parity contract is the
`\message` stream against real `tex`.

## Milestone 2 — lower the expander to fusevm

The reason this project exists. A macro body is a token list interpreted on
every call; the claim to test is that lowering bodies to fusevm bytecode and
letting the JIT compile the hot ones beats re-walking the list.

The measurement has to come first and be honest: profile a real preamble
(`latex.ltx` is the obvious target) and find what fraction of time is expansion
before claiming a compiled expander is worth anything. If expansion is not the
cost, this milestone is dead and should be recorded as dead.

## Milestone 3 — conditionals and groups

`\if`, `\ifnum`, `\ifx`, `\else`, `\fi` with correct skipping; `\begingroup`,
grouping and the save stack, so assignments are undone at group end. Needed
before any real macro package will load.

## Milestone 4 — the stomach (begun, and deliberately small)

Boxes, glue, the paragraph breaker, fonts, DVI. This is where TeX's reputation
for exactness lives and where the parity bar is byte-identical DVI.

What is done is the join that had been missing rather than the milestone: texrs
could read a document and say what its words were, and could read and write DVI,
and nothing called one from the other, so a book "ran" and produced no page.
`--dvi` (`src/typeset.rs`) measures each character in a real `.tfm`, breaks the
text into lines at a measure, stacks them down a page at a fixed leading, and
ships a file `dvitype` reads. The font side is further along than the layout
side: `.tfm`, `.vf`, `.pk`, `.otf`, `.pfb`, `.map` and `.enc` all have readers,
per-glyph fallback picks a font that has the character, and `src/pdf.rs` carries
the PDF object model and writer.

What is NOT done is everything that makes the parity bar meaningful. The line
breaker takes the first break that fits; TeX minimises total badness over every
feasible sequence of breakpoints (§813-§890). There is no hyphenation, no glue
stretching or shrinking, no page breaking by penalties, no maths, no boxes a
document can nest, and no font embedding in the PDF writer. So the parity bar
for this milestone — byte-identical DVI against `tex` — is not approached, and
saying otherwise from the existence of a `.dvi` file would be the wrong reading.
The next piece that would move it is the badness-minimising breaker, because
every layout difference downstream of it is a consequence of the first-fit
choice.

## Known divergences

Recorded in `BUGS.md` as they are found. The rule is that a divergence is
written down when it is measured, not when it is fixed.
