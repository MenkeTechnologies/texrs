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

There is now a third piece, deliberately small: `--dvi` measures the text in a
real font, breaks it into lines at a measure, stacks them down a page and ships
DVI. It is not `tex.web`'s stomach — first-fit lines where TeX minimises badness
across the whole paragraph, no hyphenation, no glue, no boxes a document can
nest — and [0x06] says exactly what that costs. It is the difference between a
document that produces nothing and one that produces something imperfect.

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
-output-directory=DIR write files in this directory
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
texrs --dvi file           # typeset it: FILE.dvi, first-fit lines, no hyphenation
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
  natively, the preamble directives consumed, `\makeatletter` as the catcode
  change it is.
- `--dvi`: a page. Text measured in a real font (`.tfm`), first-fit lines at a
  measure, stacked at a leading, shipped as DVI that `dvitype` reads.
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

There is a stomach now, and it is the smallest honest one. `--dvi` measures
characters in a real `.tfm`, breaks a paragraph into lines with the first break
that fits, stacks the lines down a page at a fixed leading, and ships DVI a
driver will open. What it is NOT is `tex.web`'s stomach: TeX considers every
feasible sequence of breakpoints and minimises total badness (§813-§890), and
this takes the first fit, which is what every word processor before TeX did and
what TeX was written to improve on. No hyphenation, no glue stretching or
shrinking, no page breaking by penalties, no maths, no boxes a document can
nest. A paragraph set here and the same paragraph set by `tex` will not agree
line for line — see `docs/ROADMAP.md`.

**Some LaTeX, no Lua.** texrs carries the part of LaTeX that lives in the mouth
and the expander, as TeX rather than as Rust: `src/latex/prelude.tex` is a file
of `\newcommand`s compiled into the binary, and a document that writes
`\documentclass` or `\usepackage` is recognised as LaTeX and lowered against it.
`\newcommand`, `\renewcommand`, `\providecommand` and `\DeclareRobustCommand`
are dispatched natively; `\documentclass`, `\usepackage`, `\RequirePackage` and
the `\PassOptionsTo*` pair are consumed and produce nothing, because a package is
TeX that builds boxes and nothing here builds boxes — `--dvi` sets lines of text,
which is not the same thing and is not enough to run a package. `\makeatletter`
is a catcode change and works as one.

What that buys, and what it does not: a macro that would have drawn something
yields its text instead, and a document whose meaning IS its layout will not
survive it. `\directlua` is consumed rather than run — there is no Lua here — so
a document whose output depended on what its Lua computed is WRONG rather than
refused, which is the one failure mode worth knowing about before trusting this.

The number, run against the 241 `.tex` files of a real LaTeX/LuaLaTeX corpus
(Pandoc-generated books of 16,000 lines and up, fontspec, TikZ, `\directlua`,
`--include-in-header` fragments): **239 run to completion**, and say 74 MB of
text. The two that do not are fixtures for `#{` parameter text, a gap listed in
`BUGS.md` and `tests/known_gaps.txt`; they are written to be refused.

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
produce what its text says. With `--dvi` there is also a page, set in Computer
Modern at whatever measure was asked for — and a document that said
`\setmainfont` gets Computer Modern anyway, because fontspec is not loaded.
Boxes a document nests, TikZ pictures, colour and maths do not exist. A draft
reads correctly; a book being sold on its typography should still be set by an
engine with a real stomach, and `scripts/texrs-pdf` says the same thing where a
`pandoc --pdf-engine` build would meet it.

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
