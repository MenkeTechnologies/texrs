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

- **`tex.web`'s stomach.** Both outputs break a paragraph as §813-§890 does —
  every feasible set of breakpoints priced by how far each line's glue is from
  its natural width, the cheapest set taken, Liang hyphenation (§891) widening
  the places a line may end. `--pdf` sets each full line to the measure with
  PDF's `Tw`; `--dvi` ships a box tree through §619-§640's `hlist_out` and
  `vlist_out` (`src/shipout.rs`), so the glue reaches the file at the width
  `hpack` set it to (§625). The `.tfm`'s ligature and kern program is on that
  path (§906-§911's `reconstitute`, `src/tfm.rs`), so `tex`'s `fi` ligature is
  texrs's, and its quotation marks and dashes with it.

  `--pdf` breaks its pages by penalty as well (§970-§1010): widows, orphans,
  hyphenated lines and stranded headings, priced over the whole document.
  `--dvi` stacks a fixed number of lines on each page.

  `tex.web`'s own machinery underneath is ported: §108's integer badness and
  §644-§679's `hpack`/`vpack` with order-of-infinity glue setting
  (`src/pack.rs`), the boxes a document can nest (`src/box_.rs`), §877-§890's
  assembly of breakpoints into lines (`src/postline.rs`), and §967-§1028's page
  builder with insertions, `\vsplit`, the marks and an output routine over
  `\box255` (`src/page.rs`). Maths is there too (§680-§767 plus §810 and
  §1204-§1206, `src/math.rs`).

  What is NOT wired is the last join: `src/shipout.rs` is handed line boxes
  built from broken strings rather than a node list with breakpoint indices, so
  `src/postline.rs`'s §877-§890 assembly and `src/page.rs`'s page builder are
  still a library beside the path a `--dvi` run takes rather than the path
  itself. `\tolerance`, `\pretolerance` and the demerit weights are constants
  in `src/linebreak.rs` rather than registers a document can set. Every
  document that both engines set now reaches STRUCTURE; what separates that
  from BYTES is where each mark lands, the `fnt_def` checksum written as zero,
  and §607-§615's compact movement encoding.

- **Box registers.** There is no `\setbox` and no `\box`, so every box register
  is void — which is what `\ifvoid`, `\ifhbox` and `\ifvbox` now answer
  (`tex.web` §462's `box(n)` is null for a register nothing filled, and that is
  all 256 of them). They agree with tex for any document that fills no box; the
  reference `tex` loads plain.tex, which fills `\box0` and puts an `\hbox` in
  `\strutbox`, so those two disagree the way `\count0` does.
  `tests/cases/box_conditionals.tex` pins it. `\count`, `\dimen`, `\skip`,
  `\toks` and `\muskip` are all registers now — `\muskip` is `\skip` in mu
  (§455 makes `mu` the only finite unit where a math glue is wanted, and no unit
  at all anywhere else), with `\muskipdef` and `\muexpr`, pinned against luatex
  by `tests/etex.rs`. `\newbox`, `\newread`, `\newwrite` and `\newinsert` are
  `\chardef`s exactly as plain TeX makes them, so the NAME is a number and it is
  the missing store rather than the missing name that stops a use.
- **`<factor><internal unit>`.** §453's `15\p@` and `3em` are not implemented:
  a dimension may be a literal with a unit or an internal dimen, but not a
  coefficient times one. That, and `em`/`ex` being absent for the same reason,
  is the whole of what stands between `article.cls` and a load — it reaches its
  own last line and stops inside `size10.clo`.
- **Mode and file conditionals.** `\ifvmode`, `\ifhmode`, `\ifmmode`,
  `\ifinner`, `\ifeof` — all of them test state that belongs to the stomach or
  to file I/O, neither of which exists yet.
- **`\ifcsname` is implemented** (etex.ch's `if_cs_code`): it reads the name and
  looks it up with `no_new_control_sequence` still true, so unlike `\csname` it
  does not define what it does not find. `\ifx` also sees a `\csname`-made
  `\relax` and the primitive `\relax` as the same command, which is what makes
  LaTeX's `\@ifundefined` — and so `\@ifpackageloaded` and `\AtBeginDocument` —
  run at all; `tests/etex.rs` compares the whole macro against luatex. It works
  in running text; inside a `\message` body the `\expandafter`-over-`\fi` half
  of the idiom does not, because a message's arms are still lowered as bounded
  token regions.
- **`\jobname`.** Stops the run with `! Undefined control sequence`. The
  resolution logic exists and is correct at `src/lua.rs`'s `jobname()`; it is
  private, and a second copy in the expander would be the same rule in two
  places.
  `\aftergroup`, `\afterassignment`, `\uppercase`/`\lowercase` and `\meaning`
  were on this list and are no longer missing: they are in `src/expand.rs` and
  `src/lower.rs`, documented in `src/corpus.rs`, and pinned by
  `tests/cases/after_tokens.tex`, `case_shift.tex` and `meaning_prim.tex`.
  `\futurelet` came off it earlier, pinned by `tests/futurelet.rs`, and so did
  `\input`, pinned differentially by `tests/input.rs` — see "Finding files"
  below for what it does differently.
`-output-directory=DIR` was on this list and is no longer: `src/cli.rs` parsed
it into `Cli::output_directory` and nothing read the field, so every output went
beside the INPUT instead, silently. It is honoured now (`src/main.rs`, pinned by
`tests/cli.rs`), and the test asserts the half that was broken -- that nothing
is left beside the input -- rather than only that the file arrives where it was
asked for. Recorded because of what it cost while it was true: a `.pdf` written
next to a `.tex` overwrites whatever reference was already there, which is how
texrs output reached tracked lualatex references in
`MenkeTechnologiesPublications`. Restored from git, byte-identical.

- **`#{` parameter text** is implemented. §476 puts the left brace in the
  parameter text so it delimits the last argument, and §473 appends it to the
  body, so `\def\a#{[X]}` called as `\a{Y}` prints `[X]{Y}`. Pinned by
  `tests/cases/param_brace_delim.tex` and `param_brace_argument.tex`. Until
  `cargo fuzz run lower` found it, the argument reader indexed past the end of
  the parameter list on the trailing `#` and PANICKED; `src/expand.rs` validates
  the parameter text at definition time and the crashing input is kept as
  `fuzz/corpus/lower/param_brace_zero_params.tex` — renamed out of its `crash_`
  prefix when it stopped crashing, which is how `tests/fuzz_smoke.rs` reports a
  seed that has started compiling. What it now compiles to is a divergence of
  its own: with `#{` there are no NUMBERED parameters, so tex reports
  `! Illegal parameter number in definition of \greet.` where texrs accepts the
  body. `tests/cases/param_brace_illegal_number.tex` pins it.
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
  pins it, and the reason it is hard is that §1279 expands a `\message` body
  while reading it from the file, so tex's context display splits the line at
  the offending token while texrs has already read to the `}`.

  Two conditions DO now report and carry on the way tex does: a constant above
  2147483647 (§445) and a character or register code out of range (§433, §434).
  Each writes `! <reason>.` followed by §311's two-line context display into the
  message stream, clamps the value where tex clamps it, and the run continues;
  `tests/cases/number_too_big.tex`, `chardef_bad_code.tex` and
  `error_context_trimmed.tex` pin all of it, including the `...` trimming at
  `half_error_line` and `error_line`. Every other error path still stops with
  one `TexError`: `\multiply` overflow is raised on the VM (`src/runtime.rs`)
  rather than in the expander, and `\outer` is not policed at all.
- **No expansion budget.** `\def\x{\x}\x` expands forever, exactly as it does in
  real tex — neither engine has a step limit, so this is parity rather than a
  bug. It is why the fuzz targets are run under a timeout (see below).

## `\directlua` does not expand inside a `\message`

`\directlua` is EXPANDABLE, so luatex runs the chunk while it reads a
`\message` body and what the chunk printed lands in the stream. Measured
against luatex 1.24.0, for `\count10=20` and
`\message{a\directlua{tex.print(tex.count[10]+5)}b}`:

```
luatex : (./pt2.tex a25b)
texrs  : (./pt2.tex a\directlua {tex.print(tex.count[10]+5)}b )
```

The chunk RUNS where the document meets it — `--text` on the same arithmetic in
a `\documentclass` document says `a25b` — but a `\message` body is read into a
token list before anything expands, so the chunk is printed rather than run.
This is the same shape as the `undefined_cs.tex` gap recorded above, and has the
same cause: tex.web §1279 expands a message body while reading it from the file
and texrs slurps it first. It is not in `tests/cases` because the oracle there
is `tex`, which has no `\directlua` to compare against.

## A section's number carries a chapter a class has not got

`\ref` is answered from `typeset::unit_numbers`, which counts chapter, section
and subsection and joins every level down to the one being asked for. A class
with no chapters still carries the chapter's nought, so `article` numbers its
first section `0.1` where LaTeX numbers it `1`:

```
\documentclass{article}\begin{document}\section{First}\label{a}See \ref{a}.
```
sets `See 0.1.` here and `See 1.` under `pdflatex`. `tests/latex.rs`'s
`a_label_written_to_the_aux_is_what_the_next_run_resolves` pins the current
answer so the divergence cannot change unnoticed. The fix is for the join to
start at the shallowest level the CLASS declares -- `report` and `book` declare
`chapter` and `article` does not -- which is a fact `src/latex.rs` already reads
for `\thesection`.

## Divergences from tex

- **One type size.** The size-selecting commands are defined as empty in
  `src/latex/prelude.tex` and `Layout::size` is a single document-wide `f64`, so
  every heading is set at body size. A document containing `\section`,
  `{\huge …}` and `{\Large …}` emits ONE distinct `Tf` size. The fix is not a
  prelude change: giving `\Large` a body would change nothing while there is no
  per-run size on the measure, break, pagination and draw paths for it to set.

- **Quote ligatures.** `` `` `` and `` '' `` are set as the ASCII characters
  themselves rather than as the opening and closing quotes every TeX engine
  renders them into. `texrs --text` on ``` ``hello there'' ``` prints
  ``` ``hello there'' ```, and the PDF's content stream carries the same four
  marks. Two consequences, and the second is the one that costs something
  measurable: the page is visibly wrong where a document quotes, and the literal
  marks measure wider than the quotes they should be, so every line carrying a
  quotation is set wider than lualatex sets it. `---` is likewise three literal
  hyphens rather than an em dash, though there the widths agree to 0.01pt so it
  is only visual. The direction is worth stating because it is easy to get
  backwards: wider lines hold FEWER words and produce MORE lines, so this cannot
  contribute to a document coming out short.

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

## `\scantokens` is not implemented

It is on the LuaTeX list and it is deliberately not done. `\scantokens` re-reads
a token list AS A FILE, so the end of that list is a file end: it fires
`\everyeof` and it closes whatever was being scanned. Measured, in luatex
itself:

    \message{[\scantokens{ab}]}
    ! File ended while scanning text of \message.

An implementation that merely re-tokenised the group would look right in a
probe like that one and diverge in every use the primitive actually has, which
all turn on the file end and `\everyeof`. It needs the pseudo-file machinery,
not a shortcut.

## texrs cannot rewrite tex's own DVI unchanged

`cargo run --bin dvi-parity -- --roundtrip` parses a DVI real `tex` wrote and
writes it back through `Dvi::rewrite`. Nothing to do with typesetting: it asks
only whether what was read can be written. None of the nine documents survives,
in two distinct ways.

The corpus is fourteen documents: the ten the ladder uses, three in
`tests/dvi_cases` carrying shapes the others lack -- a second font, rules, two
pages, a `\special` -- and Knuth's `story.tex` where TeX Live is installed.
None survives unchanged, and the richer shapes fail the same way the simple
ones do, which says the cause is the encoding rather than any one construct.

Ten come back LONGER -- 380 bytes in, 456 out; 680 in, 784 out -- because the
writer does not choose the compact operand widths tex chose, so a movement tex
wrote in two bytes is written back in four.

Three come back the same length with bytes changed, and those are precise: byte
111 is the font checksum in `fnt_def`, written as zero instead of the value
read, and postamble+21 is the maximum page width, recomputed rather than
carried. `tests/dvi_trip_floor.txt` records where each file stands.

## DVI output is not tex's, in three specific ways

`cargo run --bin dvi-parity` compares against real `tex`, and DVI is the
attainable axis: no fonts inside the file and no compression, 224 bytes against
260 for `Hello world.` where the same document in PDF is 11,729 against 615.

Every document stops at PAGES, and the reasons are three real typesetting
decisions rather than one:

- **Spaces.** A gap between words is a MOVEMENT in tex's DVI, not a character,
  so tex's text reads `Helloworld.`. texrs sets a space glyph instead.
- **Ligatures.** tex reaches for `fi` in cmr10 -- `The\u{c}rst` -- and texrs
  sets `f` and `i` separately.
- **The folio.** tex ships a page number and texrs does not, which is the same
  difference the PDF ladder reports.

`tests/dvi_parity.rs` pins all three as facts, so they are findings rather than
a rung number whose meaning nobody remembers. An empty document is the fourth
case: tex writes no DVI, texrs writes one.

## A typesetting run does not use the bytecode cache

`texrs FILE` typesets, and typesetting does not consult the shard. The cache
holds compiled chunks; the fonts a document asks for, its page colour and its
layout are read WHILE lowering and are not in it, so a cached chunk is not
enough to set a page from. Measured: `--disasm` and `--dvi` fill the shard, the
ordinary invocation and `--text` do not.

Storing the chunk anyway -- free, since it has just been compiled -- hangs the
run. `--text` performs the same `store_mode` call on the same document in 0.01s,
so the store is not at fault on its own; something about doing it on the
typesetting path is, and it is not understood yet. It is left out rather than
left in, because a pipeline that hangs is worse than one that recompiles.

The cost is real for the job texrs is meant to do: a 25,000-line book recompiles
its mouth and expander on every run. Closing this means teaching the shard to
carry the fonts, colour and layout beside the bytecode.

## PDF output is not LuaTeX's

The goal is byte-identical, and the distance is large: for `Hello world.`
luatex writes 11,729 bytes and texrs writes 15,435, both of them mostly a subsetted Computer Modern. `cargo run --bin pdf-parity`
measures it on a ladder rather than as a yes/no, because a harness that only
answered "identical?" would say no every day and say nothing else.

Where the ten corpus documents stand: seven at FONTS, two at TEXT, and the empty
document at BYTES, which is the goal. The seven climbed when texrs stopped
setting in Helvetica and began embedding a subsetted CMR10, as luatex does; the
two at TEXT differ in where their lines break, which is line breaking rather
than the writer.

The face agrees now. A document that names no family is set in Computer Modern,
because that is what TeX means by "the font" and what luatex embeds, and the
program is cut to the glyphs the page drew, so `pdffonts` reports a subsetted
CMR10 for both engines. Two bugs came out of that change and are worth recording
because neither was visible in any test: CMR10 puts `emdash` at 124 where WinAnsi
puts it at 151, so every em dash landed in an empty slot and VANISHED; and CMR10
has `suppress` rather than a space at 32, so every word gap drew a small stroke.
`add_font` now writes a `/Differences` array joining the code the driver used to
the glyph the font calls it, matched through what a glyph MEANS rather than what
it is named.

Dictionary key order is no longer a difference: `Object::Dict` holds its keys in
the order they were put in, and the page, the font, the descriptor, the object
stream and the cross-reference stream are all written in luatex's order — the
descriptor agrees with it key for key. What is left, measured on `Hello world.`:
luatex writes `/Resources 1 0 R` and `/Widths 7 0 R` where texrs inlines both;
it compresses the content stream with FlateDecode where texrs writes it plain,
and draws `[(Hello)-333(w)27(orld.)]TJ` at 9.96264 point where texrs draws
`(Hello world.) Tj` at 10, which is line breaking and kerning rather than the
writer; and it writes neither `/Encoding` nor `/ToUnicode`, leaving the subset's
own encoding to speak, where texrs writes both so a copied ligature is told
rather than guessed. `tests/typeset.rs` reads content streams as raw bytes and
reads `/F1` out of the page dictionary, so the first two are test-shape questions
as much as writer questions.

A Type 1 program is subsetted now — the cleartext header, the `/CharStrings`
dictionary and the eexec encryption all rebuilt — which took `Hello world.` from
40,094 bytes to 15,435. What is NOT cut is the 102 subroutines cmr10 carries:
hint replacement pushes a subroutine number, hands it to OtherSubr 3 and takes it
back with `pop`, so a scanner reading the operand before `callsubr` does not see
the call and would stub a subroutine the font goes on using. `xdvipdfmx`'s
`t1_subset`, which this is ported from, carries them all for the same reason.

Byte equality is only defined with `SOURCE_DATE_EPOCH` pinned: measured, luatex
reproduces itself exactly when it is set and differs run to run when it is not,
because the file carries `/CreationDate` and an `/ID`.

## Definition prefixes

`\long` and `\outer` are recorded on the macro and are part of its meaning, so
`\ifx` distinguishes a prefixed definition from a bare one exactly as tex does.
The restrictions they describe are NOT enforced.

`\outer` is only an error-detection feature: it forbids the macro in an
argument, in a group being scanned as text, and in skipped conditional text, and
every one of those is a position tex reports and recovers from while texrs
stops. Enforcing it therefore could not reach parity either, and a false
positive would refuse a document that works, so the difference is written down
instead -- `tests/cases/outer_forbidden_use.tex` pins it. The same holds for the
runaway-argument check `\long` lifts.

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

The one divergence left is the recovery from a RUN-TIME overflow. `\multiply`
past the range is detected on the VM (`src/runtime.rs`), which stops rather than
reporting, so `tests/cases/multiply_overflow.tex` is still a written-down gap.
The scanner's own limit recovers the way tex does — `\count1=99999999999`
reports `! Number too big.`, clamps to 2147483647 and carries on — and
`tests/cases/number_too_big.tex` and `tests/cases/error_context_trimmed.tex` pin
that, context display included.

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
