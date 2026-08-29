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

## Milestone 4 — the stomach

Boxes, glue, the paragraph breaker, fonts, DVI. This is where TeX's reputation
for exactness lives and where the parity bar is byte-identical DVI. Not started,
and not worth starting until 2 and 3 are solid.

## Known divergences

Recorded in `BUGS.md` as they are found. The rule is that a divergence is
written down when it is measured, not when it is fixed.
