//! `docs/reference.html`, rendered from the primitive corpus.
//!
//! The page and the language server read the SAME table (`src/corpus.rs`), so a
//! primitive cannot be documented on the site and unknown to the editor, or the
//! other way round. `tests/docs_generated.rs` fails when the committed page and
//! this renderer disagree, which is what stops the file being hand-edited back
//! into drift.
//!
//! Regenerate with:
//!
//! ```sh
//! cargo run --bin gen-docs
//! ```

use std::fmt::Write as _;

use crate::catcode::{Cat, CatTable};
use crate::corpus::{CHAPTERS, CORPUS};
use crate::lsp::Served;

/// The chrome above the generated chapters: head, header, scheme strip, title.
const HEAD: &str = "<!DOCTYPE html>
<html lang=\"en\">
<head>
  <meta charset=\"utf-8\">
  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">
  <meta name=\"color-scheme\" content=\"dark light\">
  <meta name=\"description\" content=\"texrs — Primitive reference. Every TeX primitive texrs carries: category codes, macro definition, expansion, conditionals, count registers and their arithmetic, with what each one lowers to. MIT licensed.\">
  <title>texrs &mdash; Primitive Reference</title>
  <link rel=\"preconnect\" href=\"https://fonts.googleapis.com\">
  <link rel=\"preconnect\" href=\"https://fonts.gstatic.com\" crossorigin>
  <link href=\"https://fonts.googleapis.com/css2?family=Orbitron:wght@400;600;700;900&family=Share+Tech+Mono&display=swap\" rel=\"stylesheet\">
  <link rel=\"stylesheet\" href=\"hud-static.css\">
  <link rel=\"stylesheet\" href=\"tutorial.css\">
  <style>
    .tutorial-main { max-width: 76rem; }
    .file-table { width:100%;border-collapse:collapse;margin:0.6rem 0;font-size:12px; }
    .file-table th { background:var(--bg-secondary);color:var(--cyan);font-family:'Orbitron',sans-serif;font-size:10px;font-weight:700;letter-spacing:1.2px;text-transform:uppercase;text-align:left;padding:7px 10px;border:1px solid var(--border); }
    .file-table td { padding:6px 10px;border:1px solid var(--border);color:var(--text-dim);vertical-align:middle; }
    .file-table tr:hover td { background:var(--bg-hover); }
    .file-table td:first-child { font-family:'Share Tech Mono',monospace;color:var(--accent-light);font-weight:600;white-space:nowrap; }
    .file-table code { font-size:11px;color:var(--accent-light);background:var(--bg-primary);padding:1px 4px;border-radius:2px; }
    .stat-grid { display:grid;grid-template-columns:repeat(auto-fill,minmax(14rem,1fr));gap:0.75rem;margin:1.2rem 0; }
    .stat-card { border:1px solid var(--border);border-top:3px solid var(--cyan);background:var(--bg-card);padding:1rem 1.2rem;border-radius:2px;text-align:center; }
    .stat-card .stat-val { font-family:'Orbitron',sans-serif;font-size:28px;font-weight:900;color:var(--cyan);line-height:1.1;text-shadow:0 0 20px var(--cyan-glow); }
    .stat-card .stat-val.accent { color:var(--accent);text-shadow:0 0 20px var(--accent-glow); }
    .stat-card .stat-label { font-family:'Orbitron',sans-serif;font-size:9px;font-weight:700;letter-spacing:2px;text-transform:uppercase;color:var(--text-muted);margin-top:0.5rem; }
    .feature-grid { display:grid;grid-template-columns:repeat(auto-fill,minmax(22rem,1fr));gap:0.65rem;margin:0.8rem 0; }
    .feature-card { border:1px solid var(--border);border-left:3px solid var(--cyan);background:var(--bg-card);padding:0.7rem 1rem;border-radius:2px; }
    .feature-card h4 { font-family:'Orbitron',sans-serif;font-size:10px;font-weight:700;letter-spacing:1.5px;text-transform:uppercase;color:var(--cyan);margin:0 0 0.3rem; }
    .feature-card p { margin:0;font-size:11px;color:var(--text-dim);line-height:1.55; }
    .feature-card code { font-size:10.5px;color:var(--accent-light);background:var(--bg-primary);padding:1px 4px;border-radius:2px; }
    .section-rule { border:none;border-top:1px dashed var(--border);margin:2rem 0; }
    .hub-scheme-strip { border-bottom:1px dashed var(--border);background:color-mix(in srgb, var(--bg-secondary) 85%, transparent);padding:0.55rem 1.5rem 0.65rem;position:relative; }
    .hub-scheme-strip-inner { max-width:76rem;margin:0 auto;display:flex;align-items:center;gap:0.85rem; }
    .hub-scheme-strip .hud-scheme-label { flex:0 0 auto;font-family:'Orbitron',sans-serif;font-size:9px;font-weight:700;letter-spacing:2px;text-transform:uppercase;color:var(--accent);text-align:left; }
    .hub-scheme-strip .scheme-grid { flex:1 1 auto;display:grid;grid-template-columns:repeat(5,minmax(0,1fr));gap:6px; }
    @media (max-width:720px){ .hub-scheme-strip-inner{flex-direction:column;align-items:stretch}.hub-scheme-strip .scheme-grid{grid-template-columns:repeat(2,minmax(0,1fr))} }
    .docs-build-line { margin:0.35rem 0 0;font-family:'Share Tech Mono',ui-monospace,monospace;font-size:11px;color:var(--text-dim);letter-spacing:0.03em;max-width:44rem;opacity:0.75; }
  </style>
</head>
<body>
  <div class=\"app tutorial-app\" id=\"docsApp\">
    <div class=\"crt-scanline\" id=\"crtH\" aria-hidden=\"true\"></div>
    <div class=\"crt-scanline-v\" id=\"crtV\" aria-hidden=\"true\"></div>

    <header class=\"tutorial-header\">
      <div class=\"tutorial-header-inner\">
        <div>
          <h1 class=\"tutorial-brand\">// TEXRS — PRIMITIVE REFERENCE</h1>
          <nav class=\"tutorial-crumbs\" aria-label=\"Breadcrumb\">
            <a href=\"index.html\">Docs</a>
            <span class=\"sep\">/</span>
            <a href=\"report.html\">Engineering Report</a>
            <span class=\"sep\">/</span>
            <span class=\"current\">Reference</span>
            <span class=\"sep\">/</span>
            <a href=\"https://github.com/MenkeTechnologies/texrs\" target=\"_blank\" rel=\"noopener noreferrer\">GitHub</a>
          </nav>
          <p class=\"docs-build-line\">texrs v__TEXRS_VERSION__ · TeX on fusevm · mouth → expander → command stream → bytecode → Cranelift JIT · MIT · in active development</p>
        </div>
        <div class=\"tutorial-toolbar\">
          <button type=\"button\" class=\"btn btn-secondary\" id=\"btnTheme\" title=\"Toggle light/dark\">Theme</button>
          <button type=\"button\" class=\"btn btn-secondary active\" id=\"btnCrt\" title=\"CRT scanline overlay\">CRT</button>
          <button type=\"button\" class=\"btn btn-secondary active\" id=\"btnNeon\" title=\"Neon border pulse\">Neon</button>
          <a class=\"btn btn-secondary\" href=\"index.html\">Docs</a>
          <a class=\"btn btn-secondary\" href=\"report.html\">Report</a>
          <a class=\"btn btn-secondary\" href=\"https://github.com/MenkeTechnologies/texrs\" target=\"_blank\" rel=\"noopener noreferrer\">GitHub</a>
        </div>
      </div>
    </header>

    <div class=\"hub-scheme-strip\">
      <div class=\"hub-scheme-strip-inner\">
        <span class=\"hud-scheme-label\">// Color scheme</span>
        <div class=\"scheme-grid\" id=\"hudSchemeGrid\"></div>
      </div>
    </div>

    <main class=\"tutorial-main\">
      <h2 class=\"tutorial-title\"><span class=\"step-hash\">&gt;_</span>PRIMITIVE REFERENCE</h2>
      <p class=\"tutorial-subtitle\">Every primitive texrs carries, what it does, and where it happens — at compile time while lowering, or at run time on the VM. A primitive not on this page is not implemented; <a href=\"https://github.com/MenkeTechnologies/texrs/blob/main/BUGS.md\">BUGS.md</a> says so explicitly for the ones that are commonly reached for.</p>
__TEXRS_STATS__

";

/// The chrome below the generated sections: the links and the closing tags.
/// The command-line tables above it are generated from `cli::USAGE`.
const FOOT: &str = "      <section class=\"tutorial-section\">
        <h2>Links</h2>
        <ul>
          <li><strong>Documentation</strong> — <a href=\"index.html\">index.html</a></li>
          <li><strong>Engineering report</strong> — <a href=\"report.html\">report.html</a></li>
          <li><strong>Source</strong> — <a href=\"https://github.com/MenkeTechnologies/texrs\">github.com/MenkeTechnologies/texrs</a></li>
        </ul>
      </section>
        </main>
  </div>

  <script src=\"hud-theme.js\"></script>
</body>
</html>
";

/// The full `docs/reference.html` page.
pub fn reference_html() -> String {
    format!(
        "{head}{cli}{foot}",
        head = HEAD
            .replace("__TEXRS_VERSION__", env!("CARGO_PKG_VERSION"))
            .replace("__TEXRS_STATS__", &stats(crate::cli::USAGE))
            + &chapters(&crate::lsp::served())
            + &category_codes()
            + &builtins()
            + &tiers_report()
            + &divergences(),
        cli = command_line(crate::cli::USAGE) + &environment(),
        foot = FOOT,
    )
}

/// One row of the option grammar: how it is spelled, and what it does.
#[derive(Debug, PartialEq, Eq)]
pub struct UsageRow {
    /// The spelling, exactly as `--help` prints it (`-jobname=NAME`, `-X tfm FILE.tfm [C]`).
    pub option: String,
    /// The `//` note that follows it, with the marker stripped.
    pub note: String,
}

/// One `── SECTION ──` of the option grammar and the rows under it.
#[derive(Debug, PartialEq, Eq)]
pub struct UsageSection {
    /// The rule's label, title-cased as `--help` prints it (`TEX OPTIONS`).
    pub title: String,
    /// The options in the order the grammar lists them.
    pub rows: Vec<UsageRow>,
}

/// Lift the option grammar out of `cli::USAGE`.
///
/// `--help`, the man page and this reference all have to name the same flags,
/// and the way that stops being true is three hand-maintained lists. So the
/// page is generated from the one the binary prints, and `tests/cli.rs` holds
/// the completion and the man page to it separately.
///
/// The grammar is three shapes: a section rule `  ── NAME ──…`, an option on
/// its own line whose note is indented under it, and an option with its note
/// inline after `//` — the `-X` block is written that way.
pub fn usage_sections(usage: &str) -> Vec<UsageSection> {
    let mut sections: Vec<UsageSection> = Vec::new();
    for line in usage.lines() {
        let trimmed = line.trim_end();
        if let Some(title) = section_title(trimmed) {
            sections.push(UsageSection {
                title,
                rows: Vec::new(),
            });
            continue;
        }
        let Some(current) = sections.last_mut() else {
            // The synopsis above the first rule is rendered separately.
            continue;
        };
        // A note indented under the option it describes.
        if let Some(note) = trimmed.trim_start().strip_prefix("// ") {
            if trimmed.starts_with("          ") {
                if let Some(row) = current.rows.last_mut() {
                    if row.note.is_empty() {
                        row.note = note.trim().to_string();
                    }
                }
                continue;
            }
        }
        if !trimmed.starts_with("  ") || trimmed.trim().is_empty() {
            continue;
        }
        let body = trimmed.trim();
        // `-X tfm FILE.tfm [C]      // Read a font's metrics` — one line, both halves.
        let (option, note) = match body.split_once("//") {
            Some((opt, note)) => (opt.trim(), note.trim()),
            None => (body, ""),
        };
        current.rows.push(UsageRow {
            option: option.to_string(),
            note: note.to_string(),
        });
    }
    sections.retain(|s| !s.rows.is_empty());
    sections
}

/// `  ── TEX OPTIONS ─────` → `TEX OPTIONS`.
fn section_title(line: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix("──")?;
    let title = rest.trim_matches(|c: char| c == '─' || c.is_whitespace());
    (!title.is_empty()).then(|| title.to_string())
}

/// The synopsis lines above the first section rule — the three invocation
/// forms plus the bare-prompt one.
fn synopsis(usage: &str) -> Vec<String> {
    let lines: Vec<&str> = usage
        .lines()
        .take_while(|l| section_title(l.trim_end()).is_none())
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .collect();
    // Dedent by the shallowest line rather than trimming each one: the forms
    // are column-aligned under `USAGE:` and that alignment is the point.
    let indent = lines
        .iter()
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    lines.iter().map(|l| l[indent..].to_string()).collect()
}

/// The command-line half of the page: the synopsis, then one `<section>` per
/// group of the option grammar.
fn command_line(usage: &str) -> String {
    let mut out = String::new();
    out.push_str("      <section class=\"tutorial-section\" id=\"cli-invocation\">\n        <h2>Invocation</h2>\n        <pre>");
    for line in synopsis(usage) {
        let _ = writeln!(out, "{}", escape(&line));
    }
    out.push_str("</pre>\n      </section>\n\n");

    for section in usage_sections(usage) {
        let _ = write!(
            out,
            "      <section class=\"tutorial-section\" id=\"cli-{anchor}\">\n        <h2>{title}</h2>\n        <table class=\"file-table\">\n          <thead><tr><th>Option</th><th>What it does</th></tr></thead>\n          <tbody>\n",
            anchor = anchor(&section.title),
            title = escape(&section.title),
        );
        for row in &section.rows {
            let _ = writeln!(
                out,
                "            <tr><td><code>{option}</code></td><td>{note}</td></tr>",
                option = escape(&row.option),
                note = escape(&row.note),
            );
        }
        out.push_str("          </tbody>\n        </table>\n      </section>\n\n");
    }
    out
}

/// The four figures the banner prints, as the page's own header strip: they are
/// counted from the corpus and the compiler rather than typed, so a chapter
/// added to `CHAPTERS` moves the number here too.
fn stats(usage: &str) -> String {
    let options: usize = usage_sections(usage).iter().map(|s| s.rows.len()).sum();
    let cards = [
        (CORPUS.len().to_string(), "Primitives"),
        (CHAPTERS.len().to_string(), "Chapters"),
        (crate::compiler::COUNT_SLOTS.to_string(), "Count registers"),
        (options.to_string(), "Command-line options"),
    ];
    let mut out = String::from("        <div class=\"stat-grid\">\n");
    for (value, label) in cards {
        let _ = write!(
            out,
            "          <div class=\"stat-card\">\n            <div class=\"stat-val\">{value}</div>\n            <div class=\"stat-label\">{label}</div>\n          </div>\n",
        );
    }
    out.push_str("        </div>");
    out
}

/// One `<section>` per chapter in `CHAPTERS` order, each holding one table row
/// per primitive: the name, what it does, and its syntax with an example.
///
/// The rows are the LANGUAGE SERVER's answers, not the corpus's rows —
/// `lsp::served()` reads them out of the completion and hover responses an
/// editor receives. A primitive the server stops resolving therefore leaves the
/// manual too, rather than the two drifting apart while both look right.
fn chapters(served: &[Served]) -> String {
    let mut out = String::new();
    for chapter in CHAPTERS {
        let entries: Vec<&Served> = served.iter().filter(|e| e.chapter == *chapter).collect();
        if entries.is_empty() {
            continue;
        }
        let _ = write!(
            out,
            "      <section class=\"tutorial-section\" id=\"ch-{anchor}\">\n        <h2>{chapter}</h2>\n        <table class=\"file-table\">\n          <thead><tr><th>Primitive</th><th>What it does</th><th>Syntax and example</th></tr></thead>\n          <tbody>\n",
            anchor = anchor(chapter),
        );
        for entry in entries {
            let _ = writeln!(
                out,
                "            <tr><td><code>{name}</code></td><td>{doc}</td><td><pre>{example}</pre></td></tr>",
                name = escape(&entry.name),
                doc = markdown_code(&entry.doc),
                example = escape(&entry.example),
            );
        }
        out.push_str("          </tbody>\n        </table>\n      </section>\n\n");
    }
    out
}

/// `Category codes` -> `category-codes`, for a stable in-page anchor.
fn anchor(chapter: &str) -> String {
    chapter
        .chars()
        .map(|c| match c.is_ascii_alphanumeric() {
            true => c.to_ascii_lowercase(),
            false => '-',
        })
        .collect()
}

/// The corpus doc lines are prose with `backticked` code spans; render those as
/// `<code>` and escape everything else.
fn markdown_code(doc: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    for part in doc.split('`') {
        out.push_str(&match in_code {
            true => format!("<code>{}</code>", escape(part)),
            false => escape(part),
        });
        in_code = !in_code;
    }
    out
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// `editors/emacs/texrs-stdlib.el`: the primitive names and their one-line
/// documentation, as elisp.
///
/// Generated rather than written so the Emacs mode's completion list and its
/// eldoc strings are the corpus the engine answers from — a hand-kept copy
/// drifts the moment a primitive is added, and drifts silently, since nothing
/// in Emacs would notice.
pub fn emacs_stdlib_el() -> String {
    let mut out = String::new();
    out.push_str(
        ";;; texrs-stdlib.el --- Primitive table for texrs-mode -*- lexical-binding: t; -*-\n\
         \n\
         ;; This file is GENERATED by `cargo run --bin gen-emacs-stdlib' from\n\
         ;; texrs's own primitive corpus (src/corpus.rs).  Do not edit it by hand:\n\
         ;; `tests/emacs_stdlib.rs' fails when it is not what the generator prints.\n\
         \n\
         ;;; Commentary:\n\
         \n\
         ;; The names texrs resolves, with the one-line documentation the\n\
         ;; language server hovers with.  A primitive is here only if the engine\n\
         ;; dispatches it, so completion cannot offer something that would fail.\n\
         \n\
         ;;; Code:\n\n",
    );

    out.push_str("(defconst texrs-primitive-names\n  '(");
    for (i, (name, ..)) in crate::corpus::CORPUS.iter().enumerate() {
        if i > 0 {
            out.push_str("\n    ");
        }
        out.push_str(&format!("{:?}", name));
    }
    out.push_str(")\n  \"Every primitive texrs resolves, for completion at point.\")\n\n");

    out.push_str("(defconst texrs-stdlib--docs\n  (let ((table (make-hash-table :test 'equal)))\n");
    for (name, chapter, doc, _example) in crate::corpus::CORPUS {
        // One line each: eldoc shows a line, and a doc with a newline in it
        // would push the rest of the echo area off screen.
        let summary = doc.split(". ").next().unwrap_or(doc).replace('\n', " ");
        let summary = summary.trim_end_matches('.');
        out.push_str(&format!(
            "    (puthash {:?} {:?} table)\n",
            name,
            format!("{name}  —  {summary}. [{chapter}]"),
        ));
    }
    out.push_str(
        "    table)\n  \"Primitive name to the line eldoc shows for it.\")\n\n\
         (defun texrs-stdlib-signature (name)\n  \
         \"Return the eldoc line for primitive NAME, or nil.\"\n  \
         (and (stringp name) (gethash name texrs-stdlib--docs)))\n\n\
         (provide 'texrs-stdlib)\n;;; texrs-stdlib.el ends here\n",
    );
    out
}

/// INITEX's category table, one row per category.
///
/// Built by asking [`crate::catcode::CatTable::new`] what it holds rather than
/// by transcribing `tex.web` §232, so the page states what this engine starts
/// from — which is the whole point of the table for a reader whose document
/// sets its own catcodes.
fn category_codes() -> String {
    let table = CatTable::new();
    let mut out = String::from(
        "      <section class=\"tutorial-section\" id=\"tbl-catcodes\">\n        <h2>Category codes</h2>\n        <p>The table INITEX starts from, which is what texrs starts from: no format is loaded. <code>{</code>, <code>}</code>, <code>$</code>, <code>&amp;</code>, <code>#</code>, <code>^</code>, <code>_</code> and <code>~</code> get their familiar meanings from plain.tex, not from the engine.</p>\n        <table class=\"file-table\">\n          <thead><tr><th>Code</th><th>Category</th><th>Characters INITEX puts here</th></tr></thead>\n          <tbody>\n",
    );
    for cat in Cat::ALL {
        let chars = initex_members(&table, cat);
        let _ = writeln!(
            out,
            "            <tr><td><code>{code}</code></td><td>{name}</td><td>{chars}</td></tr>",
            code = cat as u8,
            name = escape(cat.name()),
            chars = chars,
        );
    }
    out.push_str("          </tbody>\n        </table>\n      </section>\n\n");
    out
}

/// Which characters INITEX assigns to one category, as a readable cell:
/// contiguous runs collapse (`A`–`Z`), the ones with no printable form are
/// named, and a category nothing starts in says so rather than showing blank.
fn initex_members(table: &CatTable, cat: Cat) -> String {
    let members: Vec<u8> = (0u8..=255)
        .filter(|c| table.get(*c as char) == cat)
        .collect();
    if members.is_empty() {
        return "&mdash;".to_string();
    }
    // `other` is everything left over; listing 200-odd codes helps nobody.
    if members.len() > 64 {
        return format!("every other code ({} of them)", members.len());
    }
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < members.len() {
        let start = members[i];
        let mut end = start;
        while i + 1 < members.len() && members[i + 1] == end + 1 {
            i += 1;
            end = members[i];
        }
        parts.push(if start == end {
            named(start)
        } else {
            format!("{}&ndash;{}", named(start), named(end))
        });
        i += 1;
    }
    parts.join(", ")
}

/// A byte as a reader recognises it: printable ones as themselves in a
/// `<code>`, the rest by the name TeX uses for them.
fn named(c: u8) -> String {
    match c {
        0 => "NUL".to_string(),
        b'\t' => "tab".to_string(),
        b'\n' => "line feed".to_string(),
        b'\r' => "carriage return".to_string(),
        b' ' => "space".to_string(),
        127 => "DEL".to_string(),
        c if c.is_ascii_graphic() => format!("<code>{}</code>", escape(&(c as char).to_string())),
        c => format!("code {c}"),
    }
}

/// The divergence ledger, read from the file the parity gate reads.
///
/// `tests/known_gaps.txt` is the baseline `cargo test --test differential`
/// holds the engine to: a case that diverges and is not listed fails the
/// build, and so does a listed case that has started passing. Generating this
/// section from that file is what stops the manual claiming parity the gate
/// does not.
fn divergences() -> String {
    let mut out = String::from(
        "      <section class=\"tutorial-section\" id=\"tbl-divergences\">\n        <h2>Divergences from tex</h2>\n        <p>Where this engine and real <code>tex</code> disagree, taken from <code>tests/known_gaps.txt</code> &mdash; the baseline the differential gate enforces in both directions, so this list can neither grow silently nor go stale.</p>\n        <table class=\"file-table\">\n          <thead><tr><th>Case</th><th>What differs</th></tr></thead>\n          <tbody>\n",
    );
    for (case, reason) in known_gaps(include_str!("../tests/known_gaps.txt")) {
        let _ = writeln!(
            out,
            "            <tr><td><code>{case}</code></td><td>{reason}</td></tr>",
            case = escape(&case),
            reason = escape(&reason),
        );
    }
    out.push_str("          </tbody>\n        </table>\n      </section>\n\n");
    out
}

/// Parse `tests/known_gaps.txt`: a case name at column zero, then its reason,
/// which may start on that line or on the `#`-prefixed lines under it.
pub fn known_gaps(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') {
            // A continuation of the entry above, or the file's own header.
            if let Some(last) = out.last_mut() {
                let note = line.trim_start_matches('#').trim();
                if !note.is_empty() {
                    if !last.1.is_empty() {
                        last.1.push(' ');
                    }
                    last.1.push_str(note);
                }
            }
            continue;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        let (case, rest) = match trimmed.split_once(char::is_whitespace) {
            Some((c, r)) => (c, r.trim()),
            None => (trimmed, ""),
        };
        out.push((case.to_string(), rest.to_string()));
    }
    out
}

/// The environment the binary reads.
///
/// Unlike the option grammar there is no one table in the source to lift these
/// from — they are `env::var` calls in the modules that care — so the list is
/// written here and `tests/docs_reference_sections.rs` fails if a `TEXRS_*`
/// name appears in `src/` and not on the page.
const ENVIRONMENT: &[(&str, &str)] = &[
    (
        "TEXRS_CACHE",
        "Set to <code>0</code>, <code>false</code> or <code>no</code> to turn the bytecode cache off for a run; set to a path to put it somewhere else. A disabled cache is how you work around a cache you suspect.",
    ),
    (
        "TEXRS_PARALLEL",
        "Set to <code>1</code> to lex a document on several threads. Off by default: the mouth is 22% of the time on the documents measured, so the win did not pay for the coordination.",
    ),
    (
        "TEXRS_PRELEX_STATS",
        "Set to anything to print the pre-lexer's hit and miss counts when the run ends.",
    ),
    (
        "TEXRS_STATICLIB",
        "The <code>libtexrs.a</code> that <code>--aot</code> links against, when the installed copy is not the one you want.",
    ),
];

/// The environment table.
fn environment() -> String {
    let mut out = String::from(
        "      <section class=\"tutorial-section\" id=\"tbl-environment\">\n        <h2>Environment</h2>\n        <table class=\"file-table\">\n          <thead><tr><th>Variable</th><th>What it does</th></tr></thead>\n          <tbody>\n",
    );
    for (name, note) in ENVIRONMENT {
        let _ = writeln!(
            out,
            "            <tr><td><code>{name}</code></td><td>{note}</td></tr>",
        );
    }
    out.push_str("          </tbody>\n        </table>\n      </section>\n\n");
    out
}

/// One builtin the compiler emits calls to: its constant, its id, and what it
/// does, lifted from `compiler::ops`.
pub fn builtin_ops(source: &str) -> Vec<(String, u16, String)> {
    let mut out = Vec::new();
    let mut in_ops = false;
    let mut doc = String::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("pub mod ops") {
            in_ops = true;
            continue;
        }
        if !in_ops {
            continue;
        }
        if trimmed == "}" {
            break;
        }
        if let Some(note) = trimmed.strip_prefix("/// ") {
            if !doc.is_empty() {
                doc.push(' ');
            }
            doc.push_str(note);
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("pub const ") {
            if let Some((name, tail)) = rest.split_once(':') {
                if let Some(value) = tail.split('=').nth(1) {
                    if let Ok(id) = value.trim().trim_end_matches(';').parse::<u16>() {
                        out.push((name.trim().to_string(), id, std::mem::take(&mut doc)));
                        continue;
                    }
                }
            }
        }
        if !trimmed.starts_with("//") {
            doc.clear();
        }
    }
    out.sort_by_key(|(_, id, _)| *id);
    out
}

/// The builtin-call table.
///
/// These ids are a wire format and the page says so: the bytecode cache stores
/// compiled chunks on disk and `--aot` serializes one into the object it emits,
/// and both call builtins BY NUMBER. Renumbering an op still compiles; it makes
/// every cached chunk and every already-built binary call the wrong function.
fn builtins() -> String {
    let mut out = String::from(
        "      <section class=\"tutorial-section\" id=\"tbl-builtins\">\n        <h2>Builtin calls</h2>\n        <p>What the VM cannot do natively is a builtin call, emitted by the lowerer and dispatched by id. The numbers are a <strong>wire format</strong>: the bytecode cache keeps compiled chunks on disk and <code>--aot</code> serializes one into the executable it writes, and both call these BY NUMBER &mdash; so renumbering an op does not fail to build, it makes every cached chunk and every already-built binary call the wrong function.</p>\n        <table class=\"file-table\">\n          <thead><tr><th>Constant</th><th>Id</th><th>What it does</th></tr></thead>\n          <tbody>\n",
    );
    for (name, id, note) in builtin_ops(include_str!("compiler.rs")) {
        let _ = writeln!(
            out,
            "            <tr><td><code>{name}</code></td><td><code>{id}</code></td><td>{note}</td></tr>",
            name = escape(&name),
            note = escape(&note),
        );
    }
    out.push_str("          </tbody>\n        </table>\n      </section>\n\n");
    out
}

/// The lines `--tiers` prints, and what each one answers.
///
/// The report asks fusevm's own predicates rather than inferring anything, so
/// the vocabulary is worth stating: "eligible" and "compiled" are different
/// questions, and the last line is the only one that answers "did this document
/// reach native code".
const TIERS_REPORT: &[(&str, &str)] = &[
    ("ops", "How many bytecode ops the document lowered to, prologue included."),
    (
        "block-JIT eligible",
        "Whether a region of this chunk is a shape the block tier will take at all. Eligibility is not compilation.",
    ),
    (
        "block-JIT compiled",
        "Whether the block tier actually compiled one, asked of fusevm's cache rather than assumed from eligibility.",
    ),
    (
        "largest eligible region",
        "The widest run of ops the block tier would take, as <code>start..end</code> with its length, or <code>none</code>.",
    ),
    (
        "loops",
        "Every loop header the lowerer emitted, and whether the tracing tier compiled a trace for it. <code>none</code> means the document has no backward branch, which is the usual reason nothing reaches native code.",
    ),
    (
        "block-ineligible ops",
        "The ops that disqualified a region, counted by kind. A pair of <code>CallBuiltin</code>s is enough, which is why the smallest document reaches no tier.",
    ),
    (
        "reaches native code",
        "The one line that answers the question the flag was run to ask.",
    ),
];

/// The `--tiers` vocabulary table.
fn tiers_report() -> String {
    let mut out = String::from(
        "      <section class=\"tutorial-section\" id=\"tbl-tiers\">\n        <h2>The --tiers report</h2>\n        <p><code>texrs --tiers FILE</code> runs a document and then queries fusevm's own eligibility and cache predicates, so the answer comes from the compiler that would have done the work rather than from an assumption about it.</p>\n        <table class=\"file-table\">\n          <thead><tr><th>Line</th><th>What it answers</th></tr></thead>\n          <tbody>\n",
    );
    for (line, note) in TIERS_REPORT {
        let _ = writeln!(
            out,
            "            <tr><td><code>{line}</code></td><td>{note}</td></tr>",
        );
    }
    out.push_str("          </tbody>\n        </table>\n      </section>\n\n");
    out
}
