//! The `node` library: LuaTeX's node interface, both its faces.
//!
//! A port of luatexdir/lua/lnodelib.c. LuaTeX's nodes live in its node
//! memory; Lua sees them either as userdata (`node.*`) or as the bare memory
//! index (`node.direct.*`, "direct nodes"), and the two faces are the same
//! operations over the same nodes, `node.direct.todirect` and `tonode`
//! converting between them. Here the memory is an [`Arena`] of cells, one per
//! node, indexed from 1, and a direct node IS that index.
//!
//! ## The vocabulary is LuaTeX's, whole
//!
//! Every node type luatex 1.24.0 has -- the TeX82 list items, LuaTeX's
//! `local_par`, `dir`, `boundary` and `margin_kern`, the math noads, the
//! whatsits by subtype, attribute lists, `glue_spec` -- with its fields at
//! luatex's own indices (`node.fields`), its subtype names (`node.subtypes`),
//! and the values a fresh one starts with, all read off luatex 1.24.0 rather
//! than written down from the manual. A cell keeps every field a node of its
//! type has, so a chunk can build, edit and walk any list luatex can.
//!
//! ## Where the nodes meet the engine's
//!
//! texrs's stomach builds `crate::node::Node` lists, and the callbacks that
//! hand Lua a list (`crate::node::hooks`) give it one of those. [`from_engine`]
//! makes cells of a list and [`to_engine`] makes a list of cells, for the
//! callbacks and for `node.hpack`, `node.vpack` and `node.dimensions`, which
//! are `crate::pack`'s port of §649-§679. A field `crate::node` has no room
//! for -- an attribute list, a disc's penalty, a glyph's offsets -- survives
//! for as long as the list stays in Lua, and does not survive the trip back;
//! BUGS.md keeps the list.
//!
//! ## Two divergences, measured
//!
//! - **`glue_set` is wider here.** LuaTeX builds with `glue_ratio = float`,
//!   so a box stretched by 8/3 reports `2.6666667461395`; texrs's box holds an
//!   `f64` and reports `2.6666666666666665`. Integral ratios agree exactly.
//! - **Freed cells are never reused**, where luatex reuses node memory and
//!   warns that equality "can only be trusted under very limited conditions".
//!   An index that was freed stays freed, which makes `==` mean what a chunk
//!   expects and turns a use after free into an error rather than a wrong node.

use super::{ctx, from_luatex_order, to_luatex_order};
use crate::glue::Glue;
use crate::node::{BoxNode, CharNode, DiscNode, GlueNode, GlueSign, GlueSource, LeaderKind};
use crate::node::{InsNode, Node as ENode, RuleNode};
use mlua::{Lua, MetaMethod, Table, UserData, UserDataMethods, Value, Variadic};
use std::cell::RefCell;
use std::rc::Rc;

/// How many nodes a walk follows before it decides the list is a cycle:
/// `n.next = n` is one Lua assignment away, and luatex hangs on it.
const WALK_LIMIT: usize = 1 << 20;

// ── the types ────────────────────────────────────────────────────────────

/// `node.types()` in luatex 1.24.0.
const TYPES: &[(u8, &str)] = &[
    (0, "hlist"),
    (1, "vlist"),
    (2, "rule"),
    (3, "ins"),
    (4, "mark"),
    (5, "adjust"),
    (6, "boundary"),
    (7, "disc"),
    (8, "whatsit"),
    (9, "local_par"),
    (10, "dir"),
    (11, "math"),
    (12, "glue"),
    (13, "kern"),
    (14, "penalty"),
    (15, "unset"),
    (16, "style"),
    (17, "choice"),
    (18, "noad"),
    (19, "radical"),
    (20, "fraction"),
    (21, "accent"),
    (22, "fence"),
    (23, "math_char"),
    (24, "sub_box"),
    (25, "sub_mlist"),
    (26, "math_text_char"),
    (27, "delim"),
    (28, "margin_kern"),
    (29, "glyph"),
    (30, "align_record"),
    (31, "pseudo_file"),
    (32, "pseudo_line"),
    (33, "page_insert"),
    (34, "split_insert"),
    (35, "expr_stack"),
    (36, "nested_list"),
    (37, "span"),
    (38, "attribute"),
    (39, "glue_spec"),
    (40, "attribute_list"),
    (41, "temp"),
    (42, "align_stack"),
    (43, "movement_stack"),
    (44, "if_stack"),
    (45, "unhyphenated"),
    (46, "hyphenated"),
    (47, "delta"),
    (48, "passive"),
    (49, "shape"),
];

/// `node.whatsits()` in luatex 1.24.0.
const WHATSITS: &[(i64, &str)] = &[
    (0, "open"),
    (1, "write"),
    (2, "close"),
    (3, "special"),
    (4, "late_special"),
    (7, "save_pos"),
    (8, "late_lua"),
    (9, "user_defined"),
    (16, "pdf_literal"),
    (17, "pdf_late_literal"),
    (18, "pdf_refobj"),
    (19, "pdf_annot"),
    (20, "pdf_start_link"),
    (21, "pdf_end_link"),
    (22, "pdf_dest"),
    (23, "pdf_action"),
    (24, "pdf_thread"),
    (25, "pdf_start_thread"),
    (26, "pdf_end_thread"),
    (27, "pdf_thread_data"),
    (28, "pdf_link_data"),
    (29, "pdf_colorstack"),
    (30, "pdf_setmatrix"),
    (31, "pdf_save"),
    (32, "pdf_restore"),
    (33, "pdf_link_state"),
];

const HLIST: u8 = 0;
const VLIST: u8 = 1;
const RULE: u8 = 2;
const INS: u8 = 3;
const MARK: u8 = 4;
const ADJUST: u8 = 5;
const DISC: u8 = 7;
const WHATSIT: u8 = 8;
const DIR: u8 = 10;
const MATH: u8 = 11;
const GLUE: u8 = 12;
const KERN: u8 = 13;
const PENALTY: u8 = 14;
const GLYPH: u8 = 29;
const ATTRIBUTE: u8 = 38;
const GLUE_SPEC: u8 = 39;
const ATTRIBUTE_LIST: u8 = 40;

/// LuaTeX's `UNUSED_ATTRIBUTE`: the value of an attribute that is not set.
pub(super) const UNUSED_ATTRIBUTE: i64 = -0x7FFF_FFFF;

fn type_name(id: u8) -> Option<&'static str> {
    TYPES.iter().find(|(n, _)| *n == id).map(|(_, s)| *s)
}

fn type_id(name: &str) -> Option<u8> {
    TYPES.iter().find(|(_, s)| *s == name).map(|(n, _)| *n)
}

fn whatsit_id(name: &str) -> Option<i64> {
    WHATSITS.iter().find(|(_, s)| *s == name).map(|(n, _)| *n)
}

/// The id a Lua value names: a number, or a type name.
fn want_id(v: &Value) -> mlua::Result<u8> {
    if let Some(n) = v.as_integer() {
        return u8::try_from(n)
            .ok()
            .filter(|id| type_name(*id).is_some())
            .ok_or_else(|| {
                mlua::Error::runtime(format!("invalid node id for creating a node: {n}"))
            });
    }
    if let Value::String(s) = v {
        let name = s.to_str()?;
        return type_id(&name).ok_or_else(|| {
            mlua::Error::runtime(format!("invalid node id for creating a node: {}", &*name))
        });
    }
    Err(mlua::Error::runtime("invalid node id for creating a node"))
}

// ── fields ───────────────────────────────────────────────────────────────

/// What a field holds, and what a new node starts with in it.
#[derive(Clone, Copy, PartialEq, Debug)]
enum K {
    /// An integer, starting at the value given.
    I(i64),
    /// A node pointer -- a single node or a list's head -- starting nil.
    N,
    /// `glue_set`, the one float (tex.web §109).
    F,
    /// A string, starting as given.
    S(&'static str),
    /// Any Lua value, starting nil.
    V,
    /// A token list, which luatex hands out as a table; starts empty.
    Tok,
    /// A direction, starting as given (a string) or nil.
    Dir(Option<&'static str>),
    /// A glyph's `width`, `height` or `depth`: read from the font, and a
    /// write is accepted and has no effect, as in luatex.
    Derived,
}

/// The field names of a type, in luatex's index order: `node.fields(id)`
/// numbers them from -1 when `prev` is there and from 0 when it is not.
fn names(id: u8, sub: Option<i64>) -> &'static [&'static str] {
    const BOX: &[&str] = &[
        "prev",
        "next",
        "id",
        "subtype",
        "attr",
        "width",
        "depth",
        "height",
        "dir",
        "shift",
        "glue_order",
        "glue_sign",
        "glue_set",
        "head",
    ];
    const BARE: &[&str] = &["prev", "next", "id", "subtype"];
    match id {
        HLIST | VLIST => BOX,
        RULE => &[
            "prev", "next", "id", "subtype", "attr", "width", "depth", "height", "dir", "index",
            "left", "right",
        ],
        INS => &[
            "prev", "next", "id", "subtype", "attr", "cost", "depth", "height", "spec", "head",
        ],
        MARK => &["prev", "next", "id", "subtype", "attr", "class", "mark"],
        ADJUST => &["prev", "next", "id", "subtype", "attr", "head"],
        6 => &["prev", "next", "id", "subtype", "attr", "value"],
        DISC => &[
            "prev", "next", "id", "subtype", "attr", "pre", "post", "replace", "penalty",
        ],
        WHATSIT => match sub {
            Some(0) => &[
                "prev", "next", "id", "subtype", "attr", "stream", "name", "area", "ext",
            ],
            Some(1) => &["prev", "next", "id", "subtype", "attr", "stream", "data"],
            Some(2) => &["prev", "next", "id", "subtype", "attr", "stream"],
            Some(3) | Some(4) | Some(30) => &["prev", "next", "id", "subtype", "attr", "data"],
            Some(8) => &[
                "prev", "next", "id", "subtype", "attr", "reg", "data", "name", "string",
            ],
            Some(9) => &[
                "prev", "next", "id", "subtype", "attr", "user_id", "type", "value",
            ],
            Some(16) | Some(17) => &["prev", "next", "id", "subtype", "attr", "mode", "data"],
            Some(18) => &["prev", "next", "id", "subtype", "attr", "objnum"],
            Some(19) => &[
                "prev", "next", "id", "subtype", "attr", "width", "depth", "height", "objnum",
                "data",
            ],
            Some(20) => &[
                "prev",
                "next",
                "id",
                "subtype",
                "attr",
                "width",
                "depth",
                "height",
                "objnum",
                "link_attr",
                "action",
            ],
            Some(22) => &[
                "prev",
                "next",
                "id",
                "subtype",
                "attr",
                "width",
                "depth",
                "height",
                "named_id",
                "dest_id",
                "dest_type",
                "xyz_zoom",
                "objnum",
            ],
            Some(23) => &[
                "prev",
                "next",
                "id",
                "subtype",
                "action_type",
                "named_id",
                "action_id",
                "file",
                "new_window",
                "data",
                "struct_id",
            ],
            Some(24) | Some(25) => &[
                "prev",
                "next",
                "id",
                "subtype",
                "attr",
                "width",
                "depth",
                "height",
                "named_id",
                "thread_id",
                "thread_attr",
            ],
            Some(27) | Some(28) => BARE,
            Some(29) => &[
                "prev", "next", "id", "subtype", "attr", "stack", "cmd", "data",
            ],
            Some(33) => &["prev", "next", "id", "subtype", "attr", "value"],
            _ => &["prev", "next", "id", "subtype", "attr"],
        },
        9 => &[
            "prev",
            "next",
            "id",
            "subtype",
            "attr",
            "pen_inter",
            "pen_broken",
            "dir",
            "box_left",
            "box_left_width",
            "box_right",
            "box_right_width",
        ],
        DIR => &["prev", "next", "id", "subtype", "attr", "dir", "level"],
        MATH => &[
            "prev",
            "next",
            "id",
            "subtype",
            "attr",
            "surround",
            "width",
            "stretch",
            "shrink",
            "stretch_order",
            "shrink_order",
        ],
        GLUE => &[
            "prev",
            "next",
            "id",
            "subtype",
            "attr",
            "leader",
            "width",
            "stretch",
            "shrink",
            "stretch_order",
            "shrink_order",
        ],
        KERN => &[
            "prev",
            "next",
            "id",
            "subtype",
            "attr",
            "kern",
            "expansion_factor",
        ],
        PENALTY => &["prev", "next", "id", "subtype", "attr", "penalty"],
        15 => &[
            "prev",
            "next",
            "id",
            "subtype",
            "attr",
            "width",
            "depth",
            "height",
            "dir",
            "shrink",
            "glue_order",
            "glue_sign",
            "stretch",
            "span",
            "head",
        ],
        16 => &["prev", "next", "id", "subtype", "attr", "style"],
        17 => &[
            "prev",
            "next",
            "id",
            "subtype",
            "attr",
            "display",
            "text",
            "script",
            "scriptscript",
        ],
        18 => &[
            "prev", "next", "id", "subtype", "attr", "nucleus", "sub", "sup",
        ],
        19 => &[
            "prev", "next", "id", "subtype", "attr", "nucleus", "sub", "sup", "left", "degree",
            "width", "options",
        ],
        20 => &[
            "prev", "next", "id", "subtype", "attr", "width", "num", "denom", "left", "right",
            "middle", "fam", "options",
        ],
        21 => &[
            "prev",
            "next",
            "id",
            "subtype",
            "attr",
            "nucleus",
            "sub",
            "sup",
            "accent",
            "bot_accent",
            "top_accent",
            "overlay_accent",
            "fraction",
        ],
        22 => &[
            "prev", "next", "id", "subtype", "attr", "delim", "italic", "height", "depth",
            "options", "class",
        ],
        23 | 26 => &["prev", "next", "id", "subtype", "attr", "fam", "char"],
        24 | 25 => &["prev", "next", "id", "subtype", "attr", "head"],
        27 => &[
            "prev",
            "next",
            "id",
            "subtype",
            "attr",
            "small_fam",
            "small_char",
            "large_fam",
            "large_char",
        ],
        28 => &["prev", "next", "id", "subtype", "attr", "width", "glyph"],
        GLYPH => &[
            "prev",
            "next",
            "id",
            "subtype",
            "attr",
            "char",
            "font",
            "lang",
            "left",
            "right",
            "uchyph",
            "components",
            "xoffset",
            "yoffset",
            "width",
            "height",
            "depth",
            "expansion_factor",
            "data",
        ],
        33 => &[
            "prev",
            "next",
            "id",
            "subtype",
            "height",
            "last_ins_ptr",
            "best_ins_ptr",
            "width",
            "stretch",
            "shrink",
            "stretch_order",
            "shrink_order",
        ],
        34 => &[
            "prev",
            "next",
            "id",
            "subtype",
            "height",
            "last_ins_ptr",
            "best_ins_ptr",
            "broken_ptr",
            "broken_ins",
        ],
        ATTRIBUTE => &["next", "id", "number", "value"],
        GLUE_SPEC => &[
            "next",
            "id",
            "width",
            "stretch",
            "shrink",
            "stretch_order",
            "shrink_order",
        ],
        ATTRIBUTE_LIST => &["next", "id"],
        _ => BARE,
    }
}

/// What field `name` of a node of type `id` holds, and starts as; `None` when
/// the type has no such field. The starting values are luatex 1.24.0's for a
/// node fresh from `node.new`.
fn kind(id: u8, sub: Option<i64>, name: &str) -> Option<K> {
    if !names(id, sub).contains(&name) {
        return None;
    }
    Some(match (id, name) {
        (_, "attr") => K::N,
        (HLIST | VLIST, "dir") => K::Dir(None),
        (RULE, "dir") => K::Dir(None),
        (9, "dir") | (15, "dir") => K::Dir(Some("TLT")),
        (DIR, "dir") => K::Dir(Some("+TLT")),
        (HLIST | VLIST, "glue_set") => K::F,
        (RULE, "width" | "depth" | "height") => K::I(crate::node::NULL_FLAG),
        (RULE, "left" | "right") => K::I(0),
        (15, "width") => K::I(crate::node::NULL_FLAG),
        (GLYPH, "width" | "height" | "depth") => K::Derived,
        (GLYPH, "left" | "right") => K::I(1),
        (20, "fam") => K::I(-1),
        (16, "style") => K::S("display"),
        (MARK, "mark") => K::Tok,
        (
            _,
            "head" | "spec" | "pre" | "post" | "replace" | "leader" | "components" | "span"
            | "nucleus" | "sub" | "sup" | "left" | "right" | "middle" | "num" | "denom" | "degree"
            | "accent" | "bot_accent" | "top_accent" | "overlay_accent" | "delim" | "display"
            | "text" | "script" | "scriptscript" | "glyph" | "box_left" | "box_right",
        ) => K::N,
        (WHATSIT, "name" | "area" | "ext" | "file" | "link_attr" | "thread_attr") => K::S(""),
        (WHATSIT, "data") => match sub {
            Some(16) | Some(17) => K::S("data"),
            _ => K::S(""),
        },
        (WHATSIT, "string") => K::S(""),
        (WHATSIT, "value") => match sub {
            Some(9) => K::I(0),
            _ => K::V,
        },
        (WHATSIT, "reg" | "action" | "struct_id" | "cmd") => K::V,
        (33 | 34, _) if !matches!(name, "prev" | "next" | "id" | "subtype") => K::V,
        _ => K::I(0),
    })
}

/// A field value as a cell holds it.
#[derive(Clone, Debug)]
enum FV {
    I(i64),
    F(f64),
    N(usize),
    V(Value),
}

// ── the arena ────────────────────────────────────────────────────────────

/// One node.
#[derive(Clone, Debug)]
struct Cell {
    id: u8,
    /// `None` for the types luatex gives no subtype (`local_par`, the
    /// internal ones), which read as nil.
    subtype: Option<i64>,
    prev: Option<usize>,
    next: Option<usize>,
    /// The fields, by name, set ones only; an absent one has its default.
    vals: Vec<(&'static str, FV)>,
    /// A glyph's dimensions as the stomach measured it, when the list came
    /// from the engine: the font then is the stomach's and not reachable
    /// while it is building a list.
    measured: Option<(i64, i64, i64)>,
}

impl Cell {
    fn get(&self, name: &str) -> Option<&FV> {
        self.vals.iter().find(|(n, _)| *n == name).map(|(_, v)| v)
    }

    fn set(&mut self, name: &'static str, v: Option<FV>) {
        self.vals.retain(|(n, _)| *n != name);
        if let Some(v) = v {
            self.vals.push((name, v));
        }
    }

    fn int(&self, name: &str) -> i64 {
        match self.get(name) {
            Some(FV::I(n)) => *n,
            Some(FV::F(f)) => *f as i64,
            _ => match kind(self.id, self.subtype, name) {
                Some(K::I(d)) => d,
                _ => 0,
            },
        }
    }

    fn node(&self, name: &str) -> Option<usize> {
        match self.get(name) {
            Some(FV::N(n)) => Some(*n),
            _ => None,
        }
    }
}

/// The nodes one document's Lua state holds, and the properties table.
#[derive(Default)]
pub(super) struct Arena {
    cells: Vec<Option<Cell>>,
    properties: Option<mlua::RegistryKey>,
    /// `set_properties_mode(true, ...)`: copying a node copies its property.
    copy_properties: bool,
}

/// The `field` and `static` names `canonical` folds onto the stored ones.
fn canonical(name: &str) -> &str {
    match name {
        "list" => "head",
        other => other,
    }
}

impl Arena {
    fn make(&mut self, id: u8, subtype: Option<i64>) -> usize {
        self.cells.push(Some(Cell {
            id,
            subtype,
            prev: None,
            next: None,
            vals: Vec::new(),
            measured: None,
        }));
        self.cells.len()
    }

    fn cell(&self, at: usize) -> mlua::Result<&Cell> {
        self.cells
            .get(at.wrapping_sub(1))
            .and_then(|c| c.as_ref())
            .ok_or_else(|| mlua::Error::runtime("this node has been freed"))
    }

    fn cell_mut(&mut self, at: usize) -> mlua::Result<&mut Cell> {
        self.cells
            .get_mut(at.wrapping_sub(1))
            .and_then(|c| c.as_mut())
            .ok_or_else(|| mlua::Error::runtime("this node has been freed"))
    }

    fn alive(&self, at: usize) -> bool {
        self.cells
            .get(at.wrapping_sub(1))
            .is_some_and(|c| c.is_some())
    }

    fn id(&self, at: usize) -> mlua::Result<u8> {
        Ok(self.cell(at)?.id)
    }

    fn next(&self, at: usize) -> mlua::Result<Option<usize>> {
        Ok(self.cell(at)?.next)
    }

    fn prev(&self, at: usize) -> mlua::Result<Option<usize>> {
        Ok(self.cell(at)?.prev)
    }

    fn set_next(&mut self, at: usize, to: Option<usize>) -> mlua::Result<()> {
        self.cell_mut(at)?.next = to;
        Ok(())
    }

    fn set_prev(&mut self, at: usize, to: Option<usize>) -> mlua::Result<()> {
        self.cell_mut(at)?.prev = to;
        Ok(())
    }

    /// `a.next = b; b.prev = a`, either side may be absent.
    fn link(&mut self, a: Option<usize>, b: Option<usize>) -> mlua::Result<()> {
        if let Some(a) = a {
            self.set_next(a, b)?;
        }
        if let Some(b) = b {
            self.set_prev(b, a)?;
        }
        Ok(())
    }

    /// The nodes of a list from `head` up to (not including) `stop`.
    fn chain(&self, head: usize, stop: Option<usize>) -> mlua::Result<Vec<usize>> {
        let mut out = Vec::new();
        let mut at = Some(head);
        while let Some(i) = at {
            if Some(i) == stop {
                break;
            }
            if out.len() >= WALK_LIMIT {
                return Err(mlua::Error::runtime(
                    "this node list has no end: it was followed a million nodes \
                     without reaching nil, so `next` points back into it",
                ));
            }
            out.push(i);
            at = self.cell(i)?.next;
        }
        Ok(out)
    }

    fn tail(&self, head: usize) -> mlua::Result<usize> {
        Ok(*self.chain(head, None)?.last().unwrap_or(&head))
    }

    /// `copy_node`: a deep copy, nested lists included, `next`/`prev` not.
    fn copy(&mut self, at: usize) -> mlua::Result<usize> {
        let mut cell = self.cell(at)?.clone();
        cell.next = None;
        cell.prev = None;
        let nodes: Vec<(&'static str, usize)> = cell
            .vals
            .iter()
            .filter_map(|(n, v)| match v {
                FV::N(i) if *n != "attr" => Some((*n, *i)),
                _ => None,
            })
            .collect();
        for (name, head) in nodes {
            // A single-node field (`leader`, `spec`, the noad slots) is a list
            // of one; copy_list copies the whole of whatever hangs there.
            let copied = self.copy_list(head, None)?;
            cell.set(name, copied.map(FV::N));
        }
        self.cells.push(Some(cell));
        Ok(self.cells.len())
    }

    fn copy_list(&mut self, head: usize, stop: Option<usize>) -> mlua::Result<Option<usize>> {
        let chain = self.chain(head, stop)?;
        let mut made: Vec<usize> = Vec::with_capacity(chain.len());
        for at in chain {
            made.push(self.copy(at)?);
        }
        for w in made.windows(2) {
            self.link(Some(w[0]), Some(w[1]))?;
        }
        Ok(made.first().copied())
    }

    /// `flush_node`: the node and what hangs from it; the caller unlinks.
    fn flush(&mut self, at: usize) -> mlua::Result<()> {
        let nodes: Vec<usize> = self
            .cell(at)?
            .vals
            .iter()
            .filter_map(|(n, v)| match v {
                FV::N(i) if *n != "attr" => Some(*i),
                _ => None,
            })
            .collect();
        for head in nodes {
            if self.alive(head) {
                self.flush_list(head)?;
            }
        }
        self.cells[at - 1] = None;
        Ok(())
    }

    fn flush_list(&mut self, head: usize) -> mlua::Result<()> {
        for at in self.chain(head, None)? {
            self.flush(at)?;
        }
        Ok(())
    }

    /// Unlink `at` from its neighbours, which are joined.
    fn unlink(&mut self, at: usize) -> mlua::Result<()> {
        let (p, n) = {
            let c = self.cell(at)?;
            (c.prev, c.next)
        };
        if let Some(p) = p {
            if self.alive(p) && self.next(p)? == Some(at) {
                self.set_next(p, n)?;
            }
        }
        if let Some(n) = n {
            if self.alive(n) && self.prev(n)? == Some(at) {
                self.set_prev(n, p)?;
            }
        }
        let c = self.cell_mut(at)?;
        c.prev = None;
        c.next = None;
        Ok(())
    }

    // ── attributes ───────────────────────────────────────────────────────

    /// The (number, value) pairs of a node's attribute list, sorted.
    fn attributes(&self, at: usize) -> mlua::Result<Vec<(i64, i64)>> {
        let Some(list) = self.cell(at)?.node("attr") else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        let mut p = self.next(list)?;
        while let Some(a) = p {
            let c = self.cell(a)?;
            out.push((c.int("number"), c.int("value")));
            p = c.next;
        }
        Ok(out)
    }

    /// A fresh attribute list holding `pairs`: an `attribute_list` node and an
    /// `attribute` node per pair, in number order.
    fn make_attributes(&mut self, pairs: &[(i64, i64)]) -> mlua::Result<usize> {
        let head = self.make(ATTRIBUTE_LIST, None);
        let mut last = head;
        for &(number, value) in pairs {
            let a = self.make(ATTRIBUTE, None);
            let c = self.cell_mut(a)?;
            c.set("number", Some(FV::I(number)));
            c.set("value", Some(FV::I(value)));
            self.set_next(last, Some(a))?;
            last = a;
        }
        Ok(head)
    }

    /// Give `at` an attribute list of its own with `pairs` in it. luatex's
    /// lists are shared and reference counted; a change copies (`attr_list_cow`),
    /// which is what building a new one is.
    fn set_attributes(&mut self, at: usize, pairs: &[(i64, i64)]) -> mlua::Result<()> {
        let list = match pairs.is_empty() {
            true => None,
            false => Some(self.make_attributes(pairs)?),
        };
        self.cell_mut(at)?.set("attr", list.map(FV::N));
        Ok(())
    }
}

// ── the two faces ────────────────────────────────────────────────────────

/// A Lua handle on one cell: LuaTeX's node userdata.
#[derive(Clone)]
pub(super) struct NodeRef {
    arena: Rc<RefCell<Arena>>,
    at: usize,
}

/// Which face a function belongs to.
#[derive(Clone, Copy, PartialEq)]
enum Face {
    User,
    Direct,
}

/// The arena and the face a library function hands nodes out in.
#[derive(Clone)]
struct Lib {
    arena: Rc<RefCell<Arena>>,
    face: Face,
}

impl Lib {
    /// The index a Lua value stands for: a userdata node or a direct node
    /// (an integer). Either is accepted by both faces.
    fn opt(&self, v: &Value) -> mlua::Result<Option<usize>> {
        match v {
            Value::Nil | Value::Boolean(false) => Ok(None),
            Value::Integer(n) if *n > 0 => Ok(Some(*n as usize)),
            Value::Integer(_) => Ok(None),
            Value::Number(f) if *f >= 1.0 => Ok(Some(*f as usize)),
            Value::UserData(u) => Ok(Some(u.borrow::<NodeRef>()?.at)),
            _ => Err(mlua::Error::runtime(format!(
                "expected a node, not an object with type {}",
                v.type_name()
            ))),
        }
    }

    fn want(&self, v: Option<&Value>) -> mlua::Result<usize> {
        self.opt(v.unwrap_or(&Value::Nil))?
            .ok_or_else(|| mlua::Error::runtime("expected a node"))
    }

    /// A node as this face hands it out.
    fn give(&self, lua: &Lua, at: Option<usize>) -> mlua::Result<Value> {
        Ok(match at {
            None => Value::Nil,
            Some(at) => match self.face {
                Face::Direct => Value::Integer(at as i64),
                Face::User => Value::UserData(lua.create_userdata(NodeRef {
                    arena: Rc::clone(&self.arena),
                    at,
                })?),
            },
        })
    }

    fn user(&self) -> Lib {
        Lib {
            arena: Rc::clone(&self.arena),
            face: Face::User,
        }
    }
}

// ── fields, read and written ─────────────────────────────────────────────

/// A glyph's width, height and depth: from the Lua font it names, or what the
/// stomach measured when the list came from it, or from `\font`'s table while
/// a chunk runs; zero for font 0, as in luatex.
fn glyph_whd(cell: &Cell) -> (i64, i64, i64) {
    let font = cell.int("font");
    let ch = cell.int("char");
    if let Some(f) = super::font::get(font as usize) {
        if let Some(c) = f.characters.get(&ch) {
            return (c.width, c.height, c.depth);
        }
        return (0, 0, 0);
    }
    if let Some(m) = cell.measured {
        return m;
    }
    if font > 0 {
        let looked = ctx::with("glyph", |eng, _| {
            let f = eng.fonts.get(font as u16);
            Ok(char::from_u32(ch as u32)
                .and_then(|c| f.metrics(c))
                .map(|m| (m.width, m.height, m.depth)))
        });
        if let Ok(Some(m)) = looked {
            return m;
        }
    }
    (0, 0, 0)
}

/// The Lua value of field `name`, through `lib`'s face; `nil` for a name the
/// type has not got, which is luatex's answer.
fn read(lib: &Lib, lua: &Lua, at: usize, name: &str) -> mlua::Result<Value> {
    let name = canonical(name);
    let (value, node) = {
        let a = lib.arena.borrow();
        let cell = a.cell(at)?;
        match name {
            "id" => return Ok(Value::Integer(cell.id as i64)),
            "subtype" => {
                return Ok(match (cell.id, cell.subtype) {
                    (ATTRIBUTE | GLUE_SPEC | ATTRIBUTE_LIST, _) => Value::Nil,
                    (_, Some(s)) => Value::Integer(s),
                    (_, None) => Value::Nil,
                })
            }
            "next" => (None, cell.next),
            "prev" => match names(cell.id, cell.subtype).contains(&"prev") {
                true => (None, cell.prev),
                false => return Ok(Value::Nil),
            },
            _ => match kind(cell.id, cell.subtype, name) {
                None => return Ok(Value::Nil),
                Some(K::Derived) => {
                    let (w, h, d) = glyph_whd(cell);
                    let v = match name {
                        "width" => w,
                        "height" => h,
                        _ => d,
                    };
                    return Ok(Value::Integer(v));
                }
                Some(k) => match (cell.get(name), k) {
                    (Some(FV::I(n)), _) => return Ok(Value::Integer(*n)),
                    (Some(FV::F(f)), _) => return Ok(Value::Number(*f)),
                    (Some(FV::N(n)), _) => (None, Some(*n)),
                    (Some(FV::V(v)), _) => (Some(v.clone()), None),
                    (None, K::I(d)) => return Ok(Value::Integer(d)),
                    (None, K::F) => return Ok(Value::Number(0.0)),
                    (None, K::S(s)) => (Some(Value::String(lua.create_string(s)?)), None),
                    (None, K::Dir(d)) => match d {
                        Some(s) => (Some(Value::String(lua.create_string(s)?)), None),
                        None => return Ok(Value::Nil),
                    },
                    (None, K::Tok) => (Some(Value::Table(lua.create_table()?)), None),
                    (None, _) => return Ok(Value::Nil),
                },
            },
        }
    };
    match (value, node) {
        (Some(v), _) => Ok(v),
        (None, n) => lib.give(lua, n),
    }
}

/// Set field `name`. luatex's own wording for a field the type has not got.
fn write(lib: &Lib, at: usize, name: &str, value: Value) -> mlua::Result<()> {
    let name = canonical(name);
    let (id, sub) = {
        let a = lib.arena.borrow();
        let c = a.cell(at)?;
        (c.id, c.subtype)
    };
    match name {
        "id" => {
            return Err(mlua::Error::runtime(
                "a node's `id` is its type and is fixed at node.new; texrs does not let \
                 it change, because the fields already stored would then belong to no type",
            ))
        }
        "subtype" => {
            let n = want_int(&value)?;
            lib.arena.borrow_mut().cell_mut(at)?.subtype = Some(n);
            return Ok(());
        }
        "next" | "prev" => {
            let to = lib.opt(&value)?;
            let mut a = lib.arena.borrow_mut();
            match name {
                "next" => a.set_next(at, to)?,
                _ => a.set_prev(at, to)?,
            }
            return Ok(());
        }
        _ => {}
    }
    let Some(k) = kind(id, sub, name) else {
        return Err(mlua::Error::runtime(format!(
            "You cannot set field {name} in a node of type {}",
            type_name(id).unwrap_or("?")
        )));
    };
    let stored: &'static str = names(id, sub)
        .iter()
        .copied()
        .find(|n| *n == name)
        .expect("named");
    let v = match k {
        K::Derived => return Ok(()),
        K::I(_) => match &value {
            Value::Nil => None,
            Value::Boolean(b) => Some(FV::I(i64::from(*b))),
            _ => Some(FV::I(want_int(&value)?)),
        },
        K::F => Some(FV::F(
            value
                .as_number()
                .or_else(|| value.as_integer().map(|n| n as f64))
                .unwrap_or(0.0),
        )),
        K::N => lib.opt(&value)?.map(FV::N),
        K::S(_) | K::V | K::Tok | K::Dir(_) => match value {
            Value::Nil => None,
            other => Some(FV::V(other)),
        },
    };
    if stored == "stretch_order" || stored == "shrink_order" || stored == "glue_order" {
        if let Some(FV::I(n)) = &v {
            if !(0..=4).contains(n) {
                return Err(mlua::Error::runtime(format!("bad glue order {n}")));
            }
        }
    }
    lib.arena.borrow_mut().cell_mut(at)?.set(stored, v);
    Ok(())
}

fn want_int(v: &Value) -> mlua::Result<i64> {
    v.as_integer()
        .or_else(|| v.as_number().map(|n| n.round() as i64))
        .ok_or_else(|| mlua::Error::runtime("a node field wants a number"))
}

impl NodeRef {
    fn lib(&self) -> Lib {
        Lib {
            arena: Rc::clone(&self.arena),
            face: Face::User,
        }
    }

    /// luatex's `tostring`: `<node    prev <    this >    next : type subtype>`.
    fn show(&self) -> mlua::Result<String> {
        let a = self.arena.borrow();
        let cell = a.cell(self.at)?;
        let at = |i: Option<usize>| match i {
            Some(n) => n.to_string(),
            None => "nil".to_string(),
        };
        Ok(format!(
            "<node {:>7} < {:>7} > {:>7} : {} {}>",
            at(cell.prev),
            self.at,
            at(cell.next),
            type_name(cell.id).unwrap_or("?"),
            cell.subtype.unwrap_or(0)
        ))
    }
}

impl UserData for NodeRef {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::Index, |lua, this, key: Value| match &key {
            Value::String(s) => read(&this.lib(), lua, this.at, &s.to_str()?),
            _ => Ok(Value::Nil),
        });
        methods.add_meta_method(
            MetaMethod::NewIndex,
            |_, this, (key, value): (Value, Value)| {
                let Value::String(s) = &key else {
                    return Err(mlua::Error::runtime("a node field is named by a string"));
                };
                write(&this.lib(), this.at, &s.to_str()?, value)
            },
        );
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| this.show());
        // "you are actually comparing indices into the node memory".
        methods.add_meta_method(MetaMethod::Eq, |_, this, other: mlua::AnyUserData| {
            Ok(other
                .borrow::<NodeRef>()
                .map(|o| o.at == this.at)
                .unwrap_or(false))
        });
    }
}

// ── to and from the engine's node list ───────────────────────────────────

/// LuaTeX's glue subtypes (`node.subtypes("glue")`) and texrs's glue
/// sources, one for one: 1-15 the §224 parameters in their order, 16 a math
/// skip (texrs has none and reads it as ordinary glue), 17-19 the three mu
/// parameters, 98 `\nonscript`, 99 an `\mskip`.
const SOURCES: [(i64, GlueSource); 20] = [
    (0, GlueSource::Normal),
    (1, GlueSource::LineSkip),
    (2, GlueSource::BaselineSkip),
    (3, GlueSource::ParSkip),
    (4, GlueSource::AboveDisplaySkip),
    (5, GlueSource::BelowDisplaySkip),
    (6, GlueSource::AboveDisplayShortSkip),
    (7, GlueSource::BelowDisplayShortSkip),
    (8, GlueSource::LeftSkip),
    (9, GlueSource::RightSkip),
    (10, GlueSource::TopSkip),
    (11, GlueSource::SplitTopSkip),
    (12, GlueSource::TabSkip),
    (13, GlueSource::SpaceSkip),
    (14, GlueSource::XSpaceSkip),
    (15, GlueSource::ParFillSkip),
    (17, GlueSource::ThinMuSkip),
    (18, GlueSource::MedMuSkip),
    (19, GlueSource::ThickMuSkip),
    (98, GlueSource::CondMath),
];

fn glue_source(subtype: i64) -> GlueSource {
    match subtype {
        99 => GlueSource::MuGlue,
        s => SOURCES
            .iter()
            .find(|(n, _)| *n == s)
            .map(|(_, g)| *g)
            .unwrap_or(GlueSource::Normal),
    }
}

fn source_subtype(s: GlueSource) -> i64 {
    match s {
        GlueSource::MuGlue => 99,
        s => SOURCES
            .iter()
            .find(|(_, g)| *g == s)
            .map(|(n, _)| *n)
            .unwrap_or(0),
    }
}

/// LuaTeX's leader subtypes: 100 `\leaders`, 101 `\cleaders`, 102 `\xleaders`.
fn leader_kind(subtype: i64) -> LeaderKind {
    match subtype {
        100 => LeaderKind::Aligned,
        101 => LeaderKind::Centred,
        102 => LeaderKind::Expanded,
        _ => LeaderKind::Normal,
    }
}

fn glue_sign(n: i64) -> GlueSign {
    match n {
        1 => GlueSign::Stretching,
        2 => GlueSign::Shrinking,
        _ => GlueSign::Normal,
    }
}

fn glue_sign_number(s: GlueSign) -> i64 {
    match s {
        GlueSign::Normal => 0,
        GlueSign::Stretching => 1,
        GlueSign::Shrinking => 2,
    }
}

/// One cell as the node `crate::pack` and the stomach take.
fn to_engine(arena: &Arena, at: usize) -> mlua::Result<Vec<ENode>> {
    let cell = arena.cell(at)?;
    let list = |name: &str| -> mlua::Result<Vec<ENode>> {
        match cell.node(name) {
            Some(head) => list_to_engine(arena, head),
            None => Ok(Vec::new()),
        }
    };
    let glue = |c: &Cell| -> mlua::Result<Glue> {
        Ok(Glue {
            natural: c.int("width"),
            stretch: c.int("stretch"),
            stretch_order: from_luatex_order(c.int("stretch_order"))?,
            shrink: c.int("shrink"),
            shrink_order: from_luatex_order(c.int("shrink_order"))?,
        })
    };
    let sub = cell.subtype.unwrap_or(0);
    Ok(vec![match cell.id {
        HLIST | VLIST => ENode::Box(BoxNode {
            width: cell.int("width"),
            depth: cell.int("depth"),
            height: cell.int("height"),
            shift_amount: cell.int("shift"),
            list: list("head")?,
            glue_set: match cell.get("glue_set") {
                Some(FV::F(f)) => *f,
                _ => 0.0,
            },
            glue_sign: glue_sign(cell.int("glue_sign")),
            glue_order: from_luatex_order(cell.int("glue_order"))?,
            vertical: cell.id == VLIST,
        }),
        RULE => ENode::Rule(RuleNode {
            width: cell.int("width"),
            depth: cell.int("depth"),
            height: cell.int("height"),
        }),
        INS => ENode::Ins(InsNode {
            number: sub.clamp(0, 255) as u8,
            height: cell.int("height"),
            depth: cell.int("depth"),
            float_cost: cell.int("cost"),
            split_top_skip: match cell.node("spec") {
                Some(s) => glue(arena.cell(s)?)?,
                None => Glue::default(),
            },
            list: list("head")?,
        }),
        MARK => ENode::Mark(match cell.get("mark") {
            Some(FV::V(Value::String(s))) => s.to_str()?.to_string(),
            _ => String::new(),
        }),
        ADJUST => ENode::Adjust(list("head")?),
        // A LuaTeX disc holds its replacement as a list; tex.web's holds the
        // count of the nodes after it that it replaces, so the replacement is
        // put back in the list behind it.
        DISC => {
            let replace = list("replace")?;
            let mut out = vec![ENode::Disc(DiscNode {
                pre_break: list("pre")?,
                post_break: list("post")?,
                replace_count: replace.len(),
            })];
            out.extend(replace);
            return Ok(out);
        }
        WHATSIT => ENode::Whatsit(match cell.get("data") {
            Some(FV::V(Value::String(s))) => s.to_str()?.to_string(),
            _ => String::new(),
        }),
        MATH => ENode::Math(cell.int("surround")),
        GLUE => ENode::Glue(GlueNode {
            spec: glue(cell)?,
            kind: leader_kind(sub),
            source: glue_source(sub),
            leader: match cell.node("leader") {
                Some(at) => to_engine(arena, at)?.into_iter().next().map(Box::new),
                None => None,
            },
        }),
        // tex.web §155: subtype `explicit` is 1.
        KERN => ENode::Kern {
            width: cell.int("kern"),
            explicit: sub == 1,
        },
        PENALTY => ENode::Penalty(cell.int("penalty")),
        GLYPH => {
            let (width, height, depth) = glyph_whd(cell);
            let ch = CharNode {
                font: cell.int("font").max(0) as usize,
                character: char::from_u32(cell.int("char") as u32).unwrap_or('\0'),
                width,
                height,
                depth,
            };
            // A ligature (subtype bit 2) keeps its components.
            match sub & 2 != 0 {
                true => {
                    let mut lig = crate::node::Ligature::default();
                    if let Some(c) = cell.node("components") {
                        for at in arena.chain(c, None)? {
                            let comp = arena.cell(at)?;
                            if let Some(ch) = char::from_u32(comp.int("char") as u32) {
                                lig.components.push(ch);
                            }
                        }
                    }
                    ENode::Ligature(ch, lig)
                }
                false => ENode::Char(ch),
            }
        }
        // The types the stomach has no item for leave no node behind: a
        // `dir`, a `local_par`, a `boundary`, a margin kern.
        _ => return Ok(Vec::new()),
    }])
}

/// A list of cells, as the stomach's list.
pub(super) fn list_to_engine(arena: &Arena, head: usize) -> mlua::Result<Vec<ENode>> {
    let mut out = Vec::new();
    for at in arena.chain(head, None)? {
        out.extend(to_engine(arena, at)?);
    }
    Ok(out)
}

/// A stomach list as cells, the head of them, `None` for an empty list.
pub(super) fn from_engine(arena: &mut Arena, list: &[ENode]) -> mlua::Result<Option<usize>> {
    let mut made: Vec<usize> = Vec::with_capacity(list.len());
    let mut i = 0;
    while i < list.len() {
        let at = match &list[i] {
            ENode::Disc(d) => {
                // The `replace_count` nodes after a tex.web disc are LuaTeX's
                // `replace` list.
                let at = arena.make(DISC, Some(0));
                let pre = from_engine(arena, &d.pre_break)?;
                let post = from_engine(arena, &d.post_break)?;
                let end = (i + 1 + d.replace_count).min(list.len());
                let replace = from_engine(arena, &list[i + 1..end])?;
                let c = arena.cell_mut(at)?;
                c.set("pre", pre.map(FV::N));
                c.set("post", post.map(FV::N));
                c.set("replace", replace.map(FV::N));
                i = end;
                made.push(at);
                continue;
            }
            other => one_from_engine(arena, other)?,
        };
        made.push(at);
        i += 1;
    }
    for w in made.windows(2) {
        arena.link(Some(w[0]), Some(w[1]))?;
    }
    Ok(made.first().copied())
}

fn one_from_engine(arena: &mut Arena, n: &ENode) -> mlua::Result<usize> {
    Ok(match n {
        ENode::Box(b) => {
            let at = arena.make(if b.vertical { VLIST } else { HLIST }, Some(0));
            let head = from_engine(arena, &b.list)?;
            let c = arena.cell_mut(at)?;
            c.set("width", Some(FV::I(b.width)));
            c.set("depth", Some(FV::I(b.depth)));
            c.set("height", Some(FV::I(b.height)));
            c.set("shift", Some(FV::I(b.shift_amount)));
            c.set("glue_set", Some(FV::F(b.glue_set)));
            c.set("glue_sign", Some(FV::I(glue_sign_number(b.glue_sign))));
            c.set("glue_order", Some(FV::I(to_luatex_order(b.glue_order))));
            c.set("head", head.map(FV::N));
            at
        }
        ENode::Rule(r) => {
            let at = arena.make(RULE, Some(0));
            let c = arena.cell_mut(at)?;
            c.set("width", Some(FV::I(r.width)));
            c.set("depth", Some(FV::I(r.depth)));
            c.set("height", Some(FV::I(r.height)));
            at
        }
        ENode::Ins(ins) => {
            let at = arena.make(INS, Some(i64::from(ins.number)));
            let head = from_engine(arena, &ins.list)?;
            let spec = arena.make(GLUE_SPEC, None);
            set_glue(arena.cell_mut(spec)?, &ins.split_top_skip);
            let c = arena.cell_mut(at)?;
            c.set("height", Some(FV::I(ins.height)));
            c.set("depth", Some(FV::I(ins.depth)));
            c.set("cost", Some(FV::I(ins.float_cost)));
            c.set("spec", Some(FV::N(spec)));
            c.set("head", head.map(FV::N));
            at
        }
        ENode::Mark(text) => {
            let at = arena.make(MARK, Some(0));
            // The text stands for the token list; a Lua value of its own.
            let lua_text = super::with_lua(|lua| lua.create_string(text).map(Value::String));
            if let Some(Ok(v)) = lua_text {
                arena.cell_mut(at)?.set("mark", Some(FV::V(v)));
            }
            at
        }
        ENode::Adjust(list) => {
            let at = arena.make(ADJUST, Some(0));
            let head = from_engine(arena, list)?;
            arena.cell_mut(at)?.set("head", head.map(FV::N));
            at
        }
        ENode::Whatsit(text) => {
            let at = arena.make(WHATSIT, Some(3));
            let lua_text = super::with_lua(|lua| lua.create_string(text).map(Value::String));
            if let Some(Ok(v)) = lua_text {
                arena.cell_mut(at)?.set("data", Some(FV::V(v)));
            }
            at
        }
        ENode::Math(s) => {
            let at = arena.make(MATH, Some(0));
            arena.cell_mut(at)?.set("surround", Some(FV::I(*s)));
            at
        }
        ENode::Glue(g) => {
            let sub = match g.kind {
                LeaderKind::Aligned => 100,
                LeaderKind::Centred => 101,
                LeaderKind::Expanded => 102,
                LeaderKind::Normal => source_subtype(g.source),
            };
            let at = arena.make(GLUE, Some(sub));
            let leader = match &g.leader {
                Some(l) => Some(one_from_engine(arena, l)?),
                None => None,
            };
            let c = arena.cell_mut(at)?;
            set_glue(c, &g.spec);
            c.set("leader", leader.map(FV::N));
            at
        }
        ENode::Kern { width, explicit } => {
            let at = arena.make(KERN, Some(i64::from(*explicit)));
            arena.cell_mut(at)?.set("kern", Some(FV::I(*width)));
            at
        }
        ENode::Penalty(p) => {
            let at = arena.make(PENALTY, Some(0));
            arena.cell_mut(at)?.set("penalty", Some(FV::I(*p)));
            at
        }
        ENode::Char(ch) => glyph_from_engine(arena, ch, 1)?,
        ENode::Ligature(ch, lig) => {
            let at = glyph_from_engine(arena, ch, 2)?;
            let mut comps = Vec::new();
            for c in &lig.components {
                let g = arena.make(GLYPH, Some(1));
                let cell = arena.cell_mut(g)?;
                cell.set("char", Some(FV::I(*c as i64)));
                cell.set("font", Some(FV::I(ch.font as i64)));
                comps.push(g);
            }
            for w in comps.windows(2) {
                arena.link(Some(w[0]), Some(w[1]))?;
            }
            arena
                .cell_mut(at)?
                .set("components", comps.first().copied().map(FV::N));
            at
        }
        ENode::Disc(_) => unreachable!("a disc is made with its replacement by from_engine"),
    })
}

fn set_glue(c: &mut Cell, g: &Glue) {
    c.set("width", Some(FV::I(g.natural)));
    c.set("stretch", Some(FV::I(g.stretch)));
    c.set("shrink", Some(FV::I(g.shrink)));
    c.set(
        "stretch_order",
        Some(FV::I(to_luatex_order(g.stretch_order))),
    );
    c.set("shrink_order", Some(FV::I(to_luatex_order(g.shrink_order))));
}

fn glyph_from_engine(arena: &mut Arena, ch: &CharNode, subtype: i64) -> mlua::Result<usize> {
    let at = arena.make(GLYPH, Some(subtype));
    let c = arena.cell_mut(at)?;
    c.set("char", Some(FV::I(ch.character as i64)));
    c.set("font", Some(FV::I(ch.font as i64)));
    c.measured = Some((ch.width, ch.height, ch.depth));
    Ok(at)
}

// ── the library ──────────────────────────────────────────────────────────

/// A library function: it gets the face it was registered under.
type Fun = fn(&Lib, &Lua, Variadic<Value>) -> mlua::Result<Variadic<Value>>;

fn one(v: Value) -> mlua::Result<Variadic<Value>> {
    Ok(Variadic::from_iter([v]))
}

fn none() -> mlua::Result<Variadic<Value>> {
    Ok(Variadic::new())
}

fn int(n: i64) -> Value {
    Value::Integer(n)
}

fn arg(args: &Variadic<Value>, i: usize) -> &Value {
    args.get(i).unwrap_or(&Value::Nil)
}

/// `node.new(id [, subtype])`, a whatsit's subtype by number or name.
fn f_new(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let id = want_id(arg(&args, 0))?;
    let sub = match (id, arg(&args, 1)) {
        (WHATSIT, Value::String(s)) => Some(
            whatsit_id(&s.to_str()?)
                .ok_or_else(|| mlua::Error::runtime("invalid whatsit type id"))?,
        ),
        (WHATSIT, Value::Nil) => {
            return Err(mlua::Error::runtime(
                "Creating a whatsit requires the subtype number as a second argument",
            ))
        }
        (_, Value::Nil) => Some(0),
        (_, v) => Some(want_int(v)?),
    };
    if id == WHATSIT && !WHATSITS.iter().any(|(n, _)| Some(*n) == sub) {
        return Err(mlua::Error::runtime("invalid whatsit type id"));
    }
    let sub = match id {
        9 | 30..=32 | 35..=37 | 41..=49 => None,
        ATTRIBUTE | GLUE_SPEC | ATTRIBUTE_LIST => None,
        _ => sub,
    };
    let at = lib.arena.borrow_mut().make(id, sub);
    // A new node carries the attributes in force, as luatex's `new_node` gives
    // it the current attribute list.
    if names(id, sub).contains(&"attr") {
        let pairs = super::current_attributes();
        if !pairs.is_empty() {
            lib.arena.borrow_mut().set_attributes(at, &pairs)?;
        }
    }
    one(lib.give(lua, Some(at))?)
}

fn f_free(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let at = lib.want(args.first())?;
    let mut a = lib.arena.borrow_mut();
    let next = a.next(at)?;
    a.unlink(at)?;
    a.flush(at)?;
    drop(a);
    one(lib.give(lua, next)?)
}

fn f_flush_node(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    if let Some(at) = lib.opt(arg(&args, 0))? {
        let mut a = lib.arena.borrow_mut();
        a.unlink(at)?;
        a.flush(at)?;
    }
    none()
}

fn f_flush_list(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    if let Some(at) = lib.opt(arg(&args, 0))? {
        lib.arena.borrow_mut().flush_list(at)?;
    }
    none()
}

fn f_copy(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(at) = lib.opt(arg(&args, 0))? else {
        return one(Value::Nil);
    };
    let made = lib.arena.borrow_mut().copy(at)?;
    copy_property(lib, lua, at, made)?;
    one(lib.give(lua, Some(made))?)
}

fn f_copy_list(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(at) = lib.opt(arg(&args, 0))? else {
        return one(Value::Nil);
    };
    let stop = lib.opt(arg(&args, 1))?;
    let made = lib.arena.borrow_mut().copy_list(at, stop)?;
    one(lib.give(lua, made)?)
}

fn f_next(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(at) = lib.opt(arg(&args, 0))? else {
        return one(Value::Nil);
    };
    let n = lib.arena.borrow().next(at)?;
    one(lib.give(lua, n)?)
}

fn f_prev(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(at) = lib.opt(arg(&args, 0))? else {
        return one(Value::Nil);
    };
    let p = lib.arena.borrow().prev(at)?;
    one(lib.give(lua, p)?)
}

fn f_getboth(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(at) = lib.opt(arg(&args, 0))? else {
        return Ok(Variadic::from_iter([Value::Nil, Value::Nil]));
    };
    let (p, n) = {
        let a = lib.arena.borrow();
        (a.prev(at)?, a.next(at)?)
    };
    Ok(Variadic::from_iter([lib.give(lua, p)?, lib.give(lua, n)?]))
}

fn f_setnext(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let at = lib.want(args.first())?;
    let to = lib.opt(arg(&args, 1))?;
    lib.arena.borrow_mut().set_next(at, to)?;
    none()
}

fn f_setprev(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let at = lib.want(args.first())?;
    let to = lib.opt(arg(&args, 1))?;
    lib.arena.borrow_mut().set_prev(at, to)?;
    none()
}

fn f_setboth(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let at = lib.want(args.first())?;
    let (p, n) = (lib.opt(arg(&args, 1))?, lib.opt(arg(&args, 2))?);
    let mut a = lib.arena.borrow_mut();
    a.set_prev(at, p)?;
    a.set_next(at, n)?;
    none()
}

/// `setlink(a, b, ...)`: each node's `next` the one after it, nils skipped;
/// the first non-nil argument is answered.
fn f_setlink(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let mut nodes = Vec::new();
    for v in args.iter() {
        if let Some(at) = lib.opt(v)? {
            nodes.push(at);
        }
    }
    let mut a = lib.arena.borrow_mut();
    for w in nodes.windows(2) {
        a.link(Some(w[0]), Some(w[1]))?;
    }
    drop(a);
    one(lib.give(lua, nodes.first().copied())?)
}

/// `setsplit(a, b)`: cut the list between them.
fn f_setsplit(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let (a_at, b_at) = (lib.opt(arg(&args, 0))?, lib.opt(arg(&args, 1))?);
    let mut a = lib.arena.borrow_mut();
    if let Some(x) = a_at {
        a.set_next(x, None)?;
    }
    if let Some(y) = b_at {
        a.set_prev(y, None)?;
    }
    none()
}

fn f_id(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let _ = lib;
    one(int(want_id(arg(&args, 0))? as i64))
}

fn f_type(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let v = arg(&args, 0);
    let id = match v {
        Value::UserData(_) => lib.arena.borrow().id(lib.want(Some(v))?)? as i64,
        Value::Integer(n) => *n,
        _ => return one(Value::Nil),
    };
    match u8::try_from(id).ok().and_then(type_name) {
        Some(name) => one(Value::String(lua.create_string(name)?)),
        None => one(Value::Nil),
    }
}

fn f_getid(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    match lib.opt(arg(&args, 0))? {
        Some(at) => one(int(lib.arena.borrow().id(at)? as i64)),
        None => one(Value::Nil),
    }
}

fn f_getsubtype(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    match lib.opt(arg(&args, 0))? {
        Some(at) => one(read(lib, lua, at, "subtype")?),
        None => one(Value::Nil),
    }
}

fn f_setsubtype(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let at = lib.want(args.first())?;
    write(lib, at, "subtype", arg(&args, 1).clone())?;
    none()
}

fn f_getfield(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(at) = lib.opt(arg(&args, 0))? else {
        return one(Value::Nil);
    };
    match arg(&args, 1) {
        Value::String(s) => one(read(lib, lua, at, &s.to_str()?)?),
        _ => one(Value::Nil),
    }
}

fn f_setfield(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let at = lib.want(args.first())?;
    let Value::String(s) = arg(&args, 1) else {
        return Err(mlua::Error::runtime("a node field is named by a string"));
    };
    write(lib, at, &s.to_str()?, arg(&args, 2).clone())?;
    none()
}

fn f_has_field(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let at = lib.want(args.first())?;
    let Value::String(s) = arg(&args, 1) else {
        return one(Value::Boolean(false));
    };
    let name = s.to_str()?.to_string();
    let a = lib.arena.borrow();
    let c = a.cell(at)?;
    one(Value::Boolean(
        names(c.id, c.subtype).contains(&canonical(&name)),
    ))
}

/// A getter and setter pair for one named field, as the direct face spells
/// them (`getwidth`, `setwidth`, ...).
macro_rules! field_pair {
    ($get:ident, $set:ident, $name:literal) => {
        fn $get(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
            match lib.opt(arg(&args, 0))? {
                Some(at) => one(read(lib, lua, at, $name)?),
                None => one(Value::Nil),
            }
        }
        fn $set(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
            let at = lib.want(args.first())?;
            write(lib, at, $name, arg(&args, 1).clone())?;
            none()
        }
    };
}

field_pair!(f_getchar, f_setchar, "char");
field_pair!(f_getfont, f_setfont, "font");
field_pair!(f_getlang, f_setlang, "lang");
field_pair!(f_getwidth, f_setwidth, "width");
field_pair!(f_getheight, f_setheight, "height");
field_pair!(f_getdepth, f_setdepth, "depth");
field_pair!(f_getshift, f_setshift, "shift");
field_pair!(f_getpenalty, f_setpenalty, "penalty");
field_pair!(f_getfam, f_setfam, "fam");
field_pair!(f_getdata, f_setdata, "data");
field_pair!(f_getleader, f_setleader, "leader");
field_pair!(f_getcomponents, f_setcomponents, "components");
field_pair!(f_getnucleus, f_setnucleus, "nucleus");
field_pair!(f_getsub, f_setsub, "sub");
field_pair!(f_getsup, f_setsup, "sup");
field_pair!(f_getdir, f_setdir, "dir");
field_pair!(f_getexpansion, f_setexpansion, "expansion_factor");

/// `getkern` and `setkern` read a kern's `kern` and a margin kern's `width`.
fn f_getkern(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(at) = lib.opt(arg(&args, 0))? else {
        return one(Value::Nil);
    };
    // lnodelib.c pushes a kern's width as a lua_Number (it can carry a
    // font expansion), a margin kern's and a math node's as integers.
    let (field, float) = match lib.arena.borrow().id(at)? {
        KERN => ("kern", true),
        28 => ("width", false),
        MATH => ("surround", false),
        _ => return one(Value::Nil),
    };
    let v = read(lib, lua, at, field)?;
    match (float, v.as_integer()) {
        (true, Some(n)) => one(Value::Number(n as f64)),
        _ => one(v),
    }
}

fn f_setkern(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let at = lib.want(args.first())?;
    let field = match lib.arena.borrow().id(at)? {
        KERN => "kern",
        _ => "width",
    };
    write(lib, at, field, arg(&args, 1).clone())?;
    if let Value::Integer(_) | Value::Number(_) = arg(&args, 2) {
        write(lib, at, "subtype", arg(&args, 2).clone())?;
    }
    none()
}

/// `getlist`/`setlist`: the list a box, an adjust, an insert or a sub-box holds.
fn f_getlist(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    match lib.opt(arg(&args, 0))? {
        Some(at) => one(read(lib, lua, at, "head")?),
        None => one(Value::Nil),
    }
}

fn f_setlist(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let at = lib.want(args.first())?;
    write(lib, at, "head", arg(&args, 1).clone())?;
    none()
}

/// `getwhd(n [, withexpansion])`: width, height, depth; a glyph's from its
/// font.
fn f_getwhd(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(at) = lib.opt(arg(&args, 0))? else {
        return none();
    };
    let id = lib.arena.borrow().id(at)?;
    match id {
        HLIST | VLIST | RULE | 15 | GLYPH => Ok(Variadic::from_iter([
            read(lib, lua, at, "width")?,
            read(lib, lua, at, "height")?,
            read(lib, lua, at, "depth")?,
        ])),
        _ => none(),
    }
}

fn f_setwhd(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let at = lib.want(args.first())?;
    for (i, f) in ["width", "height", "depth"].iter().enumerate() {
        if let v @ (Value::Integer(_) | Value::Number(_)) = arg(&args, i + 1) {
            write(lib, at, f, v.clone())?;
        }
    }
    none()
}

/// `getdisc(n [, tails])`: pre, post, replace -- and with `true`, their tails
/// after each (luatex: "pre, post, replace, pre tail, post tail, replace
/// tail").
fn f_getdisc(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(at) = lib.opt(arg(&args, 0))? else {
        return none();
    };
    let heads: Vec<Option<usize>> = {
        let a = lib.arena.borrow();
        let c = a.cell(at)?;
        if c.id != DISC {
            return none();
        }
        ["pre", "post", "replace"]
            .iter()
            .map(|f| c.node(f))
            .collect()
    };
    let mut out: Vec<Value> = heads
        .iter()
        .map(|h| lib.give(lua, *h))
        .collect::<mlua::Result<_>>()?;
    if matches!(arg(&args, 1), Value::Boolean(true)) {
        let a = lib.arena.borrow();
        let tails: Vec<Option<usize>> = heads
            .iter()
            .map(|h| h.map(|h| a.tail(h)).transpose())
            .collect::<mlua::Result<_>>()?;
        drop(a);
        for t in tails {
            out.push(lib.give(lua, t)?);
        }
    }
    Ok(Variadic::from_iter(out))
}

/// `setdisc(n, pre, post, replace [, subtype [, penalty]])`.
fn f_setdisc(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let at = lib.want(args.first())?;
    for (i, f) in ["pre", "post", "replace"].iter().enumerate() {
        write(lib, at, f, arg(&args, i + 1).clone())?;
    }
    if !arg(&args, 4).is_nil() {
        write(lib, at, "subtype", arg(&args, 4).clone())?;
    }
    if !arg(&args, 5).is_nil() {
        write(lib, at, "penalty", arg(&args, 5).clone())?;
    }
    none()
}

/// `getglue(n)`: width, stretch, shrink, stretch_order, shrink_order of a
/// glue, a glue_spec or a math node.
fn f_getglue(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(at) = lib.opt(arg(&args, 0))? else {
        return none();
    };
    let a = lib.arena.borrow();
    let c = a.cell(at)?;
    if !matches!(c.id, GLUE | GLUE_SPEC | MATH) {
        return none();
    }
    Ok(Variadic::from_iter(
        [
            "width",
            "stretch",
            "shrink",
            "stretch_order",
            "shrink_order",
        ]
        .map(|f| int(c.int(f))),
    ))
}

/// `setglue(n, width, stretch, shrink, sto, sho)`: a missing value is zero.
fn f_setglue(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let at = lib.want(args.first())?;
    for (i, f) in [
        "width",
        "stretch",
        "shrink",
        "stretch_order",
        "shrink_order",
    ]
    .iter()
    .enumerate()
    {
        let v = match arg(&args, i + 1) {
            v @ (Value::Integer(_) | Value::Number(_)) => v.clone(),
            _ => int(0),
        };
        write(lib, at, f, v)?;
    }
    none()
}

fn f_is_zero_glue(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let at = lib.want(args.first())?;
    let a = lib.arena.borrow();
    let c = a.cell(at)?;
    one(Value::Boolean(
        ["width", "stretch", "shrink"].iter().all(|f| c.int(f) == 0),
    ))
}

/// `getoffsets`/`setoffsets`: a glyph's `xoffset` and `yoffset`.
fn f_getoffsets(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(at) = lib.opt(arg(&args, 0))? else {
        return none();
    };
    if lib.arena.borrow().id(at)? != GLYPH {
        return none();
    }
    Ok(Variadic::from_iter([
        read(lib, lua, at, "xoffset")?,
        read(lib, lua, at, "yoffset")?,
    ]))
}

fn f_setoffsets(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let at = lib.want(args.first())?;
    for (i, f) in ["xoffset", "yoffset"].iter().enumerate() {
        if let v @ (Value::Integer(_) | Value::Number(_)) = arg(&args, i + 1) {
            write(lib, at, f, v.clone())?;
        }
    }
    none()
}

fn f_getdirection(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    // The numeric spelling of `dir`: 0 for TLT, the only direction texrs sets.
    let Some(at) = lib.opt(arg(&args, 0))? else {
        return one(Value::Nil);
    };
    let d = read(lib, lua, at, "dir")?;
    let n = match &d {
        Value::String(s) => match s.to_str()?.trim_start_matches(['+', '-']) {
            "TRT" => 1,
            "LTL" => 2,
            "RTT" => 3,
            _ => 0,
        },
        _ => 0,
    };
    one(int(n))
}

fn f_setdirection(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let at = lib.want(args.first())?;
    let name = match want_int(arg(&args, 1)).unwrap_or(0) {
        1 => "TRT",
        2 => "LTL",
        3 => "RTT",
        _ => "TLT",
    };
    write(lib, at, "dir", Value::String(lua.create_string(name)?))?;
    none()
}

// ── list operations ──────────────────────────────────────────────────────

fn f_tail(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(at) = lib.opt(arg(&args, 0))? else {
        return one(Value::Nil);
    };
    let t = lib.arena.borrow().tail(at)?;
    one(lib.give(lua, Some(t))?)
}

/// `slide(head)`: the tail, with every `prev` made to agree with `next`.
fn f_slide(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(at) = lib.opt(arg(&args, 0))? else {
        return one(Value::Nil);
    };
    let mut a = lib.arena.borrow_mut();
    let chain = a.chain(at, None)?;
    for w in chain.windows(2) {
        a.set_prev(w[1], Some(w[0]))?;
    }
    let t = *chain.last().unwrap_or(&at);
    drop(a);
    one(lib.give(lua, Some(t))?)
}

fn f_length(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(at) = lib.opt(arg(&args, 0))? else {
        return one(int(0));
    };
    let stop = lib.opt(arg(&args, 1))?;
    one(int(lib.arena.borrow().chain(at, stop)?.len() as i64))
}

/// `count(id, head [, stop])`.
fn f_count(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let id = want_id(arg(&args, 0))?;
    let Some(head) = lib.opt(arg(&args, 1))? else {
        return one(int(0));
    };
    let stop = lib.opt(arg(&args, 2))?;
    let a = lib.arena.borrow();
    let n = a
        .chain(head, stop)?
        .into_iter()
        .filter(|at| a.id(*at).is_ok_and(|i| i == id))
        .count();
    one(int(n as i64))
}

/// lnodelib.c's `set_t_to_prev`: the node before `current`, found by walking
/// from `head` -- repairing every `prev` on the way, as the C does -- when
/// `current`'s own `prev` does not lead back to it.
fn prev_of(a: &mut Arena, head: usize, current: usize) -> mlua::Result<Option<usize>> {
    if let Some(t) = a.prev(current)? {
        if a.alive(t) && a.next(t)? == Some(current) {
            return Ok(Some(t));
        }
    }
    let mut t = Some(head);
    let mut steps = 0;
    while let Some(x) = t {
        let next = a.next(x)?;
        if next == Some(current) {
            return Ok(Some(x));
        }
        if let Some(n) = next {
            a.set_prev(n, Some(x))?;
        }
        t = next;
        steps += 1;
        if steps > WALK_LIMIT {
            return Err(mlua::Error::runtime("this node list has no end"));
        }
    }
    Ok(None)
}

/// `insert_before(head, current, new)`: `new` alone goes in front of
/// `current` (the tail when `current` is nil); the head changes when `current`
/// was the head. Answers head and `new`. A nil head makes `new` the list.
fn f_insert_before(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    if args.len() < 3 {
        return Err(mlua::Error::runtime(
            "Not enough arguments for node.insert_before()",
        ));
    }
    let Some(n) = lib.opt(arg(&args, 2))? else {
        return Ok(Variadic::from_iter([
            arg(&args, 0).clone(),
            arg(&args, 1).clone(),
        ]));
    };
    let mut a = lib.arena.borrow_mut();
    let Some(head) = lib.opt(arg(&args, 0))? else {
        a.set_next(n, None)?;
        a.set_prev(n, None)?;
        drop(a);
        let v = lib.give(lua, Some(n))?;
        return Ok(Variadic::from_iter([v.clone(), v]));
    };
    let current = match lib.opt(arg(&args, 1))? {
        Some(c) => c,
        None => a.tail(head)?,
    };
    if head != current {
        let Some(t) = prev_of(&mut a, head, current)? else {
            return Err(mlua::Error::runtime(
                "Attempt to node.insert_before() a non-existing node",
            ));
        };
        a.link(Some(t), Some(n))?;
    }
    a.link(Some(n), Some(current))?;
    drop(a);
    let first = if head == current { n } else { head };
    Ok(Variadic::from_iter([
        lib.give(lua, Some(first))?,
        lib.give(lua, Some(n))?,
    ]))
}

/// `insert_after(head, current, new)`: `new` alone goes after `current` (the
/// tail when `current` is nil). Answers head and `new`.
fn f_insert_after(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    if args.len() < 3 {
        return Err(mlua::Error::runtime(
            "Not enough arguments for node.insert_after()",
        ));
    }
    let Some(n) = lib.opt(arg(&args, 2))? else {
        return Ok(Variadic::from_iter([
            arg(&args, 0).clone(),
            arg(&args, 1).clone(),
        ]));
    };
    let mut a = lib.arena.borrow_mut();
    let Some(head) = lib.opt(arg(&args, 0))? else {
        a.set_next(n, None)?;
        a.set_prev(n, None)?;
        drop(a);
        let v = lib.give(lua, Some(n))?;
        return Ok(Variadic::from_iter([v.clone(), v]));
    };
    let current = match lib.opt(arg(&args, 1))? {
        Some(c) => c,
        None => a.tail(head)?,
    };
    // try_couple_nodes(n, vlink(current)); couple_nodes(current, n)
    let after = a.next(current)?;
    match after {
        Some(x) => a.link(Some(n), Some(x))?,
        None => a.set_next(n, None)?,
    }
    a.link(Some(current), Some(n))?;
    drop(a);
    Ok(Variadic::from_iter([
        lib.give(lua, Some(head))?,
        lib.give(lua, Some(n))?,
    ]))
}

/// `remove(head, current)`: answers the new head and the node that followed
/// `current`. As in lnodelib.c, the removed node keeps its own `next` and
/// `prev`; only its neighbours are relinked.
fn f_remove(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    if args.len() < 2 {
        return Err(mlua::Error::runtime(
            "Not enough arguments for node.remove()",
        ));
    }
    let Some(cur) = lib.opt(arg(&args, 1))? else {
        return Ok(Variadic::from_iter([arg(&args, 0).clone(), Value::Nil]));
    };
    let head = lib.want(args.first())?;
    let mut a = lib.arena.borrow_mut();
    let next = a.next(cur)?;
    let new_head = if head == cur {
        if let Some(p) = a.prev(cur)? {
            a.set_next(p, next)?;
        }
        if let Some(n) = next {
            let p = a.prev(cur)?;
            a.set_prev(n, p)?;
        }
        next
    } else {
        let Some(t) = prev_of(&mut a, head, cur)? else {
            return Err(mlua::Error::runtime(
                "Attempt to node.remove() a non-existing node",
            ));
        };
        a.set_next(t, next)?;
        if let Some(n) = next {
            a.set_prev(n, Some(t))?;
        }
        Some(head)
    };
    drop(a);
    Ok(Variadic::from_iter([
        lib.give(lua, new_head)?,
        lib.give(lua, next)?,
    ]))
}

// ── traversal ────────────────────────────────────────────────────────────

/// Which nodes a traverser hands out, and what it hands out with each.
#[derive(Clone, Copy)]
enum Walk {
    /// `traverse`: node, id, subtype.
    All,
    /// `traverse_id(id, head)`: node, subtype.
    Id(u8),
    /// `traverse_char`: glyphs that are characters (subtype below 256), with
    /// char and font.
    Chars,
    /// `traverse_glyph`: every glyph, with char and font.
    Glyphs,
    /// `traverse_list`: hlists and vlists, with id, subtype and list.
    Lists,
}

/// The iterator a `traverse*` call returns. Like luatex's, it reads `next` of
/// the node it last handed out at each step, so a body that removes the node
/// it was given and keeps a reference to what follows walks on correctly.
fn traverse(
    lib: &Lib,
    lua: &Lua,
    walk: Walk,
    head: Option<usize>,
) -> mlua::Result<Variadic<Value>> {
    let lib2 = lib.clone();
    let mut cursor: Option<usize> = None;
    let mut started = false;
    let mut steps = 0usize;
    let f = lua.create_function_mut(move |lua, _: Variadic<Value>| loop {
        let at = match started {
            false => {
                started = true;
                head
            }
            true => match cursor {
                Some(c) if lib2.arena.borrow().alive(c) => lib2.arena.borrow().next(c)?,
                _ => None,
            },
        };
        cursor = at;
        steps += 1;
        if steps > WALK_LIMIT {
            return Err(mlua::Error::runtime("this node list has no end"));
        }
        let Some(at) = at else {
            return Ok(Variadic::from_iter([Value::Nil]));
        };
        let (id, sub, ch, font, list) = {
            let a = lib2.arena.borrow();
            let c = a.cell(at)?;
            (
                c.id,
                c.subtype.unwrap_or(0),
                c.int("char"),
                c.int("font"),
                c.node("head"),
            )
        };
        let me = lib2.give(lua, Some(at))?;
        match walk {
            Walk::All => return Ok(Variadic::from_iter([me, int(id as i64), int(sub)])),
            Walk::Id(want) if id == want => return Ok(Variadic::from_iter([me, int(sub)])),
            Walk::Chars if id == GLYPH && sub & 0xFF00 == 0 => {
                return Ok(Variadic::from_iter([me, int(ch), int(font)]))
            }
            Walk::Glyphs if id == GLYPH => {
                return Ok(Variadic::from_iter([me, int(ch), int(font)]))
            }
            Walk::Lists if matches!(id, HLIST | VLIST) => {
                return Ok(Variadic::from_iter([
                    me,
                    int(id as i64),
                    int(sub),
                    lib2.give(lua, list)?,
                ]))
            }
            _ => continue,
        }
    })?;
    Ok(Variadic::from_iter([
        Value::Function(f),
        Value::Nil,
        Value::Nil,
    ]))
}

fn f_traverse(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    traverse(lib, lua, Walk::All, lib.opt(arg(&args, 0))?)
}

fn f_traverse_id(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let id = want_id(arg(&args, 0))?;
    traverse(lib, lua, Walk::Id(id), lib.opt(arg(&args, 1))?)
}

fn f_traverse_char(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    traverse(lib, lua, Walk::Chars, lib.opt(arg(&args, 0))?)
}

fn f_traverse_glyph(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    traverse(lib, lua, Walk::Glyphs, lib.opt(arg(&args, 0))?)
}

fn f_traverse_list(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    traverse(lib, lua, Walk::Lists, lib.opt(arg(&args, 0))?)
}

// ── glyphs ───────────────────────────────────────────────────────────────

/// `is_char(n [, font])`: the character, when `n` is a glyph that is a
/// character (subtype below 256) in `font` (any font when none is given);
/// `false` for a glyph that is not; `nil` for a node that is not a glyph.
/// With a font given and a glyph of another font, the answer is `false`
/// and the glyph's font.
fn f_is_char(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(at) = lib.opt(arg(&args, 0))? else {
        return one(Value::Nil);
    };
    let a = lib.arena.borrow();
    let c = a.cell(at)?;
    if c.id != GLYPH {
        return Ok(Variadic::from_iter([Value::Nil, int(c.id as i64)]));
    }
    if protected(c) {
        return one(Value::Boolean(false));
    }
    match arg(&args, 1) {
        Value::Integer(_) | Value::Number(_) => {
            let f = want_int(arg(&args, 1))?;
            match f != 0 && f == c.int("font") {
                true => one(int(c.int("char"))),
                false => one(Value::Boolean(false)),
            }
        }
        _ => one(int(c.int("char"))),
    }
}

/// `is_glyph(n)`: char and font of a glyph, `false` otherwise.
fn f_is_glyph(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(at) = lib.opt(arg(&args, 0))? else {
        return one(Value::Boolean(false));
    };
    let a = lib.arena.borrow();
    let c = a.cell(at)?;
    match c.id == GLYPH {
        true => Ok(Variadic::from_iter([
            int(c.int("char")),
            int(c.int("font")),
        ])),
        false => Ok(Variadic::from_iter([
            Value::Boolean(false),
            int(c.id as i64),
        ])),
    }
}

fn f_uses_font(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(head) = lib.opt(arg(&args, 0))? else {
        return one(Value::Boolean(false));
    };
    let font = want_int(arg(&args, 1))?;
    let a = lib.arena.borrow();
    for at in a.chain(head, None)? {
        let c = a.cell(at)?;
        if c.id == GLYPH && c.int("font") == font {
            return one(Value::Boolean(true));
        }
        if c.id == DISC {
            for f in ["pre", "post", "replace"] {
                if let Some(h) = c.node(f) {
                    for g in a.chain(h, None)? {
                        let gc = a.cell(g)?;
                        if gc.id == GLYPH && gc.int("font") == font {
                            return one(Value::Boolean(true));
                        }
                    }
                }
            }
        }
    }
    one(Value::Boolean(false))
}

/// texnodes.h's `is_protected_glyph_node`: a glyph whose subtype has its high
/// byte set is done, and the font processing passes over it.
fn protected(c: &Cell) -> bool {
    c.subtype.unwrap_or(0) & 0xFF00 != 0
}

/// texnodes.h's `is_simple_character`: a character, not a ligature or ghost.
fn simple_character(c: &Cell) -> bool {
    let s = c.subtype.unwrap_or(0);
    s & 1 != 0 && s & 2 == 0 && s & 4 == 0
}

/// `protect_node_indeed`: a glyph's subtype gets its high byte set (`| 0xFF00`)
/// or cleared (`& 0x00FF`); a disc has every glyph of its three lists done.
fn protect_one(a: &mut Arena, at: usize, on: bool) -> mlua::Result<()> {
    let c = a.cell_mut(at)?;
    match c.id {
        GLYPH => {
            let s = c.subtype.unwrap_or(0);
            c.subtype = Some(match on {
                true => s | 0xFF00,
                false => s & 0x00FF,
            });
        }
        DISC => {
            let lists: Vec<usize> = ["replace", "pre", "post"]
                .iter()
                .filter_map(|f| c.node(f))
                .collect();
            for h in lists {
                for g in a.chain(h, None)? {
                    if a.id(g)? == GLYPH {
                        protect_one(a, g, on)?;
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// `protect_glyph(n)` / `protect_glyphs(head [, tail])`, the tail included,
/// and their `unprotect_` mirrors.
fn protect(lib: &Lib, args: &Variadic<Value>, all: bool, on: bool) -> mlua::Result<()> {
    let Some(head) = lib.opt(arg(args, 0))? else {
        return Ok(());
    };
    let tail = if all { lib.opt(arg(args, 1))? } else { None };
    let mut a = lib.arena.borrow_mut();
    let nodes = match all {
        true => {
            let mut out = Vec::new();
            for at in a.chain(head, None)? {
                out.push(at);
                if Some(at) == tail {
                    break;
                }
            }
            out
        }
        false => vec![head],
    };
    for at in nodes {
        protect_one(&mut a, at, on)?;
    }
    Ok(())
}

fn f_protect_glyph(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    protect(lib, &args, false, true)?;
    none()
}

fn f_protect_glyphs(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    protect(lib, &args, true, true)?;
    none()
}

fn f_unprotect_glyph(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    protect(lib, &args, false, false)?;
    none()
}

fn f_unprotect_glyphs(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    protect(lib, &args, true, false)?;
    none()
}

/// `first_glyph(head [, stop])`: the first character glyph.
fn f_first_glyph(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(head) = lib.opt(arg(&args, 0))? else {
        return one(Value::Nil);
    };
    let stop = lib.opt(arg(&args, 1))?;
    let found = {
        let a = lib.arena.borrow();
        a.chain(head, stop)?.into_iter().find(|at| {
            a.cell(*at)
                .is_ok_and(|c| c.id == GLYPH && simple_character(c))
        })
    };
    one(lib.give(lua, found)?)
}

/// `has_glyph(head)`: the first glyph or glyph-like node (a glyph or a disc).
fn f_has_glyph(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(head) = lib.opt(arg(&args, 0))? else {
        return one(Value::Nil);
    };
    let found = {
        let a = lib.arena.borrow();
        a.chain(head, None)?
            .into_iter()
            .find(|at| a.cell(*at).is_ok_and(|c| matches!(c.id, GLYPH | DISC)))
    };
    one(lib.give(lua, found)?)
}

/// `end_of_math(n)`: the next math node that ends a formula.
fn f_end_of_math(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(head) = lib.opt(arg(&args, 0))? else {
        return one(Value::Nil);
    };
    let found = {
        let a = lib.arena.borrow();
        let start = match a.cell(head)?.id == MATH && a.cell(head)?.subtype == Some(1) {
            true => return one(lib.give(lua, Some(head))?),
            false => a.next(head)?,
        };
        match start {
            Some(s) => a.chain(s, None)?.into_iter().find(|at| {
                a.cell(*at)
                    .is_ok_and(|c| c.id == MATH && c.subtype == Some(1))
            }),
            None => None,
        }
    };
    one(lib.give(lua, found)?)
}

/// `protrusion_skippable(n)`: whether a node is passed over when looking for
/// a protruding character (§1078-style: everything that is not a glyph,
/// a disc, a box or a rule is).
fn f_protrusion_skippable(
    lib: &Lib,
    _: &Lua,
    args: Variadic<Value>,
) -> mlua::Result<Variadic<Value>> {
    let at = lib.want(args.first())?;
    let a = lib.arena.borrow();
    let c = a.cell(at)?;
    let skippable = match c.id {
        GLYPH | DISC | HLIST | VLIST | RULE => false,
        KERN => c.subtype != Some(0),
        _ => true,
    };
    one(Value::Boolean(skippable))
}

// ── discretionaries ──────────────────────────────────────────────────────

/// `check_discretionary(n)`: make every `prev` in a disc's three lists agree.
fn check_disc(a: &mut Arena, at: usize) -> mlua::Result<()> {
    if a.cell(at)?.id != DISC {
        return Ok(());
    }
    for f in ["pre", "post", "replace"] {
        if let Some(h) = a.cell(at)?.node(f) {
            let chain = a.chain(h, None)?;
            a.set_prev(h, None)?;
            for w in chain.windows(2) {
                a.set_prev(w[1], Some(w[0]))?;
            }
        }
    }
    Ok(())
}

fn f_check_discretionary(
    lib: &Lib,
    _: &Lua,
    args: Variadic<Value>,
) -> mlua::Result<Variadic<Value>> {
    let at = lib.want(args.first())?;
    check_disc(&mut lib.arena.borrow_mut(), at)?;
    none()
}

fn f_check_discretionaries(
    lib: &Lib,
    _: &Lua,
    args: Variadic<Value>,
) -> mlua::Result<Variadic<Value>> {
    if let Some(head) = lib.opt(arg(&args, 0))? {
        let mut a = lib.arena.borrow_mut();
        for at in a.chain(head, None)? {
            check_disc(&mut a, at)?;
        }
    }
    none()
}

/// `flatten_discretionaries(head)`: each disc replaced by its `replace`
/// list; answers the new head and how many were flattened.
fn f_flatten_discretionaries(
    lib: &Lib,
    lua: &Lua,
    args: Variadic<Value>,
) -> mlua::Result<Variadic<Value>> {
    let Some(mut head) = lib.opt(arg(&args, 0))? else {
        return Ok(Variadic::from_iter([Value::Nil, int(0)]));
    };
    let mut count = 0;
    let mut a = lib.arena.borrow_mut();
    for at in a.chain(head, None)? {
        if a.cell(at)?.id != DISC {
            continue;
        }
        count += 1;
        let (prev, next) = (a.prev(at)?, a.next(at)?);
        let replace = a.cell(at)?.node("replace");
        a.cell_mut(at)?.set("replace", None);
        let (first, last) = match replace {
            Some(r) => (Some(r), Some(a.tail(r)?)),
            None => (None, None),
        };
        match (first, last) {
            (Some(f), Some(l)) => {
                a.link(prev, Some(f))?;
                if prev.is_none() {
                    a.set_prev(f, None)?;
                }
                a.link(Some(l), next)?;
                if next.is_none() {
                    a.set_next(l, None)?;
                }
                if head == at {
                    head = f;
                }
            }
            _ => {
                a.link(prev, next)?;
                if let (None, Some(n)) = (prev, next) {
                    a.set_prev(n, None)?;
                }
                if let (Some(p), None) = (prev, next) {
                    a.set_next(p, None)?;
                }
                if head == at {
                    match next {
                        Some(n) => head = n,
                        None => {
                            a.flush(at)?;
                            drop(a);
                            return Ok(Variadic::from_iter([Value::Nil, int(count)]));
                        }
                    }
                }
            }
        }
        a.cell_mut(at)?.prev = None;
        a.cell_mut(at)?.next = None;
        a.flush(at)?;
    }
    drop(a);
    Ok(Variadic::from_iter([
        lib.give(lua, Some(head))?,
        int(count),
    ]))
}

// ── attributes ───────────────────────────────────────────────────────────

fn f_has_attribute(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(at) = lib.opt(arg(&args, 0))? else {
        return one(Value::Nil);
    };
    let number = want_int(arg(&args, 1))?;
    let want = arg(&args, 2).as_integer();
    let pairs = lib.arena.borrow().attributes(at)?;
    match pairs.iter().find(|(n, _)| *n == number) {
        Some((_, v)) if want.is_none_or(|w| w == *v) => one(int(*v)),
        _ => one(Value::Nil),
    }
}

fn f_get_attribute(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(at) = lib.opt(arg(&args, 0))? else {
        return one(Value::Nil);
    };
    let number = want_int(arg(&args, 1))?;
    let pairs = lib.arena.borrow().attributes(at)?;
    match pairs.iter().find(|(n, _)| *n == number) {
        Some((_, v)) => one(int(*v)),
        None => one(Value::Nil),
    }
}

/// `find_attribute(head, number)`: the value and the first node of the list
/// that has the attribute.
fn f_find_attribute(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(head) = lib.opt(arg(&args, 0))? else {
        return one(Value::Nil);
    };
    let number = want_int(arg(&args, 1))?;
    let found = {
        let a = lib.arena.borrow();
        let mut found = None;
        for at in a.chain(head, None)? {
            if let Some((_, v)) = a.attributes(at)?.into_iter().find(|(n, _)| *n == number) {
                found = Some((v, at));
                break;
            }
        }
        found
    };
    match found {
        Some((v, at)) => Ok(Variadic::from_iter([int(v), lib.give(lua, Some(at))?])),
        None => one(Value::Nil),
    }
}

fn f_set_attribute(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let at = lib.want(args.first())?;
    let number = want_int(arg(&args, 1))?;
    let value = want_int(arg(&args, 2))?;
    let mut a = lib.arena.borrow_mut();
    let mut pairs = a.attributes(at)?;
    match pairs.iter_mut().find(|(n, _)| *n == number) {
        Some(p) => p.1 = value,
        None => {
            pairs.push((number, value));
            pairs.sort_by_key(|p| p.0);
        }
    }
    a.set_attributes(at, &pairs)?;
    none()
}

/// `unset_attribute(n, number [, value])`: the old value when it was removed.
fn f_unset_attribute(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let at = lib.want(args.first())?;
    let number = want_int(arg(&args, 1))?;
    let want = arg(&args, 2).as_integer();
    let mut a = lib.arena.borrow_mut();
    let mut pairs = a.attributes(at)?;
    let Some(i) = pairs
        .iter()
        .position(|(n, v)| *n == number && want.is_none_or(|w| w == *v))
    else {
        return one(Value::Nil);
    };
    let (_, old) = pairs.remove(i);
    a.set_attributes(at, &pairs)?;
    one(int(old))
}

/// `current_attr()`: the attribute list TeX would give a node made now --
/// every attribute register that is set.
fn f_current_attr(lib: &Lib, lua: &Lua, _: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let pairs = super::current_attributes();
    if pairs.is_empty() {
        return one(Value::Nil);
    }
    let head = lib.arena.borrow_mut().make_attributes(&pairs)?;
    one(lib.give(lua, Some(head))?)
}

fn f_getattributelist(
    lib: &Lib,
    lua: &Lua,
    args: Variadic<Value>,
) -> mlua::Result<Variadic<Value>> {
    match lib.opt(arg(&args, 0))? {
        Some(at) => one(read(lib, lua, at, "attr")?),
        None => one(Value::Nil),
    }
}

/// `setattributelist(n, list)`: a copy of `list`, which may be another node
/// whose list is taken, an attribute list, or `true` for the current one.
fn f_setattributelist(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let at = lib.want(args.first())?;
    let pairs = match arg(&args, 1) {
        Value::Boolean(true) => super::current_attributes(),
        v => match lib.opt(v)? {
            None => Vec::new(),
            Some(src) => {
                let a = lib.arena.borrow();
                match a.id(src)? {
                    ATTRIBUTE_LIST => {
                        let mut out = Vec::new();
                        let mut p = a.next(src)?;
                        while let Some(x) = p {
                            let c = a.cell(x)?;
                            out.push((c.int("number"), c.int("value")));
                            p = c.next;
                        }
                        out
                    }
                    _ => a.attributes(src)?,
                }
            }
        },
    };
    lib.arena.borrow_mut().set_attributes(at, &pairs)?;
    none()
}

// ── properties ───────────────────────────────────────────────────────────

fn properties(lib: &Lib, lua: &Lua) -> mlua::Result<Table> {
    let mut a = lib.arena.borrow_mut();
    if let Some(k) = &a.properties {
        return lua.registry_value(k);
    }
    let t = lua.create_table()?;
    a.properties = Some(lua.create_registry_value(t.clone())?);
    Ok(t)
}

fn copy_property(lib: &Lib, lua: &Lua, from: usize, to: usize) -> mlua::Result<()> {
    if !lib.arena.borrow().copy_properties {
        return Ok(());
    }
    let t = properties(lib, lua)?;
    let v: Value = t.raw_get(from as i64)?;
    if !v.is_nil() {
        t.raw_set(to as i64, v)?;
    }
    Ok(())
}

fn f_get_properties_table(
    lib: &Lib,
    lua: &Lua,
    _: Variadic<Value>,
) -> mlua::Result<Variadic<Value>> {
    one(Value::Table(properties(lib, lua)?))
}

fn f_getproperty(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Some(at) = lib.opt(arg(&args, 0))? else {
        return one(Value::Nil);
    };
    one(properties(lib, lua)?.raw_get(at as i64)?)
}

fn f_setproperty(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let at = lib.want(args.first())?;
    properties(lib, lua)?.raw_set(at as i64, arg(&args, 1).clone())?;
    none()
}

fn f_flush_properties_table(
    lib: &Lib,
    lua: &Lua,
    _: Variadic<Value>,
) -> mlua::Result<Variadic<Value>> {
    let t = lua.create_table()?;
    lib.arena.borrow_mut().properties = Some(lua.create_registry_value(t)?);
    none()
}

/// `set_properties_mode(enabled, copy)`: the table is always there; the
/// second flag says whether `copy` copies a node's property.
fn f_set_properties_mode(
    lib: &Lib,
    _: &Lua,
    args: Variadic<Value>,
) -> mlua::Result<Variadic<Value>> {
    if let Value::Boolean(c) = arg(&args, 1) {
        lib.arena.borrow_mut().copy_properties = *c;
    }
    none()
}

// ── packaging ────────────────────────────────────────────────────────────

/// `hpack(head [, w, info, dir])` and `vpack`, over `crate::pack`'s port of
/// §649-§667 and §668-§679, answering the box and its badness. The
/// tolerances are INITEX's: they decide only what TeX would complain about,
/// and `node.hpack` reports nothing either. The box's head IS the list handed
/// in.
fn pack(
    lib: &Lib,
    lua: &Lua,
    args: &Variadic<Value>,
    vertical: bool,
) -> mlua::Result<Variadic<Value>> {
    let head = lib.opt(arg(args, 0))?;
    let w = match arg(args, 1) {
        Value::Nil => 0,
        v => want_int(v)?,
    };
    let spec = match arg(args, 2) {
        Value::String(s) => match &*s.to_str()? {
            "exactly" => crate::pack::Spec::Exactly(w),
            "additional" => crate::pack::Spec::Additional(w),
            other => {
                return Err(mlua::Error::runtime(format!(
                    "wrong mode in {}pack: '{other}'",
                    if vertical { "v" } else { "h" }
                )))
            }
        },
        _ => crate::pack::Spec::Additional(w),
    };
    let list = match head {
        Some(h) => list_to_engine(&lib.arena.borrow(), h)?,
        None => Vec::new(),
    };
    let tol = crate::pack::Tolerances::default();
    let packed = match vertical {
        true => crate::pack::vpack(list, spec, tol),
        false => crate::pack::hpack(list, spec, tol, None),
    };
    let b = &packed.node;
    let at = {
        let mut a = lib.arena.borrow_mut();
        let at = a.make(if vertical { VLIST } else { HLIST }, Some(0));
        let cell = a.cell_mut(at)?;
        cell.set("width", Some(FV::I(b.width)));
        cell.set("height", Some(FV::I(b.height)));
        cell.set("depth", Some(FV::I(b.depth)));
        cell.set("glue_sign", Some(FV::I(glue_sign_number(b.glue_sign))));
        cell.set("glue_order", Some(FV::I(to_luatex_order(b.glue_order))));
        cell.set("glue_set", Some(FV::F(b.glue_set)));
        cell.set("head", head.map(FV::N));
        at
    };
    Ok(Variadic::from_iter([
        lib.give(lua, Some(at))?,
        int(packed.badness),
    ]))
}

fn f_hpack(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    pack(lib, lua, &args, false)
}

fn f_vpack(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    pack(lib, lua, &args, true)
}

/// `dimensions([glue_set, glue_sign, glue_order,] head [, stop])`: what a list
/// measures, natural or with its glue set as a parent box set it (§625's
/// arithmetic rather than §649's).
fn f_dimensions(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let setting = matches!(
        args.first(),
        Some(Value::Number(_)) | Some(Value::Integer(_))
    ) && matches!(lib.face, Face::User)
        || (matches!(lib.face, Face::Direct) && args.len() >= 4);
    let (head, stop, set) = match setting {
        true => (
            lib.opt(arg(&args, 3))?,
            lib.opt(arg(&args, 4))?,
            Some((
                arg(&args, 0)
                    .as_number()
                    .or_else(|| arg(&args, 0).as_integer().map(|n| n as f64))
                    .unwrap_or(0.0),
                want_int(arg(&args, 1)).unwrap_or(0),
                want_int(arg(&args, 2)).unwrap_or(0),
            )),
        ),
        false => (lib.opt(arg(&args, 0))?, lib.opt(arg(&args, 1))?, None),
    };
    let Some(head) = head else {
        return Ok(Variadic::from_iter([int(0), int(0), int(0)]));
    };
    let list = {
        let a = lib.arena.borrow();
        let mut out = Vec::new();
        for at in a.chain(head, stop)? {
            out.extend(to_engine(&a, at)?);
        }
        out
    };
    let packed = crate::pack::hpack(
        list,
        crate::pack::NATURAL,
        crate::pack::Tolerances::default(),
        None,
    );
    let (mut w, h, d) = (packed.node.width, packed.node.height, packed.node.depth);
    if let Some((ratio, sign, order)) = set {
        let framing = BoxNode {
            glue_set: ratio,
            glue_sign: glue_sign(sign),
            glue_order: from_luatex_order(order)?,
            list: packed.node.list.clone(),
            ..BoxNode::null()
        };
        let mut setter = crate::pack::Setter::new(&framing);
        w = framing
            .list
            .iter()
            .map(|n| match n {
                ENode::Glue(g) => setter.glue(&g.spec),
                ENode::Box(b) => b.width,
                ENode::Rule(r) => r.width.max(0),
                ENode::Char(c) | ENode::Ligature(c, _) => c.width,
                ENode::Kern { width, .. } => *width,
                ENode::Math(s) => *s,
                _ => 0,
            })
            .sum();
    }
    Ok(Variadic::from_iter([w, h, d].map(int)))
}

/// `rangedimensions(parent, first [, last])`: `dimensions` over part of a box
/// with the box's own glue setting.
fn f_rangedimensions(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let parent = lib.want(args.first())?;
    let (gs, sign, order) = {
        let a = lib.arena.borrow();
        let c = a.cell(parent)?;
        if !matches!(c.id, HLIST | VLIST) {
            return Err(mlua::Error::runtime(
                "rangedimensions wants a parent box first",
            ));
        }
        let gs = match c.get("glue_set") {
            Some(FV::F(f)) => *f,
            _ => 0.0,
        };
        (gs, c.int("glue_sign"), c.int("glue_order"))
    };
    let mut call: Vec<Value> = vec![Value::Number(gs), int(sign), int(order)];
    call.push(arg(&args, 1).clone());
    call.push(arg(&args, 2).clone());
    let user = Lib {
        arena: Rc::clone(&lib.arena),
        face: Face::User,
    };
    f_dimensions(&user, lua, Variadic::from_iter(call))
}

/// `effective_glue(glue, parent)`: the width a glue takes in its packed box.
fn f_effective_glue(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let g = lib.want(args.first())?;
    let parent = lib.want(args.get(1))?;
    let a = lib.arena.borrow();
    let gc = a.cell(g)?;
    if gc.id != GLUE {
        return one(int(0));
    }
    let pc = a.cell(parent)?;
    let framing = BoxNode {
        glue_set: match pc.get("glue_set") {
            Some(FV::F(f)) => *f,
            _ => 0.0,
        },
        glue_sign: glue_sign(pc.int("glue_sign")),
        glue_order: from_luatex_order(pc.int("glue_order"))?,
        ..BoxNode::null()
    };
    let spec = Glue {
        natural: gc.int("width"),
        stretch: gc.int("stretch"),
        stretch_order: from_luatex_order(gc.int("stretch_order"))?,
        shrink: gc.int("shrink"),
        shrink_order: from_luatex_order(gc.int("shrink_order"))?,
    };
    let mut setter = crate::pack::Setter::new(&framing);
    // A lua_Number in lnodelib.c, the glue being set as a float.
    one(Value::Number(setter.glue(&spec) as f64))
}

// ── the vocabulary ───────────────────────────────────────────────────────

fn f_types(_: &Lib, lua: &Lua, _: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let t = lua.create_table()?;
    for (id, name) in TYPES {
        t.raw_set(*id as i64, *name)?;
    }
    one(Value::Table(t))
}

fn f_whatsits(_: &Lib, lua: &Lua, _: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let t = lua.create_table()?;
    for (id, name) in WHATSITS {
        t.raw_set(*id, *name)?;
    }
    one(Value::Table(t))
}

/// `node.subtype(name)`: a whatsit's subtype number, by name.
fn f_subtype(_: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    match arg(&args, 0) {
        Value::String(s) => one(whatsit_id(&s.to_str()?).map(int).unwrap_or(Value::Nil)),
        _ => one(Value::Nil),
    }
}

/// `node.fields(id [, subtype])`: field names by luatex's index.
fn f_fields(_: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let id = want_id(arg(&args, 0))?;
    let sub = match (id, arg(&args, 1)) {
        (WHATSIT, Value::String(s)) => whatsit_id(&s.to_str()?),
        (WHATSIT, v) => v.as_integer(),
        _ => None,
    };
    if id == WHATSIT && !WHATSITS.iter().any(|(n, _)| Some(*n) == sub) {
        return Err(mlua::Error::runtime("invalid whatsit type id"));
    }
    let list = names(id, sub);
    let base = match list.first() {
        Some(&"prev") => -1,
        _ => 0,
    };
    let t = lua.create_table()?;
    for (i, n) in list.iter().enumerate() {
        t.raw_set(base + i as i64, *n)?;
    }
    one(Value::Table(t))
}

/// `node.subtypes(id or name)`: subtype number to name, luatex 1.24.0's.
fn subtypes_of(name: &str) -> Option<&'static [(i64, &'static str)]> {
    const BOX: &[(i64, &str)] = &[
        (0, "unknown"),
        (1, "line"),
        (2, "box"),
        (3, "indent"),
        (4, "alignment"),
        (5, "cell"),
        (6, "equation"),
        (7, "equationnumber"),
        (8, "math"),
        (9, "mathchar"),
        (10, "hextensible"),
        (11, "vextensible"),
        (12, "hdelimiter"),
        (13, "vdelimiter"),
        (14, "overdelimiter"),
        (15, "underdelimiter"),
        (16, "numerator"),
        (17, "denominator"),
        (18, "limits"),
        (19, "fraction"),
        (20, "nucleus"),
        (21, "sup"),
        (22, "sub"),
        (23, "degree"),
        (24, "scripts"),
        (25, "over"),
        (26, "under"),
        (27, "accent"),
        (28, "radical"),
    ];
    const STYLE: &[(i64, &str)] = &[
        (0, "origin"),
        (1, "page"),
        (2, "always"),
        (3, "raw"),
        (4, "text"),
        (5, "font"),
        (6, "special"),
    ];
    Some(match name {
        "hlist" | "vlist" => BOX,
        "rule" => &[
            (0, "normal"),
            (1, "box"),
            (2, "image"),
            (3, "empty"),
            (4, "user"),
            (5, "over"),
            (6, "under"),
            (7, "fraction"),
            (8, "radical"),
            (9, "outline"),
        ],
        "adjust" => &[(0, "normal"), (1, "pre")],
        "boundary" => &[(0, "cancel"), (1, "user"), (2, "protrusion"), (3, "word")],
        "disc" => &[
            (0, "discretionary"),
            (1, "explicit"),
            (2, "automatic"),
            (3, "regular"),
            (4, "first"),
            (5, "second"),
        ],
        "dir" => &[(0, "normal"), (1, "cancel")],
        "math" => &[(0, "beginmath"), (1, "endmath")],
        "glue" => &[
            (0, "userskip"),
            (1, "lineskip"),
            (2, "baselineskip"),
            (3, "parskip"),
            (4, "abovedisplayskip"),
            (5, "belowdisplayskip"),
            (6, "abovedisplayshortskip"),
            (7, "belowdisplayshortskip"),
            (8, "leftskip"),
            (9, "rightskip"),
            (10, "topskip"),
            (11, "splittopskip"),
            (12, "tabskip"),
            (13, "spaceskip"),
            (14, "xspaceskip"),
            (15, "parfillskip"),
            (16, "mathskip"),
            (17, "thinmuskip"),
            (18, "medmuskip"),
            (19, "thickmuskip"),
            (98, "conditionalmathskip"),
            (99, "muglue"),
            (100, "leaders"),
            (101, "cleaders"),
            (102, "xleaders"),
            (103, "gleaders"),
        ],
        "kern" => &[
            (0, "fontkern"),
            (1, "userkern"),
            (2, "accentkern"),
            (3, "italiccorrection"),
        ],
        "penalty" => &[
            (0, "userpenalty"),
            (1, "linebreakpenalty"),
            (2, "linepenalty"),
            (3, "wordpenalty"),
            (4, "finalpenalty"),
            (5, "noadpenalty"),
            (6, "beforedisplaypenalty"),
            (7, "afterdisplaypenalty"),
            (8, "equationnumberpenalty"),
        ],
        "style" | "choice" => STYLE,
        "noad" => &[
            (0, "ord"),
            (1, "opdisplaylimits"),
            (2, "oplimits"),
            (3, "opnolimits"),
            (4, "bin"),
            (5, "rel"),
            (6, "open"),
            (7, "close"),
            (8, "punct"),
            (9, "inner"),
            (10, "under"),
            (11, "over"),
            (12, "vcenter"),
        ],
        "radical" => &[
            (0, "radical"),
            (1, "uradical"),
            (2, "uroot"),
            (3, "uunderdelimiter"),
            (4, "uoverdelimiter"),
            (5, "udelimiterunder"),
            (6, "udelimiterover"),
        ],
        "accent" => &[
            (0, "bothflexible"),
            (1, "fixedtop"),
            (2, "fixedbottom"),
            (3, "fixedboth"),
        ],
        "fence" => &[
            (0, "unset"),
            (1, "left"),
            (2, "middle"),
            (3, "right"),
            (4, "no"),
        ],
        "margin_kern" => &[(0, "left"), (1, "right")],
        "glyph" => &[
            (0, "unset"),
            (1, "character"),
            (2, "ligature"),
            (4, "ghost"),
            (8, "left"),
            (16, "right"),
        ],
        "glue_spec" | "fill" => &[
            (0, "normal"),
            (1, "fi"),
            (2, "fil"),
            (3, "fill"),
            (4, "filll"),
        ],
        "leader" => &[
            (100, "leaders"),
            (101, "cleaders"),
            (102, "xleaders"),
            (103, "gleaders"),
        ],
        _ => return None,
    })
}

fn f_subtypes(_: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let name = match arg(&args, 0) {
        Value::String(s) => s.to_str()?.to_string(),
        v => match v
            .as_integer()
            .and_then(|n| u8::try_from(n).ok())
            .and_then(type_name)
        {
            Some(n) => n.to_string(),
            None => return one(Value::Nil),
        },
    };
    let Some(list) = subtypes_of(&name) else {
        return one(Value::Nil);
    };
    let t = lua.create_table()?;
    for (n, s) in list {
        t.raw_set(*n, *s)?;
    }
    one(Value::Table(t))
}

/// `node.values(name)`, luatex 1.24.0's tables.
fn f_values(_: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let Value::String(s) = arg(&args, 0) else {
        return one(Value::Nil);
    };
    let list: &[&str] = match &*s.to_str()? {
        "dir" | "direction" => &["TLT", "TRT", "LTL", "RTT"],
        "glue" => &["normal", "fi", "fil", "fill", "filll"],
        "pdf_literal" => &["origin", "page", "always", "raw", "text", "font", "special"],
        "pdf_action" => &["page", "goto", "thread", "user"],
        "pdf_window" => &["unset", "new", "nonew"],
        "color_stack" => &["set", "push", "pop", "current"],
        "pagestate" => &["empty", "box_there", "inserts_only"],
        _ => return one(Value::Nil),
    };
    let t = lua.create_table()?;
    for (i, v) in list.iter().enumerate() {
        t.raw_set(i as i64, *v)?;
    }
    one(Value::Table(t))
}

fn f_is_node(_lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    // "returns a number (the internal index of the node)" or false.
    match arg(&args, 0) {
        Value::UserData(u) => match u.borrow::<NodeRef>() {
            Ok(n) => one(int(n.at as i64)),
            Err(_) => one(Value::Boolean(false)),
        },
        _ => one(Value::Boolean(false)),
    }
}

fn f_is_direct(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    match arg(&args, 0) {
        Value::Integer(n) if lib.arena.borrow().alive(*n as usize) => one(int(*n)),
        _ => one(Value::Boolean(false)),
    }
}

fn f_todirect(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    match lib.opt(arg(&args, 0))? {
        Some(at) => one(int(at as i64)),
        None => one(Value::Nil),
    }
}

fn f_tonode(lib: &Lib, lua: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let at = lib.opt(arg(&args, 0))?;
    one(lib.user().give(lua, at)?)
}

fn f_tostring(lib: &Lib, _: &Lua, args: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let at = lib.want(args.first())?;
    let s = NodeRef {
        arena: Rc::clone(&lib.arena),
        at,
    }
    .show()?;
    one(Value::String(
        super::with_lua(|l| l.create_string(s))
            .transpose()?
            .ok_or_else(|| mlua::Error::runtime("no Lua state"))?,
    ))
}

/// `usedlist()`: every node alive, as one list.
fn f_usedlist(lib: &Lib, lua: &Lua, _: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    let alive: Vec<usize> = {
        let a = lib.arena.borrow();
        (1..=a.cells.len()).filter(|i| a.alive(*i)).collect()
    };
    let copies: Vec<usize> = {
        let mut a = lib.arena.borrow_mut();
        let mut out = Vec::new();
        for at in alive {
            let id = a.id(at)?;
            out.push(a.make(id, Some(0)));
        }
        for w in out.windows(2) {
            a.link(Some(w[0]), Some(w[1]))?;
        }
        out
    };
    one(lib.give(lua, copies.first().copied())?)
}

fn f_family_font(_: &Lib, _: &Lua, _: Variadic<Value>) -> mlua::Result<Variadic<Value>> {
    // `\textfont n` and its siblings: texrs's maths fonts are crate::math's,
    // not numbered font identifiers.
    Err(mlua::Error::runtime(
        "node.family_font reads \\textfont and its siblings, which texrs's maths does not keep as font numbers",
    ))
}

/// The functions that reach TeX's own state -- the current list, the
/// box registers, a font's lig/kern program, the hyphenation patterns, the
/// math machinery -- and what each one waits on.
fn refuse(
    name: &'static str,
    why: &'static str,
) -> impl Fn(&Lua, Variadic<Value>) -> mlua::Result<()> {
    move |_, _| Err(mlua::Error::runtime(format!("node.{name}: {why}")))
}

const WAITING: &[(&str, &str)] = &[
    ("write", "appending to the list TeX is building is the stomach's, and it is not reachable while a chunk is lowered"),
    ("last_node", "taking the last node of the list TeX is building is the stomach's, and it is not reachable while a chunk is lowered"),
    ("ligaturing", "LuaTeX's handle_ligaturing over a font's ligature tables is not ported yet"),
    ("kerning", "LuaTeX's handle_kerning over a font's kern tables is not ported yet"),
    ("hyphenating", "LuaTeX's hyphenate over a list, with Liang's patterns and \\hjcode, is not ported yet"),
    ("mlist_to_hlist", "crate::math converts mlists of its own, not noad nodes"),
    ("make_extensible", "building an extensible from a font's parts is crate::math's, not the node library's"),
    ("prepend_prevdepth", "\\prevdepth belongs to the list TeX is building"),
    ("fix_node_lists", "repairing a list's back pointers TeX-side has no list here to repair"),
    ("getbox", "the box registers are the stomach's"),
    ("setbox", "the box registers are the stomach's"),
    ("get_synctex_fields", "texrs writes no SyncTeX"),
    ("set_synctex_fields", "texrs writes no SyncTeX"),
];

/// Every function of each face, by luatex 1.24.0's key lists.
const COMMON: &[(&str, Fun)] = &[
    ("check_discretionaries", f_check_discretionaries),
    ("check_discretionary", f_check_discretionary),
    ("copy", f_copy),
    ("copy_list", f_copy_list),
    ("count", f_count),
    ("current_attr", f_current_attr),
    ("dimensions", f_dimensions),
    ("effective_glue", f_effective_glue),
    ("end_of_math", f_end_of_math),
    ("find_attribute", f_find_attribute),
    ("first_glyph", f_first_glyph),
    ("flatten_discretionaries", f_flatten_discretionaries),
    ("flush_list", f_flush_list),
    ("flush_node", f_flush_node),
    ("flush_properties_table", f_flush_properties_table),
    ("free", f_free),
    ("get_attribute", f_get_attribute),
    ("get_properties_table", f_get_properties_table),
    ("getboth", f_getboth),
    ("getchar", f_getchar),
    ("getdisc", f_getdisc),
    ("getfield", f_getfield),
    ("getfont", f_getfont),
    ("getglue", f_getglue),
    ("getid", f_getid),
    ("getleader", f_getleader),
    ("getlist", f_getlist),
    ("getnext", f_next),
    ("getprev", f_prev),
    ("getproperty", f_getproperty),
    ("getsubtype", f_getsubtype),
    ("getwhd", f_getwhd),
    ("has_attribute", f_has_attribute),
    ("has_field", f_has_field),
    ("has_glyph", f_has_glyph),
    ("hpack", f_hpack),
    ("insert_after", f_insert_after),
    ("insert_before", f_insert_before),
    ("is_char", f_is_char),
    ("is_glyph", f_is_glyph),
    ("is_node", f_is_node),
    ("is_zero_glue", f_is_zero_glue),
    ("length", f_length),
    ("protect_glyph", f_protect_glyph),
    ("protect_glyphs", f_protect_glyphs),
    ("protrusion_skippable", f_protrusion_skippable),
    ("rangedimensions", f_rangedimensions),
    ("remove", f_remove),
    ("set_attribute", f_set_attribute),
    ("set_properties_mode", f_set_properties_mode),
    ("setfield", f_setfield),
    ("setglue", f_setglue),
    ("setproperty", f_setproperty),
    ("slide", f_slide),
    ("tail", f_tail),
    ("tostring", f_tostring),
    ("traverse", f_traverse),
    ("traverse_char", f_traverse_char),
    ("traverse_glyph", f_traverse_glyph),
    ("traverse_id", f_traverse_id),
    ("traverse_list", f_traverse_list),
    ("unprotect_glyph", f_unprotect_glyph),
    ("unprotect_glyphs", f_unprotect_glyphs),
    ("unset_attribute", f_unset_attribute),
    ("usedlist", f_usedlist),
    ("uses_font", f_uses_font),
    ("vpack", f_vpack),
    ("new", f_new),
];

/// The userdata face's own: the vocabulary and the field-style spellings.
const USER_ONLY: &[(&str, Fun)] = &[
    ("family_font", f_family_font),
    ("fields", f_fields),
    ("id", f_id),
    ("next", f_next),
    ("prev", f_prev),
    ("subtype", f_subtype),
    ("subtypes", f_subtypes),
    ("type", f_type),
    ("types", f_types),
    ("values", f_values),
    ("whatsits", f_whatsits),
];

/// The direct face's own accessors.
const DIRECT_ONLY: &[(&str, Fun)] = &[
    ("getattributelist", f_getattributelist),
    ("getcomponents", f_getcomponents),
    ("getdata", f_getdata),
    ("getdepth", f_getdepth),
    ("getdir", f_getdir),
    ("getdirection", f_getdirection),
    ("getexpansion", f_getexpansion),
    ("getfam", f_getfam),
    ("getheight", f_getheight),
    ("getkern", f_getkern),
    ("getlang", f_getlang),
    ("getnucleus", f_getnucleus),
    ("getoffsets", f_getoffsets),
    ("getpenalty", f_getpenalty),
    ("getshift", f_getshift),
    ("getsub", f_getsub),
    ("getsup", f_getsup),
    ("getwidth", f_getwidth),
    ("is_direct", f_is_direct),
    ("setattributelist", f_setattributelist),
    ("setboth", f_setboth),
    ("setchar", f_setchar),
    ("setcomponents", f_setcomponents),
    ("setdata", f_setdata),
    ("setdepth", f_setdepth),
    ("setdir", f_setdir),
    ("setdirection", f_setdirection),
    ("setdisc", f_setdisc),
    ("setexpansion", f_setexpansion),
    ("setfam", f_setfam),
    ("setfont", f_setfont),
    ("setheight", f_setheight),
    ("setkern", f_setkern),
    ("setlang", f_setlang),
    ("setleader", f_setleader),
    ("setlink", f_setlink),
    ("setlist", f_setlist),
    ("setnext", f_setnext),
    ("setnucleus", f_setnucleus),
    ("setoffsets", f_setoffsets),
    ("setpenalty", f_setpenalty),
    ("setprev", f_setprev),
    ("setshift", f_setshift),
    ("setsplit", f_setsplit),
    ("setsub", f_setsub),
    ("setsubtype", f_setsubtype),
    ("setsup", f_setsup),
    ("setwhd", f_setwhd),
    ("setwidth", f_setwidth),
    ("todirect", f_todirect),
    ("tonode", f_tonode),
];

/// Build the `node` table, with `node.direct` inside it.
pub(super) fn table(lua: &Lua, arena: &Rc<RefCell<Arena>>) -> mlua::Result<Table> {
    let node = lua.create_table()?;
    let direct = lua.create_table()?;
    let register = |t: &Table, face: Face, list: &[(&'static str, Fun)]| -> mlua::Result<()> {
        for (name, f) in list {
            let lib = Lib {
                arena: Rc::clone(arena),
                face,
            };
            let f = *f;
            t.set(
                *name,
                lua.create_function(move |lua, args: Variadic<Value>| f(&lib, lua, args))?,
            )?;
        }
        Ok(())
    };
    register(&node, Face::User, COMMON)?;
    register(&node, Face::User, USER_ONLY)?;
    register(&direct, Face::Direct, COMMON)?;
    register(&direct, Face::Direct, DIRECT_ONLY)?;
    // `todirect`/`tonode` exist on the userdata face as well? No: luatex has
    // them on `node.direct` only. The direct face has the userdata face's
    // conversion helpers for nil-safe use, and nothing else of it.
    for (name, why) in WAITING {
        let f = lua.create_function(refuse(name, why))?;
        match *name {
            "getbox" | "setbox" | "get_synctex_fields" | "set_synctex_fields" => {
                direct.set(*name, f)?
            }
            "mlist_to_hlist" | "make_extensible" | "fix_node_lists" => node.set(*name, f)?,
            _ => {
                node.set(*name, f.clone())?;
                direct.set(*name, f)?;
            }
        }
    }
    node.set("direct", direct)?;
    Ok(node)
}

/// A `glue_spec` node holding a register's five numbers, the orders already
/// in LuaTeX's numbering: `tex.skip[n]`, `tex.getskip`, `token.scan_glue`.
pub(super) fn spec_node(
    lua: &Lua,
    arena: &Rc<RefCell<Arena>>,
    glue: (i64, i64, i64, i64, i64),
) -> mlua::Result<Value> {
    let at = {
        let mut a = arena.borrow_mut();
        let at = a.make(GLUE_SPEC, None);
        let cell = a.cell_mut(at)?;
        for (name, v) in [
            ("width", glue.0),
            ("stretch", glue.1),
            ("shrink", glue.2),
            ("stretch_order", glue.3),
            ("shrink_order", glue.4),
        ] {
            cell.set(name, Some(FV::I(v)));
        }
        at
    };
    Ok(Value::UserData(lua.create_userdata(NodeRef {
        arena: Rc::clone(arena),
        at,
    })?))
}

/// The five numbers a `glue_spec` (or a glue node) holds, for a register write.
pub(super) fn spec_values(
    arena: &Rc<RefCell<Arena>>,
    v: &Value,
) -> mlua::Result<(i64, i64, i64, i64, i64)> {
    let lib = Lib {
        arena: Rc::clone(arena),
        face: Face::User,
    };
    let at = lib.want(Some(v))?;
    let a = arena.borrow();
    let cell = a.cell(at)?;
    if cell.id != GLUE_SPEC && cell.id != GLUE {
        return Err(mlua::Error::runtime("glue_spec expected"));
    }
    Ok((
        cell.int("width"),
        cell.int("stretch"),
        cell.int("shrink"),
        cell.int("stretch_order"),
        cell.int("shrink_order"),
    ))
}

/// The last node of the list at `head`.
pub(super) fn tail_of(arena: &Arena, head: usize) -> mlua::Result<usize> {
    arena.tail(head)
}

/// The first node of the list `at` is in, following `prev`.
pub(super) fn first_of(arena: &Arena, at: usize) -> mlua::Result<usize> {
    let mut first = at;
    let mut steps = 0;
    while let Some(p) = arena.prev(first)? {
        first = p;
        steps += 1;
        if steps > WALK_LIMIT {
            return Err(mlua::Error::runtime("this node list has no beginning"));
        }
    }
    Ok(first)
}

/// A node handed to Lua as userdata, for a callback's argument.
pub(super) fn handle(
    lua: &Lua,
    arena: &Rc<RefCell<Arena>>,
    at: Option<usize>,
) -> mlua::Result<Value> {
    Lib {
        arena: Rc::clone(arena),
        face: Face::User,
    }
    .give(lua, at)
}

/// The index a callback's answer stands for.
pub(super) fn index_of(arena: &Rc<RefCell<Arena>>, v: &Value) -> mlua::Result<Option<usize>> {
    Lib {
        arena: Rc::clone(arena),
        face: Face::User,
    }
    .opt(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_field_tables_are_luatexs() {
        // node.fields(29) in luatex 1.24.0, from index -1.
        let g = names(GLYPH, Some(0));
        assert_eq!(g[0], "prev");
        assert_eq!(g.iter().position(|n| *n == "char"), Some(5)); // index 4
        assert_eq!(kind(GLYPH, Some(0), "left"), Some(K::I(1)));
        assert_eq!(
            kind(RULE, Some(0), "width"),
            Some(K::I(crate::node::NULL_FLAG))
        );
        assert_eq!(kind(GLUE_SPEC, None, "attr"), None);
        assert_eq!(
            names(WHATSIT, Some(16)),
            &["prev", "next", "id", "subtype", "attr", "mode", "data"]
        );
    }

    #[test]
    fn a_disc_crosses_to_the_engine_with_its_replacement_after_it() {
        let mut a = Arena::default();
        let list = vec![
            ENode::Disc(DiscNode {
                pre_break: vec![ENode::Penalty(1)],
                post_break: vec![],
                replace_count: 2,
            }),
            ENode::Penalty(2),
            ENode::Penalty(3),
            ENode::Penalty(4),
        ];
        let head = from_engine(&mut a, &list).unwrap().unwrap();
        // The disc and the penalty after its replacement: two nodes at top.
        assert_eq!(a.chain(head, None).unwrap().len(), 2);
        let back = list_to_engine(&a, head).unwrap();
        assert_eq!(back.len(), 4);
        assert!(matches!(&back[0], ENode::Disc(d) if d.replace_count == 2));
        assert!(matches!(back[3], ENode::Penalty(4)));
    }
}
