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

`--pdf` breaks its pages by penalty too, and over the whole document rather
than page by page: `\widowpenalty`, `\clubpenalty` and `\brokenpenalty` at
the values LaTeX sets them to, `\@secpenalty` before a heading and `\nobreak`
after one, and the plan taken is the cheapest total (tex.web §970-§1010).
`--dvi` stacks a fixed number of lines on each page.

Underneath both, `tex.web`'s own machinery is now ported rather than
approximated: `src/pack.rs` carries §108's integer badness and §644-§679's
`hpack`/`vpack` with order-of-infinity glue setting, `src/box_.rs` the boxes a
document can nest, `src/postline.rs` §877-§890's assembly of breakpoints into
lines, and `src/page.rs` §967-§1028's page builder with insertions, `\vsplit`,
the marks and an output routine over `\box255`. Maths is done too:
`src/math.rs` carries §680-§698's mlist, §764's spacing table verbatim and
§704-§767's `mlist_to_hlist`, driven by the `fontdimen`s of §700-§701 — so a
formula's geometry is TeX's own even where the glyphs are drawn from the
document's face.

The ligature and kern program and the shipper both landed. `src/tfm.rs` carries
§906-§911's `reconstitute` with all eight ligature ops and the boundary
characters, applied where the text is measured and where it is drawn so the two
cannot disagree; `src/shipout.rs` carries §619-§640's `ship_out`, `hlist_out`
and `vlist_out`, and `--dvi` draws a `\vbox` of `\hbox`es through it. Each
font's own `\fontdimen2/3/4` sets the interword glue (§1042), where cmr10's
fractions had stood in for every face. Every document that both engines set now
reaches STRUCTURE.

What is still NOT done, and what now bounds the parity bar: the shipper is
handed line boxes built from broken strings rather than a node list with
breakpoint indices, so `src/postline.rs`'s §877-§890 assembly and
`src/page.rs`'s page builder remain a library beside the path a `--dvi` run
takes rather than the path itself — `--dvi` still stacks a fixed number of lines
on each page where `--pdf` prices the whole document. `\tolerance`,
`\pretolerance` and the demerit weights are constants rather than registers a
document sets. STRUCTURE to BYTES is now positional plus the encoding: the
writer does not choose tex's compact `w`/`x`/`y`/`z` movement reuse (§607-§615)
and writes the `fnt_def` checksum as zero.

The subsetter's own untested edge is nesting. `tests/glyf.rs`'s
`an_accented_letter_brings_the_letter_with_it` pins one level — asking for
e-acute keeps the `e` and the accent it is drawn from — but it walks
`components` one deep, and `src/sfnt.rs:324` closes over "the parts a composite
is built out of, and the parts of those". A composite whose component is itself
composite is the case no test asserts.

## Known divergences

Recorded in `BUGS.md` as they are found. The rule is that a divergence is
written down when it is measured, not when it is fixed.
