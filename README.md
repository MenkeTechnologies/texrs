```
████████╗███████╗██╗  ██╗██████╗ ███████╗
╚══██╔══╝██╔════╝╚██╗██╔╝██╔══██╗██╔════╝
   ██║   █████╗   ╚███╔╝ ██████╔╝███████╗
   ██║   ██╔══╝   ██╔██╗ ██╔══██╗╚════██║
   ██║   ███████╗██╔╝ ██╗██║  ██║███████║
   ╚═╝   ╚══════╝╚═╝  ╚═╝╚═╝  ╚═╝╚══════╝
                                         
```

[![CI](https://github.com/MenkeTechnologies/texrs/actions/workflows/ci.yml/badge.svg)](https://github.com/MenkeTechnologies/texrs/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/texrs?style=flat-square&color=05d9e8)](https://crates.io/crates/texrs)
![Rust](https://img.shields.io/badge/Rust-2021-05d9e8?style=flat-square)
![license](https://img.shields.io/badge/license-MIT-ff2a6d?style=flat-square)
![status](https://img.shields.io/badge/status-active%20%C2%B7%20in%20development-9b5de5?style=flat-square)

### `[TEX'S MOUTH AND EXPANDER, COMPILED TO BYTECODE — NOT INTERPRETED]`

> *"Every TeX since 1982 interprets the expander. This one compiles it."*

A TeX engine in Rust: Knuth's **mouth** and **expander**, lowered onto
[`fusevm`](https://github.com/MenkeTechnologies/fusevm) bytecode and run on the
shared three-tier Cranelift JIT — the same engine behind `zshrs`, `stryke`,
`rubylang`, `pythonrs` and `scalars`.

---

## Table of Contents

- [\[0x00\] What it is](#0x00-what-it-is)
- [\[0x01\] Install](#0x01-install)
- [\[0x02\] Usage](#0x02-usage)
- [\[0x03\] What works](#0x03-what-works)
- [\[0x04\] Intercepts](#0x04-intercepts)
- [\[0x05\] Inline Rust](#0x05-inline-rust)
- [\[0x06\] What does not](#0x06-what-does-not)
- [\[0x07\] How it runs](#0x07-how-it-runs)
- [\[0x08\] Parity](#0x08-parity)
- [\[0x09\] Fuzzing](#0x09-fuzzing)
- [\[0x0A\] Benchmarks](#0x0a-benchmarks)
- [\[0x0B\] Releasing](#0x0b-releasing)
- [\[0x0C\] Documentation](#0x0c-documentation)
- [\[0xFF\] Licence](#0xff-licence)

---

A TeX engine in Rust: Knuth's **mouth** and **expander**, built to be lowered
onto a bytecode VM.

## [0x00] What it is

TeX is two machines. The *mouth* turns bytes into tokens under a mutable
category-code table; the *expander* turns tokens into other tokens — `\def`,
`\csname`, `\the`, the conditionals. Only after that does the *stomach* build
boxes and ship DVI.

texrs implements the first two. That is the half a macro-heavy document spends
its time in, and the half where a compiled implementation has something to prove:
every mainstream engine (pdfTeX, XeTeX, LuaTeX) descends from `tex.web` through
web2c and *interprets* the expander.

There is now a third piece. Both outputs break a paragraph the way `tex.web`
§813 does — minimising the total demerits of the whole paragraph over every
feasible set of breakpoints, with Liang hyphenation to widen the places a line
may end. `--pdf` writes the PDF itself; `--dvi` ships a box tree through
`hlist_out`/`vlist_out` (§619-§640), so `hpack` sets each line's glue and the
file carries it at the width it was set to. `--pdf` breaks its PAGES the same
way, by LaTeX's widow, orphan, broken-line and heading penalties over the whole
document. Maths is set from `tex.web`'s own `mlist_to_hlist`, the boxes, glue
setting and page builder underneath are ported, and the `.tfm`'s ligature and
kern program is consulted — so `tex`'s `fi` ligature is texrs's too. [0x06] says
what is left.

## [0x01] Install

```sh
# Homebrew (macOS + Linux)
brew install MenkeTechnologies/menketech/texrs

# Or via crates.io
cargo install texrs

# Or from source
git clone https://github.com/MenkeTechnologies/texrs && cd texrs && cargo build
```

#### Zsh tab completion

```sh
cp completions/_texrs /usr/local/share/zsh/site-functions/_texrs
```

#### Editors

`texrs --lsp` and `texrs --dap` are ordinary LSP/DAP servers over stdio, so any
client can drive them. `editors/` carries ready-made configuration:

```sh
editors/coc-settings.json      # coc.nvim
editors/helix-languages.toml   # Helix
editors/texrs.vim              # Vim / Neovim (native LSP on 0.8+)
editors/texrs.lua              # the same, as a lua module
editors/vscode-settings.json   # VS Code
editors/intellij/              # the JetBrains plugin
```

#### Man pages

```sh
cp man/man1/texrs.1 man/man1/texrsall.1 /usr/local/share/man/man1/
man texrs        # the quick reference
man texrsall     # the comprehensive one, modeled on zshall(1)
```

## [0x02] Usage

`texrs` takes `tex`'s own command line, in all three of its invocation forms:

```sh
texrs [OPTIONS] [FILE[.tex]]... [COMMANDS]   # files, then the rest as input
texrs [OPTIONS] '\FIRST-LINE'                # the arguments ARE the input
texrs [OPTIONS] '&FMT' ARGS                  # with a named format
```

A bare name gets `.tex` appended, so `texrs doc` and `texrs doc.tex` are the
same run. Options may be spelled with one dash or two, and a value may follow
an `=` or a space — `-jobname=x`, `--jobname=x` and `-jobname x` all agree.

tex's options:

```sh
-interaction=MODE     batchmode, nonstopmode, scrollmode or errorstopmode
-jobname=NAME         set the job name
-output-directory=DIR write the output there instead of beside the input
-progname=NAME        set the program name
-fmt=NAME             use a named format
-ini                  be initex
-halt-on-error        stop at the first error
-file-line-error      file:line:error style messages
-recorder             record the files read
-8bit                 write 8-bit characters as themselves
```

texrs's own:

```sh
texrs                      # no arguments: the prompt
texrs file.tex             # run it, print the \message stream
texrs file                 # the same: .tex is appended when there is no extension
texrs '\message{hi}\end'   # the arguments are the input, as in tex
texrs file '\message{x}'   # more input, read after the file
texrs a.tex b.tex c.tex    # compile a batch, one document per core
texrs --jobs=N file...     # bound that to N workers
texrs --repl               # interactive prompt; state carries across lines
texrs --lsp                # Language Server Protocol over stdio, for an editor
texrs --dap                # Debug Adapter Protocol over stdio: breakpoints, stepping
texrs --dump-tokens file   # the mouth's token stream, no expansion
texrs --dump-ast file      # the command stream the frontend lowered to
texrs --disasm file        # the lowered fusevm bytecode
texrs --tiers file         # run it, then say which fusevm tier took it
texrs --text file          # print the document's text, not only its messages
texrs --dvi file           # typeset it: FILE.dvi, total-fit lines, hyphenated
texrs --pdf file           # typeset it: FILE.pdf, total-fit lines, hyphenated
texrs --build file         # compile into the bytecode cache and stop
texrs --aot file           # compile it to a standalone native executable
texrs --no-cache file      # compile this run instead of reading the cache
texrs --cache-stats        # what the bytecode cache holds, and where
texrs --cache-clear        # delete it; it holds only what can be recompiled
texrs --help
texrs --version
```

`-X` is the second half of the command line: the document commands, and a reader
for each binary format a TeX installation is made of. Every one of them prints
what it read rather than acting on it, which is what makes them usable to find
out why a build is wrong.

```sh
texrs -X new [DIR]         # make a document (Texrs.toml + index.tex)
texrs -X init              # make one here, named after this directory
texrs -X build             # build the document this directory is in
texrs -X watch             # rebuild it whenever an input changes
texrs -X show              # say what the document is and can produce
texrs -X dump              # build to stdout, writing nothing
texrs -X bundle fetch URL  # download a support-file bundle into the cache
texrs -X bundle list       # say which bundles have been fetched
texrs -X dvi FILE.dvi      # read what real tex shipped, or diff two files
texrs -X bib FILE.bib      # read a bibliography database
texrs -X bib FILE.aux      # say what a document cites, and what is missing
texrs -X bst FILE.bst      # read a bibliography style, and check its names
texrs -X bibtex FILE.aux   # run the style: write the .bbl a document reads
texrs -X tfm FILE.tfm [C]  # read a font's metrics, or one character's
texrs -X vf FILE.vf [C]    # read a virtual font: what it really sets
texrs -X pk FILE.pk [C]    # read a packed bitmap font, and draw a character
texrs -X otf FILE.otf [C]  # read an OpenType font: its tables and its cmap
texrs -X pfb FILE.pfb [C]  # read a Type 1 font: its glyphs and their widths
texrs -X map FILE.map      # read a font map: what a TeX font name means
texrs -X enc FILE.enc      # read an encoding: what each code is called
texrs -X itar FILE.tar     # index a tar bundle, or read one file out of it
```

Two places the grammar departs from `tex`, both because texrs takes several
files where tex takes one: a non-option argument is a FILE unless it begins
with `\`, and options are recognised anywhere rather than only before the
first file, so `texrs doc -halt-on-error` sets the flag instead of typesetting
it.

Output is written the way tex writes it on the terminal, which is what the
parity harnesses compare:

```
$ texrs examples/macros.tex
(./examples/macros.tex HELLO-WORLD [1|2] )
```

`examples/` carries a runnable program per construct — macros with delimited
parameters, count arithmetic, conditionals, groups, `\csname`, `\edef` — and
`tests/examples.rs` holds every one of them in parity with real `tex`, with no
known-gap escape hatch. Documentation that has drifted from the engine is worse
than none.

## [0x03] What works

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
- `\input`, which is what every real document does first: the file is read where
  it is named, and its own `(./name.tex …)` nests inside the outer one's.
- Verbatim environments — `verbatim`, `Verbatim` and the fancyvrb family,
  `lstlisting`, `minted`, `alltt`, `filecontents` — where the catcodes are
  suspended, so `#`, `&` and `\` inside one are characters rather than markup.
- The LaTeX layer of [0x06]: `\newcommand` and its three relatives dispatched
  natively, `latex.ltx`'s counters, cross references, options, footnotes,
  captions and bibliography ported into `src/latex/kernel.tex`, a real load
  attempted for every `\usepackage`, `\makeatletter` as the catcode change it
  is.
- Maths. `$…$`, `$$…$$`, `\(`, `\[`, `equation` and its relatives are parsed
  into an mlist and converted by `mlist_to_hlist`: styles, sub- and
  superscripts with §756-§759's shifts, `\over`/`\frac`, `\sqrt`,
  `\left…\right` with delimiter growth, `\limits`, the operator names, and the
  Greek and symbol tables plain.tex names — with §764's spacing table ported
  verbatim.
- Lua. `\directlua` runs its chunk in PUC-Lua 5.3, with `tex`, `token`,
  `texio`, `status` and `luatexbase` reaching the engine's real state.
- `--dvi`: a page. Text measured in a real font (`.tfm`), first-fit lines at a
  measure, stacked at a leading, shipped as DVI that `dvitype` reads.
- `\label`, `\ref` and `\pageref`, resolved against the pass that finds the
  pages — `\ref` gives the sectioning number, `\pageref` the page it fell on.
  Worth knowing what this is NOT evidence of: the corpus has 88,341 `\label{`
  and zero `\ref{` or `\pageref{`, because Pandoc writes labels and never
  references them. So this moves no book in the sweep. It is correctness for
  LaTeX documents generally, not a corpus result, and re-running the sweep to
  look for a change will find none.
- `--pdf`: the PDF itself, and the better half of the typesetter. Lines are
  chosen by minimising total demerits across the whole paragraph (`tex.web`
  §813-§890) with Liang hyphenation, and set to the measure with PDF's `Tw`,
  which is what makes pricing glue usable at all.
- Readers for the binary formats a TeX installation is made of, each printing
  what it read: `.tfm`, `.vf`, `.pk`, `.otf`, `.pfb`, `.map`, `.enc`, `.dvi`,
  `.bib`/`.aux`/`.bst`, and tar bundles.

## [0x04] Intercepts

```tex
\def\greet#1{HELLO-#1}
\def\trace{[in]}
\def\loud{<<\proceed>>}

\intercept{before}{greet}{\trace}      % => [in]HELLO-WORLD
\intercept{after}{sec*}{\note}         % every sectioning macro, including
                                       % the ones a package defines later
\intercept{around}{greet}{\loud}       % => <<HELLO-WORLD>>
```

Advice on macro expansion — `before`, `after`, `around`, with `\proceed`
standing for the original expansion inside an `around` handler. The pattern is a
**glob over macro names**, which is what makes it useful on a macro package: the
advice is registered before the macros it will catch exist.

Expansion is a compile-time act here, so advice is woven into the token stream
and is undone by the group that registered it, like any other assignment. A
handler that calls the macro it advises does not weave itself — a call inside
advice is not advised.

## [0x05] Inline Rust

```tex
\rust{
    #[no_mangle]
    pub extern "C" fn twice(n: i64) -> i64 { n * 2 }
}
\catcode`\{=1 \catcode`\}=2
\count1=21
\message{\rustcall twice \count1 \endrust}   % => 42
```

The block is compiled by `rustc`, loaded, and its exported functions become
callable. Its body is Rust, not TeX, so it is lifted out of the file **before
the mouth reads it** — `#`, `{`, `}` and `&` are category codes the mouth would
act on. A call is a *number* wherever TeX reads one: a register assignment, an
arithmetic operand, a conditional, or a `\message` body.

The compiled library is cached by body hash, so only the first run pays for the
compile, and a block that does not compile stops the run with rustc's own
diagnostic rather than a missing-function error later.

## [0x06] What does not

There is a stomach now. Both outputs break a paragraph as `tex.web` §813-§890
does: every feasible set of breakpoints is priced by how far each line's glue is
from its natural width, the cheapest set wins, and Liang hyphenation widens the
places a line may end when no set between words is good enough. `--dvi` took the
first break that fits until it could ship a box tree, because a breaker that
decides some lines should be SHRUNK had nowhere to put that answer;
`src/shipout.rs` ports §619-§640's `hlist_out` and `vlist_out`, so the glue goes
into the file at the width `hpack` set it to (§625) and the answer has somewhere
to go. The `.tfm`'s ligature and kern program is on that path too (§906-§911's
`reconstitute`), so `tex`'s `fi` ligature is texrs's, and its quotation marks
and dashes with it — every document the two engines both set now agrees with
`tex` on its text and its structure.

`--pdf` breaks its pages by penalty too, over the whole document rather than
page by page: `\widowpenalty` and `\clubpenalty` keep one line of a paragraph
off the top and the bottom of a page, `\brokenpenalty` discourages ending a
page on a hyphenated line, and a heading is never left at the foot of a page
away from the text it introduces (`\@secpenalty`, and the `\nobreak`
`\@startsection` writes after a title). `--dvi` stacks a fixed number of lines
on each page.

Under both, `tex.web`'s own machinery is ported rather than approximated:
`hpack`/`vpack` with §108's integer badness and §658's order-of-infinity glue
setting (`src/pack.rs`); the boxes a document can nest — `\hbox`, `\vbox`,
`\vtop`, `\raise`, `\unhbox`, `\leaders`, `\lastbox`, `\unskip`
(`src/box_.rs`); lines assembled from breakpoints with `\rightskip`,
`\parshape` and `\vadjust` (`src/postline.rs`, §877-§890); and a page builder
with `\insert`, `\vsplit`, `\topmark`/`\firstmark`/`\botmark` and an output
routine over `\box255` (`src/page.rs`, §967-§1028). Maths is there too: `$…$`,
`$$…$$`, `\(`, `\[` and the display environments are read into an mlist and
converted by `mlist_to_hlist` (§719-§767), out of `cmr`/`cmmi`/`cmsy`/`cmex`'s
`fontdimen`s (`src/math.rs`).

What that leaves: `\tolerance`, `\pretolerance` and the demerit weights are
constants rather than registers the document can set, and the shipper still
writes runs of strings rather than a box tree, so `src/postline.rs` and
`src/page.rs` are a library beside the path a run takes rather than the path
itself.

What is still missing is the ligature and kern program on the DVI path: `tex`
writes the `fi` ligature (character 0x0C) where texrs writes `f` and `i`, and
that is the whole of the remaining text difference on the two DVI cases that
have one. Seven of the ten now reach STRUCTURE — see `BUGS.md`.

**Some LaTeX, and Lua.** texrs carries the part of LaTeX that lives in the mouth
and the expander, as TeX rather than as Rust, in two files compiled into the
binary: `src/latex/prelude.tex` is a stand-in — a macro that would have drawn
something yields its text instead — and `src/latex/kernel.tex` is a port of
`latex.ltx`, with the place each definition came from written above it and every
substitution named. Counters, cross references, `\newenvironment`, the option
machinery, footnotes, captions and the bibliography are ported. `\documentclass`
and `\usepackage` are no longer silently consumed: the file is found with
`kpsewhich` and its load is attempted, and a package that will not go through is
reported by name with the control sequence that stopped it — `texrs: package
xcolor is not loadable: Unsupported \edef body`. Eleven files load all the way
through — `minimal.cls`, and `ifthen`, `textcomp`, `inputenc`, `keyval`,
`upquote`, `multirow`, `float`, `footnote`, `fontenc` and `lmodern` — and a
package's own `\RequirePackage`s are followed, so `keyval` is read before the
`graphicx` that asks for it. `article.cls` reaches its last line and reads
`size10.clo`; what stops it there is the dimen scanner rather than the class,
because `tex.web` §453's `<factor><internal unit>` is not implemented, so
`10\p@` is `! Illegal unit of measure (pt inserted).` and `em` and `ex` are
absent for the same reason. A package that loads and then breaks what the
preamble already promised is reported rather than committed — measured, letting
`calc` through took the corpus sweep from 229 documents to 145. For everything
still refused, the report is the list of what each one wants. What is kept
out of them is the page: a type size in the class options sets the text at that
size on the leading LaTeX pairs with it, and `[margin=...]{geometry}` sets the
margins and the measure and text height they leave on the paper. `\makeatletter`
is a catcode change and works as one.

`\directlua` runs its chunk, in PUC-Lua 5.3 — the version LuaTeX embeds — and
what the chunk prints is read back as input, so `\count10=20
a\directlua{tex.print(tex.count[10]+5)}b` typesets `a25b`. The `tex` table
reaches the real registers, `token` reaches the input the chunk stands in front
of, and a chunk that fails stops the run with its Lua error as a TeX error. What
there is a `node` library over the engine's own node list, and what it carries
is the half that does not need the document: a chunk builds a list and
`node.hpack` measures it with §649's arithmetic, agreeing with luatex on the
width, the badness, the glue ratio and both orders. `tex.skip` and the `\muskip`
family hand over the `glue_spec` node the manual describes. What is absent is
the other half, and not for want of a data structure: texrs sets a page from
runs of strings, so there is no CURRENT node list — `tex.getbox`, `node.write`
and every callback that would pass Lua the list TeX is building refuse by name,
and a document that walks the document's own list is refused rather than quietly
wrong.

The number, run against the 274 `.tex` files of a real LaTeX/LuaLaTeX corpus
(Pandoc-generated books of 16,000 lines and up, fontspec, TikZ, `\directlua`,
`--include-in-header` fragments, and texrs's own fixtures, including the ones
written to be refused): **229 of 274 run to completion**, and say 75,627,678
bytes of text.

That number went DOWN when Lua started running: the same corpus was 266 of 274
before it. 42 of the 46 failures are one family of header fragments carrying an
unsubstituted `@FALLBACK_LIST@` placeholder inside a `\directlua`, which is not
valid Lua — texrs used to "complete" them only because it consumed the chunk
unread. `lualatex` refuses those same files too (`! LaTeX Error: \usepackage
before \documentclass`), so the drop is the engine reading what it used to skip
rather than a capability lost. Of the remaining four, three are texrs's own DVI
fixtures needing `\hsize` and one is written to fail.

That is a measurement, so it is re-measurable rather than remembered:

```sh
bash scripts/publications.sh                  # ../MenkeTechnologiesPublications
bash scripts/publications.sh /path/to/corpus  # any tree of .tex
bash scripts/publications.sh --gate           # exit non-zero if any document failed
```

Every document that fails is printed with the line it stopped on; there is no
allowlist, so a corpus carrying a file written to be refused shows it among
them. The sweep is where the LaTeX layer's faults have actually been found —
tests pin behaviour a sentence at a time and `tests/cases` pins byte-for-byte
parity with `tex`, and neither of them says whether a 16,000-line book
compiles.

"Run" is the exact claim: the mouth and the expander read the whole document and
produce what its text says. `--dvi` sets a page, `--pdf` writes the PDF itself,
and three of the things a document controls survive the trip:

- **Fonts.** `\setmainfont{Arimo}` is honoured, both ways a document writes it.
  A family the machine has installed is resolved through `fc-match`; a document
  that ships its own font names a FILE instead, with fontspec's `Path=`,
  `UprightFont=` and `Extension=`, and that is read too — in either spelling.
  `\setmainfont{Arimo}[Path=…, Extension=.ttf, UprightFont=Arimo-VF]` embeds
  `CRYLIS+Arimo-VF`, and so does `\setmainfont{Arimo-VF.ttf}[Path=…]`: a
  mandatory argument ending in `.ttf`, `.otf`, `.ttc` or `.otc` — in any case —
  names the FILE, relative to `Path=`, which is what lualatex does with it. An
  explicit `UprightFont=`/`Extension=` still wins where the document writes
  both. What is NOT acted on is `Scale=`: it parses and does nothing, so a
  family scaled to match the body face is set at full size. The same file
  through both engines, differing only by `Scale=0.5`:

  ```
  lualatex   9.96264 pt  ->  4.98132 pt
  texrs        10.0  pt  ->    10.0  pt   (byte-identical PDFs)
  ```

  `Scale=MatchLowercase` is the form that matters in practice, because it is
  what a document writes to bring a display face down to its body face's
  x-height — and a sans family whose x-height is larger than the body's is then
  set too large here, in every heading that uses it.
  `pdffonts` is how you tell: a document that meant to ship its own face
  and reports `Helvetica Type 1 no` got the fallback. The difference matters
  because `fc-match` ALWAYS answers — asked for a font nobody has installed it
  returns its default, so a book whose fonts travel with it was set in whatever
  that default happened to be. `Path=` is written when the document is built
  and is regularly a directory that no longer exists, so a stale one is retried
  against the directory the document was read from, where the fonts are.
  The file is carried in the PDF as `/FontFile2` — `pdffonts` reports
  `GLGNCA+Arimo-VF TrueType yes yes`, embedded AND subset — once, and referred
  to from every page; lines are
  broken on that font's own advance widths, out of its `hmtx` through its
  `cmap`. A family nothing can be found for falls back to whichever of the
  fourteen carries the same metrics, not to Computer Modern. The face is
  SUBSET: only the glyphs the document actually set are kept, with `cmap`,
  `glyf`, `loca`, `hmtx`, `maxp` and `head` rebuilt rather than copied, and each
  subset carries its own six-letter tag. A 5.6MB book comes out with
  `GLGNCA+Arimo-VF`, `TQIODW+Arimo-Italic-VF`, `HHQUFN+ShareTechMono-Regular`
  and `MAZEDP+ArialUnicode`, and the four embedded programs come to **177,248
  bytes** where the three bundled faces alone are 1,082,736 on disk and the
  fourth is a system face borrowed for glyphs the others lack. That pair is the
  claim about subsetting.
  The whole-file totals are a DIFFERENT claim and worth not confusing with it:
  this book is 2.5MB against lualatex's 968,737, but the file also carries a
  contents, folios, and four faces where lualatex embeds one subsetted Latin
  Modern. Read as a subsetting result it says the subsetter is poor; it is
  comparing two font sets.
  The descriptor states what the METRICS file states rather than the bounding
  box's extremes — `/Ascent 694 /CapHeight 683 /Descent -194 /XHeight 431` for
  `cmr10`, out of `cmr10.afm`, which is what luatex writes for it byte for
  byte and none of which is anywhere in the `.pfb`. A Type 1 program still goes
  in whole.
- **Colour.** `\definecolor`, `\providecolor` and `\colorlet` build the palette
  and `\color`, `\textcolor` and `\pagecolor` use it, in the `HTML`, `rgb`,
  `RGB`, `gray` and `cmyk` models. `\color` is a switch and ends with the group
  holding it; `\textcolor` puts the previous colour back. Under `--pdf` this is
  PDF's own `rg` operator, and `\pagecolor` is painted under the text; under
  `--dvi` it is the `color push rgb R G B` / `color pop` `\special` pair that
  dvipdfmx and dvips both read.
- **TikZ.** A `tikzpicture` (or `pgfpicture`) is drawn on the page under
  `--pdf`. Its body is read RAW — a picture is TikZ, not TeX, and the prelude
  used to consume `\draw … ;` and emit nothing, so every diagram in every
  document reached the page as blank space — and it travels the text stream as
  one marker the typesetter parses and draws. The picture takes its bounding
  box's height, computed the way PGF computes one (a curve's control points
  count, a stroked path grows by half its line width), so the paragraphs either
  side flow around it; a bare coordinate is a centimetre, which is PGF's own
  unit vector. Built against PGF's source rather than an approximation of it:
  `--`, `-|`, `|-`, `..controls..`, `to` with `out=`/`in=`/`bend`, `rectangle`,
  `circle`, `ellipse`, `arc`, `grid`, `parabola` and `cycle`, painted by
  `\draw`, `\fill`, `\filldraw`, `\path`, `\clip`, `\shade`, `\shadedraw` or
  nothing, under either fill rule. Colour goes through the document's own
  palette; `line width=`, the named widths, the dash patterns, caps, joins and
  `opacity=` all reach an operator, transparency through a registered
  `/ExtGState`. `\shade`'s axis, radial and ball ramps are painted by `sh`
  through the path as a clip, with the `/Shading` entry carried on the page.
  `->`, `<-`, `<->`, `-stealth` and `-latex` are drawn as the paths PGF draws
  them as, on a line shortened to make room; `snake`, `zigzag`, `saw` and
  `brace` decorations replace the segment they are put on. Nodes carry text
  through the same typesetter the rest of the page uses, placed by any of the
  nine anchors or by the border at any angle, in a rectangle, circle, ellipse or
  diamond. Coordinates may be polar, named, a node's anchor, relative, `pgfmath`
  arithmetic or `calc`; `\foreach` and nested `scope`s are read. Not there:
  patterns (a `pattern=` turns the fill off rather than hatching), the matrix
  and graph libraries, `pic`s, and INLINE placement — lualatex sets a picture in
  the line where it stands and this gives it lines of its own. `--dvi` and
  `--text` draw no picture at all: DVI would need a `\special` every driver
  reads differently, and a picture has no words.
- **`\titleformat` discards the format it is given.** `titlesec` is a prelude
  stub (`src/latex/prelude.tex:229-234`): the arguments are consumed and thrown
  away. So a document that styles its headings through it gets the class's
  default for that level, not what it asked for — and neither half of the ask
  arrives. Measured, on a `\titleformat{\section}{\sffamily\Huge}{}{0pt}{}`:

  ```
  {\Huge …} written directly      24.79 pt, and the sans face if asked
  the same through \titleformat   14.35 pt, CMR10 only — no Orbitron embedded
  ```

  14.35 is `article`'s own `\section` size. Worth separating from the family
  gap it is easy to confuse with: `\setsansfont` IS honoured and `\sffamily`
  DOES reach it — a document that writes `{\sffamily …}` in running text gets
  `MICUKK+Orbitron-VF` embedded. It is only headings routed through `titlesec`
  that see neither the face nor the size.
- **Ligatures and type sizes were here and are not.** Both were listed as
  limitations in this section until v0.6.0. `\section`, `{\huge …}` and
  `{\Large …}` in one file now emit three distinct `Tf` sizes rather than one,
  and `` `` ``/`` '' ``/`--`/`---` now set “ ” – — where they used to set the
  marks doubled. Extracting the same source through both engines gives the same
  line:

  ```
  lualatex   en – dash, em — dash, open “ close ”, single ‘ and ’
  texrs      en – dash, em — dash, open “ close ”, single ‘ and ’
  ```

  The exclusions are right too: `\texttt{--flag}` keeps both hyphens, `-{}-`
  stays two, and `----` is an em dash followed by a hyphen.

  Recorded rather than deleted because a claim was derived from the size
  limitation and does not survive it: this README argued that headings set at
  body size were why a book came out with fewer lines AND more lines per page
  than lualatex sets it. With per-run sizes that reasoning is void, and no page
  ratio was ever published here — see the note under the corpus figure about
  which numbers are facts about the engine and which are facts about a run.
- **Images.** `\includegraphics` embeds the file and reserves the room it takes.
  Measured on the same two documents that used to prove the opposite — one with
  a figure, one without, a real PNG present:

  ```
  with a figure     24,329 bytes, /Subtype /Image present, "After." baseline y=523.2
  without           16,183 bytes, no image,                "After." baseline y=703.2
  ```

  180pt of room reserved, and the text below moves down for it. An image given
  no size is bounded to the measure and the text height rather than set at the
  file's own — a diagram exported at 1600 pixels is 1600 big points wide, which
  is wider than the page. A file the reader cannot rasterise (a `.pdf` named
  where a `.png` sits beside it) falls back to a sibling of the same name.

  This entry said the opposite until v0.6.0, and said it with numbers. Three
  faults were stacked and the first hid the other two: the primitive was wired
  to nothing, `\pandocbounded` boxed every figure without setting the box, and
  once figures did reach the page they were placed at the file's own size.

What `--dvi` still does not do is draw a picture or break its pages by penalty:
it stacks a fixed number of lines on each, where `--pdf` prices the whole
document. A draft reads correctly either way; a book being sold on its
typography should still be set by an engine that has been doing it for forty
years, and `scripts/texrs-pdf` says the same thing where a build would meet it.

That script is a `.tex` to a `.pdf`, not a `pandoc --pdf-engine`: pandoc
validates that flag against a fixed list of engines it knows and refuses any
other name, so texrs cannot be named there under any wrapper. It does not need
to be — a pandoc book build already writes the `.tex` on its first pass, and
that is the whole input an engine takes, so replacing the second pandoc call
also drops a second markdown-to-LaTeX pass over the same document.

## [0x07] How it runs

texrs is a **fusevm frontend**, not an interpreter: mouth → expander → command
stream → fusevm bytecode → the VM runs it. A count register is a VM **slot**, so
`\advance\count0 by 5` is `GetSlot / LoadInt / Add / SetSlot` — native ops the
JIT can compile. `\ifnum` is `NumGt` + `JumpIfFalse`, a real branch.

A conditional whose truth depends only on the macro table (`\iftrue`, `\ifx`)
is folded while lowering instead, because there is nothing for the VM to test.

`tests/lowering.rs` asserts the emitted bytecode rather than the printed output —
output parity alone would not distinguish a frontend from a tree-walker.

## [0x08] Parity

The contract is the `\message` stream, compared byte-for-byte against the real
`tex` binary. No expectation is written by hand:

```sh
cargo run --bin parity          # the committed corpus
cargo run --bin parity -- case.tex   # one file or directory
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

## [0x09] Fuzzing

Hand-written cases only cover what someone thought to write down.

```sh
cargo run --bin parity-fuzz -- --programs 500        # random programs, both engines, diffed
cargo run --bin parity-fuzz -- --seed 7 --once   # one file, verbose
cargo +nightly fuzz run lower -- -timeout=10
```

`parity-fuzz` generates seeded programs confined to the implemented subset and
runs each through both engines. Each program packs 40 independent **probes**,
because the oracle is the expensive part — a `tex` invocation costs ~0.5s of
process start and format load, so one construct per invocation would spend the
whole budget on startup. On a divergence the probe list is minimized to the one
that actually diverges before it is reported, and every program is a pure
function of its index, so a finding replays exactly with `--seed N --once`.

`fuzz/` is a cargo-fuzz crate (targets `lex`, `lower`, `run`) looking for panics
rather than divergences. `tests/fuzz_smoke.rs` replays each target on its seed
corpus under stable Rust, and `tests/fuzz_mass_replay.rs` points the mouth at
generated mutations of every `.tex` in the tree, so `cargo test` still exercises
the harness on a machine with no nightly toolchain.

## [0x0A] Benchmarks

```sh
cargo bench                  # the pipeline against itself: mouth, frontend, VM
bash bench/compare.sh        # the same documents through texrs and real tex
```

`cargo bench` separates the stages, because one end-to-end number cannot tell a
slow expander from a slow VM, and sweeps document size, because a frontend
that is quadratic in the number of macro calls looks fine on a small file and
stops being usable on a real one. That sweep has already earned its keep: it
caught a line-number lookup that was O(n) per token, and fixing it took an
800-statement document from 248 ms to 11.7 ms.

`bench/compare.sh` is the comparison that says whether the engine is fast
rather than where its time goes, and prints the two caveats with the numbers:
`tex` loads the plain format on every run while texrs loads nothing, and texrs
implements the mouth and expander only.

### Measured

Against `tex` 3.141592653 (TeX Live 2026) on a 10-core machine. Every figure
below is a median of repeated interleaved runs rather than a best-of, because
the machine was under other load while they were taken; a quiet machine gives
larger margins, not smaller. Read them with the two caveats above: `tex` is
doing format loading that texrs does not do, and texrs is not doing the
typesetting that `tex` does.

**One document** (5.6 MB, 120k statements, 691k tokens):

| | median | min |
|---|---|---|
| texrs | 0.259 s | 0.147 s |
| `tex` | 1.038 s | 0.791 s |
| | **4.0x** | **5.4x** |

**Where that time goes**, which is why the mouth is not worth parallelising:

| stage | time | share |
|---|---|---|
| mouth (lexing) | 0.143 s | 22% |
| expander + lowerer | 0.308 s | **48%** |
| code generation | 0.014 s | 2% |
| VM execution | 0.171 s | 27% |

**Many documents** (60 files, one thread each). This is the case that
parallelises, and the one `tex` cannot do at all: one process compiles one
file, so the comparison is against running it 60 times.

| | median |
|---|---|
| texrs, all cores | **0.198 s** |
| texrs, one at a time | 1.827 s |
| `tex`, one at a time | 54.076 s |
| | **272x faster than `tex`** |

Scaling with `--jobs`, on those same 60 documents:

| jobs | 1 | 2 | 4 | 8 | 10 | 16 |
|---|---|---|---|---|---|---|
| speedup | 1.00x | 1.85x | 2.71x | 4.96x | 6.73x | 5.58x |

Ten cores, so the fall at 16 is oversubscription rather than a limit in the
work. A single document is still a single thread: this parallelises the batch,
not the engine.

**Iteration depth.** `\def\r{... \ifnum ... \r \fi}` is TeX's loop, and
`tex` recurses through it — one input-stack level per turn, giving up at
`[input stack size=10000]`. texrs lowers that shape to a backward jump, so its
stack use is constant and the bound is arithmetic rather than memory:

| iterations | 9,000 | 12,000 | 1,000,000 |
|---|---|---|---|
| `tex` | ok | **capacity exceeded** | capacity exceeded |
| texrs | ok | ok | ok |

## [0x0B] Releasing

```sh
bash scripts/bump.sh patch          # 0.4.0 -> 0.4.1, everywhere
bash scripts/bump.sh 1.2.3 --dry-run
```

The version lives in six tracked files — the manifest, two hand-written docs
pages, the generated reference page, two man pages, and the IntelliJ plugin's
`gradle.properties`. Nothing in a build or a test run notices when they
disagree, which is how v0.1.0 once sat in the docs through v0.3.0. So
`tests/version_sync.rs` fails when any of them drifts, and `scripts/bump.sh` is
the one command that stamps all six, regenerates the two that are derived from
the corpus rather than substituted, runs the full verify, then tags, pushes and
publishes.

## [0x0C] Documentation

- **Docs hub** — [menketechnologies.github.io/texrs](https://menketechnologies.github.io/texrs/) (`docs/index.html`)
- **Engineering report** — architecture, what lowering forces, parity posture, dependencies (`docs/report.html`)
- **Primitive reference** — every primitive texrs carries and where it happens (`docs/reference.html`, generated from `src/corpus.rs` with `cargo run --bin gen-docs`)
- **Known gaps** — the ledger, each entry pinned by a case the suite gates on (`BUGS.md`)
- **Roadmap** — what the stomach would take (`docs/ROADMAP.md`)

## [0xFF] Licence

MIT.
