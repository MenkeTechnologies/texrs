# Changelog

All notable changes to texrs are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Page structure. `\newpage`, `\clearpage`, `\cleardoublepage` and `\pagebreak`
  were defined by the prelude to expand to nothing, and `\chapter` was its own
  argument -- the heading text and nothing else. So no break in a document
  reached the page: a book's title page, copyright page and first chapter ran
  together as one stream of prose, and page one of the scifi2 novel had the
  title, the copyright notice and the start of the next section drawn over each
  other, reading `feaCr oitpyright`. The novel set at 144 pages where lualatex
  sets it at 270. A break is carried through the text as a form feed -- what
  the character means -- and split out before words are, since Rust counts it
  as whitespace and would drop it. Two breaks in a row are one break, so
  `\clearpage` after `\newpage` leaves no blank sheet. `\chapter` starts a page
  in all three of its forms, and the star of `\chapter*` no longer becomes the
  first character of the heading. That novel is 174 pages now, its title page
  is its own, and its copyright page is the next one. The remaining gap to 270
  is vertical: no space around headings, no `\begin{center}`, and the document's
  own type size and leading are not read yet.

- `\numexpr` and `\dimexpr`, the eTeX expression primitives LuaTeX carries:
  `+`, `-`, `*`, `/` with ordinary precedence and parentheses, closed by an
  optional `\relax`. Division ROUNDS, half away from zero — `\numexpr 7/2` is 4
  where `\divide` gives 3 — so the two are separate operations here. Not in
  `tex` 3.141592653, so the oracle is LuaTeX 1.24.0 and the comparison lives in
  `tests/etex.rs` rather than in the parity corpus.
- Token registers: `\toks`, `\toksdef`, register-to-register copying, and
  `\the`. A token list is stored VERBATIM — nothing in the braces expands,
  which is the difference between it and a macro — and is scoped by a group as
  the category codes are. `\the` writes it back by the token-list rule rather
  than `\string`'s: a control word carries a trailing space however short, a
  one-character control sequence does not. Measured against `tex -ini`.
- A PNG with an alpha channel now goes into a PDF as a picture and a soft
  mask, which closes the gap the image port left open. It is the one picture
  whose pixels are taken apart rather than copied: PNG interleaves the alpha
  with the colour where PDF wants it separate, so the data is inflated, its row
  filters undone, split in two and deflated again. `flate2` becomes a
  dependency in its own right for it, having been in the tree beneath `zip`.
- Glue: `\skip` registers, `\skipdef`, and the infinite orders. A glue is three
  dimensions and two orders, and an infinite stretch (`fil`, `fill`, `filll`)
  beats any finite one however large. `\the` writes the components back with
  `plus` and `minus`, omitting a zero one; `\number` gives the natural
  component alone. Measured against `tex -ini`.
- Colour, and the font a document ships with itself -- the two reasons a book
  whose preamble is a palette and a `\setmainfont` came out black and in the
  wrong face.
  - `\definecolor`, `\providecolor` and `\colorlet` build the palette;
    `\color`, `\textcolor` and `\pagecolor` use it. The prelude defined all six
    to swallow their arguments and emit nothing, so a book with 2,119 colour
    definitions and a `\color{neonCyan}` on every heading produced a PDF with
    no colour operator in it at all. `\color` is a switch and ends with the
    group that holds it; `\textcolor` puts the previous colour back. Models
    read: `HTML`, `rgb`, `RGB`, `gray`, `cmyk` -- and a model that is none of
    those defines nothing rather than guessing.
  - `\pagecolor` is painted under the text rather than ignored. A document that
    sets a dark page sets light text to go on it, so honouring one without the
    other leaves white on white.
  - fontspec's `Path=`, `UprightFont=` and `Extension=` are read. A document
    that ships its own font names a FILE, not an installed family: looking
    `Arimo` up among the installed families finds nothing, and `fc-match`
    answers anyway with its default, which is what the whole book was then set
    in. `Path=` is written when the document is built and is regularly a
    scratch directory that no longer exists, so a stale one is retried by its
    last component against the directory the document was read from -- where
    the fonts actually are.
  - A font is written into the PDF once and referred to from every page.
    Embedding it per page is also correct PDF and unusable: a 144-page book
    carrying Arimo came to 72 MB, one copy of the font per page. It is
    1.7 MB now. Images are shared the same way.

- Dimensions: `\dimen` registers, the units that reach them, `\dimendef`, and
  `\the`/`\number`. A dimension is an integer count of scaled points and every
  unit is an exact integer ratio to a point (`tex.web` §458) rather than a
  float, which is why `1in` is 72.26999pt; printing is Knuth's `print_scaled`
  (§103), the fewest digits that read back as the same integer. All nine units
  were checked against `tex -ini`.
- The per-character tables besides `\catcode`: `\mathcode`, `\lccode`,
  `\uccode`, `\sfcode` and `\delcode`, read and written the same way and scoped
  by a group the same way. INITEX's defaults are measured against `tex -ini`,
  which is the only correct oracle for a default — plain `tex` has loaded a
  format that changes several of them.
- Octal and hexadecimal constants (`'777`, `"FF`). The hex digits are
  UPPERCASE, the opposite of `^^` notation's lowercase, so `"FF` is 255 and
  `"ff` is an error — measured, not inferred from the symmetry. plain.tex
  writes every `\mathcode` as a hexadecimal constant, so nothing above worked
  without this.
- `\mathchardef`, which is `\chardef` with a wider range (to "7FFF) and its own
  message, `! Bad mathchar (N).`.
- The four fontspec family directives in the reference. `\setmainfont`,
  `\setromanfont`, `\setsansfont` and `\setmonofont` were dispatched but
  documented nowhere, so editor completion did not offer them and the reference
  page did not describe them. The coverage gate caught two of the four; the
  other two escaped it because a `k @ (...)` binding hid the first and last
  name in the arm from the scanner that lifts dispatch literals. The arm now
  matches the names directly and reads the name it took, so all four are
  visible to the gate, and all four carry an entry saying what the engine
  really does with them -- `\setmainfont` and `\setromanfont` fill the family
  the PDF backend embeds or maps, while `\setsansfont` and `\setmonofont` are
  recorded and not yet selected by either backend.

- The font a document names. `\setmainfont{Georgia}` was read and then ignored:
  every document came out in Computer Modern whatever it asked for, because
  nothing resolved the family and the page had only cmr10 to set in. The family
  is now resolved through `fc-match` (then a walk of the font directories, since
  `fc-match` always answers, with a default when it has no match), and the font
  file is carried in the PDF as `/FontFile2` with a `/FontDescriptor` built from
  its `head` and `hhea`. Lines are broken on that font's own advance widths,
  read from `hmtx` through `cmap` and scaled to 1/1000 em, so a line set in it
  ends where the font says rather than where cmr10 would have. Measured:
  `pdffonts` on the output reports `Georgia TrueType yes`. A family the machine
  does not have falls back to whichever of the fourteen carries the same metrics
  — Arimo's are Arial's are Helvetica's — rather than back to Computer Modern.
  CFF-flavoured OpenType is refused rather than embedded, because `/FontFile2`
  must be a TrueType program. There is no subsetting yet: a font with a large
  repertoire is embedded whole, and Arial Unicode alone is 23 MB.

- Active characters. A character of category 13 is a command rather than text:
  `\catcode`\~=13 \def~{...}` defines `~` itself, it may take parameters, and
  `\let~=\x` works. An active `~` and the control sequence `\~` stay different
  things — measured, tex gives `[a][b]` for the pair — so an active character
  interns under a name no source can spell rather than sharing the control
  sequence table's spelling. plain.tex needs this at line 20,
  `\outer\def^^L{\par}`, where the macro being defined IS the form feed.
- Glyph names to Unicode (`agl`), ported from `agl.c` in `xdvipdfmx`, and the
  `/ToUnicode` map an embedded font is written with. A PDF says which glyph to
  draw and never which character it is, so a reader asked to copy a paragraph
  out has a glyph called `ff` and no idea it is two f's. Names that say their
  own value (`uni0041`, `u1D400`, `a.sc`, `f_f_i`) are worked out by rule and
  the rest come from the installation's `glyphlist.txt`. `pdffonts` now reports
  a font texrs embeds as carrying a usable map, where it reported none before.
- `\long` and `\outer`, the two definition prefixes texrs did not have (it had
  `\global`). Both attach to the definition that follows and are spent by it,
  and all three chain in any order. They are part of the MEANING, not
  decoration, so `\ifx` tells `\long\def\a{}` from `\def\a{}` — measured, tex
  says the same. `\outer`'s restriction is recorded but not policed; see
  BUGS.md.
- Reading a `\special`, ported from the `spc_*.c` family in `xdvipdfmx`:
  dvips's colour stack, `papersize`, an included figure with its bounding box,
  `pdf:` destinations and operators, HTML links, and anything else kept whole.
  `-X special TEXT`. Held against tex both ways -- the specials a real document
  carries are read back out of the DVI tex wrote, and the dimension arithmetic
  is compared with tex's own for eighty-one dimensions, which is how it came to
  be TeX's arithmetic rather than the one that looks right.
- Reading the `CFF ` table, ported from `cff.c` and `cff_dict.c` in
  `xdvipdfmx`: the header, the INDEXes, the Top and Private DICTs with their
  packed operands and nybble reals, the charset in all three formats, the 391
  standard strings, and enough of a Type 2 interpreter to find each glyph's
  width -- which means following `callsubr` and `callgsubr`, since most of a
  font's glyphs begin by calling one. `Sfnt::glyph_names` now answers for a CFF
  font as well as a TrueType one, which closes the hole the OpenType port left,
  and every name of four fonts matches `otfinfo -g`. The widths are held
  against `hmtx`: the same numbers, stated twice in one font by two parts of
  the same tool, in entirely different ways.
- `\chardef` and `\countdef`, the two ways TeX names a number. `\chardef\active=13`
  makes a constant usable wherever a number is scanned, which is how plain.tex
  writes `\catcode`\~=\active`; `\countdef\pageno=0` names a count register and
  works in every position the register does — assignment, `\advance` and `\the`
  all reach the same register through either spelling. Both are limited to
  0..255, with tex's two different messages kept apart (`Bad character code`
  against `Bad register code`) because that is how an author finds which they
  got wrong. Measured against real tex; `tests/cases` pins all of it.
- Embedding a Type 1 font in a PDF, ported from `pdf_font_load_type1` in
  `xdvipdfmx`: the `FontFile` stream with the three lengths that divide a Type
  1 font into its cleartext, its encrypted body and its closing zeros, a
  `FontDescriptor`, the font's own encoding as a `/Differences` array, and the
  widths out of the charstrings. This is what a TeX document needs and a
  base-14 name cannot give -- nobody has Computer Modern installed, so the font
  travels with the document. `pdffonts` reports it embedded, and both xpdf and
  Ghostscript read the text back out of a page set in it.
- `--dvi` typesets: the join between reading a document and writing one that had
  been missing. texrs could say what a document's words were, and could read and
  write DVI, and nothing called one from the other, so a book "ran" and produced
  no page. `src/typeset.rs` measures each character in a real `.tfm`, breaks the
  text into lines at a measure, stacks them down a page at a fixed leading, and
  ships a file `dvitype` reads. It is NOT `tex.web`'s stomach: TeX chooses
  breakpoints by minimising total badness over every feasible sequence
  (§813-§890) and this takes the first break that fits, which is what every word
  processor before TeX did and what TeX was written to improve on. No
  hyphenation, no glue stretching or shrinking, no page breaking by penalties,
  no maths, no boxes a document can nest. A page you can open, which is the
  difference between producing nothing and producing something imperfect.
- Per-glyph font fallback while typesetting: a character the current font does
  not have is set from one that does, rather than dropped or set as a missing
  glyph. This is the piece LuaTeX was previously required for.
- `scripts/texrs-pdf`, texrs as a `pandoc --pdf-engine`: `texrs --dvi` then
  `dvipdfmx`, which is the pair tex itself has always used. The script states
  what a publication build gives up by pointing at it — Computer Modern whatever
  `\setmainfont` asked for, no TikZ, no colour, first-fit lines — because that
  is the trade it exists to offer, not a detail to discover afterwards.
- Writing PDF (`pdf::Pdf`, `pdf::document`), ported from `pdfobj.c` and
  `pdfdoc.c` in `xdvipdfmx`: the object model, the writer with its
  cross-reference table, and enough document structure to make a page --
  catalogue, page tree, content stream, resources. This is the far end of the
  chain the font readers built, and what the stomach will write when there is
  one. Held against three readers that had no part in writing it: `pdfinfo`
  reads the trailer and the page tree, `pdftotext` reads the content streams,
  and Ghostscript interprets the whole file -- all three with a quiet stderr,
  because a reader repairs a broken table and says so rather than refusing.
- `\input FILE`, which reads another file sharing every piece of state with it,
  so a macro it defines is defined afterwards and a `\catcode` it sets stays
  set. The name is scanned per `tex.web` §537 and `.tex` supplied when it has no
  extension; the file gets its own nested paren group in the terminal output,
  closed hard against the last message the way tex writes it. Files are found in
  the working directory and on `TEXINPUTS` — never via `kpsewhich`, so a
  document does not need TeX Live installed to run. Fifteen text input levels
  are allowed, tex's own limit and wording. Until this existed no real document
  could run at all, because a real document's first line loads a format or a
  package. `tests/input.rs` compares all of it against real tex.

### Fixed

- The lowering depth bound is re-measured, and the runaway it guards is bounded
  again rather than crashing. The bound exists to stop a runaway before the
  STACK does, so it is a measurement, and a measurement moves when a frame on
  the recursive path grows. Adding a primitive to the number scanner made every
  level fatter: 98 levels still lower and 100 abort, where it had been 128 and
  192 when the bound was set to 100. The bound was therefore sitting exactly on
  the cliff, and two mutually recursive macros died with a stack overflow
  instead of reporting `capacity exceeded` -- which is precisely the crash the
  bound is there to prevent. 64 restores the margin. If a document legitimately
  nests deeper than that, the fix is a bigger stack for the lowering thread,
  not a bigger number.

- tests/typeset.rs no longer fails on a machine with no TeX installation. The
  metrics it measures in belong to an installation, not to this crate, so on a
  runner with neither `kpsewhich` nor a texmf tree all thirteen panicked before
  asserting anything. They say so and stop now, which is how tests/fontmap.rs
  has always guarded. Installing the metrics part-way is worse than not at all:
  `texlive-base` carries cmr10 and cmsy10 and makes these pass, and fontmap.rs
  then starts asserting against an installation naming 438 fonts where it wants
  more than a thousand. A font that is FOUND and unreadable still fails.

- The lowering test's register prologue is measured rather than written down.
  It skipped a hardcoded 512 ops -- two for each of 256 count registers -- and
  when dimension registers took the bank to 512 that skip landed inside the
  prologue, so the test failed on ops that were never the document's. It takes
  the leading run of line-0 ops now, which keeps the check as strong: a zero
  anywhere after the prologue is still a failure.

- Long file names in a tar bundle. A path too long for a header's hundred bytes
  is written as an extra entry before the real one, and the tars disagree on
  which: GNU tar writes an `L` block whose data is the path, bsdtar a pax `x`
  block whose data is `length key=value` records, and only `ustar`'s
  prefix/name split was understood. The other two were skipped as "not a file",
  which left the truncated hundred bytes from the header behind them -- a name
  that reads plausibly in a listing and can never be looked up, so a bundle's
  deepest packages were unreachable. Both extensions are read now. The tests
  that covered this built their archive with the system `tar`, so they asserted
  whatever the machine's tar happened to write: green on macOS, where bsdtar
  splits the name, and red on CI, where GNU tar does not. The new ones build
  both archives byte by byte and hold everywhere.

- Glyph names on a machine with no TeX installation. The Adobe glyph list is
  found with `kpsewhich`, so a developer machine resolves every name out of the
  installed `glyphlist.txt` and the built-in map is never reached. The built-in
  map had no entry for any of the 52 letters -- the names a font uses more than
  any others -- so without a TeX installation `A` did not resolve to `A`. The
  letters are seeded from the range now (the list really does carry `A;0041`),
  and the built-in map is built by its own function so a test can read it
  directly rather than through whatever the machine happens to have installed:
  the two tests that covered this passed locally and failed only on CI.

- `^^` notation reads its hex form. `tex.web` §352 has two: `^^` and two
  LOWERCASE hex digits is that hex code, and `^^` and anything else is one
  character shifted by 64. Only the second was implemented, so `^^41` read as
  `t1` where tex reads `A`. Measured: `^^4a` is `J` but `^^4A` is `tA`, because
  `A` is not a lowercase hex digit.
- `^^` notation applies inside a control sequence name. The substitution belongs
  to the input processor (§353) and runs before anything is classified, so
  `\catcode`\^^K=7` names the control character — plain.tex line 16. Reading it
  raw made it `\^` followed by junk, which is where loading plain.tex stopped.

- A numeric constant is limited to TeX's 2147483647, not the host `i64`. The
  scanner reported `Number too big` only when the digits overflowed an `i64`, so
  `\count1=99999999999` was accepted and printed back where tex reports and
  clamps (`tex.web` §445). A `\count` register is a 32-bit word, and this was
  the remaining way to get a wider value into one.

- `\newcommand*` defines the command. The star asks for a restriction nothing
  here reads, and an unrecognised one made the name scan give up, which dropped
  the DEFINITION: Pandoc writes `\newcommand*\pandocbounded[1]{...}`, so every
  document with a figure in it stopped at an undefined `\pandocbounded`.
- `\newcommand{\x}[n][default]` matches its optional argument at the CALL. The
  default was recorded and never read, so the bracket group reached the text and
  shifted every argument behind it. The bracket is matched where the arguments
  are matched, so it works from inside an expansion as well -- which is what
  `\textcolor` needs, called as it is from inside `\NormalTok`, and what lets
  `\textcolor{red}{text}` run as well as `\textcolor[rgb]{1,0,0}{text}`.
- Only the taken arm of a decided conditional (`\ifx`, `\iftrue`, `\iffalse`)
  is lowered. Lowering the other arm EXECUTED the compile-time assignments in
  it, so `\ifx\a\b\let\x\y\else\let\x\z\fi` ran both `\let`s and the second
  won whichever way the test went. That is what stopped `\@ifnextchar` from
  working, and with it every optional argument and every starred form written
  the way LaTeX writes one; `\@ifstar` is now in the prelude and
  `\titleformat*`, `\titlespacing*` and `\vspace*` dispatch on the star.
  `\ifnum` still lowers both arms -- its test is a register read -- as
  `BUGS.md` records.
- The constant pool is interned, and a document that outgrows it is refused
  rather than aborted. Coalescing text runs across line directives took the
  books under the 65,536 entries a `LoadConst` operand can address; a 4 MB
  reference still went past it and the compile panicked inside fusevm.
  Identical strings are now one constant, which bounds the pool by what a
  document says rather than by how often it says it, and `Compiler::compile`
  returns a `Result` so the case beyond that is a message rather than a panic.
- An optional argument composed IN FRONT of the argument macro never fired:
  `\def\setmainfont{\@eatopt\@setmainfontargs}` peeks at `\@setmainfontargs`,
  not at the `[` behind it, so `\setmathfont[]{STIX Two Math}` still put
  `]STIX Two Math` in the text. These are declared optional arguments now, and
  fontspec's other spelling -- `\setmainfont{Arimo}[Path=...]`, which these
  books write -- is consumed too.
- Stubs whose arity did not match the package's signature: `\includegraphics`,
  `\hyperref`, `\rule`, `\item`, `\Verb`, `\captionsetup`, `\UseMicrotypeSet`,
  `\definecolor`, `\pagecolor`, `\defaultfontfeatures`, `\newfontfamily`,
  `\titleformat`, `\titlespacing`, and the environments that take options at
  `\begin` -- `Highlighting`, `minipage`, `longtable`, `figure`, `tabular`,
  `tikzpicture`, `scope`, `tcolorbox`. A surplus argument does not fail loudly;
  it lands in the text, which is how `\begin{Highlighting}[]` put an empty pair
  of brackets in front of 571 listings in one book and `\vspace*{1cm}` printed
  its own `1cm` on every title page.
- A header fragment included with `--include-in-header` is recognised as LaTeX.
  It has no preamble of its own -- it IS preamble -- so `\makeatletter`,
  `\newenvironment` and `\begin{document}` join the markers that select the
  LaTeX layer.

### Added

- Reading an indexed tar bundle, ported from `tectonic_bundles::itar`: the tar
  headers, the index that makes an archive seekable (`name offset length`), and
  reading a file by seeking to it rather than walking the archive. `-X itar
  FILE.tar [NAME]`. Held against `tar` itself -- the same names it lists and the
  same bytes it extracts, for a name too long for a header field, for lengths
  that straddle the 512-byte block, and for an empty file -- and a test that
  reads a 16-byte file from the far end of a 20 MB archive faster than a
  megabyte at the front, which is the property the format exists for.
- Reading font maps and encoding files, ported from `fontmap.c` and
  `t1_load_enc` in `xdvipdfmx`. This is the join between the other font
  readers: a `.dvi` names `ptmr8r`, and the map is what turns that into a real
  file, an encoding to read it through, and the `SlantFont`/`ExtendFont` a
  document asked for. `-X map FILE.map [NAME]` and `-X enc FILE.enc`. Checked
  against the installation it describes -- every line of the installation's own
  46,000-line `pdftex.map` reads, every file a line names is one `kpsewhich`
  finds, and the whole chain from TeX font name through map, encoding and Type
  1 font to the `.tfm` agrees about which glyph a code is and how wide.
- Reading Type 1 fonts, ported from `type1.c` and `t1_char.c` in `xdvipdfmx`:
  PFB segments and PFA hexadecimal, the eexec and charstring decryptions, the
  cleartext header, the font's own encoding, and the `hsbw`/`sbw` at the front
  of each charstring that declares its width. `-X pfb FILE.pfb [CHAR]`. Held
  against `t1disasm` for every glyph of four Computer Modern fonts, and the
  widths against both the `.afm` beside the font and the `.tfm` TeX sets it
  with -- three formats by three authors agreeing to the unit.
- Reading OpenType and TrueType fonts, ported from `sfnt.c`, `tt_table.c`,
  `tt_cmap.c` and `tt_post.c` in `xdvipdfmx`: the table directory (collections
  included), `head`, `hhea`, `maxp`, `hmtx`, `name`, `cmap` formats 0, 4, 6 and
  12, and `post` glyph names with the 258 Macintosh names a font numbers rather
  than spells. `-X otf FILE.otf [CHAR]`. Held against `otfinfo` on four fonts:
  the table directory table for table, the `name` table string for string,
  every one of 2000+ `cmap` mappings, and every glyph name.
- `--dump-ast` prints the command stream the frontend lowered the document to.
  TeX has no expression grammar to parse into a tree; the mouth and the expander
  hand the stomach a flat run of primitive commands, and that run is this
  frontend's AST (`src/ir.rs`). It is the stage the other two listings straddle:
  `--dump-tokens` runs before expansion, so a macro is still a control sequence,
  and `--disasm` runs after code generation, so a conditional is already a jump.
  In between, a macro shows as what it expanded to, a conditional still has two
  named branches, and a tail-recursive macro shows as the loop it lowered to
  rather than as inlined copies. `texrs::commands()` is the same stage as a
  library call, and `compile()` now goes through it, so the listing cannot drift
  from what the code generator was handed.
- `scripts/publications.sh`, the sweep the README's corpus number comes from:
  every `.tex` under a tree is run with `--text`, one per core, and the count,
  the bytes of text and every document that failed are reported. It is where
  the LaTeX layer's faults have been found -- the unit tests pin behaviour a
  sentence at a time and `tests/cases` pins parity with `tex`, and neither says
  whether a 16,000-line book compiles. No allowlist: `--gate` exits non-zero if
  anything failed, and what failed is printed either way.
- `texrs::compile_text`, the bytecode `run_text` runs: the same pipeline with
  the document's own words lowered as well as its messages.
- Reading `.pk`, the packed bitmap font, ported from `pkfont.c` in
  `xdvipdfmx`. A glyph is stored as the lengths of its runs of black and white
  in a nybble stream with three encodings in it, plus a repeat count that
  arrives mid-row and applies when the row ends. `-X pk FILE.pk [CHAR]` draws
  one. Held against `gftype`'s own picture of the same font, pixel for pixel,
  for every character of `cmr10`, `cmti10`, `cmsy10` and `cmtt10` at 600 dpi --
  and the widths are checked against the `.tfm`'s, which is what lets a driver
  mix a bitmap font with an outline one.
- Reading `.vf`, the virtual font, ported from the VF half of `xdvipdfmx`
  (`vftovp.web` in C). A virtual font has no glyphs: each character is a little
  DVI program saying what to set in some other font, which is how TeX's
  encodings were carried onto PostScript fonts laid out differently, and a
  `.dvi` naming one cannot be read without it. The packets are DVI, so they go
  through the DVI reader rather than a second copy of it. `-X vf FILE.vf
  [CHAR]`. Held against `vftovp` for every character of `ptmr7t`, `phvr7t`,
  `pcrr7t` and `ptmri7t` -- width and program, step for step.
- Writing DVI (`dvi::Writer`), the other half of the format ported from
  tectonic's `xdv`. It is what the stomach will call: a DVI file is a linked
  list read backwards -- every page points at the one before it, the postamble
  points at the last page, the last four bytes point at the postamble -- so the
  pointers can only be filled in while writing, and the postamble's counted
  maxima are counted here rather than asked for. `Dvi::rewrite` re-emits what
  was read. Held against `dvitype`, Knuth's own reader: it accepts a page built
  from nothing, checksum and all, and accepts a file of real tex's read and
  written back, which compares equal to the original as a document.
- The `.bst` interpreter, ported from `bibtex.web`: the stack machine, its 37
  builtins, `SORT`/`ITERATE`/`REVERSE`, crossref inheritance with BibTeX's
  min-crossrefs rule, and the 79-column line breaking that gives a `.bbl` its
  shape. `-X bibtex FILE.aux` does what `bibtex` does, end to end. Held against
  the real program: `plain`, `unsrt`, `abbrv` and `alpha` are run over a
  database built to reach the awkward parts -- a von name, a junior, an
  accented name, a corporate name in braces, `others`, a crossref pair, a
  `\noopsort` key, a title that has to wrap -- and the `.bbl` files must match
  byte for byte.
- `.tfm` reading, ported from xetex's `read_font_info` (`tex.web` §539-§576):
  character widths, heights, depths and italic corrections, the ligature and
  kern program, and the `FONTDIMEN` parameters. `-X tfm FILE.tfm [CHAR]` prints
  a font's own description or one character's, and takes a font name as well as
  a path. Held against `tftopl`, Knuth's own reader, for every character of
  cmr10, cmmi10, cmsy10, cmex10 and cmtt10, and for cmr10's whole lig/kern
  program. This is the piece nothing that sets type can be right without, and it
  is what `width$` in a `.bst` was waiting on.

## [0.4.0] - 2026-08-30

Versions 0.2.0, 0.3.0 and 0.3.1 were released without cutting a section here;
everything below had accumulated under Unreleased and is filed under 0.4.0,
which is the release that carries it to crates.io and the Homebrew tap.

### Added

- A LaTeX layer for the half of LaTeX that lives in the mouth and the expander.
  `src/latex/prelude.tex` is a file of `\newcommand`s compiled into the binary,
  and a document naming `\documentclass`, `\usepackage`, `\PassOptionsToPackage`
  or `\RequirePackage` is recognised as LaTeX and lowered against it.
  `\newcommand`, `\renewcommand`, `\providecommand` and
  `\DeclareRobustCommand` are dispatched natively rather than through
  latex.ltx's `\ifnum` chain, which cannot run here because lowering emits both
  arms of a conditional. The preamble directives are consumed and produce
  nothing; `\makeatletter` and `\makeatother` are the catcode change they are.
  Against a 167-file LaTeX/LuaLaTeX corpus this moves the compile count from 0
  to 2 and moves the remaining failures out of the preamble directives and into
  the packages themselves — 122 now stop at `\defaultfontfeatures`, 41 at
  `\directlua`.
- The eleven LaTeX control sequences are in `src/corpus.rs` under a new "LaTeX"
  chapter, so they reach `docs/reference.html`, editor completion and hover the
  same way every other primitive does.
- `-X bst FILE.bst` reads a BibTeX style: the fields an entry may carry, the
  `MACRO` abbreviations, the functions, and the `READ`/`SORT`/`ITERATE` that say
  what it does. It also names every function the style calls that nothing
  defines. bibtex reports those one per run, at run time, so a style with three
  gaps costs three builds to find; this asks once and exits non-zero. Read
  against the styles TeX Live ships (`plain`, `unsrt`, `abbrv`, `alpha`), which
  parse whole with nothing undefined. The interpreter is deliberately not here:
  it needs `width$`, which measures a string in a font and so needs a `.tfm`.
- `-X build --help`, and the same for `-X watch` and `-X dump`, print the usage
  instead of reporting `--help` as an unknown argument.
- `tests/cli.rs` now holds the `-X` commands to the same drift guard the options
  have: every command the usage text lists must be offered by the zsh completion
  and documented in the man page, and the completion may offer none the binary
  does not list.
- Bytecode cache: what a document compiled to is kept in an rkyv shard keyed by
  path, valid while the source's mtime matches to the nanosecond, so a second
  run skips the mouth, the expander and the lowerer. `--no-cache`,
  `--cache-stats`, `--cache-clear`, and `TEXRS_CACHE=0`.
- `--build` compiles a document into the bytecode cache and stops. A build step
  that runs it leaves the run that follows starting from bytecode: on a hit the
  mouth, the expander and the lowerer are skipped entirely. Nothing the document
  does at RUN time happens, because none of that is compilation.
- `scripts/bump.sh` stamps the version in all six files that state one — the
  manifest, both hand-written docs pages, both man pages (with the date), and
  the IntelliJ plugin — regenerates the two derived from the corpus, runs the
  full verify, then tags, pushes and publishes. `tests/version_sync.rs` is the
  gate under it, and it checks version SLOTS rather than every version-shaped
  string: a sentence dating a change is true and a gate that failed on it would
  push whoever holds the release to falsify it. Drift in any of the six now
  fails the suite rather than sitting
  in a page that claims a version the binary has not been for three releases,
  which is what had happened (v0.1.0 in the docs through v0.3.0, the man pages
  three behind at v0.3.1, the plugin at 0.1.0 against a 0.4.0 crate).
- A `\count` register is 32 bits, as TeX's is. `\advance` wraps
  (`2147483647 + 1` is `-2147483648`, measured, no error), and `\multiply` and
  `\divide` check and raise `Arithmetic overflow` rather than growing into an
  i64 answer tex would never print — including a division by zero, which used to
  produce an infinity. The wrap is branch-free and stays JIT-eligible, because
  `\advance` is what a TeX loop does on every turn.
- `Chunk::set_builtin_argc_is_arity`, which unlocked the AOT path for a builtin
  whose RESULT is used. Without it the checked arithmetic gave the right answer
  in the interpreter and a wrong one in an `--aot` binary — fusevm cannot reason
  about a builtin's stack effect unless the frontend states that `argc` is the
  arity, and in texrs it always was: every handler pops exactly its argc.
- The tracing JIT is switched on. It was compiled in, measured by `--tiers`, and
  described in the README — and never enabled on the run path, so every document
  ran interpreted while `--tiers` reported `trace-eligible=true traced=false` on
  a loop that was correctly rotated and waiting. A 2,000,000-iteration TeX loop
  goes from 4.4-5.7s to 0.02s, printing the same answer to the byte. A debug run
  stays interpreted on purpose: the JIT compiles a hot loop into native code
  that does not call the `--dap` line marker, so a debugger under it would
  silently stop stopping.
- `tests/eval.rs`, the unit layer the siblings carry: one behaviour per test, so
  a regression in delimited-parameter matching reads as "a delimited parameter
  matches up to its delimiter" rather than as "macros.tex diverges". Nineteen
  rules — the mouth's space handling, `\let` copying a meaning, `\ifx`
  comparing meanings, `\edef` freezing where `\def` defers, a group restoring
  both a macro and a register, `\divide` truncating toward zero, `\ifodd` on a
  negative number. Every expectation was produced by running the snippet through
  real tex while writing it, because one typed from memory is a belief about TeX
  rather than a measurement of it.
- The examples are frozen too. They are the documentation, they were checked
  only against a live tex, and CI has none — so on every push the pages a reader
  copies from were the least verified thing in the tree. `--freeze` now records
  both corpora, and the replay knows the difference between them: an example
  must match tex, an `examples/extensions/` one uses constructs tex does not
  have and only has to run. The gate that pairs them caught its own gap first —
  the freeze covered `examples/` and not `examples/extensions/`, and the test
  said so by name.
- Frozen parity, ported from the siblings: `cargo run --bin parity -- --freeze`
  records what the oracle said into `tests/data/parity_expected.txt`, and
  `tests/parity.rs` replays it with no TeX installed. CI has none, so the live
  differential test has been skipping there on every push — the corpus was
  verified only on a machine with tex. The two are not redundant: the frozen
  replay catches a regression in texrs, the live comparison catches a wrong
  belief about tex. Gates come with it — a case with no frozen block fails, a
  frozen block whose case was deleted fails, and the file has to name the engine
  version it came from.
- The oracle is one implementation, in `src/parity.rs`, shared by the
  differential tests, the new `parity` binary and `parity-fuzz`. Two harnesses
  that extract the message stream differently are asking the oracle two
  different questions, and the shell versions kept that logic in bash and perl
  beside the Rust copy — the arrangement that drifts. `scripts/parity.sh` and
  `scripts/lib.sh` are gone with it; `cargo run --bin parity` is the same report
  over `tests/cases` or any directory of `.tex` files.
- `parity-fuzz`, the differential fuzzer as a binary, ported in shape from the
  sibling frontends' — and for their reason. The oracle is the expensive part:
  a `tex` invocation costs ~0.5s of process start and format load, so a fuzzer
  running one construct per invocation spends its budget on startup. Each
  program now packs 40 independent probes, and a divergence is minimized to the
  single probe responsible before it is reported. 1600 probes cost 80 tex
  invocations rather than 1600. It replaces `scripts/fuzz_parity.sh` and
  `scripts/fuzz/gen.pl`: one implementation of the harness, in the language the
  engine is written in, with no bash or perl in the loop.
- `tests/opcodes.rs` guards the builtin id space. The ids are a wire format,
  not an internal detail: a cached chunk and an `--aot` object both call
  builtins by number, so renumbering one does not fail to compile — it makes
  every artefact written before the change call the wrong function. The gate
  fails on a duplicate id, a declared-but-unregistered op, a double
  registration, a call written as a bare number, and any movement of the five
  ids already on disk.
- `tests/embed.rs` pins the library API from an embedder's side: output comes
  back as values rather than going to the terminal, a failure is a `Result`
  rather than a panic or an exit, and one run leaves nothing behind for the
  next — no macro, no register, no catcode, no message buffer, no fault. The
  binary runs one document and exits, so a leak between runs is invisible to
  it; these are what say so out loud.
- The command line is `tex`'s. A bare name gets `.tex`; a first argument
  beginning with `\` makes the whole list a line of input with no file; further
  arguments after a file are input read after it — unless the file's own `\end`
  stopped the run, which `tex` also honours; `-interaction=batchmode` writes
  nothing to the terminal; and `-jobname`, `-output-directory`, `-progname`,
  `-fmt`, `-ini`, `-halt-on-error`, `-file-line-error`, `-recorder` and `-8bit`
  are accepted in tex's own spelling, so an invocation written for `tex` drives
  texrs unchanged. `tests/cli_tex.rs` compares each form against the real binary,
  including which side of the closing paren a message lands on.
- `texrs` with no arguments opens the prompt rather than printing its usage.
  `tex` prompts for input here too: an engine given nothing to do should ask.
- `--help` is painted in the house palette — yellow `USAGE:`, bold program name
  and flags, cyan section rules, green `//` — and only when stdout is a
  terminal. A pipe gets plain bytes, which is what keeps an escape out of a file
  someone is grepping and what `tests/cli.rs` reads the flag list out of. The
  banner follows the same rule.
- The banner is the fleet's — logo, live-stats box, tagline — shared by the
  prompt, `--help` and `--version`, with every count read from the tables at
  call time so it cannot go stale. `--help` prints it above a sectioned option
  list in house style.
- Intercepts: `\intercept{before|after|around}{<glob>}{\handler}` weaves advice
  into macro expansion, with `\proceed` standing for the original expansion
  inside an `around` handler. The pattern is a glob over macro names, so advice
  registered now catches macros a package defines later — a registry keyed by
  exact name would need the document to know every name up front, which is the
  thing a macro package makes impossible. Expansion is a compile-time act here,
  so advice is woven into the token stream and undone by the group that
  registered it. A call inside advice is not advised, which is what keeps a
  handler that calls the macro it advises from weaving itself forever; the depth
  travels in the token stream on two markers the mouth cannot produce.
- Inline Rust: a `\rust{ … }` block is compiled by `rustc`, loaded, and its
  exported functions are callable as `\rustcall <name> <numbers…>\endrust` —
  a number wherever TeX reads one. The block is lifted out of the file before
  the mouth reads it, because its body is Rust and `#`, `{`, `}` and `&` are
  category codes the mouth would act on. The replacement carries no braces, so
  it reads correctly whatever the catcodes are where the block appeared. A block
  that does not compile stops the run with rustc's own diagnostic.
- `--aot`: compile a document to a standalone native executable. The chunk is
  emitted as a relocatable object through fusevm's ahead-of-time compiler and
  linked against the texrs runtime staticlib, so the result runs with no
  interpreter dispatch loop and needs no texrs on the machine. It prints exactly
  what an ordinary run prints, which `tests/aot.rs` checks byte for byte —
  "compiled" that behaves differently is not a compiler. Short compared with the
  sibling frontends' AOT paths for a structural reason: their closures live in a
  host table outside the bytecode and have to be smuggled through the chunk and
  rebuilt, while texrs has nothing outside the chunk. A macro is gone by the
  time the VM starts.
- `--repl`: an interactive prompt. A line is read with every line before it
  still in effect — the session re-lowers and re-runs the document it has built,
  so a `\catcode` changes how the next line reads and a register assignment
  survives, because it IS the same program with one more line. A line that fails
  is rolled back rather than left in the document, `\end` ends the line rather
  than the session, and with stdin on a pipe the line editor is skipped so
  `texrs --repl < doc.tex` works.
- `src/banner.rs`: the version line names the TeX level first and the engine
  second, so nothing is misrepresented as TeX Live.
- `--dap`: a debug adapter over stdio — source-line breakpoints, single
  stepping, a stack frame, and the `\count` registers as the variables scope.
  What is debuggable is what survives lowering: macros expand at compile time,
  so a breakpoint stops on lines that left run-time work behind, and one set on
  a line that did not is reported unverified rather than silently never firing.
- Source lines on every op. The lowerer emits a line directive when the line
  changes and the code generator stamps it onto each op, so `--disasm` reads
  against the document and the debugger has something to map. Before this every
  op reported line 0.
- `--tiers`: run a document, then report what fusevm's tiers did with its
  bytecode — block-tier eligibility, the largest eligible op region, every loop
  header and whether the tracing tier compiled it, and the ops the block tier
  refuses with their counts. The answers come from fusevm's own predicates,
  because enabling the JIT is not the same as being compiled by it.
- `--lsp`: a language server over stdio. Completion and hover answer from
  `src/corpus.rs` — the same table that generates `docs/reference.html`, so the
  editor and the site cannot disagree — and diagnostics come from the engine's
  own lowerer, landing on the line the mouth had reached.
- `src/corpus.rs`, the primitive reference table, with `tests/corpus_coverage.rs`
  holding it against the engine's dispatch in both directions: a primitive the
  engine gained and the corpus never heard of fails, and so does an entry naming
  a control sequence the engine no longer dispatches.
- `cargo run --bin gen-docs` regenerates `docs/reference.html` from the corpus,
  and `tests/docs_generated.rs` fails when the committed page and the generator
  disagree.
- `--dump-tokens` and `--disasm`: the mouth's token stream, and the lowered
  fusevm bytecode.
- Differential fuzzing: `the parity-fuzz binary` generates seeded random
  programs confined to the implemented subset, runs both engines in parallel,
  and reduces whatever diverges to a minimal case.
- cargo-fuzz targets `lex`, `lower` and `run`, with `tests/fuzz_smoke.rs`
  replaying them on stable and `tests/fuzz_mass_replay.rs` pointing the mouth at
  generated mutations of every `.tex` in the tree.
- `examples/`, held in parity with real tex by `tests/examples.rs` with no
  known-gap escape hatch.
- `editors/` configuration for coc.nvim, Helix, Vim/Neovim (native LSP) and
  VS Code, pointing at `texrs --lsp` and `texrs --dap`.
- `docs/_config.yml`, excluding Markdown from the Pages build: GitHub Pages
  ships Jekyll 3.10, whose Liquid parser aborts the whole site on a TeX brace.
- Man pages (`texrs(1)`, `texrsall(1)`), a zsh completion, and the docs site
  (`docs/index.html`, `docs/report.html`, `docs/reference.html`).
- A JetBrains plugin under `editors/intellij`, ported from the sibling engines':
  highlighting by category code (the sixteen classes of `tex.web` §207, with the
  primitives texrs implements told apart from the control sequences a document
  defines), run configurations over the CLI including `--dump-tokens`,
  `--disasm` and `--no-cache`, `%` comments, brace matching, spell-checking that
  reads prose and skips markup, new-file templates that set the category codes
  INITEX leaves ordinary, and a colour settings page. Since the engine grew
  `--lsp` and `--dap`, the plugin drives both: completion over the primitives
  the engine implements, hover and diagnostics from the real mouth and expander,
  and a debugger with line breakpoints, stepping, frames, scopes, variables,
  evaluation and run-to-cursor. A paid IDE is required, as for the sibling
  plugins, because the platform LSP API is not in the Community editions.

- Benchmarks. `cargo bench` measures the pipeline against itself — the mouth
  alone, the frontend, the VM alone, and the whole run — so a slow expander and
  a slow VM cannot hide behind one end-to-end number, plus a size sweep that
  shows how the cost scales. `bench/compare.sh` measures the only comparison
  that says whether the engine is fast: the same documents through texrs and
  real `tex`, end to end, with the two caveats printed beside the numbers.

### Fixed

- `\edef` dropped its parameter text. `\edef\pair#1,#2.{…}` matched nothing and
  left its delimiters in the output where tex prints the arguments; `\edef`
  differs from `\def` only in WHEN the body is expanded, not in whether it takes
  parameters. Found by `parity-fuzz` on its third program, and pinned by
  `tests/cases/edef_with_parameters.tex`.

- `Lexer::line` counted newlines from the start of the file on every call, which
  is O(n) per token and O(n²) per document: the scaling benchmark caught it on
  its first run, where quadrupling the input cost 27x. It now counts only what
  has been consumed since the last answer. The 800-statement document went from
  248 ms to 11.7 ms; the sweep is close to linear rather than quadratic.

- A panic in the argument reader on TeX's `#{` parameter form, found by
  `cargo fuzz run lower`. The parameter text is now validated at definition
  time as `tex.web` §476 validates it.
- The parity harnesses misread any tex output over 79 columns: tex wraps its
  terminal output at `max_print_line` and the break can land right after the
  filename. `max_print_line` is now pinned high and continuation lines are
  joined regardless. The extractor also cut at the first close paren, truncating
  any message that printed one.

## [0.1.0] - 2026-08-29

### Added

- TeX's mouth and expander, lowered onto fusevm bytecode: category codes, the
  three-state line scanner, `^^X`, `\def` with delimited parameters, `\let`,
  `\edef`/`\xdef`/`\gdef`/`\global`, `\csname`, `\string`, `\the`, `\number`,
  `\expandafter`, `\noexpand`, the conditionals, groups scoping both the macro
  table and the registers written inside them, the `\count` registers with
  `\advance`/`\multiply`/`\divide`, and `\message`.
- The differential corpus in `tests/cases`, compared against the real `tex`
  binary with no hand-written expectations, and `tests/known_gaps.txt` recording
  every case that does not match yet with its reason.
- `tests/lowering.rs`, which asserts the emitted bytecode rather than the
  printed output — output parity alone would not distinguish a frontend from a
  tree-walker.

[Unreleased]: https://github.com/MenkeTechnologies/texrs/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/MenkeTechnologies/texrs/releases/tag/v0.1.0
