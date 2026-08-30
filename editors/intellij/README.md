# texrs JetBrains Plugin

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![IDE](https://img.shields.io/badge/IDE-2025.2%2B-orange.svg)](https://plugins.jetbrains.com/)
[![JDK](https://img.shields.io/badge/JDK-17-blue.svg)](https://adoptium.net/)
[![Plugin SDK](https://img.shields.io/badge/IntelliJ%20Platform%20Gradle-2.16-purple.svg)](https://plugins.jetbrains.com/docs/intellij/tools-intellij-platform-gradle-plugin.html)

IDE support for [`texrs`](https://github.com/MenkeTechnologies/texrs) — a TeX
engine that compiles Knuth's mouth and expander to
[`fusevm`](https://github.com/MenkeTechnologies/fusevm) bytecode rather than
interpreting it.

## What it does

**Highlighting by category code.** TeX has no fixed grammar: a document decides
what every character means, and `\catcode`\@=11` is a legal thing to write. The
lexer here classifies the way the engine does — the sixteen classes of
`tex.web` §207, the same ones `src/catcode.rs` implements — so comments,
control words, control symbols, group braces, math shift, alignment tabs,
parameters, super/subscripts and active characters each get their own colour.
Control sequences texrs implements are told apart from the ones a document
defines, which is the distinction a reader actually wants.

**Run configurations** over the `texrs` CLI: run a document, or stop early with
`--dump-tokens` (after the mouth) or `--disasm` (after lowering), and force a
cold compile with `--no-cache`. The gutter and the context menu both offer it
on any `.tex` file.

**The editing furniture:** `%` line comments, `{`/`}` brace matching,
spell-checking that reads prose and skips markup, `File → New → TeX Document`
templates that set the category codes INITEX leaves ordinary, and a colour
settings page with a slot per category.

**LSP integration** over `texrs --lsp`: completion over the primitives the
engine implements, triggered by `\` since that is what opens a control
sequence, hover documentation, and diagnostics produced by the real mouth and
expander rather than by a re-implementation of them that would drift. Only what
the server advertises is opted into — an opt-in for a capability it does not
have is a feature that silently does nothing, which is worse than a missing one
because it looks present.

**Debugger** over `texrs --dap`: line breakpoints, step over/into/out, pause,
frames with their scopes and variables, expression evaluation, and
run-to-cursor. The adapter is stdio-only, so its stdout carries protocol frames
and nothing else; the document's own `\message` output reaches the console as
DAP `output` events.

## Build

```sh
cd editors/intellij
./gradlew test          # the lexer, commenter and settings tests
./gradlew buildPlugin   # build/distributions/texrs-intellij-<version>.zip
./gradlew runIde        # a sandbox IDE with the plugin loaded
```

A paid JetBrains IDE (2025.2+) is required, because the platform LSP API is not
in the Community editions — the same requirement the sibling plugins carry.

JDK 17 is required (the Kotlin version the platform plugin pins cannot parse
newer JDK version strings). Set `org.gradle.java.home` in your **user-level**
`~/.gradle/gradle.properties` if `java -version` reports something else — not
in this repo, where an absolute path would break every other machine.

## Install

`Settings → Plugins → ⚙ → Install Plugin from Disk…` and pick the zip from
`build/distributions/`. Point `Settings → Tools → texrs` at a `texrs` binary if
it is not on `$PATH`.
