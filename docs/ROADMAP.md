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

The breaker that was named here as the next piece has been written.
`src/linebreak.rs` minimises the total demerits of a whole paragraph over every
feasible set of breakpoints (§813-§890) and hyphenates with Liang patterns
(§891), and `--pdf` uses it: a full line is set to the measure with PDF's `Tw`,
which is what makes pricing glue usable at all. `--dvi` does not, and the reason
is worth recording rather than fixing twice — a DVI driver cannot set a run to a
width, so a breaker that decides some lines should be SHRUNK has nowhere to put
that answer. An earlier attempt was reverted for exactly this: every shrunk line
drew out past the measure.

What is still NOT done, and what keeps the parity bar out of reach: no page
breaking by penalties, no maths, no boxes a document can nest. `\tolerance`,
`\pretolerance` and the demerit weights are constants rather than registers a
document sets, and the interword glue stretches by cmr10's fractions rather than
by each font's own. So byte-identical DVI against `tex` is not approached, and
reading the existence of a `.dvi` or a `.pdf` as progress toward it would be
wrong. Font embedding, listed here as missing, is done: `/FontFile2`, whole
rather than subset.

## Known divergences

Recorded in `BUGS.md` as they are found. The rule is that a divergence is
written down when it is measured, not when it is fixed.
