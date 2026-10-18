//! LuaTeX's PDF backend as a document reaches it: `\pdfvariable`,
//! `\pdffeedback` and `\pdfextension` (`textoken.c`'s `do_variable_pdf` and
//! `do_feedback_pdf`, `extensions.c`'s `do_extension_pdf`, and the scanners in
//! `pdfobj.c`, `pdfannot.c`, `pdfdest.c`, `pdflink.c`, `pdfoutline.c`,
//! `pdfaction.c`, `pdfthread.c` and `writeimg.c`).
//!
//! Three kinds of thing are asked of it, and they land in three places:
//!
//! - **Parameters.** `\pdfvariable compresslevel` expands to an integer
//!   parameter that has no name of its own, as `do_variable_backend_int` makes
//!   it; here it is the `crate::intpar` entry `pdfvariable compresslevel`, so
//!   it is assigned, read, grouped and `\the`d as every parameter is. The
//!   dimensions and token lists are `crate::params` entries the same way.
//! - **The document's own objects.** `\pdfextension obj`, `info`, `catalog`,
//!   `names`, `trailer` and `outline` are numbered when they are read, as
//!   luatex numbers them -- `\pdffeedback lastobj` is the number -- and an
//!   `\immediate` object, an outline's title and its action are WRITTEN when
//!   they are read, into the file luatex writes them into at that moment. So
//!   the backend holds the file being written ([`crate::pdfdoc::Writer`]) from
//!   the first object a document writes, and the pages are added to it after.
//! - **Whatsits.** A literal, a destination, an annotation, a link, a thread,
//!   `save`, `restore`, `setmatrix`, `colorstack` and `refobj` are placed in the
//!   list being built and do their work when the page holding them ships
//!   ([`PdfWhatsit`]); `crate::pdfpage::PageWriter` is what writes them.

use crate::expand::Engine;
use crate::lexer::Lexer;
use crate::token::Token;
use crate::TexError;
use std::collections::BTreeMap;

type R<T> = Result<T, TexError>;

/// `do_variable_pdf`'s integer keys, in its order (`textoken.c`).
pub const INT_VARIABLES: [&str; 28] = [
    "compresslevel",
    "decimaldigits",
    "imageresolution",
    "pkresolution",
    "uniqueresname",
    "majorversion",
    "minorversion",
    "pagebox",
    "inclusionerrorlevel",
    "ignoreunknownimages",
    "gamma",
    "imageapplygamma",
    "imagegamma",
    "imagehicolor",
    "imageaddfilename",
    "objcompresslevel",
    "inclusioncopyfonts",
    "gentounicode",
    "pkfixeddpi",
    "suppressoptionalinfo",
    "omitcidset",
    "recompress",
    "omitcharset",
    "omitinfodict",
    "omitmediabox",
    "linking",
    "omitprocset",
    "ptexprefix",
];

/// Its dimension keys.
pub const DIMEN_VARIABLES: [&str; 6] = [
    "horigin",
    "vorigin",
    "threadmargin",
    "destmargin",
    "linkmargin",
    "xformmargin",
];

/// Its token-list keys.
pub const TOKS_VARIABLES: [&str; 7] = [
    "pageattr",
    "pageresources",
    "pagesattr",
    "xformattr",
    "xformresources",
    "pkmode",
    "trailerid",
];

/// The parameter `\pdfvariable key` expands to.
pub fn parameter(key: &str) -> String {
    format!("pdfvariable {key}")
}

/// What `luatex.fmt` and `lualatex.fmt` leave a variable at when the document
/// says nothing (`pdftexconfig.tex`, read by `luatexconfig.tex`): texrs loads
/// no format, so a variable nothing assigned is read at the value the format a
/// luatex run starts from would have given it. `\the` still answers INITEX's
/// zero, as luatex's INITEX does.
pub fn format_default(key: &str) -> i64 {
    match key {
        "compresslevel" => 9,
        "objcompresslevel" => 2,
        "decimaldigits" => 3,
        "pkresolution" => 600,
        "minorversion" => 7,
        "majorversion" => 1,
        // `1 true in`, in scaled points.
        "horigin" | "vorigin" => 4_736_286,
        _ => 0,
    }
}

/// A destination's identifier: `num` or `name`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DestId {
    Num(i64),
    Name(String),
}

/// `pdf_dest_type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DestKind {
    Xyz { zoom: Option<i64> },
    Fit,
    FitH,
    FitV,
    FitB,
    FitBH,
    FitBV,
    FitR,
}

/// `scan_alt_rule`: a width, height and depth, each one given or running.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AltRule {
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub depth: Option<i64>,
}

/// `scan_action`.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// `user {...}`: the dictionary, written as it stands.
    User(String),
    /// `goto` or `thread`, with what identifies the target.
    Goto {
        thread: bool,
        file: Option<String>,
        structure: Option<DestId>,
        target: Target,
        new_window: Option<bool>,
    },
}

/// Where a `goto` goes.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// `page N {spec}`.
    Page(i64, String),
    Dest(DestId),
}

/// The colour stack commands (`colorstack_set` ... `colorstack_current`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackCmd {
    Set,
    Push,
    Pop,
    Current,
}

/// A whatsit of LuaTeX's PDF backend, as the list a page is built from holds
/// it: what each `\pdfextension` that is placed rather than done says.
#[derive(Clone, Debug, PartialEq)]
pub enum PdfWhatsit {
    /// `literal` (`late` for `lateliteral` or `literal shipout`, whose tokens
    /// wait for the shipout to be expanded).
    Literal {
        mode: crate::pdfpage::LiteralMode,
        tokens: Vec<Token>,
        late: bool,
    },
    Dest {
        id: DestId,
        structure: Option<i64>,
        kind: DestKind,
        rule: AltRule,
    },
    /// `annot`, numbered when it was read.
    Annot {
        object: u32,
        rule: AltRule,
        data: String,
    },
    Save,
    Restore,
    SetMatrix(String),
    LinkState(i64),
    RefObj(u32),
    ColorStack {
        stack: usize,
        cmd: StackCmd,
        data: Option<String>,
    },
    /// `startlink`, numbered when it was read.
    StartLink {
        object: u32,
        rule: AltRule,
        attr: Option<String>,
        action: Action,
    },
    EndLink,
    /// `startthread` (`start`) and `thread`.
    Thread {
        start: bool,
        rule: AltRule,
        attr: Option<String>,
        id: DestId,
    },
    EndThread,
    /// `\savepos`.
    SavePos,
    /// `\useimageresource`: the picture numbered by `\saveimageresource`.
    UseImage {
        object: u32,
        rule: AltRule,
    },
    /// `\useboxresource`.
    UseBox {
        object: u32,
        rule: AltRule,
    },
}

/// A raw object (`\pdfextension obj`), held until it is written.
#[derive(Clone, Debug, PartialEq)]
pub struct RawObject {
    pub stream: bool,
    pub attr: Option<String>,
    pub file: bool,
    pub uncompressed: bool,
    pub data: String,
}

/// One outline entry (`obj_outline_*`).
#[derive(Clone, Debug, Default, PartialEq)]
struct Outline {
    number: u32,
    title: u32,
    action: u32,
    count: i64,
    attr: Option<String>,
    parent: u32,
    prev: u32,
    next: u32,
    first: u32,
    last: u32,
}

/// A colour stack (`colstack_type`): its literal mode, whether it starts each
/// page afresh, and what is on it.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorStack {
    pub mode: crate::pdfpage::LiteralMode,
    pub page_start: bool,
    pub stack: Vec<String>,
}

/// A picture read by `\saveimageresource`.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageResource {
    pub object: u32,
    pub file: String,
    pub rule: AltRule,
    pub page: i64,
    pub attr: Option<String>,
}

/// The backend's state for one document.
#[derive(Default)]
pub struct Backend {
    /// The file, from the first object written into it.
    writer: Option<crate::pdfdoc::Writer>,
    /// The numbers handed out before the file was begun.
    pending_numbers: u32,
    pub last_obj: u32,
    pub last_annot: u32,
    pub last_link: u32,
    pub retval: i64,
    /// Objects numbered and not written: written when a page refers to them.
    pub raw: BTreeMap<u32, RawObject>,
    pub info: String,
    pub catalog: String,
    pub open_action: Option<u32>,
    pub names: String,
    pub trailer: String,
    outlines: Vec<Outline>,
    first_outline: u32,
    last_outline: u32,
    parent_outline: u32,
    /// Destinations an action named, by identifier, with their numbers --
    /// `pdf_get_obj(obj_type_dest, ...)`; a named one is a name-tree entry.
    pub dests: BTreeMap<DestId, u32>,
    /// The names in the order `append_dest_name` saw them.
    pub dest_names: Vec<(String, u32)>,
    /// Page objects something asked for by number before the page existed.
    pub pages: BTreeMap<i64, u32>,
    pub threads: BTreeMap<DestId, u32>,
    /// Colour stack 0 is the default one, set up as `\pdfextension colorstack`
    /// finds it.
    pub color_stacks: Vec<ColorStack>,
    pub images: Vec<ImageResource>,
    pub last_image: u32,
    pub last_image_pages: i64,
    pub last_box: u32,
    /// What luatex printed as a warning, in order, for the run to print.
    pub warnings: Vec<String>,
    /// `obj_type` of the objects a document made, by number.
    kinds: BTreeMap<u32, &'static str>,
    /// lpdflib's `pdf.data` strings (`pdf.setcatalog`, `pdf.setinfo`,
    /// `pdf.setnames`, `pdf.settrailer`, `pdf.setpageresources`,
    /// `pdf.setpageattributes`, `pdf.setpagesattributes`, `pdf.settrailerid`),
    /// kept apart from `\pdfextension`'s and written after them
    /// (`print_pdf_table_string`). An empty one is not written.
    pub lua: LuaStrings,
}

/// lpdflib's dictionary strings: see [`Backend::lua`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LuaStrings {
    pub catalog: String,
    pub info: String,
    pub names: String,
    pub trailer: String,
    pub page_resources: String,
    pub page_attributes: String,
    pub pages_attributes: String,
    /// When set, the trailer's `/ID` is this rather than the one `print_ID`
    /// makes.
    pub trailer_id: String,
}

impl Backend {
    /// The variables the writer starts with, from what the document assigned
    /// and the format's values for the rest.
    pub fn variables(eng: &Engine) -> crate::pdfdoc::Variables {
        let int = |key: &str| {
            crate::intpar::index(&parameter(key))
                .and_then(|i| eng.intpars.assigned(i))
                .unwrap_or_else(|| format_default(key))
        };
        crate::pdfdoc::Variables {
            compress_level: int("compresslevel").clamp(0, 9) as u32,
            objcompress_level: int("objcompresslevel").clamp(0, 3) as u32,
            decimal_digits: int("decimaldigits").clamp(0, 4) as u32,
            gen_tounicode: int("gentounicode") > 0,
            major_version: int("majorversion").max(1) as u32,
            minor_version: int("minorversion").max(0) as u32,
        }
    }

    /// `pdf_create_obj`: the next number, from the file if it is begun.
    fn create(&mut self) -> u32 {
        match self.writer.as_mut() {
            Some(w) => w.create(),
            None => {
                self.pending_numbers += 1;
                self.pending_numbers
            }
        }
    }

    /// [`Self::create`], recording the object's `obj_type` for
    /// [`Self::object_kind`].
    fn create_as(&mut self, kind: &'static str) -> u32 {
        let k = self.create();
        self.kinds.insert(k, kind);
        k
    }

    /// `pdf.getobjtype(n)`: what kind of object `n` was made as -- `obj`,
    /// `annot`, `page`, `dest`, `outline` -- or `None` for one the backend made
    /// for itself or never made.
    pub fn object_kind(&self, n: u32) -> Option<&'static str> {
        self.kinds.get(&n).copied()
    }

    /// How many numbers have been handed out so far.
    pub fn objects(&self) -> u32 {
        match &self.writer {
            Some(w) => w.count(),
            None => self.pending_numbers,
        }
    }

    /// `pdf.reserveobj()`: a raw object's number, for a later `useobjnum`.
    pub fn reserve(eng: &mut Engine) -> u32 {
        let k = eng.backend.create_as("obj");
        eng.backend.last_obj = k;
        k
    }

    /// `pdf.reserveobj("annot")`.
    pub fn reserve_annot(eng: &mut Engine) -> u32 {
        let k = eng.backend.create_as("annot");
        eng.backend.last_annot = k;
        k
    }

    /// `pdf.obj`, `pdf.immediateobj` and `pdf.obj{...}`: what `scan_obj` does
    /// once it has read the object -- numbered (or `number` reused), made
    /// `lastobj`, and written now when `immediate`, held for a page otherwise.
    /// `raw.uncompressed` is `objcompression=false` (`OBJSTM_NEVER`).
    pub fn lua_obj(eng: &mut Engine, number: Option<u32>, raw: RawObject, immediate: bool) -> u32 {
        let k = number.unwrap_or_else(|| eng.backend.create_as("obj"));
        eng.backend.last_obj = k;
        match immediate {
            true => write_raw(eng, k, &raw),
            false => {
                eng.backend.raw.insert(k, raw);
            }
        }
        k
    }

    /// The file, begun now if it is not yet: `ensure_output_state(pdf,
    /// ST_HEADER_WRITTEN)`, with the numbers handed out so far reserved.
    fn writer(&mut self, vars: crate::pdfdoc::Variables) -> &mut crate::pdfdoc::Writer {
        if self.writer.is_none() {
            let mut w = crate::pdfdoc::Writer::new(vars);
            for _ in 0..self.pending_numbers {
                w.create();
            }
            self.writer = Some(w);
        }
        self.writer.as_mut().expect("just made")
    }

    /// Everything the document gave the backend, for the file to be finished
    /// with: see [`crate::pdfdoc::write`].
    pub fn take(&mut self, eng: &Engine) -> Prepared {
        let toks = |key: &str| {
            crate::params::toks_register(&parameter(key))
                .map(|r| eng.toks_text(r))
                .unwrap_or_default()
        };
        let mut outlines = std::mem::take(&mut self.outlines);
        // `print_outlines` writes them in `head_tab` order: newest first.
        outlines.reverse();
        let pending = self.pending_numbers;
        let writer = std::mem::take(&mut self.writer).unwrap_or_else(|| {
            let mut w = crate::pdfdoc::Writer::new(Self::variables(eng));
            for _ in 0..pending {
                w.create();
            }
            w
        });
        Prepared {
            writer: Some(writer),
            vars: Self::variables(eng),
            info: std::mem::take(&mut self.info),
            catalog: std::mem::take(&mut self.catalog),
            open_action: self.open_action,
            names: std::mem::take(&mut self.names),
            trailer: std::mem::take(&mut self.trailer),
            page_attr: toks("pageattr"),
            page_resources: toks("pageresources"),
            pages_attr: toks("pagesattr"),
            outlines: outlines
                .into_iter()
                .map(|o| OutlineEntry {
                    number: o.number,
                    title: o.title,
                    action: o.action,
                    count: o.count,
                    attr: o.attr,
                    parent: o.parent,
                    prev: o.prev,
                    next: o.next,
                    first: o.first,
                    last: o.last,
                })
                .collect(),
            first_outline: self.first_outline,
            last_outline: self.last_outline,
            parent_outline: self.parent_outline,
            dests: std::mem::take(&mut self.dests),
            dest_names: std::mem::take(&mut self.dest_names),
            pages: std::mem::take(&mut self.pages),
            lua: std::mem::take(&mut self.lua),
        }
    }
}

/// An outline entry as the file is finished with it.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineEntry {
    pub number: u32,
    pub title: u32,
    pub action: u32,
    pub count: i64,
    pub attr: Option<String>,
    pub parent: u32,
    pub prev: u32,
    pub next: u32,
    pub first: u32,
    pub last: u32,
}

/// What the run hands the file at its end.
pub struct Prepared {
    pub writer: Option<crate::pdfdoc::Writer>,
    pub vars: crate::pdfdoc::Variables,
    pub info: String,
    pub catalog: String,
    pub open_action: Option<u32>,
    pub names: String,
    pub trailer: String,
    pub page_attr: String,
    pub page_resources: String,
    pub pages_attr: String,
    /// Newest first, as `print_outlines` walks them.
    pub outlines: Vec<OutlineEntry>,
    pub first_outline: u32,
    pub last_outline: u32,
    pub parent_outline: u32,
    pub dests: BTreeMap<DestId, u32>,
    pub dest_names: Vec<(String, u32)>,
    pub pages: BTreeMap<i64, u32>,
    pub lua: LuaStrings,
}

impl Default for Prepared {
    fn default() -> Self {
        Prepared {
            writer: None,
            vars: crate::pdfdoc::Variables::default(),
            info: String::new(),
            catalog: String::new(),
            open_action: None,
            names: String::new(),
            trailer: String::new(),
            page_attr: String::new(),
            page_resources: String::new(),
            pages_attr: String::new(),
            outlines: Vec::new(),
            first_outline: 0,
            last_outline: 0,
            parent_outline: 0,
            dests: BTreeMap::new(),
            dest_names: Vec::new(),
            pages: BTreeMap::new(),
            lua: LuaStrings::default(),
        }
    }
}

// ── scanning ─────────────────────────────────────────────────────────────

/// `scan_keyword`, reading the file (not only pending tokens).
fn keyword(eng: &mut Engine, lx: &mut Lexer, word: &str) -> R<bool> {
    eng.scan_keyword(lx, word, false)
}

/// The first of `words` that is next, in the order luatex tries them.
fn first_keyword<'a>(eng: &mut Engine, lx: &mut Lexer, words: &[&'a str]) -> R<Option<&'a str>> {
    for w in words {
        if keyword(eng, lx, w)? {
            return Ok(Some(w));
        }
    }
    Ok(None)
}

/// `scan_toks(false, true)`: a general text, expanded as `\xdef` expands.
fn expanded_text(eng: &mut Engine, lx: &mut Lexer) -> R<Vec<Token>> {
    let text = eng.scan_general_text(lx)?;
    eng.expand_edef_body(&text)
}

/// The same as a string, `tokenlist_to_cstring`.
fn expanded_string(eng: &mut Engine, lx: &mut Lexer) -> R<String> {
    let toks = expanded_text(eng, lx)?;
    Ok(eng.tokens_text(&toks))
}

/// `scan_int`.
fn int(eng: &mut Engine, lx: &mut Lexer) -> R<i64> {
    eng.scan_number_file(lx)
}

/// "Scan an optional space": one space token after a keyword is absorbed.
fn optional_space(eng: &mut Engine, lx: &mut Lexer) -> R<()> {
    match eng.next_unexpandable(lx)? {
        Some((t, _)) if t.is_space() => {}
        Some((t, _)) => lx.push_back(&[t]),
        None => {}
    }
    Ok(())
}

/// `scan_alt_rule`.
fn alt_rule(eng: &mut Engine, lx: &mut Lexer) -> R<AltRule> {
    let mut rule = AltRule::default();
    loop {
        match first_keyword(eng, lx, &["width", "height", "depth"])? {
            Some("width") => rule.width = Some(eng.scan_dimen(lx, false)?),
            Some("height") => rule.height = Some(eng.scan_dimen(lx, false)?),
            Some("depth") => rule.depth = Some(eng.scan_dimen(lx, false)?),
            _ => return Ok(rule),
        }
    }
}

/// `scan_action`.
fn action(eng: &mut Engine, lx: &mut Lexer) -> R<Action> {
    let kind = match first_keyword(eng, lx, &["user", "goto", "thread"])? {
        Some(k) => k,
        None => return Err(TexError("action type missing".into())),
    };
    if kind == "user" {
        return Ok(Action::User(expanded_string(eng, lx)?));
    }
    let file = match keyword(eng, lx, "file")? {
        true => Some(expanded_string(eng, lx)?),
        false => None,
    };
    let structure = match keyword(eng, lx, "struct")? {
        false => None,
        true if file.is_some() => Some(DestId::Name(expanded_string(eng, lx)?)),
        true => match first_keyword(eng, lx, &["name", "num"])? {
            Some("name") => Some(DestId::Name(expanded_string(eng, lx)?)),
            Some(_) => Some(DestId::Num(int(eng, lx)?)),
            None => return Err(TexError("identifier type missing".into())),
        },
    };
    let target = match first_keyword(eng, lx, &["page", "name", "num"])? {
        Some("page") => {
            let n = int(eng, lx)?;
            Target::Page(n, expanded_string(eng, lx)?)
        }
        Some("name") => Target::Dest(DestId::Name(expanded_string(eng, lx)?)),
        Some(_) => Target::Dest(DestId::Num(int(eng, lx)?)),
        None => return Err(TexError("identifier type missing".into())),
    };
    let new_window = match first_keyword(eng, lx, &["newwindow", "nonewwindow"])? {
        Some(w) => {
            optional_space(eng, lx)?;
            Some(w == "newwindow")
        }
        None => None,
    };
    Ok(Action::Goto {
        thread: kind == "thread",
        file,
        structure,
        target,
        new_window,
    })
}

/// `scan_thread_id`.
fn thread_id(eng: &mut Engine, lx: &mut Lexer) -> R<DestId> {
    match first_keyword(eng, lx, &["num", "name"])? {
        Some("num") => Ok(DestId::Num(int(eng, lx)?)),
        Some(_) => Ok(DestId::Name(expanded_string(eng, lx)?)),
        None => Err(TexError("identifier type missing".into())),
    }
}

/// The literal mode keywords (`direct`, `page`, `text`, `raw`, `origin`).
fn literal_mode(eng: &mut Engine, lx: &mut Lexer) -> R<crate::pdfpage::LiteralMode> {
    use crate::pdfpage::LiteralMode as M;
    Ok(
        match first_keyword(eng, lx, &["direct", "page", "text", "raw", "origin"])? {
            Some("direct") => M::DirectAlways,
            Some("page") => M::DirectPage,
            Some("text") => M::DirectText,
            Some("raw") => M::DirectRaw,
            _ => M::SetOrigin,
        },
    )
}

// ── \pdfvariable ─────────────────────────────────────────────────────────

/// `\pdfvariable`: the parameter the key names, put back to be read as the
/// assignment or the quantity it stands for (`do_variable_pdf`). An unknown
/// key is luatex's warning, and nothing.
pub fn variable(eng: &mut Engine, lx: &mut Lexer) -> R<()> {
    let keys = INT_VARIABLES
        .iter()
        .chain(DIMEN_VARIABLES.iter())
        .chain(TOKS_VARIABLES.iter());
    for key in keys {
        if keyword(eng, lx, key)? {
            lx.back_input(&[Token::cs(&parameter(key))]);
            return Ok(());
        }
    }
    warn(eng, "unexpected use of \\pdfvariable");
    Ok(())
}

/// A backend warning, printed as luatex prints one on the terminal:
/// `warning  (pdf backend): ...` on a line of its own.
fn warn(eng: &mut Engine, text: &str) {
    eng.terminal_note(&format!("warning  (pdf backend): {text}"));
    eng.backend.warnings.push(text.to_string());
}

// ── \pdffeedback ─────────────────────────────────────────────────────────

/// `\pdffeedback`: what the backend knows, as characters
/// (`do_feedback_pdf`). `version` is pdfTeX's 140 and `revision` its `0`,
/// which "will not change any more".
pub fn feedback(eng: &mut Engine, lx: &mut Lexer) -> R<()> {
    let words = [
        "lastlink",
        "retval",
        "lastobj",
        "lastannot",
        "xformname",
        "creationdate",
        "fontname",
        "fontobjnum",
        "fontsize",
        "pageref",
        "colorstackinit",
        "version",
        "revision",
    ];
    let Some(word) = first_keyword(eng, lx, &words)? else {
        eng.report(lx, "unexpected use of \\pdffeedback");
        return Ok(());
    };
    let text = match word {
        "lastlink" => eng.backend.last_link.to_string(),
        "retval" => eng.backend.retval.to_string(),
        "lastobj" => eng.backend.last_obj.to_string(),
        "lastannot" => eng.backend.last_annot.to_string(),
        // The form's resource number, `obj_info`: forms are numbered from 1
        // in the order they were saved.
        "xformname" => {
            let n = int(eng, lx)?;
            n.to_string()
        }
        "creationdate" => crate::pdf::pdf_date(crate::pdf::stamp()),
        "fontname" | "fontobjnum" => {
            // The font's resource and object: numbered by the shipout that
            // first draws it, which no frontend sees.
            let _ = eng.fontname_text(lx, false)?;
            "0".to_string()
        }
        "fontsize" => {
            let name = eng.fontname_text(lx, false)?;
            font_size_text(&name)
        }
        "pageref" => {
            let n = int(eng, lx)?;
            if n <= 0 {
                return Err(TexError("invalid page number when asking 'pageref'".into()));
            }
            page_object(&mut eng.backend, n).to_string()
        }
        "colorstackinit" => {
            let page_start = keyword(eng, lx, "page")?;
            let mode = literal_mode(eng, lx)?;
            let init = expanded_string(eng, lx)?;
            let stacks = &mut eng.backend.color_stacks;
            if stacks.is_empty() {
                stacks.push(default_color_stack());
            }
            stacks.push(ColorStack {
                mode,
                page_start,
                stack: vec![init],
            });
            (stacks.len() - 1).to_string()
        }
        "version" => "140".to_string(),
        _ => {
            lx.push_back(&crate::expand::str_toks("0"));
            return Ok(());
        }
    };
    lx.push_back(&crate::expand::str_toks(&text));
    Ok(())
}

/// Stack 0, which `colorstackused` counts from the start: luatex's
/// `initialize` makes it with the literal `0 g 0 G` page-start colour in
/// direct-page mode.
fn default_color_stack() -> ColorStack {
    ColorStack {
        mode: crate::pdfpage::LiteralMode::DirectPage,
        page_start: true,
        stack: vec!["0 g 0 G".to_string()],
    }
}

/// `print_scaled(font_size(f))` and `pt` for the font `\fontname` names:
/// the `at` size it was loaded at, or its TFM's design size.
fn font_size_text(name: &str) -> String {
    let (file, at) = match name.split_once(" at ") {
        Some((f, a)) => (f, Some(a.to_string())),
        None => (name, None),
    };
    if let Some(at) = at {
        return at;
    }
    let size = crate::kpse::locate(&format!("{file}.tfm"))
        .and_then(|p| crate::tfm::Tfm::open(p).ok())
        .map(|t| (t.design_size * 65536.0).round() as i64)
        .unwrap_or(0);
    format!("{}pt", crate::dimen::print_scaled(size))
}

/// `pdf_get_obj(pdf, obj_type_page, n, false)`: the object page `n` will be,
/// numbered now if nothing has asked for it before.
fn page_object(b: &mut Backend, n: i64) -> u32 {
    if let Some(&k) = b.pages.get(&n) {
        return k;
    }
    let k = b.create_as("page");
    b.pages.insert(n, k);
    k
}

/// `pdf_get_obj(pdf, obj_type_dest, id, ...)`: a destination's object,
/// numbered now if this is the first mention of it; a named one is entered in
/// the name tree as `append_dest_name` enters it.
fn dest_object(b: &mut Backend, id: &DestId) -> u32 {
    if let Some(&k) = b.dests.get(id) {
        return k;
    }
    let k = b.create_as("dest");
    b.dests.insert(id.clone(), k);
    if let DestId::Name(name) = id {
        b.dest_names.push((name.clone(), k));
    }
    k
}

/// `write_action`: an action as the body of its object.
fn action_text(b: &mut Backend, a: &Action, decimal_digits: u32) -> String {
    let _ = decimal_digits;
    let (thread, file, structure, target, new_window) = match a {
        Action::User(text) => return format!("\n{text}\n"),
        Action::Goto {
            thread,
            file,
            structure,
            target,
            new_window,
        } => (*thread, file, structure, target, new_window),
    };
    let mut out = String::from("<<");
    if let Some(f) = file {
        out.push_str(&format!(" /F {} ", pdf_string(f)));
        if let Some(w) = new_window {
            out.push_str(&format!(" /NewWindow {w}"));
        }
    }
    match (thread, target) {
        (false, Target::Page(n, spec)) => {
            out.push_str(" /S /GoTo /D [");
            match file {
                None => {
                    let page = page_object(b, *n);
                    out.push_str(&format!(" {page} 0 R"));
                }
                Some(_) => out.push_str(&format!(" {}", n - 1)),
            }
            out.push_str(&format!(" {spec} ]"));
        }
        (false, Target::Dest(id)) => match file {
            None => {
                out.push_str(" /S /GoTo");
                let d = dest_object(b, id);
                match id {
                    DestId::Name(name) => out.push_str(&format!(" /D {}", pdf_string(name))),
                    DestId::Num(_) => out.push_str(&format!(" /D {d} 0 R")),
                }
            }
            Some(_) => {
                out.push_str(" /S /GoToR");
                if let DestId::Name(name) = id {
                    out.push_str(&format!(" /D {}", pdf_string(name)));
                }
            }
        },
        (true, Target::Dest(id)) => {
            out.push_str(" /S /Thread");
            if file.is_none() {
                let d = match b.threads.get(id) {
                    Some(&d) => d,
                    None => {
                        let d = b.create();
                        b.threads.insert(id.clone(), d);
                        d
                    }
                };
                match id {
                    DestId::Name(name) => out.push_str(&format!(" /D {}", pdf_string(name))),
                    DestId::Num(_) => out.push_str(&format!(" /D {d} 0 R")),
                }
            }
        }
        (true, Target::Page(..)) => out.push_str(" /S /Thread"),
    }
    if let Some(sd) = structure {
        match file {
            None => {
                let d = b.create();
                let _ = sd;
                out.push_str(&format!(" /SD {d} 0 R"));
            }
            Some(_) => {
                if let DestId::Name(name) = sd {
                    out.push_str(&format!(" /SD {name}"));
                }
            }
        }
    }
    out.push_str(" >>");
    out
}

/// `pdf_print_str`: a string written as it stands when it is already one --
/// `(...)` or a hexadecimal `<...>` -- and in parentheses otherwise.
pub fn pdf_string(s: &str) -> String {
    let b = s.as_bytes();
    if b.len() >= 2 && b[0] == b'(' && b[b.len() - 1] == b')' {
        return s.to_string();
    }
    let hex = b.len() >= 2
        && b[0] == b'<'
        && b[b.len() - 1] == b'>'
        && b.len() % 2 == 0
        && b[1..b.len() - 1].iter().all(u8::is_ascii_hexdigit);
    match hex {
        true => s.to_string(),
        false => format!("({s})"),
    }
}

// ── \pdfextension ────────────────────────────────────────────────────────

/// `\pdfextension` (`do_extension_pdf`): what is done now is done, and what
/// a page has to carry comes back as the whatsit to put in its list.
pub fn extension(eng: &mut Engine, lx: &mut Lexer, immediate: bool) -> R<Option<PdfWhatsit>> {
    let words = [
        "literal",
        "lateliteral",
        "dest",
        "annot",
        "save",
        "restore",
        "linkstate",
        "setmatrix",
        "obj",
        "refobj",
        "colorstack",
        "startlink",
        "endlink",
        "startthread",
        "endthread",
        "thread",
        "outline",
        "glyphtounicode",
        "catalog",
        "fontattr",
        "mapfile",
        "mapline",
        "includechars",
        "info",
        "names",
        "trailer",
    ];
    let Some(word) = first_keyword(eng, lx, &words)? else {
        eng.report(lx, "unexpected use of \\pdfextension");
        return Ok(None);
    };
    Ok(match word {
        "literal" | "lateliteral" => {
            let late = word == "lateliteral" || keyword(eng, lx, "shipout")?;
            let mode = literal_mode(eng, lx)?;
            let tokens = match late {
                true => eng.scan_general_text(lx)?,
                false => expanded_text(eng, lx)?,
            };
            Some(PdfWhatsit::Literal { mode, tokens, late })
        }
        "dest" => Some(dest(eng, lx)?),
        "annot" => annot(eng, lx)?,
        "save" => Some(PdfWhatsit::Save),
        "restore" => Some(PdfWhatsit::Restore),
        "linkstate" => Some(PdfWhatsit::LinkState(int(eng, lx)?)),
        "setmatrix" => Some(PdfWhatsit::SetMatrix(expanded_string(eng, lx)?)),
        "obj" => {
            obj(eng, lx, immediate)?;
            None
        }
        "refobj" => {
            let n = int(eng, lx)?;
            Some(PdfWhatsit::RefObj(n.max(0) as u32))
        }
        "colorstack" => color_stack(eng, lx)?,
        "startlink" => {
            let object = eng.backend.create();
            let rule = alt_rule(eng, lx)?;
            let attr = match keyword(eng, lx, "attr")? {
                true => Some(expanded_string(eng, lx)?),
                false => None,
            };
            let action = action(eng, lx)?;
            eng.backend.last_link = object;
            Some(PdfWhatsit::StartLink {
                object,
                rule,
                attr,
                action,
            })
        }
        "endlink" => Some(PdfWhatsit::EndLink),
        "startthread" | "thread" => {
            let rule = alt_rule(eng, lx)?;
            let attr = match keyword(eng, lx, "attr")? {
                true => Some(expanded_string(eng, lx)?),
                false => None,
            };
            let id = thread_id(eng, lx)?;
            Some(PdfWhatsit::Thread {
                start: word == "startthread",
                rule,
                attr,
                id,
            })
        }
        "endthread" => Some(PdfWhatsit::EndThread),
        "outline" => {
            outline(eng, lx)?;
            None
        }
        // `glyph_to_unicode`: two general texts, the glyph and its code.
        "glyphtounicode" => {
            let _ = expanded_text(eng, lx)?;
            let _ = expanded_text(eng, lx)?;
            None
        }
        "catalog" => {
            let text = expanded_string(eng, lx)?;
            append(&mut eng.backend.catalog, &text);
            if keyword(eng, lx, "openaction")? {
                if eng.backend.open_action.is_some() {
                    return Err(TexError("duplicate of openaction".into()));
                }
                let a = action(eng, lx)?;
                let dd = Backend::variables(eng).decimal_digits;
                // Numbered before the action is written, which numbers the
                // destination it names.
                let k = eng.backend.create();
                let body = action_text(&mut eng.backend, &a, dd);
                let vars = Backend::variables(eng);
                eng.backend.writer(vars).object(k, 1, body.as_bytes());
                eng.backend.open_action = Some(k);
            }
            None
        }
        "fontattr" | "includechars" => {
            let _ = eng.fontname_text(lx, false)?;
            let _ = expanded_text(eng, lx)?;
            None
        }
        "mapfile" | "mapline" => {
            let _ = expanded_text(eng, lx)?;
            None
        }
        "info" => {
            let text = expanded_string(eng, lx)?;
            append(&mut eng.backend.info, &text);
            None
        }
        "names" => {
            let text = expanded_string(eng, lx)?;
            append(&mut eng.backend.names, &text);
            None
        }
        _ => {
            let text = expanded_string(eng, lx)?;
            append(&mut eng.backend.trailer, &text);
            None
        }
    })
}

/// `concat_tokens`: the lists of `info`, `catalog`, `names` and `trailer`
/// accumulate, joined as their tokens were.
fn append(to: &mut String, text: &str) {
    to.push_str(text);
}

/// `scan_obj`.
fn obj(eng: &mut Engine, lx: &mut Lexer, immediate: bool) -> R<()> {
    if keyword(eng, lx, "reserveobjnum")? {
        optional_space(eng, lx)?;
        let k = eng.backend.create_as("obj");
        eng.backend.last_obj = k;
        if immediate {
            return Err(TexError(
                "\\pdfextension obj 'reserveobjnum' cannot be used with \\immediate".into(),
            ));
        }
        return Ok(());
    }
    let k = match keyword(eng, lx, "useobjnum")? {
        true => int(eng, lx)?.max(0) as u32,
        false => eng.backend.create_as("obj"),
    };
    let uncompressed = keyword(eng, lx, "uncompressed")?;
    let stream = keyword(eng, lx, "stream")?;
    let attr = match stream && keyword(eng, lx, "attr")? {
        true => Some(expanded_string(eng, lx)?),
        false => None,
    };
    let file = keyword(eng, lx, "file")?;
    let data = expanded_string(eng, lx)?;
    eng.backend.last_obj = k;
    let raw = RawObject {
        stream,
        attr,
        file,
        uncompressed,
        data,
    };
    match immediate {
        true => write_raw(eng, k, &raw),
        false => {
            eng.backend.raw.insert(k, raw);
        }
    }
    Ok(())
}

/// `pdf_write_obj`.
fn write_raw(eng: &mut Engine, k: u32, raw: &RawObject) {
    let data = match raw.file {
        true => crate::kpse::locate(&raw.data)
            .or_else(|| Some(std::path::PathBuf::from(&raw.data)))
            .and_then(|p| std::fs::read(p).ok())
            .unwrap_or_default(),
        false => raw.data.clone().into_bytes(),
    };
    let vars = Backend::variables(eng);
    let w = eng.backend.writer(vars);
    w.vars = vars;
    match raw.stream {
        true => {
            let keys = raw.attr.clone().unwrap_or_default();
            w.stream(k, keys.trim(), &data, !raw.uncompressed);
        }
        false => {
            let threshold = match raw.uncompressed {
                true => u32::MAX,
                false => 1,
            };
            w.object(k, threshold, &data);
        }
    }
}

/// `scan_annot`.
fn annot(eng: &mut Engine, lx: &mut Lexer) -> R<Option<PdfWhatsit>> {
    if keyword(eng, lx, "reserveobjnum")? {
        let k = eng.backend.create_as("annot");
        optional_space(eng, lx)?;
        eng.backend.last_annot = k;
        return Ok(None);
    }
    let k = match keyword(eng, lx, "useobjnum")? {
        true => int(eng, lx)?.max(0) as u32,
        false => eng.backend.create_as("annot"),
    };
    let rule = alt_rule(eng, lx)?;
    let data = expanded_string(eng, lx)?;
    eng.backend.last_annot = k;
    Ok(Some(PdfWhatsit::Annot {
        object: k,
        rule,
        data,
    }))
}

/// `scan_pdfdest`.
fn dest(eng: &mut Engine, lx: &mut Lexer) -> R<PdfWhatsit> {
    let structure = match keyword(eng, lx, "struct")? {
        true => Some(int(eng, lx)?),
        false => None,
    };
    let id = match first_keyword(eng, lx, &["num", "name"])? {
        Some("num") => DestId::Num(int(eng, lx)?),
        Some(_) => DestId::Name(expanded_string(eng, lx)?),
        None => return Err(TexError("identifier type missing".into())),
    };
    let kinds = [
        "xyz", "fitbh", "fitbv", "fitb", "fith", "fitv", "fitr", "fit",
    ];
    let kind = match first_keyword(eng, lx, &kinds)? {
        Some("xyz") => DestKind::Xyz {
            zoom: match keyword(eng, lx, "zoom")? {
                true => Some(int(eng, lx)?),
                false => None,
            },
        },
        Some("fitbh") => DestKind::FitBH,
        Some("fitbv") => DestKind::FitBV,
        Some("fitb") => DestKind::FitB,
        Some("fith") => DestKind::FitH,
        Some("fitv") => DestKind::FitV,
        Some("fitr") => DestKind::FitR,
        Some(_) => DestKind::Fit,
        None => return Err(TexError("destination type missing".into())),
    };
    optional_space(eng, lx)?;
    let rule = match kind {
        DestKind::FitR => alt_rule(eng, lx)?,
        _ => AltRule::default(),
    };
    Ok(PdfWhatsit::Dest {
        id,
        structure,
        kind,
        rule,
    })
}

/// `\pdfextension colorstack`.
fn color_stack(eng: &mut Engine, lx: &mut Lexer) -> R<Option<PdfWhatsit>> {
    let mut n = int(eng, lx)?;
    if eng.backend.color_stacks.is_empty() {
        eng.backend.color_stacks.push(default_color_stack());
    }
    if n >= eng.backend.color_stacks.len() as i64 {
        eng.report(lx, &format!("Unknown color stack number {n}"));
        n = 0;
    }
    if n < 0 {
        eng.report(lx, "Invalid negative color stack number");
        n = 0;
    }
    let cmd = match first_keyword(eng, lx, &["set", "push", "pop", "current"])? {
        Some("set") => StackCmd::Set,
        Some("push") => StackCmd::Push,
        Some("pop") => StackCmd::Pop,
        Some(_) => StackCmd::Current,
        None => {
            eng.report(lx, "Color stack action is missing");
            return Ok(None);
        }
    };
    let data = match cmd {
        StackCmd::Set | StackCmd::Push => Some(expanded_string(eng, lx)?),
        _ => None,
    };
    Ok(Some(PdfWhatsit::ColorStack {
        stack: n as usize,
        cmd,
        data,
    }))
}

/// `scan_pdfoutline`: the action and the title are written now, and the entry
/// is linked into the tree as `count` says it nests.
fn outline(eng: &mut Engine, lx: &mut Lexer) -> R<()> {
    let attr = match keyword(eng, lx, "attr")? {
        true => Some(expanded_string(eng, lx)?),
        false => None,
    };
    let (mut j, act) = match keyword(eng, lx, "useobjnum")? {
        true => (int(eng, lx)?.max(0) as u32, None),
        false => (0, Some(action(eng, lx)?)),
    };
    let count = match keyword(eng, lx, "count")? {
        true => int(eng, lx)?,
        false => 0,
    };
    let title = expanded_string(eng, lx)?;
    let dd = Backend::variables(eng).decimal_digits;
    if j == 0 {
        // Numbered before the action is written, which numbers the
        // destination it names (`scan_pdfoutline`).
        j = eng.backend.create();
        let body = act
            .as_ref()
            .map(|a| action_text(&mut eng.backend, a, dd))
            .unwrap_or_default();
        let vars = Backend::variables(eng);
        eng.backend.writer(vars).object(j, 1, body.as_bytes());
    }
    let k = eng.backend.create_as("outline");
    let l = eng.backend.create();
    let title_body = format!("{}\n", pdf_string(&title));
    let vars = Backend::variables(eng);
    eng.backend.writer(vars).object(l, 1, title_body.as_bytes());
    let b = &mut eng.backend;
    let mut entry = Outline {
        number: k,
        title: l,
        action: j,
        count,
        attr,
        parent: b.parent_outline,
        ..Outline::default()
    };
    if b.first_outline == 0 {
        b.first_outline = k;
    }
    if b.last_outline == 0 {
        if b.parent_outline != 0 {
            if let Some(p) = b.outline_mut(b.parent_outline) {
                p.first = k;
            }
        }
    } else {
        let last = b.last_outline;
        if let Some(p) = b.outline_mut(last) {
            p.next = k;
        }
        entry.prev = last;
    }
    b.outlines.push(entry);
    b.last_outline = k;
    if count != 0 {
        b.parent_outline = k;
        b.last_outline = 0;
    } else if b.parent_outline != 0
        && b.list_count(k) == b.outline(b.parent_outline).map_or(0, |p| p.count.abs())
    {
        let mut j = b.last_outline;
        loop {
            let parent = b.parent_outline;
            if let Some(p) = b.outline_mut(parent) {
                p.last = j;
            }
            j = parent;
            b.parent_outline = b.outline(parent).map_or(0, |p| p.parent);
            let done = b.parent_outline == 0
                || b.list_count(j) < b.outline(b.parent_outline).map_or(0, |p| p.count.abs());
            if done {
                break;
            }
        }
        b.last_outline = match b.parent_outline {
            0 => b.first_outline,
            p => b.outline(p).map_or(0, |o| o.first),
        };
        while let Some(next) = b
            .outline(b.last_outline)
            .map(|o| o.next)
            .filter(|&n| n != 0)
        {
            b.last_outline = next;
        }
    }
    Ok(())
}

impl Backend {
    fn outline(&self, k: u32) -> Option<&Outline> {
        self.outlines.iter().find(|o| o.number == k)
    }

    fn outline_mut(&mut self, k: u32) -> Option<&mut Outline> {
        self.outlines.iter_mut().find(|o| o.number == k)
    }

    /// `outline_list_count`: how many entries share `k`'s level up to it.
    fn list_count(&self, k: u32) -> i64 {
        let mut n = 1;
        let mut p = k;
        while let Some(prev) = self.outline(p).map(|o| o.prev).filter(|&p| p != 0) {
            n += 1;
            p = prev;
        }
        n
    }
}

// ── image and box resources ──────────────────────────────────────────────

/// `\saveimageresource` (`scan_pdfximage`): the picture is numbered now, as
/// `read_image` numbers it, and `\lastsavedimageresourceindex` is the number.
pub fn save_image(eng: &mut Engine, lx: &mut Lexer) -> R<()> {
    let rule = alt_rule(eng, lx)?;
    let attr = match keyword(eng, lx, "attr")? {
        true => Some(expanded_string(eng, lx)?),
        false => None,
    };
    let mut page = 1;
    match first_keyword(eng, lx, &["named", "page"])? {
        Some("named") => {
            let _ = expanded_text(eng, lx)?;
            warn(eng, "named pages are not supported, using page 1");
        }
        Some(_) => page = int(eng, lx)?,
        None => {}
    }
    for word in ["userpassword", "ownerpassword", "visiblefilename"] {
        if keyword(eng, lx, word)? {
            let _ = expanded_text(eng, lx)?;
        }
    }
    if keyword(eng, lx, "colorspace")? {
        let _ = int(eng, lx)?;
    }
    // `scan_pdf_box_spec`.
    let _ = first_keyword(
        eng,
        lx,
        &["mediabox", "cropbox", "bleedbox", "trimbox", "artbox"],
    )?;
    let file = expanded_string(eng, lx)?;
    let object = eng.backend.create();
    eng.backend.images.push(ImageResource {
        object,
        file,
        rule,
        page,
        attr,
    });
    eng.backend.last_image = object;
    eng.backend.last_image_pages = 1;
    Ok(())
}

/// `\saveboxresource` (`scan_pdfxform`): the keywords, the box register,
/// and a form numbered as luatex numbers one.
pub fn save_box(eng: &mut Engine, lx: &mut Lexer) -> R<()> {
    let object = eng.backend.create();
    if keyword(eng, lx, "type")? {
        let _ = int(eng, lx)?;
    }
    for word in ["attr", "resources"] {
        if keyword(eng, lx, word)? {
            let _ = expanded_text(eng, lx)?;
        }
    }
    if keyword(eng, lx, "margin")? {
        let _ = int(eng, lx)?;
    }
    let _ = int(eng, lx)?;
    eng.backend.last_box = object;
    Ok(())
}

/// `\useimageresource` (`scan_pdfrefximage`): a rule of the picture's size
/// in the list, to be drawn as the picture.
pub fn use_image(eng: &mut Engine, lx: &mut Lexer) -> R<PdfWhatsit> {
    let rule = alt_rule(eng, lx)?;
    let _ = keyword(eng, lx, "keepopen")?;
    let object = int(eng, lx)?.max(0) as u32;
    Ok(PdfWhatsit::UseImage { object, rule })
}

/// `\useboxresource` (`scan_pdfrefxform`).
pub fn use_box(eng: &mut Engine, lx: &mut Lexer) -> R<PdfWhatsit> {
    let rule = alt_rule(eng, lx)?;
    let object = int(eng, lx)?.max(0) as u32;
    Ok(PdfWhatsit::UseBox { object, rule })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `pdf_print_str`: a string that is already one is left alone.
    #[test]
    fn a_pdf_string_is_wrapped_only_when_it_is_not_one() {
        assert_eq!(pdf_string("Intro"), "(Intro)");
        assert_eq!(pdf_string("(Intro)"), "(Intro)");
        assert_eq!(pdf_string("<FEFF0041>"), "<FEFF0041>");
        assert_eq!(pdf_string("<FEFF004>"), "(<FEFF004>)");
        assert_eq!(pdf_string("<xy>"), "(<xy>)");
    }

    /// The parameter names a key expands to are the ones the tables hold.
    #[test]
    fn every_variable_is_a_parameter() {
        for key in INT_VARIABLES {
            assert!(crate::intpar::index(&parameter(key)).is_some(), "{key}");
        }
        for key in DIMEN_VARIABLES {
            assert!(
                crate::params::register_slot(&parameter(key)).is_some(),
                "{key}"
            );
        }
        for key in TOKS_VARIABLES {
            assert!(
                crate::params::toks_register(&parameter(key)).is_some(),
                "{key}"
            );
        }
    }
}
