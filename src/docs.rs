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

use crate::corpus::{Entry, CHAPTERS, CORPUS};

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

    
      <h2 class=\"tutorial-title\"><span class=\"step-hash\">&gt;_</span>PRIMITIVE REFERENCE</h2>
      <p class=\"tutorial-subtitle\">Every primitive texrs carries, what it does, and where it happens — at compile time while lowering, or at run time on the VM. A primitive not on this page is not implemented; <a href=\"https://github.com/MenkeTechnologies/texrs/blob/main/BUGS.md\">BUGS.md</a> says so explicitly for the ones that are commonly reached for.</p>

";

/// The chrome below them: the command-line table, the links, and the closing
/// tags. Not corpus-driven — the flags are the binary's, not the language's.
const FOOT: &str = "      <section class=\"tutorial-section\">
        <h2>Command line</h2>
        <table class=\"file-table\">
          <thead><tr><th>Invocation</th><th>What it does</th></tr></thead>
          <tbody>
            <tr><td><code>texrs FILE.tex</code></td><td>Run the file and print its <code>\\message</code> stream as <code>(./FILE.tex … )</code>.</td></tr>
            <tr><td><code>texrs --dump-tokens FILE</code></td><td>Print the mouth's token stream and exit. No expansion happens.</td></tr>
            <tr><td><code>texrs --disasm FILE</code></td><td>Print the lowered fusevm bytecode and exit.</td></tr>\n            <tr><td><code>texrs --tiers FILE</code></td><td>Run it, then report which fusevm tier took its bytecode, asked of fusevm's own eligibility and cache predicates.</td></tr>\n            <tr><td><code>texrs --lsp</code></td><td>Speak the Language Server Protocol over stdio: completion and hover from this page's own corpus, diagnostics from the engine's lowerer.</td></tr>\n            <tr><td><code>texrs --dap</code></td><td>Speak the Debug Adapter Protocol over stdio: source-line breakpoints, stepping, and the count registers as the variables scope.</td></tr>
            <tr><td><code>texrs --no-cache FILE</code></td><td>Compile this run rather than reading the bytecode cache. The result is identical either way.</td></tr>
            <tr><td><code>texrs --cache-stats</code></td><td>Print where the bytecode cache is, how many documents it holds and how large it is.</td></tr>
            <tr><td><code>texrs --cache-clear</code></td><td>Delete the bytecode cache. Every document compiles again on its next run.</td></tr>
            <tr><td><code>texrs --help</code></td><td>Print the option grammar.</td></tr>
            <tr><td><code>texrs --version</code></td><td>Print the version banner.</td></tr>
          </tbody>
        </table>
        <p>Errors go to stderr as <code>! &lt;reason&gt;.</code>, the way tex writes them, and the exit status is 1. <code>TEXRS_CACHE=0</code> (or <code>false</code>, or <code>no</code>) turns the bytecode cache off for a run without deleting it.</p>
      </section>

      <section class=\"tutorial-section\">
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
        "{head}{body}{foot}",
        head = HEAD.replace("__TEXRS_VERSION__", env!("CARGO_PKG_VERSION")),
        body = chapters(CORPUS),
        foot = FOOT,
    )
}

/// One `<section>` per chapter in `CHAPTERS` order, each holding one table row
/// per primitive: the name, what it does, and its syntax with an example.
fn chapters(corpus: &[Entry]) -> String {
    let mut out = String::new();
    for chapter in CHAPTERS {
        let entries: Vec<&Entry> = corpus.iter().filter(|(_, c, ..)| c == chapter).collect();
        if entries.is_empty() {
            continue;
        }
        let _ = write!(
            out,
            "      <section class=\"tutorial-section\" id=\"ch-{anchor}\">\n        <h2>{chapter}</h2>\n        <table class=\"file-table\">\n          <thead><tr><th>Primitive</th><th>What it does</th><th>Syntax and example</th></tr></thead>\n          <tbody>\n",
            anchor = anchor(chapter),
        );
        for (name, _chapter, doc, example) in entries {
            let _ = writeln!(
                out,
                "            <tr><td><code>{name}</code></td><td>{doc}</td><td><pre>{example}</pre></td></tr>",
                name = escape(name),
                doc = markdown_code(doc),
                example = escape(example),
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
