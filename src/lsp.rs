//! Language Server Protocol over stdio (`texrs --lsp`).
//!
//! Self-contained and read-only: diagnostics come from the same lowerer the
//! runtime uses, so a document that the editor shows as clean is a document
//! `texrs` will run; completion and hover draw on the reference corpus in
//! `src/corpus.rs`, so the editor and `docs/reference.html` cannot disagree
//! about what a primitive does. No output reaches the terminal — JSON-RPC on
//! stdio only. Structure follows the sibling frontends' `lsp.rs`.

use std::collections::HashMap;

use lsp_server::{Connection, ErrorCode, ExtractError, Message, Request, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _,
    PublishDiagnostics,
};
use lsp_types::request::{Completion, HoverRequest, Request as _};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, Hover, HoverContents, HoverParams, HoverProviderCapability,
    MarkupContent, MarkupKind, Position, PublishDiagnosticsParams, Range, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions, Uri,
};

use crate::corpus::{Entry, CORPUS};

/// Open document text keyed by URI, kept current from the sync notifications so
/// hover can look up the control sequence under the cursor.
type Docs = HashMap<String, String>;

/// Entry point for `texrs --lsp`.
pub fn run() -> Result<(), String> {
    spawn_orphan_guard();
    let (conn, io_threads) = Connection::stdio();
    let (init_id, _params) = conn
        .initialize_start()
        .map_err(|e| format!("lsp initialize: {e}"))?;
    let init_result = serde_json::json!({
        "capabilities": server_capabilities(),
        "serverInfo": { "name": "texrs", "version": env!("CARGO_PKG_VERSION") },
    });
    conn.sender
        .send(Response::new_ok(init_id, init_result).into())
        .map_err(|e| format!("lsp send: {e}"))?;

    let mut docs: Docs = HashMap::new();
    for msg in &conn.receiver {
        match msg {
            Message::Request(req) => {
                if conn
                    .handle_shutdown(&req)
                    .map_err(|e| format!("lsp shutdown: {e}"))?
                {
                    break;
                }
                dispatch_request(&conn, &docs, req);
            }
            Message::Notification(not) => dispatch_notification(&conn, &mut docs, not),
            Message::Response(_) => {}
        }
    }
    drop(conn);
    io_threads.join().map_err(|_| "lsp io join".to_string())?;
    Ok(())
}

fn server_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Options(
            TextDocumentSyncOptions {
                open_close: Some(true),
                change: Some(TextDocumentSyncKind::FULL),
                ..Default::default()
            },
        )),
        completion_provider: Some(CompletionOptions {
            resolve_provider: Some(false),
            // A control sequence starts with the escape character, so that is
            // what should open the list without the user asking for it.
            trigger_characters: Some(vec!["\\".to_string()]),
            ..Default::default()
        }),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        ..Default::default()
    }
}

fn handle<P, R>(conn: &Connection, req: Request, f: impl FnOnce(P) -> R)
where
    P: serde::de::DeserializeOwned,
    R: serde::Serialize,
{
    let method = req.method.clone();
    let id = req.id.clone();
    match req.extract::<P>(&method) {
        Ok((id, params)) => {
            let value = serde_json::to_value(f(params)).unwrap_or(serde_json::Value::Null);
            let _ = conn.sender.send(Response::new_ok(id, value).into());
        }
        Err(ExtractError::JsonError { error, .. }) => {
            let _ = conn.sender.send(
                Response::new_err(id, ErrorCode::InvalidParams as i32, error.to_string()).into(),
            );
        }
        Err(ExtractError::MethodMismatch(_)) => unreachable!("method matched before extract"),
    }
}

fn dispatch_request(conn: &Connection, docs: &Docs, req: Request) {
    match req.method.as_str() {
        Completion::METHOD => handle(conn, req, |_p: CompletionParams| {
            CompletionResponse::Array(completion_items())
        }),
        HoverRequest::METHOD => handle(conn, req, |p: HoverParams| {
            let pos = p.text_document_position_params.position;
            let uri = p.text_document_position_params.text_document.uri;
            let text = docs.get(uri.as_str()).map(String::as_str).unwrap_or("");
            Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: hover_markdown(text, pos.line, pos.character),
                }),
                range: None,
            }
        }),
        _ => {
            let _ = conn.sender.send(
                Response::new_err(req.id, ErrorCode::MethodNotFound as i32, "unhandled".into())
                    .into(),
            );
        }
    }
}

fn dispatch_notification(conn: &Connection, docs: &mut Docs, not: lsp_server::Notification) {
    match not.method.as_str() {
        DidOpenTextDocument::METHOD => {
            if let Ok(p) = serde_json::from_value::<DidOpenTextDocumentParams>(not.params) {
                let uri = p.text_document.uri;
                docs.insert(uri.as_str().to_string(), p.text_document.text.clone());
                publish_diagnostics(conn, &uri, &p.text_document.text);
            }
        }
        DidChangeTextDocument::METHOD => {
            if let Ok(p) = serde_json::from_value::<DidChangeTextDocumentParams>(not.params) {
                if let Some(change) = p.content_changes.into_iter().last() {
                    let uri = p.text_document.uri;
                    docs.insert(uri.as_str().to_string(), change.text.clone());
                    publish_diagnostics(conn, &uri, &change.text);
                }
            }
        }
        DidCloseTextDocument::METHOD => {
            if let Ok(p) = serde_json::from_value::<DidCloseTextDocumentParams>(not.params) {
                let uri = p.text_document.uri;
                docs.remove(uri.as_str());
                publish_diagnostics(conn, &uri, "");
            }
        }
        _ => {}
    }
}

/// One completion item per documented primitive.
///
/// Public so `tests/lsp.rs` can hold the list against the corpus without
/// standing up a stdio server.
pub fn completion_items() -> Vec<CompletionItem> {
    CORPUS
        .iter()
        .map(|(name, chapter, doc, example)| CompletionItem {
            label: (*name).to_string(),
            kind: Some(match *chapter {
                "Registers" => CompletionItemKind::VARIABLE,
                "Grouping" => CompletionItemKind::OPERATOR,
                "Macro definition" => CompletionItemKind::FUNCTION,
                // Category codes, expansion and the conditionals are all
                // primitives the mouth or the expander acts on directly.
                _ => CompletionItemKind::KEYWORD,
            }),
            detail: Some((*doc).to_string()),
            documentation: Some(lsp_types::Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: format!("```tex\n{example}\n```"),
            })),
            ..Default::default()
        })
        .collect()
}

/// The markdown shown when hovering line `line`, column `col` of `text`.
///
/// Public for the same reason as [`completion_items`].
pub fn hover_markdown(text: &str, line: u32, col: u32) -> String {
    let Some(word) = token_at(text, line, col) else {
        return banner();
    };
    let matches: Vec<&Entry> = CORPUS.iter().filter(|(name, ..)| *name == word).collect();
    if matches.is_empty() {
        return banner();
    }
    let mut out = String::new();
    for (name, chapter, doc, example) in matches {
        out.push_str(&format!(
            "**`{name}`** — _{chapter}_\n\n{doc}\n\n```tex\n{example}\n```\n\n"
        ));
    }
    out.trim_end().to_string()
}

fn banner() -> String {
    "**texrs** — TeX's mouth and expander on the fusevm bytecode VM.".to_string()
}

/// The control sequence (or group character) spanning the given position.
///
/// TeX's own rule, not an identifier rule: a backslash followed by letters is
/// one control WORD, a backslash followed by anything else is a control SYMBOL
/// exactly one character long, and `{` and `}` are tokens in their own right.
fn token_at(text: &str, line: u32, col: u32) -> Option<String> {
    let line = text.lines().nth(line as usize)?;
    let chars: Vec<char> = line.chars().collect();
    let col = (col as usize).min(chars.len().saturating_sub(1));
    if chars.is_empty() {
        return None;
    }
    if chars[col] == '{' || chars[col] == '}' {
        return Some(chars[col].to_string());
    }
    // `^^X` is one token to the mouth and one entry in the corpus, and the
    // cursor can be on any of its three characters. Recognised before the
    // walk below, which is looking for an escape character and would refuse.
    if let Some(caret) = caret_notation_at(&chars, col) {
        return Some(caret);
    }
    // A backtick is the corpus's name for the character-code prefix; it is a
    // token a document writes and a reader hovers, not a control sequence.
    if chars[col] == '`' {
        return Some("`".to_string());
    }

    // Walk left to the escape character. Letters may precede the cursor; a
    // backslash ends the walk because it opens the sequence.
    let mut start = col;
    while start > 0 && chars[start].is_ascii_alphabetic() {
        start -= 1;
    }
    if chars[start] != '\\' {
        return None;
    }
    let mut end = start + 1;
    // A control symbol is one character; a control word runs while letters do.
    if end < chars.len() && !chars[end].is_ascii_alphabetic() {
        end += 1;
    } else {
        while end < chars.len() && chars[end].is_ascii_alphabetic() {
            end += 1;
        }
    }
    if end <= start + 1 && end >= chars.len() {
        return None;
    }
    Some(chars[start..end].iter().collect())
}

/// `^^X` spanning `col`, if the cursor is anywhere in one.
///
/// The corpus documents the notation itself rather than a particular control
/// character, so any `^^` pair plus its following character answers as `^^X` —
/// which is the name the entry is filed under.
fn caret_notation_at(chars: &[char], col: usize) -> Option<String> {
    // The cursor may sit on the first caret, the second, or the character.
    for start in [col, col.saturating_sub(1), col.saturating_sub(2)] {
        if chars.get(start) == Some(&'^')
            && chars.get(start + 1) == Some(&'^')
            && chars.get(start + 2).is_some()
            && col <= start + 2
        {
            return Some("^^X".to_string());
        }
    }
    None
}

fn publish_diagnostics(conn: &Connection, uri: &Uri, text: &str) {
    let params = PublishDiagnosticsParams {
        uri: uri.clone(),
        diagnostics: diagnostics(text),
        version: None,
    };
    let not = lsp_server::Notification::new(PublishDiagnostics::METHOD.to_string(), params);
    let _ = conn.sender.send(not.into());
}

/// Lower the whole document with the engine's own lowerer; a failure becomes one
/// diagnostic on the line the mouth had reached.
///
/// Public for the same reason as [`completion_items`].
pub fn diagnostics(text: &str) -> Vec<Diagnostic> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    match crate::compile_located(text) {
        Ok(_) => Vec::new(),
        Err((e, line)) => {
            let line = line.saturating_sub(1);
            vec![Diagnostic {
                range: Range {
                    start: Position { line, character: 0 },
                    end: Position {
                        line,
                        character: 200,
                    },
                },
                severity: Some(DiagnosticSeverity::ERROR),
                // tex writes its errors as `! <reason>.`; so does the editor.
                message: format!("! {}.", e.0),
                ..Default::default()
            }]
        }
    }
}

/// Exit if reparented to pid 1 (the editor died) so we never leak.
fn spawn_orphan_guard() {
    std::thread::spawn(|| {
        #[cfg(target_os = "linux")]
        // SAFETY: prctl(PR_SET_PDEATHSIG, ...) only registers a signal disposition.
        unsafe {
            libc::prctl(
                libc::PR_SET_PDEATHSIG,
                libc::SIGKILL as libc::c_ulong,
                0,
                0,
                0,
            );
        }
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            // SAFETY: getppid takes no arguments and never fails.
            if unsafe { libc::getppid() } == 1 {
                std::process::exit(0);
            }
        }
    });
}

/// One primitive exactly as the language server serves it.
///
/// Not read from the corpus: `name`, `doc` and `example` come out of the
/// completion response an editor receives, and `chapter` out of the hover
/// response for the same name. The reference manual is rendered from these, so
/// the page cannot claim anything the server would not tell an editor — if
/// hover stops resolving a primitive, the manual loses it and
/// `tests/lsp_serves_the_reference.rs` fails rather than the page quietly
/// shrinking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Served {
    /// The completion item's label.
    pub name: String,
    /// The chapter, parsed back out of the hover markdown.
    pub chapter: String,
    /// The completion item's `detail`.
    pub doc: String,
    /// The `tex` block the completion item's documentation carries.
    pub example: String,
}

/// Ask the server for everything it documents.
pub fn served() -> Vec<Served> {
    completion_items()
        .into_iter()
        .map(|item| {
            let doc = item.detail.clone().unwrap_or_default();
            let example = match &item.documentation {
                Some(lsp_types::Documentation::MarkupContent(m)) => unfence(&m.value),
                _ => String::new(),
            };
            let chapter = hover_chapter(&item.label).unwrap_or_default();
            Served {
                name: item.label,
                chapter,
                doc,
                example,
            }
        })
        .collect()
}

/// The chapter the hover response states for `name`, or `None` when hover does
/// not resolve it — which is a gap in the server, not in the corpus.
pub fn hover_chapter(name: &str) -> Option<String> {
    let markdown = hover_markdown(name, 0, 0);
    // `**`\catcode`** — _Category codes_`
    let (_, rest) = markdown.split_once("— _")?;
    let (chapter, _) = rest.split_once('_')?;
    Some(chapter.to_string())
}

/// Strip the ```tex fence the completion documentation wraps an example in.
fn unfence(markdown: &str) -> String {
    markdown
        .trim()
        .trim_start_matches("```tex")
        .trim_end_matches("```")
        .trim_matches('\n')
        .to_string()
}
