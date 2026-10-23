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
shared three-tier Cranelift JIT — the same engine every other fusevm frontend
runs on.

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

A TeX engine in Rust: Knuth's **mouth** and **expander** lowered onto a bytecode
VM, and a stomach that typesets to DVI and PDF.

## [0x00] What it is

TeX is three machines. The *mouth* turns bytes into tokens under a mutable
category-code table; the *expander* turns tokens into other tokens — `\def`,
`\csname`, `\the`, the conditionals; the *stomach* builds boxes out of what is
left and ships them.

texrs compiles the first two to bytecode. That is the half a macro-heavy
document spends its time in, and the half where a compiled implementation has
something to prove: every mainstream engine (pdfTeX, XeTeX, LuaTeX) descends
from `tex.web` through web2c and *interprets* the expander.

The stomach is there too. Both outputs break a paragraph the way `tex.web`
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
editors/emacs/                 # a major mode, with its primitive table
```

`editors/emacs/` is the one that is more than configuration: `texrs-mode.el` is
a major mode, and `texrs-stdlib.el` beside it is generated from the same corpus
as `docs/reference.html` — `cargo run --bin gen-emacs-stdlib`, gated by
`tests/emacs_stdlib.rs`, so completion and hover there cannot describe a
primitive the engine does not carry.

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
texrs --jobs=N file...     # bound that to N workers; TEXRS_THREADS=N sets every pool, output is identical at any N
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
Files found through `kpsewhich` (fonts, hyphenation patterns, the glyph list,
packages) are resolved once per run, by one `kpsewhich -interactive` process that
reads the TeX databases once and answers every name after, and the paths are
remembered in `<cache dir>/texrs/kpse-*.tsv` (disabled by `TEXRS_CACHE=0`). That
file is stamped with the size and modification time of every `ls-R` database and
`texmf.cnf` kpathsea reads, and is started over when the stamp no longer matches,
as after a TeX Live update. A remembered path is used only while the file still
exists and no file of that name sits in the working directory.


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
texrs -X special TEXT      # say what a \special means to a driver
```

A document is a `Texrs.toml` naming its inputs and one or more outputs. The
output `type` is `messages`, `tokens` or `disasm` — the `\message` stream, the
mouth's token stream, or the lowered bytecode — and `-X build --profile` picks
one by NAME, so a document may carry several of the same kind. A page is not
among them: `--pdf` and `--dvi` are flags on a file, not profiles on a document.
`texrs(1)` has the whole schema, including `preamble` (files dumped as a format
so a macro file compiles once) and `bundle` (a zip searched after the
document's own directory).

The environment it reads is on the generated reference page in full. One of
them is worth knowing before you compare two files: a PDF carries the time it
was written, so two runs of one document differ in their dates and nothing
else. `SOURCE_DATE_EPOCH` (reproducible-builds.org's specification, which
LuaTeX honours too) replaces the wall clock with a fixed time, and with it set
the same document twice gives the same bytes:

```sh
$ SOURCE_DATE_EPOCH=1000000000 texrs --pdf doc.tex && md5 doc.pdf
d03b89054b9baadcf040211a10a842e6
$ SOURCE_DATE_EPOCH=1000000000 texrs --pdf doc.tex && md5 doc.pdf
d03b89054b9baadcf040211a10a842e6      # without it, a different sum each run
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
- `\csname`/`\endcsname`, `\string`, `\the`, `\number`, `\romannumeral`,
  `\jobname`, `\expandafter`, `\inputlineno`, `\show` (the meaning
  and the error context tex prints after it) and `\showthe` (the value, the
  same way).
- `\let`, `\edef`/`\xdef`, `\gdef`, `\global`, `\begingroup`/`\endgroup`.
- Conditionals: `\iftrue`, `\iffalse`, `\ifnum`, `\ifodd`, `\ifx`, `\if`, `\ifcat`, `\ifcase`
  with `\or`, `\else`, `\fi` — nested, and inside a `\message` body.
- Every primitive luatex has is a primitive here too, with the command code
  luatex gives it (`src/luatex_primitives.rs`, written from luatex's own table),
  so `\meaning`, `\ifdefined` and `\ifcsname` answer for `\numexpr`,
  `\attribute` or `\Umathchar` as luatex does, whether or not texrs executes it;
  `BUGS.md` has the census of which ones it does. `\ifdefined` of a character
  that is not active is true, as in luatex.
- `\detokenize`, `\unexpanded` and `\expanded` read their text as e-TeX's
  `scan_general_text` does, expanding up to the brace (`\detokenize\expandafter{\x}`),
  in running text, in an `\edef` body and in a `\message`; `\expanded`'s result is
  expanded again where it lands, so a `\noexpand` in it protects one pass only.
- `\numexpr` and `\dimexpr` as luatex's `scan_expr` evaluates them -- precedence,
  parentheses, a multiplication-then-division taken as one rounded scaling,
  `! Arithmetic overflow` and `Missing ) inserted for expression` -- with
  register operands read when the program runs, in an assignment, a
  conditional and a `\message` alike. `\glueexpr` and `\muexpr` the same way over
  glue, and `\gluestretch`, `\glueshrink`, `\gluestretchorder`,
  `\glueshrinkorder`, `\gluetomu` and `\mutoglue`.
- e-TeX's and LuaTeX's version numbers and strings (`\eTeXversion`, `\eTeXrevision`,
  `\luatexversion`, `\luatexrevision`, ...), `\currentgrouplevel` and
  `\currentgrouptype`, `\showtokens`, `\showgroups`, and the four interaction
  modes with `\interactionmode`: global, ending the terminal line, and batch mode
  withholding what the lowerer would print.
- e-TeX's and LuaTeX's parameters -- every integer one luatex has
  (`\tracingassigns`, `\lastlinefit`, `\suppresslongerror`, `\outputmode`, ...),
  the page dimensions and offsets, `\mathsurroundskip` and `\everyeof` --
  assigned, read and scoped like TeX82's, from luatex's INITEX values.
  `\suppresslongerror` lets a `\par` into a macro argument, and `\hoffset` and
  `\pageleftoffset` set each other an inch apart. What the stomach and the
  backend make of the rest is theirs.
- LuaTeX's conditionals `\ifabsnum`, `\ifabsdim`, `\ifincsname`, `\ifprimitive`
  and `\ifcondition`, decided in running text, in an `\edef` and in a
  `\message`, and over a register the run holds; `\begincsname` and
  `\lastnamedcs` wherever they expand; `\immediateassignment` and
  `\immediateassigned` in running text.
- `\tracinggroups` and `\tracingassigns` write what luatex writes, in its
  order: a group is reported entering and leaving (after its restores, as
  luatex's `unsave` does), and an assignment as `changing`, `into` or
  `reassigning` -- a macro, a `\let`, a `\chardef` (which is `\relax` while its
  number is read), a parameter, a token list, and a count or dimension whose
  value the lowerer follows. Tracing output after the last message reaches
  the terminal.
- LuaTeX's category-code tables: `\catcodetable`, `\savecatcodetable` and
  `\initcatcodetable`, with luatex's checks (`! Invalid \catcode table`) and a
  group restoring a `\catcode` into the table it was made in. Every prefixable
  LuaTeX command takes `\global`.
- `\end` reports the conditionals it found incomplete, with the line each
  began on (§1335), and run as a LuaTeX (`-progname=luatex`, or any LaTeX
  document) ends as luatex does: the file's `)` against the last message, and
  the open groups listed.
- `\currentiflevel`, `\currentiftype` and `\currentifbranch`, for a conditional
  the lowerer decided and for the arms of one the run decides alike.
- `\scantokens` and `\scantextokens`: the text read again as a file, a line at a
  time under the category codes in force, with `\everyeof` at its end.
- The engine's call points for LuaTeX's input callbacks (`src/input_hooks.rs`):
  `find_read_file` and `open_read_file` where `\input` and `\openin` open a file,
  and `process_input_buffer` on each line of the document and the files it
  inputs as the mouth reaches it, without its line end.
- Groups, which scope the macro table AND the registers they write; a
  register assigned `\global` inside one keeps what the last global assignment
  left, even when a local one follows it (§283).
- Registers: `\count`, `\dimen`, `\skip`, `\toks` and `\muskip`, with `` `x ``
  character codes and `\advance`/`\multiply`/`\divide`. `\dimen0=10pt \advance\dimen0
  by 5pt` reads back `15.0pt`; `\skip0=3pt plus 1fil` reads back with its stretch.
  The dimension, glue and token parameters of `tex.web` §247, §224 and §230 --
  `\hsize`, `\parindent`, `\baselineskip`, `\thinmuskip`, `\everypar` and the
  rest -- are registers in the same sense, at INITEX's zero: assigned, read,
  coerced and restored by a group like `\dimen`, `\skip` and `\toks`. On the string
  path they are stores: the breaker's measure is `Layout`'s, not `\hsize`,
  and `\everypar` is not inserted when a paragraph starts. A plain document's
  list path (`--dvi`) reads them as tex does, `\everypar` in front of the
  token that started the paragraph (§1090-§1091).
- Box registers and the lists they hold, built while the program runs
  (`src/stomach.rs`): `\setbox`, `\box`, `\copy`, `\lastbox`, `\vsplit`,
  `\hbox`/`\vbox`/`\vtop` with `to` and `spread`, `\raise` and its siblings,
  `\unhbox` and its siblings, glue, kerns, penalties, rules, leaders,
  discretionaries, marks, `\insert`, `\vadjust` and `\shipout`, with
  `\font\x=cmr10 at 12pt` loading the `.tfm` and the main loop of §1034-§1040
  applying its ligatures and kerns. A `\setbox` in a group is undone at its
  end and a `\global` one is not (§283); `\wd`, `\ht`, `\dp`, `\badness` and
  `\lastpenalty` read as registers wherever a number or dimension is read,
  `\wd0=` changes the box in place, `\ifvoid`/`\ifhbox`/`\ifvbox` are run-time
  branches, and `\showbox` prints §198's display. `tests/cases` holds three
  documents of them against tex's terminal output. In a plain document only:
  a LaTeX prelude defines `\hbox` as a text macro.
- The integer parameters of `tex.web` §236 — `\tolerance`, `\escapechar`,
  `\mag`, `\time` and the rest — at INITEX's values (§240) and the local
  clock's date (§241): assigned, read by `\the`/`\number` and wherever a
  number is scanned, changed by `\advance`/`\multiply`/`\divide`, scoped by
  groups. `\escapechar` decides what `\string`, `\meaning` and `\message`
  print in front of a name, and `-1` prints nothing. `\endlinechar` is what
  the mouth appends to each line as it reads it (`-1` appends nothing, and an
  assignment changes the NEXT line), and `\newlinechar` is the character a
  `\message` prints as a line end; a control character prints in `^^`
  notation. The `--pdf` line breaker reads `\tolerance`, `\pretolerance`,
  `\linepenalty`, the hyphen penalties and the demerit weights a document
  set, paragraph by paragraph and under `--dvi` too, and its page breaker
  reads `\clubpenalty`, `\widowpenalty` and `\brokenpenalty`.
- `\errmessage`, reported as tex reports an error and carried on from.
- `\message`, and `\immediate\openout`, `\immediate\write` and
  `\immediate\closeout` on LuaTeX's 128 write streams. A file gets LuaTeX's
  name -- `.tex` added when the name has none, a braced name keeping its
  spaces, `\jobname` expanded inside it -- and LuaTeX's bytes, every character
  raw and `\newlinechar` ending the line; `tests/write_streams.rs` holds the
  files byte for byte against luatex's. A closed stream or one past 127 writes
  to the terminal and a negative one to the log alone. A file written in the run
  is read back by a later `\input` or `\openin` as it now stands, not as the
  run before left it. On a plain document's list path (`--dvi`) a `\write`,
  `\openout` or `\closeout` WITHOUT `\immediate` is a whatsit node of its
  page and is carried out when that page ships (§1373), in the order the page
  is drawn -- a `\write` to the terminal between the page's `[1` and its `]`,
  reading its registers then -- and a `\special` is a whatsit node whose `xxx`
  the DVI carries where it stands. On the string path all four wait for a
  shipout that never comes, and [0x06] says why.
- `\input`, which is what every real document does first: the file is read where
  it is named, and its own `(./name.tex …)` nests inside the outer one's. `\endinput`
  ends it with the line it is on.
- `\openin`, `\closein`, `\read` and `\ifeof`: a stream opens on the file the
  name finds, as `\input` finds one, and `\read` takes it a line at a time
  (more while a brace is open), closing it when a read finds nothing left. A
  `\read` from a closed stream would read the terminal, and stops the run.
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
  `texio`, `status` and `luatexbase` reaching the engine's real state. The
  state is built the way LuaTeX's `luainterpreter()` builds it: LPeg,
  slnunicode (`unicode`), LuaFileSystem and `bit32` compiled from the C sources
  LuaTeX itself ships (`vendor/luatex/`), `zlib` and `gzip` over zlib, `md5`
  and `sha2`, `fio` and `sio`, the `string` and `os` extensions, the compatibility functions
  TeX Live's Lua is built with (`module`, `package.seeall`, `math.pow`,
  `__ipairs`), `kpse`, `lua` (bytecode registers, `\luabytecode`), `callback`
  (LuaTeX's registry and slot numbers) and `status` answering for the LuaTeX
  1.24 interface. `require` finds a module as a file of kpathsea's `lua`
  format, through `kpsewhich`, so lualibs loads as it does in luatex. A library
  function a package keeps in a local reaches the input of whichever chunk
  later calls it. `node` is LuaTeX's whole vocabulary on both faces (userdata
  and `node.direct`), with attribute lists and the properties table; `tex`
  reads and assigns the internal parameters texrs holds and the catcode,
  lccode, uccode, sfcode, mathcode and delcode tables through the engine's
  own assignments; `font` defines fonts from Lua tables and reads TFMs as
  luatex does; `pdf` hands its dictionary strings, `\pdfvariable`s and objects to
  the backend that `\pdfextension` writes through, so they reach the file. A list callback
  (`hpack_filter`, `pre_linebreak_filter` and the rest) registered from a chunk
  is called by the stomach while the program runs, with luatex's arguments, and
  its answer is what gets packed. TeX Live's luaotfload loads under ltluatex
  and registers its definer on `define_font`, as in luatex.
- `--dvi`: a page. A plain document is set as `tex` sets it, plain.tex
  preloaded and every list built by the stomach, and its DVI is tex's byte for
  byte on every document dvi-parity measures and on the formulas and displays
  of `tests/math_cases`. A LaTeX document is
  measured in a real font (`.tfm`), broken into total-fit lines by the same
  breaker as `--pdf`, packed into boxes and shipped through
  `hlist_out`/`vlist_out` -- a fixed number of lines to a page, and no maths (a
  formula's scripts are dropped; `--pdf` sets them).
- `\label`, `\ref` and `\pageref`, resolved against the pass that finds the
  pages — `\ref` gives `\@currentlabel` as the class's `\the<counter>` writes
  it: a heading's number (`1` in an article, `1.1` in a report or book, lettered
  after `\appendix`), a float caption's, or the equation number a numbered
  display sets beside it (`equation`, the `align` family per row, `\tag`,
  `leqno`); a float or display is a group, so its number does not outlive it.
  `\counterwithin` / `\counterwithout` are the kernel's; `\numberwithin` and
  `\eqref` exist only behind `\usepackage{amsmath}`, as under pdflatex.
  `\pageref` gives the page it fell on.
  Worth knowing what this is NOT evidence of: the corpus has 88,341 `\label{`
  and zero `\ref{` or `\pageref{`, because Pandoc writes labels and never
  references them. So this moves no book in the sweep. It is correctness for
  LaTeX documents generally, not a corpus result, and re-running the sweep to
  look for a change will find none.
- OpenType shaping as LuaHBTeX's `mode=harf` does it (`src/shape.rs`), ported
  from luaotfload's `luaotfload-harf-define.lua` and `luaotfload-harf-plug.lua`
  with HarfBuzz through harfrust: a font's characters table and parameters at a
  size, and a list cut into runs, shaped, and turned back into glyph, kern and
  glue nodes -- discretionaries a ligature spans reshaped as luaotfload reshapes
  them, `/ActualText` spans where a cluster is not one glyph. Checked list by
  list against luahbtex (`tests/harf_parity.rs`, frozen by
  `scripts/harf-freeze.sh`): Latin in a CFF and in two TrueType faces, Greek,
  Cyrillic, Arabic right to left, Devanagari and Han agree on every glyph's slot,
  offsets and dimensions and on every kern, glue and discretionary. A `\font`
  request string is read as luaotfload reads it (`src/fontreq.rs`, the
  `font_request` grammar of `luaotfload-parsers.lua`): `file:`, `name:`, `[path]`,
  a subfont, the slash modifiers and the feature list.
- OpenType fonts as luaotfload's default `mode=node` loads and processes them
  (`src/otf/`), ported from the ConTeXt fontloader luaotfload carries rather
  than from HarfBuzz: the font tables read as `font-otr`, `font-cff` and
  `font-dsp` read them and unified as `font-oup` unifies them, so a font's
  characters table -- slots, private slots, widths, heights, depths,
  `/ToUnicode` -- and its parameters come out as luaotfload hands them to the
  engine (`tests/node_chars.rs`); and a list processed as `font-ots` and
  `font-otj` process it: the features a request asks for, worked out as
  `apply_default_features` works them out, applied lookup by lookup with their
  discretionary handling, the positioning injected as font kerns, offsets and
  wider spaces, and luaotfload's own processors around them (NFC
  normalisation, the Arabic joining analyzer and mark reordering, `notdef`,
  default ignorables, variation sequences, `tlig` and the other extra
  features). Checked list by list against luahbtex in `mode=node`
  (`tests/node_parity.rs`, frozen by `scripts/node-freeze.sh`): Latin with
  kerns, ligatures, space kerns and hyphenation discretionaries in CFF and
  TrueType faces, optional features and alternates, Greek, Cyrillic, combining
  marks, Han and ideographic variation sequences, and Arabic right to left.
  A variable TrueType font is loaded at the instance `axis=` or `instance=`
  asks for -- advances, bounding boxes from the varied outlines, and the GPOS
  values that vary -- and checked against luahbtex at several instances
  (`tests/node_var.rs`). A math font loaded as unicode-math loads one
  (`script=math`) carries luaotfload's MathConstants, variant chains,
  extensible recipes, italics, top accents and math kerns, checked against
  luahbtex for Latin Modern, STIX Two, TeX Gyre Termes and Pagella Math
  (`tests/node_math.rs`). `name:` and unprefixed requests resolve through
  luaotfload's own names database and its lookup cache, read where luaotfload
  keeps them (`src/otf/names.rs`, checked request by request against
  luaotfload's resolver by `tests/node_names.rs`).
  Neither this nor the harf shaper is on the typesetting path yet; [0x06] says
  what is missing.
- The font programs luatex embeds for an OpenType font, as libraries for the
  PDF writer: a TrueType face's `/FontFile2` subset (`src/otf/ttsubset.rs`,
  writetype2.c's `make_tt_subset` with sfnt.c's table writer), a CFF face's
  `/FontFile3` subset (`src/otf/cffsubset.rs`, writecff.c's `write_cff` and
  `write_cid_cff`: name-keyed fonts made CID-keyed, subroutines inlined), each
  with its `/CIDSet`, and the CID font's `/ToUnicode` CMap
  (`src/otf/cidcmap.rs`, tounicode.c's `write_cid_tounicode`). Each is
  compared byte for byte with what luahbtex writes for the same glyphs
  (`tests/ttsubset.rs` and `tests/cffsubset.rs`, frozen by
  `scripts/ttsubset-freeze.sh` and `scripts/cffsubset-freeze.sh`): DejaVu
  Sans, Arimo and Noto Serif; Latin Modern Roman, TeX Gyre Termes, STIX Two
  Math and the CID-keyed Harano Aji Mincho.
- `--pdf`: the PDF itself, and the better half of the typesetter. Lines are
  chosen by minimising total demerits across the whole paragraph (`tex.web`
  §813-§890) with Liang hyphenation, and set to the measure with PDF's `Tw`,
  which is what makes pricing glue usable at all. The file is written as
  LuaTeX's backend writes one: its objects numbered, ordered and packed into
  object streams as luatex does, its streams deflated by zlib at luatex's
  level, a TeX font's widths and descriptor taken from its TFM, its subset tag
  and the file's `/ID` made by luatex's recipes. LuaTeX's page-stream writer is
  ported too (`src/pdfpage.rs`: text in `BT`/`TJ` with kerns, rules, `pdf:`
  specials, save, restore and matrices), and so is its shipout
  (`src/pdfship.rs`: `hlist_out` and `vlist_out` over the stomach's box trees).
  Plain luatex's page for `Hello world.`, built as its output routine builds it
  and shipped there, is luatex's file object for object but for the font
  program and the information dictionary. No run ships through it yet: a
  `--pdf` page is still drawn from broken strings.
- LuaTeX's backend commands. `\pdfvariable` expands to the parameter its key
  names, assigned, read and grouped like any; `\pdffeedback` answers
  `lastobj`, `lastannot`, `lastlink`, `pageref`, `colorstackinit`, `version`
  and the rest; `\pdfextension` reads every one of luatex's keywords. Objects,
  outlines (with the destinations they name, in a name tree), `info`,
  `catalog` with `openaction`, `names`, `trailer`, `pageattr`, `pagesattr` and
  `pageresources` reach the file numbered, ordered and spelled as luatex writes
  them: measured on a document using all of them, every object agrees with
  luatex's byte for byte but the content stream, the font program and the
  information dictionary (`tests/pdf_backend.rs`).
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
`\@startsection` writes after a title).

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

A plain document's `--dvi`, and its message run, now take the list path.
plain.tex is preloaded as `tex` preloads it, and the stomach builds every list
(`src/stomach.rs`, [0x03]). Paragraphs are broken from their node lists
(`src/parbreak.rs`, §813-§876, with §891-§918's hyphenation), plain's
`\output` runs when the page builder fires it, and the DVI is written as
§583-§643 write it (`src/dvi/out.rs`). Every document
`cargo run --bin dvi-parity` measures comes out byte for byte as tex writes
it. Formulas are built there too: math mode's commands make an mlist
(§1136-§1193, `src/stomach/math.rs`), `mlist_to_hlist` sets it from the
families the document loaded, and a display is placed with its equation number
as §1199-§1205 place it. The documents in `tests/math_cases` come out as tex's
DVI byte for byte, and so do `\accent` (§1123-§1125), `\parshape` (§1248)
and alignments (§768-§812): tables, and plain.tex's displays and matrices
built with `\halign` (`tests/align_cases`). `BUGS.md` lists what that path
still lacks.
`--pdf` and every LaTeX document still take the string path. There the page
breaker reads its penalties once per document, and the shipper is handed line
boxes built from already-broken strings.

The same missing node list is why, on the string path, a `\write`,
`\openout`, `\closeout` or `\special` without `\immediate` does nothing yet, nor a `\pdfextension
literal`, `dest`, `annot`, link, thread, `save`, `restore`, `setmatrix`,
`colorstack` or `refobj`, a `\useimageresource` or a `\savepos`. Each is read and built as the
whatsit §1341 describes, and `Lowerer::ship_whatsit` is §1373's `out_what` for
it -- the `\write` expanded when it is shipped, not when it was read -- but a
whatsit has to sit in the list of the page it is shipped with, and until pages
are built from node lists the whatsits wait in `deferred_whatsits` for a
shipout that never comes. `\immediate` does all three now. On a plain
document's list path the four have their nodes: a `\special` reaches the DVI
with its page, and the other three are done when their page ships.

On the DVI ladder nine of the ten documents reach STRUCTURE: the text and its
structure agree with `tex`, the bytes do not — see `BUGS.md`.

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
xcolor needs \cs_new_protected:Npn` — and a package's own `\RequirePackage`s
are followed, so `keyval` is read before the `graphicx` that asks for it. Which
of the classes and packages real documents use load, and whether the page they
set says what lualatex's says, is a table `scripts/package-status.sh` measures
and writes into BUGS.md ("LaTeX, loaded the way lualatex loads it").

The genuine kernel is the way out of the stand-in, and `texrs -ini
-progname=lualatex latex.ltx` is how far it has got: TeX Live 2026's own
`latex.ltx`, found with kpsewhich, read through its e-TeX and LuaTeX
checks, the texsys probe, the allocator and every register it `\countdef`s
past 255, LuaTeX's catcode tables and the UnicodeData.txt pass that gives
every letter its category, `ltluatex.lua`, and into `expl3.ltx`, which stops
on Lua the engine has not got yet (BUGS.md has the list, in the order the
kernel reaches it). What made that possible is that the frontend now follows
every register's value as the program assigns it (`src/known.rs`), and the
expander reads that table: a conditional over registers is decided where the
mouth reaches it, as §498 decides it, an `\edef` freezes `\the\count`,
`\the\dimen` and `\the\skip`, and a register only the run knows (a
`\rustcall`'s) is the one case still decided at run time. Run as a LuaTeX —
`-progname=luatex`, `lualatex` or `luahbtex`, or any LaTeX document — texrs
has LuaTeX's 65536 registers of each kind, its attribute registers, and a
category code for every Unicode character; plain `-ini` keeps TeX82's 256.
`10\p@` reads as `10.0pt`, whether the factor is written out or arrives from a
macro (`\def\x{10}\dimen0=\x\p@`), in plain TeX and in a LaTeX preamble, as
luatex and lualatex read it: §453's `<factor><internal unit>` is implemented,
and so are `em` and `ex`, which are the font's `\fontdimen6` and `\fontdimen5`
out of `cmr10.tfm` and agree with tex to the scaled point —
`[em=10.00002pt][ex=4.30554pt]` from both engines. A package that loads and then breaks what the
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
reaches the real registers and parameters, `token` reaches the input the chunk
stands in front of, and a chunk that fails stops the run with its Lua error as a
TeX error. The node lists TeX builds are the stomach's and exist while the
program runs, after the chunks have run while it was lowered: so a chunk cannot
reach `tex.box[n]`, `tex.getlist` or the current list (`node.write`,
`node.last_node`), and those refuse by name; a callback registered from a chunk
is how Lua reaches the stomach's lists (BUGS.md lists which callbacks are
called). `node.ligaturing`, `node.kerning`,
`node.hyphenating` and `node.mlist_to_hlist` are not ported. Of LuaTeX's other
libraries, `lang`, `vf`, `img`, `mplib`, `ffi` and `socket` are not there, and a font a `define_font` callback
answers sets type from its characters' dimensions but does not yet reach the
PDF as the file it names. The shell is disabled (`os.execute`, `io.popen` and
`kpse.check_permission` answer as `luatex -no-shell-escape` does), and a file a chunk opens, removes or makes goes through kpathsea's `openin_any`/`openout_any` checks as in luatex, and an error
a Rust library function raises is a userdata that prints, concatenates and
matches like the string luatex's C would raise, but is not `type` string. See
BUGS.md for each.

The number, run against 274 `.tex` files — a real LaTeX/LuaLaTeX corpus of
Pandoc-generated books of 16,000 lines and up, with fontspec, TikZ,
`\directlua` and `--include-in-header` fragments, plus texrs's own fixtures
including the ones written to be refused: **229 of 274 run to completion**, and
say 75,627,678 bytes of text.

Which 274: 167 from the publications tree `scripts/publications.sh` defaults to,
plus the `.tex` files then under `tests/cases`, `examples/`, `tests/pdf_cases`
and `tests/dvi_cases`. `tests/cases` has grown since, so the set is not
reproducible from HEAD alone. Point the sweep at a parent of both trees to
sweep the whole set; point it at the default and you are measuring the
publications tree alone, which is a different number and a different claim.

That number went DOWN when Lua started running: the same corpus was 266 of 274
before it. 42 of the 46 failures are one family of header fragments carrying an
unsubstituted `@FALLBACK_LIST@` placeholder inside a `\directlua`, which is not
valid Lua — texrs used to "complete" them only because it consumed the chunk
unread. `lualatex` refuses those same files too (`! LaTeX Error: \usepackage
before \documentclass`), so the drop is the engine reading what it used to skip
rather than a capability lost. Of the remaining four, three are texrs's own DVI
fixtures, which stop on `\special`, `\font` and `\vfill`, and one is written to
fail.

Two things decide that count besides the engine, and a re-run that ignores them
reports failures texrs did not cause. The sweep defaults to `target/debug/texrs`
and to a 120-second limit per document; a debug build is several times slower
than a release one, and 120s is short for a 16,000-line book. A run of mine over
the 167-file publications tree put 35 documents over that limit — and the first
of them, given 600s, finished and produced 780,253 bytes of text. Those are
`rc=124`, the harness killing the run, and the script now says so and prints the
re-run that settles it rather than listing them beside real refusals.

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
  both. `Scale=` is applied, including `Scale=MatchLowercase`, which is the form
  a document actually writes: the factor is the ratio of the main face's
  x-height to this family's, read from OS/2's `sxHeight`. Measured, with the
  same file differing only by the option:

  ```
  Scale=0.5                       10 pt -> 5 pt
  \setsansfont{Orbitron}          {\sffamily\Huge} = 24.787 pt
  the same with MatchLowercase    22.578 pt, a factor of 0.9109
  ```

  0.9109 is Arimo's `sxHeight` over Orbitron's, 1082/2048 over 580/1000 — the
  ratio the code computes, not the 0.9076 that lualatex's own rounded output
  divides to. A face whose `sxHeight` is absent — the versions of OS/2 predating
  the field — is left unscaled rather than scaled by a guess. Bold and italic are cuts of
  the main family and take its scale.
  `pdffonts` is how you tell: a document that meant to ship its own face
  and reports `Helvetica Type 1 no` got the fallback. The difference matters
  because `fc-match` ALWAYS answers — asked for a font nobody has installed it
  returns its default, so a book whose fonts travel with it was set in whatever
  that default happened to be. `Path=` is written when the document is built
  and is regularly a directory that no longer exists, so a stale one is retried
  against the directory the document was read from, where the fonts are.
  The file is carried in the PDF as `/FontFile2` — `pdffonts` reports
  `KAZDEO+Arimo-VF TrueType yes yes`, embedded AND subset — once, and referred
  to from every page; lines are
  broken on that font's own advance widths, out of its `hmtx` through its
  `cmap`. No kerning and no OpenType ligature reaches this path yet: the
  harf shaper and the node-mode processing of [0x03] are libraries beside it
  (`BUGS.md`, "OpenType fonts are not loaded as luaotfload loads them, yet"). A family nothing can be found for falls back to whichever of the
  fourteen carries the same metrics, not to Computer Modern. The face is
  SUBSET: only the glyphs the document actually set are kept, with `cmap`,
  `glyf`, `loca`, `hmtx`, `maxp` and `head` rebuilt rather than copied, and each
  subset carries its own six-letter tag. A 5.6MB book comes out with five faces —
  `Arimo-VF`, `Arimo-Italic-VF`, `Orbitron-VF`, `ShareTechMono-Regular` and a
  system `ArialUnicode` borrowed for glyphs the others lack — and the five
  embedded programs come to **183,944 bytes** where the four bundled faces alone
  are 1,121,312 on disk. That pair is the claim about subsetting.
  The whole-file totals are a DIFFERENT claim and worth not confusing with it:
  this book is 2.6MB against lualatex's 968,737, but the file also carries a
  contents, folios, pictures, and five faces where lualatex embeds one subsetted
  Latin Modern. Read as a subsetting result it says the subsetter is poor; it is
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
- **The face axes compose now, and one fallback is left.** Family, series and
  shape were a single slot until v0.6.0, so the last switch won and the earlier
  one was lost. They are three axes, as NFSS has them:

  ```
  {\sffamily\Huge X}            Orbitron-VF     was Orbitron-VF
  {\sffamily\bfseries\Huge X}   Orbitron-VF     was Helvetica-Bold
  {\bfseries\Huge X}            Helvetica-Bold  unchanged
  ```

  The middle row is the fix: `{\sffamily\bfseries\Huge}` is the ordinary way to
  write a heading format, and the family used to be lost to the series. What the
  third row shows is what is left — asking for bold in a family that declares no
  bold cut gets base-14 Helvetica-Bold rather than a bold of that family.
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
  with a figure     /Subtype /Image present, "After." baseline y = 523.2
  without           no image,                "After." baseline y = 703.2
  ```

  180pt of room reserved, and the text below moves down for it. The baselines
  are the evidence rather than the file sizes: a byte count moves with every
  feature the writer gains, where the offset is the thing being claimed. An image given
  no size is bounded to the measure and the text height rather than set at the
  file's own — a diagram exported at 1600 pixels is 1600 big points wide, which
  is wider than the page. A file the reader cannot rasterise (a `.pdf` named
  where a `.png` sits beside it) falls back to a sibling of the same name.

  This entry said the opposite until v0.6.0, and said it with numbers. Three
  faults were stacked and the first hid the other two: the primitive was wired
  to nothing, `\pandocbounded` boxed every figure without setting the box, and
  once figures did reach the page they were placed at the file's own size.

What `--dvi` still does not do is draw a picture, or set the maths of a LaTeX
document. A plain document is set as tex sets it, formulas included. A LaTeX
one takes the string path, which
stacks a fixed number of lines on each page where `--pdf` prices the whole
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

There are three contracts, against two oracles, and only the first is a byte
comparison that already holds.

The `\message` stream is compared byte-for-byte against the real `tex` binary.
No expectation is written by hand:

```sh
cargo run --bin parity          # the committed corpus
cargo run --bin parity -- case.tex   # one file or directory
cargo test                      # the same comparison, as a gate
```

No expectation is written by hand in the other direction either: the freeze
that records them, `cargo run --bin parity -- --freeze`, re-records from `tex`
and not from texrs. A reader who assumed it captured texrs's own output would
have the whole gate backwards — it would then be asserting that texrs still
agrees with itself.

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

LuaTeX's own primitives -- e-TeX's, LuaTeX's, the ones TeX82 has not got -- are
compared against `luatex` the same way, by the cases in `tests/luatex_cases`:
INITEX luatex with every extra primitive enabled under its own name, the state
a LaTeX format starts from, against texrs run as a LuaTeX (`-progname=luatex`).
Each case names the primitives it exercises, the gaps are listed in `tests/luatex_known_gaps.txt` under the same two-way rule,
and `bash scripts/primitive-census.sh` measures every primitive luatex has
against texrs and writes the verdicts into `BUGS.md`:

```sh
bash scripts/primitive-census.sh                 # BUGS.md and the freeze
cargo test --test luatex_primitives             # the replay, and the live check
```

The page is not compared that way, because nothing reaches byte equality yet.
`tests/dvi_parity.rs` and `tests/pdf_parity.rs` are RATCHETS instead: each
document has a rung it last reached, recorded in `tests/dvi_floor.txt` and
`tests/pdf_floor.txt`, and the gate fails when one drops rather than asserting
agreement it does not have.

A rung is only half a ratchet: climbing one changes nothing until it is
recorded, and the commands that do it are

```sh
cargo run --bin dvi-parity -- --record
cargo run --bin pdf-parity -- --record
```

which is the opposite direction from `parity --freeze`. That one re-reads the
ORACLE; these two re-read texrs, and what they write down is how close it got.
Running either after a change that lost ground would lower the floor and retire
the very regression the file exists to catch, so they are run deliberately after
a climb rather than as part of a build.

```
DVI   NONE < PARSES < PAGES < TEXT < STRUCTURE < BYTES
PDF   NONE < PRODUCED < PAGES < PAGESIZE < TEXT < LINES < FONTS < BYTES
```

Ten documents on each. Today the DVI floor stands at STRUCTURE for nine of
them, and the PDF floor at FONTS for seven and TEXT for two, with
`empty_document.tex` the only case at BYTES, and the only one at NONE on the
other axis. The oracle differs too: DVI is
against `tex`, PDF against `luatex`, and both skip loudly rather than passing
when their oracle or the tools that read the output are missing.

Byte equality is a reachable goal for DVI, which carries no fonts and no
compression, and a distant one for PDF: `Hello world.` is 224 bytes from `tex`
against 192 from texrs, where the same document in PDF is 11,729 from `luatex`
against 14,592. The file is laid out, numbered and compressed as luatex lays it
out (`src/pdfdoc.rs`), so what still differs is inside the objects: the page's
content stream, which waits on pages shipped from node lists, and the font
program; `BUGS.md` has the object-by-object count.

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
The probes cover register arithmetic on counts, dimensions, glue and token
registers, the `\if` family, `\afterassignment`, `\aftergroup`, `\futurelet`,
`\expandafter`, `\csname`, case shifting, `\string`/`\meaning`, catcode and
`\escapechar` changes inside groups, and `\edef` freezing. The oracle loads the
plain format and texrs starts from INITEX, so a probe may not depend on a
catcode or a macro only plain sets.

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
`tex` loads the plain format on every run while texrs loads nothing, and the two
no longer do the same work — texrs typesets and writes a PDF by default where
`tex` writes DVI.

### Measured, at v0.3.0

**These numbers are from v0.3.0 and have not been re-taken.** They were measured
when texrs implemented the mouth and the expander and shipped no page at all —
before the line breaker, the page builder, fonts, images, maths, TikZ and the
PDF writer. Every one of those does work per run that was not being done when
these were recorded, so the ratios below describe an engine that no longer
exists. They are kept, dated, rather than deleted or quietly refreshed: the
shape of the finding — where the time goes, and that a batch parallelises where
`tex` cannot — is what the section is for, and a number presented as current is
worse than one presented as historical.

Against `tex` 3.141592653 (TeX Live 2026) on a 10-core machine. Every figure
below is a median of repeated interleaved runs rather than a best-of, because
the machine was under other load while they were taken. Read them with the
caveats above, and with the one that has since changed: at the time texrs was
NOT typesetting and `tex` was.

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
work. At v0.3.0 a single document was a single thread: this parallelised the
batch, not the engine. Line breaking, page drawing, and page encoding on the
`--dvi` path now fan out inside one document; see below and the 2026-10-17 measurement.

**PDF assembly.** `pdf::document` writes each font and picture once and
finds an already-written one by address before comparing contents, so a font
shared by every page costs one pointer comparison per use rather than a read of
the whole font program. Measured on a synthetic 600-page document with 40
embedded faces, that lookup went from about 1.9 s to about 0.13 s. A picture
repeated by value (each page holding its own copy) is matched first by size,
depth and data length, and the full comparisons that confirm those matches run
in parallel; two pictures that match that way and still differ send the
document back to the one-by-one comparison. Object numbers are handed out by a
serial walk in the order the old single loop added objects, and each page's
content stream and page dictionary are then built in parallel into the numbers
reserved for them; each page's content moves into its stream rather than
being copied there. The content streams are deflated at luatex's level 9 on
the pool before the file is laid out, each reading only its own page; the
`/ID` is luatex's, a digest of the time, the directory and the file's name, so
the file itself is no longer hashed. Measured 2026-10-17 on a 6000-paragraph
plain document (`--pdf`, release, best of 8 at load averages 25 to 30): 3.06 s
before the luatex layout and 3.11 s after, for a file of 176,967 bytes where it
was 1,918,708. Subsetting and the cross-reference table stay
sequential; output is byte-identical across runs and thread counts (set
`SOURCE_DATE_EPOCH` to pin the creation date). Medians of interleaved runs
at 8 threads, `pdf::document` alone: 55 to 16 ms on a 1401-page document
repeating twenty pictures 4000 times, 59 to 43 ms on a 3328-page mixed one, 14
to 13 ms on the 1001-page LaTeX document below, where the digest is most of
what is left.

**Page breaking.** `plan_pages` picks the least-cost sequence of page breaks
backwards from the end of the document, and only that choice is serial: where
a page started at a given line may end, and what each end costs before what
follows it is added, depend on that start alone. On more than one thread
those offers are gathered in parallel a block of starts at a time into
buffers kept from block to block, while the calling thread chooses from the
block gathered before it; on one thread each start's offers are gathered into one reused
buffer just before they are chosen from. The break penalties and the leading
of each line are read per line in parallel, and the walks that join them run
in order. The plan is the one the single walk made: the same page boundaries,
feet and repeated heads. On the 1001-page LaTeX document below, medians of
interleaved runs: the planner 21 to 11 ms at one thread and to 5 ms at 8, the
penalties 11 to 2 ms and the leadings 5 to 1 ms at 8.

**Page drawing.** Once the lines are broken
and cut into pages, drawing a page reads only its own lines, the colour, face
and size stacks as they stood where the page begins, and its folio. What a
run of lines does to a stack does not depend on what the stack holds: it pops
some number of entries it did not push (stopping at the floor every stack
keeps) and leaves the entries it did push, each of which is either named by its
marker or made from the entry in force (a face code modifies the face below, so
a chain of codes is a table over the twelve faces). So on more than one thread
each page's effect on the stacks is read in parallel, and a serial walk only
applies those effects in page order to record each page's start; one thread, or
a handful of pages, walks the lines in turn. The pages are then drawn
independently and collected in page order, so a single document's page content
scales across cores and the file is identical at any thread count. A document
of only a few pages is drawn on the calling thread. Each page also shares its
fonts by reference (`Arc`) instead of copying a font program per drawn run,
which was the larger share of this stage before the fan-out.

**Line breaking.** What one paragraph hands the next is only the colour, face
and size stacks, the centring flag and the list depth. On a pool of more than
one thread, each paragraph's effect on that state is read in parallel without
measuring anything (the stacks as for page drawing; the centring flag and the
depth are each set or left alone), a serial walk applies the effects in order
and records the state at the head of each chunk of paragraphs, and the chunks
fill in their own paragraphs' start states in parallel; the paragraphs are then
broken in parallel and collected in document order, and the paragraph space between
two is decided on the collected lines. One thread, or a document of fewer than
64 paragraphs, walks the paragraphs in turn. Liang's patterns and the
`\hyphenation` exceptions are held in a letter trie, as `tex.web` holds them,
so a word costs one table step per letter from each position rather than a
hash of every substring; it breaks every word of `/usr/share/dict/words`
where the hash table did. Measured 2026-10-17 on the 1001-page document below,
under load averages between 43 and 110, median wall went from 1191 ms to 839 ms
at 8 threads and from 1664 ms to 1462 ms at one; output is byte-identical to
the previous build at both. Reading the paragraph and page effects in parallel
then took the two prescans on that document from 28 and 23 ms to 7 and 2 ms at
8 threads (median of 11 interleaved runs, load averages 6 to 9; median wall
379 ms to 333 ms), and the page prescan from 22 ms to 10 ms at one, where it no
longer cuts every line into runs; PDF and DVI are byte-identical to the
previous build at 1 and 8 threads.

Lowering is serial, but a paragraph it has finished is finished, so on a pool
of more than one thread a PDF run of a source of 64 KiB or more breaks
paragraphs while the rest of the document is still being lowered
(`typeset::Pipeline`). The lowerer hands on the text of its document-level
commands as far as they can no longer change (an open text run up to its last
paragraph break); a job on the pool fills each formula slot as its batch of
settings comes back, and breaks every 64 or more finished paragraphs from the
state the ones before them left. The page builder then breaks only the rest.
Nothing is assumed to match: the paragraphs are used only when the text they
were read from opens the text the run produced (after `\ref`, `\pageref` and the
contents are resolved), and the families and the layout are the ones they were
measured with, the page penalties aside. A conditional or loop the run decides,
or a colour switch, stops the feed; a `\setmainfont`, a fallback family or a
geometry or class option read after paragraphs were broken sends the whole
text to the page builder, as before. Measured 2026-10-17 (median of 15
interleaved runs, load averages 9 to 15), median wall at 8 threads went from
345 to 313 ms on the 1001-page LaTeX document below, 887 to 837 ms on a
formula-heavy 2801-page one, 120 to 107 ms on a 394-page plain TeX one and 101
to 88 ms on a 200-page LaTeX one; at one thread nothing changes. PDF, DVI and
messages are byte-identical to the previous build at 1, 2 and 8 threads.

With the paragraphs broken alongside it, a large document's wall time follows
the lowerer's own speed, so the work it does per token was cut. The lowerer
looks at the last character of the text run rather than comparing suffixes,
offers its commands to the paragraph feed when they have grown (or every 1024
tokens) instead of after every token, asks the mouth for invalid characters
and the register tracker for owed marks only when there are some, and answers
a character that is not a math shift without calling into the formula reader.
A macro's definition is held by `Rc`, so a call no longer copies its parameter
text and body; the meaning table and the interned names hash with FxHash; the
mouth reads `TEXRS_PARALLEL` once per process; and a picture's nodes use one
shared copy of xcolor's palette instead of building it per node. Measured
2026-10-17 (medians of interleaved runs, load averages 8 to 13), `Lowerer::lower`
alone at 8 threads went from 148 to 101 ms on the 1001-page LaTeX document,
58 to 37 ms on the 394-page plain TeX one, 417 to 292 ms on a 3328-page mixed
one and 258 to 217 ms on an 800-page formula-heavy one (203 to 166, 48 to 32,
709 to 626 and 702 to 671 ms at one thread). Median wall at 8 threads went
from 372 to 323 ms, 826 to 729 ms and 569 to 564 ms on the LaTeX, mixed and
formula documents, and stayed at 130-134 ms on the plain one, whose wall the
lowerer does not decide; at one thread, 1043 to 1011 ms and 2518 to 2316 ms on
the LaTeX and mixed ones. PDF, DVI, stdout and stderr are byte-identical to the
previous build at 1 and 8 threads.

The formula reader was the larger part of what was left. A noad held a built
box inline, which made every field of every noad the size of a box; it is now
boxed, so a formula is a fraction of the bytes to read, copy and drop. A single
formula's spelling and setting read its one list in place instead of a
flattened copy, the spelling is built in one string, a symbol's math code is
looked up by the token's id in plain.tex's table hashed once rather than
searched per control sequence, the mouth reads a control word into a buffer it
keeps, and a group argument is read without its brace instead of having it
taken out after. Measured 2026-10-17 on a loaded machine, as the lowering
thread's CPU time (medians of interleaved runs, load averages 39 to 48 at 8
threads, 19 to 20 at one), `Lowerer::lower` went from 132 to 107 ms on the
1001-page LaTeX document, 235 to 129 ms on the 800-page formula-heavy one, 371
to 283 ms on the 3328-page mixed one and 45 to 42 ms on the 394-page plain TeX
one at 8 threads (186 to 151, 751 to 539, 696 to 527 and 37 to 34 ms at one
thread). Median wall (5 interleaved runs, load averages 21 to 23) went from
410 to 377 ms, 666 to 568 ms, 959 to 864 ms, 178 to 168 ms and 266 to 238 ms
at 8 threads on those four and the 200-page LaTeX one, and from 1110 to 1082,
2037 to 1832, 2689 to 2533, 473 to 426 and 349 to 347 ms at one thread. PDF,
DVI, `--text` output, stdout and stderr are byte-identical to the previous
build at 1 and 8 threads.

**Pictures, images and formulas.** Before the pages are fitted, every TikZ
picture is parsed again against the document's own face to size its nodes, and
every `\includegraphics` line has its lengths resolved against the measure and
the file it names. Each such line depends only on itself, the fonts and that
file, so once a document has eight or more of them the lines are remeasured in
parallel and collected in document order. The files are found and read before
that, once per distinct path (on the rayon pool when there are eight or more
image lines and more than one file), and every line naming a path shares the
one picture read from it, for measuring and for drawing; the PDF writer then
finds repeated uses to be one XObject by address instead of comparing their
pixels. Before, each use found and read its file twice and kept its own copy of
the pixels until the file was written. On a document of 4,000 figures drawn
from 10 photographs (release build, 18-core aarch64 macOS, load averages 20 to
27, medians of 9 interleaved PDF runs, 2026-10-17) wall went from 659 to 236 ms
at one thread and 304 to 159 ms at 8, and peak resident memory from 1.7 GB to
62 MB; the mixed document of text, pictures, images and formulas was unchanged
within noise (2324 to 2381 ms at one thread, 767 to 743 ms at 8). Measured
earlier the same day under heavy machine load, the remeasuring stage went from
about 150 ms to 19 ms with 6,000 pictures, 130 ms to
33 ms with 4,000 images, and 185 ms to 33 ms for a mix of both with formulas.
A picture's marker is also decoded through a lookup table rather than a search
of the base64 alphabet, which had been about a quarter of a drawing-heavy
document's serial time.

A TikZ picture's marker carries the height it reserves, measured by parsing the
whole picture against no font, and that parse was a third of lowering on the
mixed document below and two thirds on the drawing one. The height is still
needed (the contents and `\pageref` passes fit pages from it before pictures
are remeasured), but the parse reads only the picture's own options and body,
so the lowerer now writes the span with a slot and hands the source to the
batches formulas go in (see **Formulas** below): one index space, the same
32-item threshold, the same part-batch at the join and the same fill in
document order, including the page builder's early breaking. With fewer than
32 formulas and pictures together, or at one thread, everything is measured on
the lowering thread at the end. Measured 2026-10-17
(release build, 18-core aarch64 macOS, load averages 12 to 21, medians of 15
interleaved PDF runs; lowering-thread CPU from a separately instrumented build,
medians of 5):

| document | wall at 1 thread | wall at 8 | lowering-thread CPU at 8 | of it, pictures |
|---|---|---|---|---|
| mixed text, pictures, formulas | 2232 → 2226 ms | 724 → 634 ms | 230 → 159 ms | 81 → 4 ms |
| 6,000 pictures | 795 → 741 ms | 313 → 262 ms | 107 → 44 ms | 70 → 3 ms |
| LaTeX text and formulas, no pictures | 1027 → 1000 ms | 313 → 317 ms | 96 → 97 ms | none |

PDF, DVI, stdout and stderr are byte-identical to the previous build at 1 and
8 threads on every benchmark document.

**Formulas.** A formula is READ while the document is lowered, because reading
it expands macros against the engine and consumes the input, but it is no longer
SET there. The lowerer writes the formula's marker with a slot where the
setting's spec goes and keeps the parsed mlist with what setting it depends on:
the style, the math families at the type size in force, whether it is a
display, its environment's columns and the measure for an equation number.
What the lowerer reads back from its own output is the last character of the
text run, for a ligature's first half or a face marker, and a marker ends the
same way whatever its spec holds, so nothing it decides waits on a setting.
After lowering the formulas are set, in parallel once there are 16 or more,
and each spec is written into the slot its index names. Measured 2026-10-17 on
a synthetic document of about 35,000 formulas (load averages 18 to 36, medians
of 9 to 11 interleaved runs), wall went from 1128 ms to 760 ms at the default
thread count and from 1331 ms to 1037 ms at 8, for 22% more user CPU (2450 to
3000 ms); at one thread it is unchanged (2352 against 2356 ms). Output is
byte-identical to the previous build at 1 and 8 threads.

A formula's inputs are fixed once it is read, so its setting no longer waits
for lowering to end: formulas are gathered in batches of 32 and each full batch
goes to the rayon pool as it fills, while the lowerer reads on. At the end the
lowerer sets the last part-batch itself, collects the batches by the index of
their first formula, and fills the slots; the settings are freed on a worker.
A document of fewer than 32 formulas, or a run with `TEXRS_THREADS=1`, sets
every formula on the lowering thread at the end, as before. Setting reports no
diagnostics, so there is no message order for the overlap to change. Measured
2026-10-17 (load averages 16 to 33, medians of 9 interleaved runs, PDF): the
35,000-formula document went from 1089 ms to 820 ms at 8 threads and 978 ms to
778 ms at the default count; a mixed text, picture and formula document from
1510 ms to 1317 ms at 8 and 1304 ms to 1245 ms at the default. PDF and DVI are
byte-identical to the previous build at 1, 2 and 8 threads.

The workers' own cost was then the limit: setting a formula spent about half
its time copying and freeing. `mlist_to_hlist` copied every list it converted
(and, recursing, every sublist again), copied each noad and the one after it
to look at them, packed a copy of each noad's hlist only to read its height and
depth, and copied every hlist once more in the second pass. Now a list is
copied only when a `\mathchoice` in it is spliced, a noad only when a make_*
procedure rewrites one of its fields, heights and depths are measured off the
list in place, the second pass moves each hlist out, `hpack` keeps the list it
is given unless an adjustment list takes nodes out of it, and a spec's numbers
are spelled without `core::fmt`. Measured 2026-10-17 (load averages 14 to 37,
medians of 9 interleaved runs, PDF): setting the 35,000-formula document's
formulas went from 377 ms to 143 ms of CPU on one thread and from 720 ms to
295 ms summed over the workers at 8; the lowerer's wait for the last batches
at the join went from 22 ms to under 1 ms. Wall went from 1660 ms to 1402 ms at
one thread and 488 ms to 443 ms at 8; on the mixed document from 2307 ms to
2081 ms at one thread and unchanged at 8 (694 against 695 ms), and on a LaTeX
document with few formulas from 1011 ms to 950 ms at one thread and unchanged
at 8 (307 against 313 ms).
At 8 threads lowering itself and the page drawing after it are what the wall
waits on now, not the formulas. PDF, DVI, stdout and stderr are byte-identical
to the previous build at 1 and 8 threads.

**DVI pages.** On the `--dvi` path a page is its own lines and its number, so
each page's box is built and its `bop`..`eop` commands are encoded by a writer
of its own, in parallel; the pages are then appended in order, which is where
each `bop`'s pointer to the previous page is filled in and the postamble's
maximum height, width and stack depth are taken over the pages. The preamble,
font definitions and postamble are written serially. The file is byte-identical
to the single-writer output at any thread count, and a document of only a few
pages is encoded on the calling thread.

**Setup and writing out.** Outside the lowerer, a LaTeX run's longest serial
stretch was its preamble: each package the document names is first tried in a
throwaway lowerer of its own (see `src/latex/load.rs`), one after another, and
the register and counter scans read the whole source once per command name.
The tries read nothing another writes and are memoised by name, so on more
than one thread they are made side by side, at most `TEXRS_THREADS` at once,
and the preamble is then assembled from them in the document's order as
before; the per-command scans of a source of 64 KiB or more run on the pool
and are joined in command order. The code generator's string pool hashes with
FxHash. Measured 2026-10-17 at 8 threads (phase medians of 9 runs, load
averages 13 to 17), the preamble went from 13.4 to 5.7 ms on the 200-page
LaTeX document and from 21.2 to 10.1 ms on the 1001-page one, code generation
from 2.0 to 0.9 and 10.1 to 5.5 ms, and the PDF's stream copy, digest and
object stream from 12.2 to 8.3 ms on the 1001-page one. Median wall (15
interleaved runs, load averages 9 to 13) went from 69.6 to 62.4 ms and 244.3
to 229.7 ms at 8 threads on those two, 82.3 to 79.9 ms on the 394-page plain
TeX one and 13.9 to 13.0 ms on a one-line document; at one thread, 963.7 to
914.9 ms on the 1001-page document and within a few milliseconds on the rest.
PDF, DVI, `.aux`, stdout and stderr are byte-identical to the previous build at
1 and 8 threads. What stays serial at 8 threads, largest first on the 1001-page
document: lowering (about 100 ms), the SHA-256 over the file (about 8 ms), the
page planner's choice, code generation, process start and exit, and the file
write.

**Bibliographies.** `-X bibtex` runs an `ITERATE` or `REVERSE` of 256 or more
entries in chunks at once. An entry's own variables belong to it, so all one
entry's run can hand the next is the global variables, the stack and the
half-written output line. Each chunk after the first starts from the globals as
they stood before the walk, with an empty stack and line, and records which
globals it read before writing them. A chunk is kept only when the chunk before
it ended with an empty stack and line and no earlier chunk wrote a global it
read first; the first chunk that fails, and every one after it, runs again in
order. A walk whose first entry reads a global and then writes it (a running
maximum such as `longest.label`, alpha's previous label) stays in order from the
start. Database and `.aux` reading stay serial: the `.bib` of 50,000 entries
below reads in about 70 ms, and an `@string` changes the meaning of every
record after it. Before the parallel walk the database reader, the `.aux`
reader and the entry loading found repeated keys, citations and
cross-reference targets by scanning every earlier entry, which was quadratic in
the entry count; they now use hash sets. Measured 2026-10-17 (release build,
18-core aarch64 macOS, load averages 18 to 36, medians of 5 to 7 interleaved
runs) on a generated `.bib` with `plain.bst`, wall / user CPU in ms:

| entries, cited | before | 1 thread | 4 | 8 | 18 |
|---|---|---|---|---|---|
| 50,000, 25,000 | 11770 / 10110 | 2020 / 1900 | 870 / 2260 | 920 / 2370 | 660 / 2590 |
| 5,000, 2,000 | 270 / 250 | 210 / 200 | 140 / 280 | 120 / 310 | 120 / 360 |

`alpha.bst` at 50,000 went from 10730 ms to 2080 ms at one thread and 560 ms at
18. The `.bbl` and the warnings are byte-identical to the previous build at
every thread count, and match `bibtex` 0.99e for plain, alpha and unsrt.
abbrv still differs from `bibtex` where it did before, on a braced First part
such as `{van der}`, which `bibtex` abbreviates to `v.`.

**Cross-reference pass.** A LaTeX document that both labels and refers (or
cites and has a bibliography) is lowered twice per run: once by the pass that
collects its labels into the `.aux`, once to typeset against the `.aux`. The
pass reads only the source, so when an `.aux` from an earlier run is on disk
the two run concurrently, the typesetting against the `.aux` as it stands; when
the pass is done its `.aux` is written as before, and if that file would parse
differently from the old one the lowering is redone against the new one
(`latex::aux::update_then`). Package reports on stderr are held on both sides
and released in sequential order. It stays sequential at one thread, on a
first run, and for documents using Lua, `\rust` blocks or URL inputs.
Measured 2026-10-17 on a 1001-page article with 2000 labels and 2000
references (load averages 18 to 30, medians of 15 interleaved runs, default
thread count): an unchanged rerun went from 1146 ms to 686 ms wall, user CPU
2022 to 2087 ms, peak RSS about 205 to 335 MB; a run after an edit that
renumbers every label went from 1000 ms to 1123 ms wall (2005 to 2637 ms user),
the cost of a lowering thrown away. stdout, stderr, the PDF and the `.aux` are
byte-identical to the previous build and across thread counts.

**Iteration depth.** `\def\r{... \ifnum ... \r \fi}` is TeX's loop, and
`tex` recurses through it — one input-stack level per turn, giving up at
`[input stack size=10000]`. texrs lowers that shape to a backward jump, so its
stack use is constant and the bound is arithmetic rather than memory:

| iterations | 9,000 | 12,000 | 1,000,000 |
|---|---|---|---|
| `tex` | ok | **capacity exceeded** | capacity exceeded |
| texrs | ok | ok | ok |

### Measured, 2026-10-17 at `0d7158a91a`

Taken at commit `0d7158a91a`, release build, on an 18-core aarch64 macOS machine
that was under other load the whole time (1-minute load average between 15 and
26). Every figure is a median of interleaved runs — 9 per thread count for the
large document, 7 for the other two — with a warm kpse path cache and
`SOURCE_DATE_EPOCH=1`, on the default PDF path. The stage split comes from
timers placed between stages in a scratch build; they are not in the tree.

**Where one document's time goes**: a 1001-page LaTeX document from 4.3 MB of
source, wall time in ms.

| stage | 1 thread | 8 threads |
|---|---|---|
| process start and exit | 8 | 8 |
| argument parsing, reading the source | 8 | 9 |
| desugar, `.aux` update | 3 | 3 |
| preamble text (`latex::preamble_at`) | 100 | 102 |
| preamble preload | 3 | 3 |
| lowering (`Lowerer::lower`) | 190 | 194 |
| compile and VM run | 4 | 4 |
| font setup, references, contents | 1 | 1 |
| line breaking (`typeset::break_lines_measured`) | 394 | 408 |
| pagination (`typeset::paginate`) | 39 | 39 |
| page-start prescan (`typeset::page_starts`) | 24 | 24 |
| page drawing (parallel) | 419 | 79 |
| `pdf::document` | 48 | 51 |
| page count of the written PDF, twice | 43 | 45 |
| file write | 7 | 17 |
| drops and returns | 3 | 4 |
| **sum of stage medians** | **1294** | **990** |
| **median wall** | **1351** | **990** |
| median user CPU | 1250 | 1430 |

The kpse startup prefetch runs on a thread of its own and overlaps the stages
after it, so it has no row. At one thread the pages are drawn on the pool's one
worker while the calling thread waits, which is the drawing row.

Page drawing is the only stage that fans out. At 8 threads the other ~910 ms is
serial, and most of it is in three places:

- **Line breaking**, `typeset::break_lines_measured`: one loop over paragraphs
  carrying the colour, face and size stacks, the centring flag and the list
  depth. A paragraph's `fill` depends on nothing else, so it can be split the
  way drawing was — a serial pass recording that state per paragraph, then the
  paragraphs in parallel. About 40% of it is `linebreak::Hyphenator::points`.
  Since this measurement it is split that way and the patterns are a trie;
  see **Line breaking** above.
- **Lowering**, `lower::Lowerer::lower`: the expander, whose macro and register
  state carries from token to token. Serial by nature. Since this measurement
  the paragraphs it has finished are broken on the pool while it lowers the
  rest; see **Line breaking** above.
- **Preamble text**, `latex::preamble_at`: package loading (`load::preamble`,
  ~42 ms), the counter scan in `latex::support` (~22 ms), the class check in
  `numbering` (~22 ms) and the label seeds (~8 ms) — independent scans of the
  same source. Since this measurement the class check and both label seedings
  run on threads of their own beside the package load, and `support`'s four
  scans run side by side, so only the load and `support` are left in series.

Smaller: `pdf::document` spends most of its time in the SHA-256 of the whole
file for `/ID` (`pdf::file_identifier`), hashed in software on aarch64 at the
time (sha2's `asm` feature, which uses the ARMv8 SHA-2 instructions, is on
since, and the `/ID` has since become luatex's, which hashes no file at all); the page count was taken by inflating
the finished PDF, once in `main.rs` `write_pdf` and again in
`pdf_output_line` (since this measurement it is the writer's own count,
carried beside the bytes in `texrs::Pdf`, and nothing is inflated);
`paginate` is sequential by definition (since this measurement `page_starts`
reads each page in parallel and only applies the per-page effects in series,
and `paginate` gathers its page-break offers, penalties and leadings in
parallel; see **Page breaking** above).

**Scaling with `TEXRS_THREADS`**, median wall in ms:

| document | 1 | 2 | 4 | 8 | 16 |
|---|---|---|---|---|---|
| LaTeX, 1001 pages, 4.3 MB | 1351 | 1188 | 1097 | 990 | 933 |
| plain TeX, 394 pages, 2.0 MB | 425 | 383 | 343 | 310 | 302 |
| LaTeX, 200 pages, 0.9 MB | 271 | 240 | 221 | 211 | 209 |

## [0x0B] Releasing

```sh
bash scripts/bump.sh patch          # 0.4.0 -> 0.4.1, everywhere
bash scripts/bump.sh 1.2.3 --dry-run
```

The version lives in seven tracked files — the manifest, two hand-written docs
pages, the generated reference page, two man pages, and the IntelliJ plugin's
`gradle.properties`. (`tests/version_sync.rs` reads all seven; the count in this
sentence said six while the list beside it named seven.) Nothing in a build or a test run notices when they
disagree, which is how v0.1.0 once sat in the docs through v0.3.0. So
`tests/version_sync.rs` fails when any of them drifts, and `scripts/bump.sh` is
the one command that stamps all seven, regenerates the two that are derived from
the corpus rather than substituted, runs the full verify, then tags, pushes and
publishes.

## [0x0C] Documentation

- **Docs hub** — [menketechnologies.github.io/texrs](https://menketechnologies.github.io/texrs/) (`docs/index.html`)
- **Engineering report** — architecture, what lowering forces, parity posture, dependencies (`docs/report.html`)
- **Primitive reference** — every primitive texrs carries and where it happens (`docs/reference.html`, generated from `src/corpus.rs` with `cargo run --bin gen-docs`)
- **Known gaps** — the ledger, each entry pinned by a case the suite gates on (`BUGS.md`)
- **Roadmap** — the milestones, and what the stomach still lacks (`docs/ROADMAP.md`)

## [0xFF] Licence

MIT.
