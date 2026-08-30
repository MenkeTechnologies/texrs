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
- [\[0x0B\] Documentation](#0x0b-documentation)
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
texrs file.tex             # run it, print the \message stream
texrs a.tex b.tex c.tex    # compile a batch, one document per core
texrs --jobs=N file...     # bound that to N workers
texrs --repl               # interactive prompt; state carries across lines
texrs --lsp                # Language Server Protocol over stdio, for an editor
texrs --dap                # Debug Adapter Protocol over stdio: breakpoints, stepping
texrs --dump-tokens file   # the mouth's token stream, no expansion
texrs --disasm file        # the lowered fusevm bytecode
texrs --tiers file         # run it, then say which fusevm tier took it
texrs --aot file           # compile it to a standalone native executable
texrs --no-cache file      # compile this run instead of reading the cache
texrs --cache-stats        # what the bytecode cache holds, and where
texrs --cache-clear        # delete it; it holds only what can be recompiled
texrs --help
texrs --version
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

No boxes, no glue, no paragraph breaking, no fonts, no DVI. This is not a
typesetter yet — see `docs/ROADMAP.md`.

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
bash scripts/parity.sh          # the committed corpus
bash scripts/parity.sh case.tex # one ad-hoc case
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
bash scripts/fuzz_parity.sh -n 500        # random programs, both engines, diffed
bash scripts/fuzz_parity.sh -1 case.tex   # one file, verbose
cargo +nightly fuzz run lower -- -timeout=10
```

`scripts/fuzz_parity.sh` generates seeded random programs confined to the
implemented subset, runs them under both engines in parallel, and REDUCES
whatever diverges — dropping statements for as long as the divergence survives,
and refusing a reduction that changes the divergence into a different one — so
what it hands back is small enough to commit to `tests/cases`. The same seed
generates the same corpus on any machine.

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

## [0x0B] Documentation

- **Docs hub** — [menketechnologies.github.io/texrs](https://menketechnologies.github.io/texrs/) (`docs/index.html`)
- **Engineering report** — architecture, what lowering forces, parity posture, dependencies (`docs/report.html`)
- **Primitive reference** — every primitive texrs carries and where it happens (`docs/reference.html`, generated from `src/corpus.rs` with `cargo run --bin gen-docs`)
- **Known gaps** — the ledger, each entry pinned by a case the suite gates on (`BUGS.md`)
- **Roadmap** — what the stomach would take (`docs/ROADMAP.md`)

## [0xFF] Licence

MIT.
