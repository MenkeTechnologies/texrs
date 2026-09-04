//! The `node` library: LuaTeX's node interface, over texrs's own node list.
//!
//! ## The honest verdict, first
//!
//! LuaTeX's `node` library is two things wearing one name, and only one of
//! them can be put over `crate::node`.
//!
//! The first is a **vocabulary**: a numbered set of node types, a field name
//! per type, a subtype numbering, and the packagers `node.hpack` and
//! `node.vpack`. That is a documented contract (LuaTeX manual 1.24.0, "Nodes"),
//! and `crate::node` carries the same information for the types TeX82 has, so
//! the vocabulary CAN be spoken here — and every number it answers with is one
//! `crate::pack`'s port of §649-§667 computed, not one this module invented.
//!
//! The second is a **handle on the document**: `tex.box[0].head`,
//! `node.write`, `tex.getlist('page_head')`, and the callbacks
//! (`pre_linebreak_filter`, `hpack_filter`, `mlist_to_hlist`, …) that hand Lua
//! the list TeX is building. That one CANNOT be spoken here, and the reason is
//! not the shape of `crate::node` — it is that **there is no such list**.
//! `src/typeset.rs` sets a page from runs of strings; the only `hpack` on the
//! path a run takes is `set_footline`'s, which builds a list, measures it and
//! throws it away. `crate::postline` and `crate::page` are a library beside
//! that path rather than the path itself. So there is nothing to hand Lua, and
//! every entry point that would hand it something REFUSES, by name, with a
//! message saying which of the two engines' worlds the chunk has walked into.
//!
//! That split is deliberate and it is the whole design. A document that walks
//! the document's node list is still refused rather than quietly wrong, because
//! it cannot obtain the list in the first place. A chunk that BUILDS a list and
//! measures it gets TeX's own arithmetic.
//!
//! ## What is refused, and why each one
//!
//! - **The document's list.** `node.write`, `node.last_node`, `node.usedlist`,
//!   `tex.box`/`tex.getbox`/`tex.getlist` (in the parent module). No current
//!   list exists to append to, pop from, or read.
//! - **Fonts.** `glyph` nodes are creatable, but `.font` accepts only `0`.
//!   texrs has no `font` library and no font table for Lua to index, so `0` —
//!   LuaTeX's "no font" — is the only id a chunk here could ever hold. Measured:
//!   in luatex 1.24.0 a glyph with `font = 0` has `width = 0` whatever `char`
//!   is, and a write to `.width` is accepted and ignored because the width is
//!   derived from the font. This module does exactly that, so a glyph agrees
//!   with luatex in the only configuration reachable from here rather than
//!   answering a stored number luatex would not answer.
//! - **`node.ligaturing`, `node.kerning`, `node.hyphenating`.** Each needs the
//!   font's ligature/kern program or Liang's patterns applied to a node list;
//!   the lig/kern program is not on texrs's path at all (see BUGS.md).
//! - **`node.mlist_to_hlist`.** `crate::math` has the mlist machinery, but the
//!   noad types (LuaTeX ids 16-27) have no `crate::node::Node` variant, so
//!   there is no node for Lua to build an mlist out of.
//! - **Attributes on nodes.** `attr`, `node.set_attribute` and friends: an
//!   attribute list is a node in LuaTeX and texrs's `Node` has no field for one.
//!   (`tex.attribute` in the parent module is storage a chunk can use, which is
//!   a different thing and does not reach a node.)
//! - **`ins`, `mark`, `whatsit`, `unset`, `boundary`, `local_par`, `dir`,
//!   `margin_kern`, the noads, `glue_spec`.** Either `crate::node` has no
//!   variant for them, or its variant does not carry LuaTeX's fields: texrs's
//!   `InsNode.split_top_skip` is a `Glue` where LuaTeX's `spec` is a
//!   `glue_spec` NODE, and `Node::Mark` is already-expanded text where LuaTeX's
//!   `mark` is a token list. `node.id`, `node.type` and `node.types` still
//!   answer for every one of them, because those numbers are the documented
//!   contract; it is `node.new` that refuses.
//! - **Fields texrs's node has no room for.** `dir` on a box, `expansion_factor`
//!   on a kern, `replace` on a disc (texrs holds tex.web's `replace_count`, an
//!   integer, where LuaTeX holds a list), the mathskip fields on a math node.
//!   Reading one is an ERROR rather than `nil`: `nil` is what LuaTeX answers for
//!   a name that is not a field at all, so answering `nil` for a field LuaTeX
//!   DOES have would be the quiet wrongness this module exists to avoid.
//!
//! ## Two divergences that are not refusals, measured
//!
//! - **Glue orders are LuaTeX's, not tex.web's.** LuaTeX has a fourth, finer
//!   infinity called `fi` and numbers from it, so its `fil` is 2 where TeX82's
//!   is 1. Measured at the NODE level, not assumed: `\hbox to 20pt{\hskip 0pt
//!   plus 1fil}` gives `glue.stretch_order == 2` and `box.glue_order == 2` in
//!   luatex 1.24.0, and `1fill` gives 3. `super::to_luatex_order` and
//!   `super::from_luatex_order` are the same translation the register interface
//!   uses, and `fi` is refused here as it is there.
//! - **`glue_set` is wider here.** LuaTeX builds with `glue_ratio = float`, so
//!   a box stretched by 8/3 reports `2.6666667461395`; `crate::node::BoxNode`
//!   holds an `f64` and reports `2.6666666666666665`. texrs's is the more exact
//!   of the two and it is not this module's number to change, so a chunk that
//!   prints a fractional `glue_set` disagrees with luatex in the low digits.
//!   Integral ratios agree exactly.

use super::{from_luatex_order, to_luatex_order};
use crate::glue::Glue;
use crate::node::{BoxNode, CharNode, DiscNode, GlueNode, GlueSign, GlueSource, LeaderKind};
use crate::node::{Node as ENode, RuleNode, Scaled};
use mlua::{Lua, MetaMethod, Table, UserData, UserDataMethods, Value, Variadic};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// How many nodes a walk will follow before it decides the list is a cycle.
///
/// `n.next = n` is one Lua assignment away, and luatex hangs on it. Hanging is
/// not an option for an engine a test suite drives, so a walk that goes this
/// far stops with an error naming the node it started from.
const WALK_LIMIT: usize = 1 << 20;

// ── the type table ───────────────────────────────────────────────────────

/// LuaTeX's node ids, read out of `node.types()` in luatex 1.24.0.
///
/// The whole list, including the types `node.new` refuses: `node.id('glyph')`
/// answering 29 is a documented constant a chunk may compare against long
/// before it tries to make one.
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

const HLIST: u8 = 0;
const VLIST: u8 = 1;
const RULE: u8 = 2;
const ADJUST: u8 = 5;
const DISC: u8 = 7;
const MATH: u8 = 11;
const GLUE: u8 = 12;
const KERN: u8 = 13;
const PENALTY: u8 = 14;
const GLYPH: u8 = 29;
const GLUE_SPEC: u8 = 39;

/// The name for an id, or `None` if LuaTeX has no such id.
fn type_name(id: u8) -> Option<&'static str> {
    TYPES.iter().find(|(n, _)| *n == id).map(|(_, s)| *s)
}

/// The id for a name.
fn type_id(name: &str) -> Option<u8> {
    TYPES.iter().find(|(_, s)| *s == name).map(|(n, _)| *n)
}

/// The id a Lua value names: a number is one, a string is looked up.
fn want_id(v: &Value) -> mlua::Result<u8> {
    if let Some(n) = v.as_integer() {
        return u8::try_from(n)
            .ok()
            .filter(|id| type_name(*id).is_some())
            .ok_or_else(|| mlua::Error::runtime(format!("there is no node type {n}")));
    }
    if let Value::String(s) = v {
        let name = s.to_str()?;
        return type_id(&name)
            .ok_or_else(|| mlua::Error::runtime(format!("there is no node type '{name}'")));
    }
    Err(mlua::Error::runtime("expected a node type id or name"))
}

/// Why `node.new` will not make one of these.
///
/// Every type LuaTeX has that `crate::node::Node` cannot carry faithfully, with
/// the reason stated per type rather than as one blanket sentence: a chunk's
/// author is owed the specific thing that is missing.
fn why_not_new(id: u8) -> &'static str {
    match id {
        3 => {
            "an `ins` node's `spec` is a glue_spec NODE in LuaTeX, where \
              texrs's InsNode carries a plain Glue"
        }
        4 => {
            "a `mark` node's `mark` is a token list in LuaTeX, where texrs's \
              Node::Mark carries already-expanded text"
        }
        8 => {
            "a `whatsit` is differentiated by subtype alone and texrs's \
              Node::Whatsit carries one string, not LuaTeX's zoo of subtypes"
        }
        6 | 9 | 10 | 15 | 28 => {
            "texrs's node list is TeX82's (tex.web \
              §133-§161) and has no variant for this LuaTeX addition"
        }
        16..=27 => {
            "this is a math noad, and texrs's mlists (crate::math) are \
              not made of crate::node::Node"
        }
        38 | 40 => "texrs's nodes carry no attribute list",
        _ => {
            "this is one of LuaTeX's internal types, which are not part of a \
              document's node list"
        }
    }
}

// ── fields ───────────────────────────────────────────────────────────────

/// What a field holds.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// An integer: a dimension, a penalty, a character code.
    Num,
    /// The head of a nested list, or `nil`.
    List,
    /// `glue_set`, the one float in the structure (tex.web §109).
    Ratio,
    /// A glyph's `width`/`height`/`depth`. Read as 0 and written to no effect,
    /// which is what luatex 1.24.0 does for a glyph whose font is 0 — measured:
    /// `g.width = 12345` returns without error and `g.width` reads back `0`,
    /// because the dimension comes from the font rather than from the node.
    Derived,
}

/// One field of one node type: LuaTeX's own index for it, its name, its kind.
struct Field {
    index: i64,
    name: &'static str,
    kind: Kind,
}

const fn f(index: i64, name: &'static str, kind: Kind) -> Field {
    Field { index, name, kind }
}

/// The fields texrs carries for a node type, at LuaTeX's own indices.
///
/// Read out of `node.fields(id)` in luatex 1.24.0 and then cut down to what
/// `crate::node` holds; [`absent_field`] names what was cut and why.
fn fields_of(id: u8) -> &'static [Field] {
    const BOX: &[Field] = &[
        f(4, "width", Kind::Num),
        f(5, "depth", Kind::Num),
        f(6, "height", Kind::Num),
        f(8, "shift", Kind::Num),
        f(9, "glue_order", Kind::Num),
        f(10, "glue_sign", Kind::Num),
        f(11, "glue_set", Kind::Ratio),
        f(12, "head", Kind::List),
    ];
    const RULE_F: &[Field] = &[
        f(4, "width", Kind::Num),
        f(5, "depth", Kind::Num),
        f(6, "height", Kind::Num),
    ];
    const ADJUST_F: &[Field] = &[f(4, "head", Kind::List)];
    const DISC_F: &[Field] = &[f(4, "pre", Kind::List), f(5, "post", Kind::List)];
    const MATH_F: &[Field] = &[f(4, "surround", Kind::Num)];
    const GLUE_F: &[Field] = &[
        f(4, "leader", Kind::List),
        f(5, "width", Kind::Num),
        f(6, "stretch", Kind::Num),
        f(7, "shrink", Kind::Num),
        f(8, "stretch_order", Kind::Num),
        f(9, "shrink_order", Kind::Num),
    ];
    const KERN_F: &[Field] = &[f(4, "kern", Kind::Num)];
    const PENALTY_F: &[Field] = &[f(4, "penalty", Kind::Num)];
    // Measured: `node.fields(39)` in luatex 1.24.0 is `0:next 1:id 2:width
    // 3:stretch 4:shrink 5:stretch_order 6:shrink_order` -- no `prev`, no
    // `subtype`, and `node.has_field(spec,'subtype')` is false.
    const SPEC_F: &[Field] = &[
        f(2, "width", Kind::Num),
        f(3, "stretch", Kind::Num),
        f(4, "shrink", Kind::Num),
        f(5, "stretch_order", Kind::Num),
        f(6, "shrink_order", Kind::Num),
    ];
    const GLYPH_F: &[Field] = &[
        f(4, "char", Kind::Num),
        f(5, "font", Kind::Num),
        f(13, "width", Kind::Derived),
        f(14, "height", Kind::Derived),
        f(15, "depth", Kind::Derived),
    ];
    match id {
        HLIST | VLIST => BOX,
        RULE => RULE_F,
        ADJUST => ADJUST_F,
        DISC => DISC_F,
        MATH => MATH_F,
        GLUE => GLUE_F,
        KERN => KERN_F,
        PENALTY => PENALTY_F,
        GLYPH => GLYPH_F,
        GLUE_SPEC => SPEC_F,
        _ => &[],
    }
}

/// A field LuaTeX has on this node type that texrs has no room for, with the
/// reason. Reading or writing one is an error rather than `nil`: `nil` is what
/// LuaTeX itself answers for a name that is not a field, so `nil` here would
/// say "LuaTeX has no such field" when the truth is "texrs has not got it".
fn absent_field(id: u8, name: &str) -> Option<&'static str> {
    let reason = match (id, name) {
        (_, "attr") => "texrs's nodes carry no attribute list",
        (HLIST | VLIST | RULE, "dir") => {
            "direction is a LuaTeX addition; texrs's boxes are TeX82's, left to right"
        }
        (RULE, "index" | "left" | "right") => {
            "these are LuaTeX's rule extensions; texrs's RuleNode is tex.web §138's three dimensions"
        }
        (DISC, "replace") => {
            "texrs's DiscNode holds tex.web's `replace_count`, an integer, where LuaTeX holds a list"
        }
        (DISC, "penalty") => {
            "tex.web charges \\hyphenpenalty at the break rather than storing it on the node"
        }
        (MATH, "width" | "stretch" | "shrink" | "stretch_order" | "shrink_order") => {
            "these are LuaTeX's \\mathskip extension; tex.web §147's math node is one surround"
        }
        (KERN, "expansion_factor") => "font expansion is a pdfTeX/LuaTeX addition texrs has not got",
        (GLYPH, "lang" | "left" | "right" | "uchyph") => {
            "hyphenation state is not carried on texrs's char node"
        }
        (GLYPH, "components") => "texrs's Node::Ligature carries no component list",
        (GLYPH, "xoffset" | "yoffset" | "expansion_factor" | "data") => {
            "these are LuaTeX glyph extensions texrs has not got"
        }
        _ => return None,
    };
    Some(reason)
}

/// `head` and `list` name the same field on a box (`node.has_field(h,'list')`
/// is true in luatex 1.24.0), and `next`/`prev` are spelled either way.
fn canonical(name: &str) -> &str {
    match name {
        "list" => "head",
        other => other,
    }
}

// ── the arena ────────────────────────────────────────────────────────────

/// One node, as the arena holds it.
#[derive(Clone, Default)]
struct Cell {
    id: u8,
    subtype: u16,
    prev: Option<usize>,
    next: Option<usize>,
    /// The integer fields, by name. Absent means zero, as `node.new` leaves
    /// every field ("All its fields are initialized to either zero or nil").
    nums: HashMap<&'static str, i64>,
    /// The list-valued fields, by name. Absent means `nil`.
    lists: HashMap<&'static str, usize>,
    glue_set: f64,
}

/// The nodes one document's Lua state has made.
///
/// Freed cells are left as `None` and their indices are never reused. LuaTeX
/// warns that "equality tests can only be trusted under very limited
/// conditions […] in that case, there will be false positives" precisely
/// because it DOES reuse node memory; not reusing costs nothing here and makes
/// `==` mean what a chunk expects.
#[derive(Default)]
pub(super) struct Arena {
    cells: Vec<Option<Cell>>,
}

impl Arena {
    fn make(&mut self, id: u8, subtype: u16) -> usize {
        self.cells.push(Some(Cell {
            id,
            subtype,
            ..Cell::default()
        }));
        self.cells.len() // 1-based, so index 0 is never a node
    }

    fn get(&self, at: usize) -> mlua::Result<&Cell> {
        self.cells
            .get(at.wrapping_sub(1))
            .and_then(|c| c.as_ref())
            .ok_or_else(|| mlua::Error::runtime("this node has been freed"))
    }

    fn get_mut(&mut self, at: usize) -> mlua::Result<&mut Cell> {
        self.cells
            .get_mut(at.wrapping_sub(1))
            .and_then(|c| c.as_mut())
            .ok_or_else(|| mlua::Error::runtime("this node has been freed"))
    }

    fn free(&mut self, at: usize) -> mlua::Result<Option<usize>> {
        let next = self.get(at)?.next;
        // Unlink, so a neighbour still held by a chunk does not point at a
        // hole: LuaTeX leaves that to the caller, but a dangling index here
        // would be an error message rather than the wrong number, and the
        // cheap version is simply not to leave one.
        if let Some(p) = self.get(at)?.prev {
            if let Ok(c) = self.get_mut(p) {
                c.next = next;
            }
        }
        if let Some(n) = next {
            let p = self.get(at)?.prev;
            if let Ok(c) = self.get_mut(n) {
                c.prev = p;
            }
        }
        self.cells[at - 1] = None;
        Ok(next)
    }

    /// The nodes of a list, head first, with the cycle guard.
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
            at = self.get(i)?.next;
        }
        Ok(out)
    }

    /// A deep copy of one node: nested lists included, `next` not (the manual:
    /// "Only the `next` field is not copied").
    fn copy(&mut self, at: usize) -> mlua::Result<usize> {
        let mut cell = self.get(at)?.clone();
        cell.next = None;
        cell.prev = None;
        let lists: Vec<(&'static str, usize)> = cell.lists.iter().map(|(k, v)| (*k, *v)).collect();
        for (name, head) in lists {
            match self.copy_list(head, None)? {
                Some(copied) => cell.lists.insert(name, copied),
                None => cell.lists.remove(name),
            };
        }
        self.cells.push(Some(cell));
        Ok(self.cells.len())
    }

    /// A deep copy of a whole list, stopping before `stop`.
    ///
    /// `None` when there is nothing to copy: measured, `node.copy_list(a, a)`
    /// answers nil in luatex 1.24.0 rather than an empty something.
    fn copy_list(&mut self, head: usize, stop: Option<usize>) -> mlua::Result<Option<usize>> {
        let chain = self.chain(head, stop)?;
        let mut made: Vec<usize> = Vec::with_capacity(chain.len());
        for at in chain {
            made.push(self.copy(at)?);
        }
        for w in made.windows(2) {
            self.get_mut(w[0])?.next = Some(w[1]);
            self.get_mut(w[1])?.prev = Some(w[0]);
        }
        Ok(made.first().copied())
    }
}

// ── the userdata ─────────────────────────────────────────────────────────

/// A Lua handle on one arena cell. `node.is_node` answers the index, which is
/// what luatex answers ("returns a number (the internal index of the node)").
#[derive(Clone)]
struct NodeRef {
    arena: Rc<RefCell<Arena>>,
    at: usize,
}

impl NodeRef {
    fn read(&self, lua: &Lua, name: &str) -> mlua::Result<Value> {
        let name = canonical(name);
        let arena = self.arena.borrow();
        let cell = arena.get(self.at)?;
        // A glue_spec is not a list item: luatex gives it `next` and `id` and
        // nothing else, and answers nil for `subtype` and `prev`.
        if cell.id == GLUE_SPEC && matches!(name, "subtype" | "prev") {
            return Ok(Value::Nil);
        }
        match name {
            "id" => return Ok(Value::Integer(cell.id as i64)),
            "subtype" => return Ok(Value::Integer(cell.subtype as i64)),
            "next" | "prev" => {
                let to = match name {
                    "next" => cell.next,
                    _ => cell.prev,
                };
                drop(arena);
                return match to {
                    Some(at) => self.sibling(lua, at),
                    None => Ok(Value::Nil),
                };
            }
            _ => {}
        }
        let Some(field) = fields_of(cell.id).iter().find(|f| f.name == name) else {
            // A field LuaTeX has and texrs has not is an error; a name that is
            // not a field in either engine is `nil`, as luatex answers.
            return match absent_field(cell.id, name) {
                Some(why) => Err(mlua::Error::runtime(format!(
                    "`{name}` on a {} node: {why}",
                    type_name(cell.id).unwrap_or("?")
                ))),
                None => Ok(Value::Nil),
            };
        };
        match field.kind {
            Kind::Num => Ok(Value::Integer(cell.nums.get(name).copied().unwrap_or(0))),
            Kind::Ratio => Ok(Value::Number(cell.glue_set)),
            Kind::Derived => Ok(Value::Integer(0)),
            Kind::List => match cell.lists.get(name).copied() {
                Some(at) => {
                    drop(arena);
                    self.sibling(lua, at)
                }
                None => Ok(Value::Nil),
            },
        }
    }

    fn write(&self, name: &str, value: Value) -> mlua::Result<()> {
        let name = canonical(name);
        let id = self.arena.borrow().get(self.at)?.id;
        match name {
            "id" => {
                return Err(mlua::Error::runtime(
                    "a node's `id` is its type and is fixed at node.new; luatex \
                     lets you overwrite it and texrs does not, because the \
                     fields already stored would then belong to no type",
                ))
            }
            "subtype" => {
                let n = want_int(&value)?;
                let subtype = u16::try_from(n).map_err(|_| bad_subtype(n))?;
                self.arena.borrow_mut().get_mut(self.at)?.subtype = subtype;
                return Ok(());
            }
            "next" | "prev" => {
                let to = as_node_opt(&value)?;
                let mut arena = self.arena.borrow_mut();
                // Both directions, because LuaTeX's own list operations keep
                // `prev` in step and a chunk that sets only `next` and then
                // calls `node.tail` would otherwise get a broken answer.
                let cell = arena.get_mut(self.at)?;
                match name {
                    "next" => cell.next = to,
                    _ => cell.prev = to,
                }
                if let Some(other) = to {
                    let back = arena.get_mut(other)?;
                    match name {
                        "next" => back.prev = Some(self.at),
                        _ => back.next = Some(self.at),
                    }
                }
                return Ok(());
            }
            _ => {}
        }
        let Some(field) = fields_of(id).iter().find(|f| f.name == name) else {
            return Err(match absent_field(id, name) {
                Some(why) => mlua::Error::runtime(format!(
                    "`{name}` on a {} node: {why}",
                    type_name(id).unwrap_or("?")
                )),
                // luatex's own wording, verbatim: "You cannot set field width
                // in a node of type penalty".
                None => mlua::Error::runtime(format!(
                    "You cannot set field {name} in a node of type {}",
                    type_name(id).unwrap_or("?")
                )),
            });
        };
        match field.kind {
            Kind::Num => {
                let n = want_int(&value)?;
                // A glue order is LuaTeX's numbering; `fi` has no unit here and
                // is refused by the same translation the registers use.
                if name == "stretch_order" || name == "shrink_order" {
                    from_luatex_order(n)?;
                }
                self.arena
                    .borrow_mut()
                    .get_mut(self.at)?
                    .nums
                    .insert(field.name, n);
            }
            Kind::Ratio => {
                let r = value
                    .as_number()
                    .ok_or_else(|| mlua::Error::runtime("glue_set wants a number"))?;
                self.arena.borrow_mut().get_mut(self.at)?.glue_set = r;
            }
            // Accepted and dropped, which is luatex's own behaviour: a glyph's
            // dimensions come from the font, so the assignment has no effect
            // there either.
            Kind::Derived => {}
            Kind::List => {
                let to = as_node_opt(&value)?;
                let mut arena = self.arena.borrow_mut();
                let cell = arena.get_mut(self.at)?;
                match to {
                    Some(at) => cell.lists.insert(field.name, at),
                    None => cell.lists.remove(field.name),
                };
            }
        }
        Ok(())
    }

    /// Another node of the same arena, as Lua sees it.
    fn sibling(&self, lua: &Lua, at: usize) -> mlua::Result<Value> {
        Ok(Value::UserData(lua.create_userdata(NodeRef {
            arena: Rc::clone(&self.arena),
            at,
        })?))
    }

    /// luatex's own `tostring`, whose shape a chunk may well be printing:
    /// `<node    241 <    248 >    nil : kern 0>`.
    fn show(&self) -> mlua::Result<String> {
        let arena = self.arena.borrow();
        let cell = arena.get(self.at)?;
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
            cell.subtype
        ))
    }
}

impl UserData for NodeRef {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::Index, |lua, this, key: Value| match &key {
            Value::String(s) => this.read(lua, &s.to_str()?),
            _ => Ok(Value::Nil),
        });
        methods.add_meta_method(
            MetaMethod::NewIndex,
            |_, this, (key, value): (Value, Value)| {
                let Value::String(s) = &key else {
                    return Err(mlua::Error::runtime("a node field is named by a string"));
                };
                this.write(&s.to_str()?, value)
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

fn bad_subtype(n: i64) -> mlua::Error {
    mlua::Error::runtime(format!("{n} is not a subtype"))
}

fn want_int(v: &Value) -> mlua::Result<i64> {
    v.as_integer()
        .or_else(|| v.as_number().map(|n| n.round() as i64))
        .ok_or_else(|| mlua::Error::runtime("a node field wants a number"))
}

/// The arena index a Lua value stands for, or `None` for `nil`.
fn as_node_opt(v: &Value) -> mlua::Result<Option<usize>> {
    match v {
        Value::Nil => Ok(None),
        Value::UserData(u) => Ok(Some(u.borrow::<NodeRef>()?.at)),
        _ => Err(mlua::Error::runtime("expected a node or nil")),
    }
}

/// The arena index a Lua value stands for.
fn as_node(v: &Value) -> mlua::Result<usize> {
    as_node_opt(v)?.ok_or_else(|| mlua::Error::runtime("expected a node"))
}

// ── to texrs's own node list ─────────────────────────────────────────────

/// A subtype as `crate::node`'s glue source (tex.web §149's subtype).
///
/// The LuaTeX subtypes texrs has no name for map to `Normal`, which loses
/// nothing a chunk can see: the source is carried "because the page builder and
/// `\showlists` both name it, not because the arithmetic reads it"
/// (`crate::node`), the arena keeps the real subtype, and this conversion feeds
/// `hpack` alone.
fn glue_source(subtype: u16) -> GlueSource {
    match subtype {
        1 => GlueSource::LineSkip,
        2 => GlueSource::BaselineSkip,
        3 => GlueSource::ParSkip,
        8 => GlueSource::LeftSkip,
        9 => GlueSource::RightSkip,
        10 => GlueSource::TopSkip,
        11 => GlueSource::SplitTopSkip,
        15 => GlueSource::ParFillSkip,
        _ => GlueSource::Normal,
    }
}

/// LuaTeX's leader subtypes: 100 `\leaders`, 101 `\cleaders`, 102 `\xleaders`.
fn leader_kind(subtype: u16) -> LeaderKind {
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

impl Cell {
    fn num(&self, name: &str) -> Scaled {
        self.nums.get(name).copied().unwrap_or(0)
    }
}

/// One arena node as the node `crate::pack` packages.
fn to_engine(arena: &Arena, at: usize) -> mlua::Result<ENode> {
    let cell = arena.get(at)?;
    let list = |name: &str| -> mlua::Result<Vec<ENode>> {
        match cell.lists.get(name).copied() {
            Some(head) => list_to_engine(arena, head),
            None => Ok(Vec::new()),
        }
    };
    Ok(match cell.id {
        HLIST | VLIST => ENode::Box(BoxNode {
            width: cell.num("width"),
            depth: cell.num("depth"),
            height: cell.num("height"),
            shift_amount: cell.num("shift"),
            list: list("head")?,
            glue_set: cell.glue_set,
            glue_sign: glue_sign(cell.num("glue_sign")),
            glue_order: from_luatex_order(cell.num("glue_order"))?,
            vertical: cell.id == VLIST,
        }),
        RULE => ENode::Rule(RuleNode {
            width: cell.num("width"),
            depth: cell.num("depth"),
            height: cell.num("height"),
        }),
        ADJUST => ENode::Adjust(list("head")?),
        DISC => ENode::Disc(DiscNode {
            pre_break: list("pre")?,
            post_break: list("post")?,
            replace_count: 0,
        }),
        MATH => ENode::Math(cell.num("surround")),
        GLUE => ENode::Glue(GlueNode {
            spec: Glue {
                natural: cell.num("width"),
                stretch: cell.num("stretch"),
                stretch_order: from_luatex_order(cell.num("stretch_order"))?,
                shrink: cell.num("shrink"),
                shrink_order: from_luatex_order(cell.num("shrink_order"))?,
            },
            kind: leader_kind(cell.subtype),
            source: glue_source(cell.subtype),
            leader: match cell.lists.get("leader").copied() {
                Some(at) => Some(Box::new(to_engine(arena, at)?)),
                None => None,
            },
        }),
        // tex.web §155: subtype `explicit` is 1, and an explicit kern is a
        // legal breakpoint where a font kern is not.
        KERN => ENode::Kern {
            width: cell.num("kern"),
            explicit: cell.subtype == 1,
        },
        PENALTY => ENode::Penalty(cell.num("penalty")),
        GLYPH => {
            // Dimensions zero: the only font id reachable here is 0, and a
            // glyph with font 0 measures zero in luatex too.
            let ch = CharNode {
                font: 0,
                character: char::from_u32(cell.num("char") as u32).unwrap_or('\0'),
                width: 0,
                height: 0,
                depth: 0,
            };
            match cell.subtype {
                2 => ENode::Ligature(ch),
                _ => ENode::Char(ch),
            }
        }
        GLUE_SPEC => {
            return Err(mlua::Error::runtime(
                "a glue_spec is a register's value, not an item of a list: put \
                 its numbers on a glue node instead",
            ))
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "a {} node cannot be packaged here: {}",
                type_name(other).unwrap_or("?"),
                why_not_new(other)
            )))
        }
    })
}

fn list_to_engine(arena: &Arena, head: usize) -> mlua::Result<Vec<ENode>> {
    arena
        .chain(head, None)?
        .into_iter()
        .map(|at| to_engine(arena, at))
        .collect()
}

// ── the library ──────────────────────────────────────────────────────────

/// Build the `node` table.
pub(super) fn table(lua: &Lua, arena: &Rc<RefCell<Arena>>) -> mlua::Result<Table> {
    let node = lua.create_table()?;
    let hold = |a: &Rc<RefCell<Arena>>| Rc::clone(a);

    // — the type vocabulary, answered for every LuaTeX id whether or not
    //   `node.new` will make one —
    node.set(
        "id",
        lua.create_function(|_, v: Value| Ok(want_id(&v)? as i64))?,
    )?;
    node.set(
        "type",
        lua.create_function(|_, v: Value| {
            if let Value::UserData(u) = &v {
                let r = u.borrow::<NodeRef>()?;
                let id = r.arena.borrow().get(r.at)?.id;
                return Ok(type_name(id).map(str::to_string));
            }
            // "returns nil if the argument is not a node type".
            Ok(want_id(&v).ok().and_then(type_name).map(str::to_string))
        })?,
    )?;
    node.set(
        "types",
        lua.create_function(|lua, ()| {
            let t = lua.create_table()?;
            for (id, name) in TYPES {
                t.set(*id as i64, *name)?;
            }
            Ok(t)
        })?,
    )?;
    node.set(
        "subtypes",
        lua.create_function(|lua, v: Value| {
            let t = lua.create_table()?;
            for (n, name) in subtypes_of(want_id(&v)?) {
                t.set(*n, *name)?;
            }
            Ok(t)
        })?,
    )?;
    node.set(
        "fields",
        lua.create_function(|lua, v: Value| {
            let id = want_id(&v)?;
            let t = lua.create_table()?;
            if id != GLUE_SPEC {
                t.set(-1, "prev")?;
                t.set(2, "subtype")?;
            }
            t.set(0, "next")?;
            t.set(1, "id")?;
            for field in fields_of(id) {
                t.set(field.index, field.name)?;
            }
            Ok(t)
        })?,
    )?;

    // — making and unmaking —
    let a = hold(arena);
    node.set(
        "new",
        lua.create_function(move |lua, args: Variadic<Value>| {
            let id = want_id(args.first().unwrap_or(&Value::Nil))?;
            if fields_of(id).is_empty() {
                return Err(mlua::Error::runtime(format!(
                    "node.new('{}'): {}",
                    type_name(id).unwrap_or("?"),
                    why_not_new(id)
                )));
            }
            let subtype = match args.get(1) {
                Some(v) if !v.is_nil() => {
                    let n = want_int(v)?;
                    u16::try_from(n).map_err(|_| bad_subtype(n))?
                }
                _ => 0,
            };
            let at = a.borrow_mut().make(id, subtype);
            lua.create_userdata(NodeRef {
                arena: Rc::clone(&a),
                at,
            })
        })?,
    )?;
    let a = hold(arena);
    node.set(
        "free",
        lua.create_function(move |lua, v: Value| {
            let at = as_node(&v)?;
            let next = a.borrow_mut().free(at)?;
            match next {
                Some(n) => Ok(Value::UserData(lua.create_userdata(NodeRef {
                    arena: Rc::clone(&a),
                    at: n,
                })?)),
                None => Ok(Value::Nil),
            }
        })?,
    )?;
    let a = hold(arena);
    node.set(
        "flush_node",
        lua.create_function(move |_, v: Value| {
            a.borrow_mut().free(as_node(&v)?)?;
            Ok(())
        })?,
    )?;
    let a = hold(arena);
    node.set(
        "flush_list",
        lua.create_function(move |_, v: Value| {
            let head = as_node(&v)?;
            let mut arena = a.borrow_mut();
            for at in arena.chain(head, None)? {
                arena.free(at)?;
            }
            Ok(())
        })?,
    )?;
    let a = hold(arena);
    node.set(
        "copy",
        lua.create_function(move |lua, v: Value| {
            let at = a.borrow_mut().copy(as_node(&v)?)?;
            lua.create_userdata(NodeRef {
                arena: Rc::clone(&a),
                at,
            })
        })?,
    )?;
    let a = hold(arena);
    node.set(
        "copy_list",
        lua.create_function(move |lua, args: Variadic<Value>| {
            let head = as_node(args.first().unwrap_or(&Value::Nil))?;
            let stop = match args.get(1) {
                Some(v) => as_node_opt(v)?,
                None => None,
            };
            let at = a.borrow_mut().copy_list(head, stop)?;
            Ok(match at {
                Some(at) => Value::UserData(lua.create_userdata(NodeRef {
                    arena: Rc::clone(&a),
                    at,
                })?),
                None => Value::Nil,
            })
        })?,
    )?;

    // — walking —
    for (name, forward) in [
        ("next", true),
        ("prev", false),
        ("getnext", true),
        ("getprev", false),
    ] {
        let a = hold(arena);
        node.set(
            name,
            lua.create_function(move |lua, v: Value| {
                let at = as_node(&v)?;
                let to = {
                    let arena = a.borrow();
                    let cell = arena.get(at)?;
                    match forward {
                        true => cell.next,
                        false => cell.prev,
                    }
                };
                Ok(match to {
                    Some(at) => Value::UserData(lua.create_userdata(NodeRef {
                        arena: Rc::clone(&a),
                        at,
                    })?),
                    None => Value::Nil,
                })
            })?,
        )?;
    }
    let a = hold(arena);
    node.set(
        "getboth",
        lua.create_function(move |lua, v: Value| {
            let at = as_node(&v)?;
            let (p, n) = {
                let arena = a.borrow();
                let cell = arena.get(at)?;
                (cell.prev, cell.next)
            };
            let hand = |i: Option<usize>| -> mlua::Result<Value> {
                Ok(match i {
                    Some(at) => Value::UserData(lua.create_userdata(NodeRef {
                        arena: Rc::clone(&a),
                        at,
                    })?),
                    None => Value::Nil,
                })
            };
            Ok(Variadic::from_iter([hand(p)?, hand(n)?]))
        })?,
    )?;
    for name in ["tail", "slide"] {
        let a = hold(arena);
        node.set(
            name,
            lua.create_function(move |lua, v: Value| {
                let head = as_node(&v)?;
                let last = *a.borrow().chain(head, None)?.last().unwrap_or(&head);
                lua.create_userdata(NodeRef {
                    arena: Rc::clone(&a),
                    at: last,
                })
            })?,
        )?;
    }
    let a = hold(arena);
    node.set(
        "length",
        lua.create_function(move |_, args: Variadic<Value>| {
            let head = as_node(args.first().unwrap_or(&Value::Nil))?;
            let stop = match args.get(1) {
                Some(v) => as_node_opt(v)?,
                None => None,
            };
            Ok(a.borrow().chain(head, stop)?.len() as i64)
        })?,
    )?;
    let a = hold(arena);
    node.set(
        "count",
        lua.create_function(move |_, args: Variadic<Value>| {
            let id = want_id(args.first().unwrap_or(&Value::Nil))?;
            let head = as_node(args.get(1).unwrap_or(&Value::Nil))?;
            let stop = match args.get(2) {
                Some(v) => as_node_opt(v)?,
                None => None,
            };
            let arena = a.borrow();
            let mut n = 0;
            for at in arena.chain(head, stop)? {
                if arena.get(at)?.id == id {
                    n += 1;
                }
            }
            Ok(n)
        })?,
    )?;

    // "for n, id, subtype in node.traverse(head) do" — three values per step,
    // measured in luatex 1.24.0.
    for (name, filter) in [
        ("traverse", Filter::All),
        ("traverse_id", Filter::Id),
        ("traverse_list", Filter::Lists),
        ("traverse_char", Filter::Chars),
        ("traverse_glyph", Filter::Glyphs),
    ] {
        let a = hold(arena);
        node.set(
            name,
            lua.create_function(move |lua, args: Variadic<Value>| {
                traverse(lua, &a, filter, &args)
            })?,
        )?;
    }

    // — editing —
    let a = hold(arena);
    node.set(
        "insert_after",
        lua.create_function(move |lua, args: Variadic<Value>| insert(lua, &a, &args, true))?,
    )?;
    let a = hold(arena);
    node.set(
        "insert_before",
        lua.create_function(move |lua, args: Variadic<Value>| insert(lua, &a, &args, false))?,
    )?;
    let a = hold(arena);
    node.set(
        "remove",
        lua.create_function(move |lua, args: Variadic<Value>| {
            let head = as_node(args.first().unwrap_or(&Value::Nil))?;
            let cur = as_node(args.get(1).unwrap_or(&Value::Nil))?;
            let (prev, next) = {
                let arena = a.borrow();
                let cell = arena.get(cur)?;
                (cell.prev, cell.next)
            };
            {
                let mut arena = a.borrow_mut();
                if let Some(p) = prev {
                    arena.get_mut(p)?.next = next;
                }
                if let Some(n) = next {
                    arena.get_mut(n)?.prev = prev;
                }
                let cell = arena.get_mut(cur)?;
                cell.prev = None;
                cell.next = None;
            }
            // "returns the new head and current" — and when the removed node
            // was the head, the new head is what followed it. Measured: with
            // the tail removed, `current` comes back nil.
            let new_head = match cur == head {
                true => next,
                false => Some(head),
            };
            let hand = |i: Option<usize>| -> mlua::Result<Value> {
                Ok(match i {
                    Some(at) => Value::UserData(lua.create_userdata(NodeRef {
                        arena: Rc::clone(&a),
                        at,
                    })?),
                    None => Value::Nil,
                })
            };
            Ok(Variadic::from_iter([hand(new_head)?, hand(next)?]))
        })?,
    )?;

    // — the getters and setters that name a field —
    let a = hold(arena);
    node.set(
        "getfield",
        lua.create_function(move |lua, (v, name): (Value, String)| {
            let at = as_node(&v)?;
            NodeRef {
                arena: Rc::clone(&a),
                at,
            }
            .read(lua, &name)
        })?,
    )?;
    let a = hold(arena);
    node.set(
        "setfield",
        lua.create_function(move |_, (v, name, value): (Value, String, Value)| {
            let at = as_node(&v)?;
            NodeRef {
                arena: Rc::clone(&a),
                at,
            }
            .write(&name, value)
        })?,
    )?;
    let a = hold(arena);
    node.set(
        "has_field",
        lua.create_function(move |_, (v, name): (Value, String)| {
            let at = as_node(&v)?;
            let id = a.borrow().get(at)?.id;
            let name = canonical(&name);
            let structural = match id {
                GLUE_SPEC => ["id", "next"].contains(&name),
                _ => ["id", "subtype", "next", "prev"].contains(&name),
            };
            Ok(structural || fields_of(id).iter().any(|f| f.name == name))
        })?,
    )?;
    for (name, field) in [
        ("getid", "id"),
        ("getsubtype", "subtype"),
        ("getchar", "char"),
        ("getfont", "font"),
        ("getlist", "head"),
        ("getleader", "leader"),
    ] {
        let a = hold(arena);
        node.set(
            name,
            lua.create_function(move |lua, v: Value| {
                let at = as_node(&v)?;
                NodeRef {
                    arena: Rc::clone(&a),
                    at,
                }
                .read(lua, field)
            })?,
        )?;
    }
    let a = hold(arena);
    node.set(
        "getwhd",
        lua.create_function(move |_, v: Value| {
            let at = as_node(&v)?;
            let arena = a.borrow();
            let cell = arena.get(at)?;
            // A glyph's are the font's, and the only font here is 0.
            let whd = match cell.id {
                GLYPH => (0, 0, 0),
                _ => (cell.num("width"), cell.num("height"), cell.num("depth")),
            };
            Ok(Variadic::from_iter(
                [whd.0, whd.1, whd.2].map(Value::Integer),
            ))
        })?,
    )?;
    let a = hold(arena);
    node.set(
        "is_node",
        lua.create_function(move |_, v: Value| match as_node_opt(&v) {
            Ok(Some(at)) if a.borrow().get(at).is_ok() => Ok(Value::Integer(at as i64)),
            _ => Ok(Value::Boolean(false)),
        })?,
    )?;
    let a = hold(arena);
    node.set(
        "is_char",
        lua.create_function(move |_, v: Value| {
            let at = as_node(&v)?;
            let arena = a.borrow();
            let cell = arena.get(at)?;
            // Measured: for a non-glyph luatex answers nil, not false.
            Ok(match cell.id == GLYPH {
                true => Value::Integer(cell.num("char")),
                false => Value::Nil,
            })
        })?,
    )?;
    let a = hold(arena);
    node.set(
        "is_glyph",
        lua.create_function(move |_, v: Value| {
            let at = as_node(&v)?;
            let arena = a.borrow();
            let cell = arena.get(at)?;
            // Measured: `false, <id>` for a non-glyph.
            Ok(match cell.id == GLYPH {
                true => Variadic::from_iter([Value::Integer(cell.num("char")), Value::Integer(0)]),
                false => {
                    Variadic::from_iter([Value::Boolean(false), Value::Integer(cell.id as i64)])
                }
            })
        })?,
    )?;
    let a = hold(arena);
    node.set(
        "tostring",
        lua.create_function(move |_, v: Value| {
            let at = as_node(&v)?;
            NodeRef {
                arena: Rc::clone(&a),
                at,
            }
            .show()
        })?,
    )?;

    // — glue —
    let a = hold(arena);
    node.set(
        "getglue",
        lua.create_function(move |_, v: Value| {
            let at = as_node(&v)?;
            let arena = a.borrow();
            let cell = arena.get(at)?;
            want_glue(cell)?;
            Ok(Variadic::from_iter(
                [
                    cell.num("width"),
                    cell.num("stretch"),
                    cell.num("shrink"),
                    cell.num("stretch_order"),
                    cell.num("shrink_order"),
                ]
                .map(Value::Integer),
            ))
        })?,
    )?;
    let a = hold(arena);
    node.set(
        "setglue",
        lua.create_function(move |_, args: Variadic<Value>| {
            let at = as_node(args.first().unwrap_or(&Value::Nil))?;
            let mut arena = a.borrow_mut();
            want_glue(arena.get(at)?)?;
            // "If you pass no values or if a value is not a number the
            // corresponding property will become a zero."
            let n = |i: usize| args.get(i).and_then(|v| v.as_integer()).unwrap_or(0);
            for (i, name) in [
                "width",
                "stretch",
                "shrink",
                "stretch_order",
                "shrink_order",
            ]
            .into_iter()
            .enumerate()
            {
                let v = n(i + 1);
                if name.ends_with("order") {
                    from_luatex_order(v)?;
                }
                arena.get_mut(at)?.nums.insert(
                    fields_of(GLUE)
                        .iter()
                        .find(|f| f.name == name)
                        .expect("glue field")
                        .name,
                    v,
                );
            }
            Ok(())
        })?,
    )?;
    let a = hold(arena);
    node.set(
        "is_zero_glue",
        lua.create_function(move |_, v: Value| {
            let at = as_node(&v)?;
            let arena = a.borrow();
            let cell = arena.get(at)?;
            want_glue(cell)?;
            Ok(["width", "stretch", "shrink"]
                .into_iter()
                .all(|f| cell.num(f) == 0))
        })?,
    )?;
    // "the effective width of the glue in the parent box": what §625's setter
    // gives this glue node, which is texrs's own exact arithmetic — cur_glue
    // and cur_g carried from one glue node to the next, so a box of glue comes
    // out exactly as wide as it was asked to be. luatex answers a float
    // (`3.0000000596046` for a glue that should be exactly 3sp), because it
    // multiplies by a single-precision glue ratio and does not carry the
    // rounding; the integer here is the width the engine would really set.
    let a = hold(arena);
    node.set(
        "effective_glue",
        lua.create_function(move |_, (g, parent): (Value, Value)| {
            let at = as_node(&g)?;
            let boxed = as_node(&parent)?;
            let arena = a.borrow();
            want_glue(arena.get(at)?)?;
            let ENode::Box(b) = to_engine(&arena, boxed)? else {
                return Err(mlua::Error::runtime(
                    "node.effective_glue wants the box the glue is in",
                ));
            };
            let mut setter = crate::pack::Setter::new(&b);
            let head = arena.get(boxed)?.lists.get("head").copied();
            let Some(head) = head else {
                return Err(mlua::Error::runtime("that box is empty"));
            };
            for i in arena.chain(head, None)? {
                let cell = arena.get(i)?;
                if cell.id != GLUE {
                    continue;
                }
                let ENode::Glue(gn) = to_engine(&arena, i)? else {
                    unreachable!("a glue cell converts to a glue node")
                };
                let w = setter.glue(&gn.spec);
                if i == at {
                    return Ok(w);
                }
            }
            Err(mlua::Error::runtime("that glue is not in that box"))
        })?,
    )?;

    // — packaging: the one place a chunk reaches tex.web's own arithmetic —
    for (name, vertical) in [("hpack", false), ("vpack", true)] {
        let a = hold(arena);
        node.set(
            name,
            lua.create_function(move |lua, args: Variadic<Value>| pack(lua, &a, &args, vertical))?,
        )?;
    }
    let a = hold(arena);
    node.set(
        "dimensions",
        lua.create_function(move |_, args: Variadic<Value>| dimensions(&a, &args))?,
    )?;

    // — everything that would reach the document, or a font, or an attribute —
    for (name, why) in REFUSED {
        node.set(
            *name,
            lua.create_function(move |_, _: Variadic<Value>| {
                Err::<(), _>(mlua::Error::runtime(format!("node.{name}: {why}")))
            })?,
        )?;
    }
    // `node.direct` is the same library over bare integers instead of userdata.
    // Every one of its entries would have to refuse or answer, and a table that
    // refuses as a whole says the same thing in one place.
    let direct = lua.create_table()?;
    let mt = lua.create_table()?;
    let refuse = lua.create_function(|_, _: Variadic<Value>| {
        Err::<(), _>(mlua::Error::runtime(
            "node.direct is LuaTeX's unchecked view of node memory as bare \
             integers; texrs's nodes are arena handles and there is no address \
             to hand out. Use the node library itself.",
        ))
    })?;
    mt.set("__index", refuse.clone())?;
    mt.set("__newindex", refuse)?;
    direct.set_metatable(Some(mt))?;
    node.set("direct", direct)?;

    Ok(node)
}

/// What each refused entry point is, and why it is refused.
///
/// Named one by one rather than left absent: "attempt to call a nil value"
/// would not tell a document's author which of the two engines' worlds it had
/// walked into.
const REFUSED: &[(&str, &str)] = &[
    (
        "write",
        "there is no current node list to append to. texrs sets a page from \
         runs of strings (src/typeset.rs); crate::postline and crate::page are \
         a library beside that path rather than the path itself",
    ),
    (
        "last_node",
        "there is no current node list to take the last node off",
    ),
    (
        "usedlist",
        "texrs's nodes live in a per-document arena rather than in one node \
         memory it can enumerate",
    ),
    ("current_attr", "texrs's nodes carry no attribute list"),
    ("has_attribute", "texrs's nodes carry no attribute list"),
    ("get_attribute", "texrs's nodes carry no attribute list"),
    ("set_attribute", "texrs's nodes carry no attribute list"),
    ("unset_attribute", "texrs's nodes carry no attribute list"),
    ("find_attribute", "texrs's nodes carry no attribute list"),
    (
        "ligaturing",
        "this applies the font's ligature program to a list, and texrs does not \
         run a .tfm's lig/kern program at all (see BUGS.md)",
    ),
    (
        "kerning",
        "this applies the font's kern program to a list, and texrs does not run \
         a .tfm's lig/kern program at all (see BUGS.md)",
    ),
    (
        "hyphenating",
        "this needs Liang's patterns applied to a node list; texrs hyphenates \
         inside the line breaker, over text rather than over nodes",
    ),
    (
        "mlist_to_hlist",
        "the noad types (LuaTeX ids 16-27) have no crate::node::Node variant, \
         so there is no node list to make an mlist out of",
    ),
    (
        "first_glyph",
        "a glyph here can only carry font 0, so there is no glyph with a font \
         to find",
    ),
    ("has_glyph", "a glyph here can only carry font 0"),
    ("uses_font", "texrs has no font table for Lua to index"),
    ("family_font", "texrs has no math font families"),
    (
        "make_extensible",
        "texrs has no extensible recipes on the Lua side",
    ),
    ("protect_glyph", "texrs's glyphs have no protection flag"),
    ("protect_glyphs", "texrs's glyphs have no protection flag"),
    ("unprotect_glyph", "texrs's glyphs have no protection flag"),
    ("unprotect_glyphs", "texrs's glyphs have no protection flag"),
    (
        "getdisc",
        "texrs's DiscNode holds tex.web's replace_count, an integer, where \
         LuaTeX's third value is a list",
    ),
    (
        "check_discretionaries",
        "texrs's DiscNode holds tex.web's replace_count rather than a replace list",
    ),
    (
        "flatten_discretionaries",
        "flattening writes the result back into the document's list, and there \
         is none",
    ),
    (
        "fix_node_lists",
        "this repairs prev pointers in TeX's own memory; the arena keeps prev \
         in step on every assignment already",
    ),
    (
        "prepend_prevdepth",
        "\\prevdepth belongs to the page builder, which is not on the path a \
         run takes",
    ),
    (
        "end_of_math",
        "texrs's math node is tex.web §147's surround and carries no begin/end \
         subtype pair",
    ),
    (
        "protrusion_skippable",
        "protrusion is a pdfTeX/LuaTeX addition texrs has not got",
    ),
    (
        "rangedimensions",
        "this measures a range inside a parent box's set list, which needs the \
         parent to be a box TeX itself packaged",
    ),
    ("getproperty", "texrs keeps no per-node properties table"),
    ("setproperty", "texrs keeps no per-node properties table"),
    (
        "get_properties_table",
        "texrs keeps no per-node properties table",
    ),
    (
        "flush_properties_table",
        "texrs keeps no per-node properties table",
    ),
    (
        "set_properties_mode",
        "texrs keeps no per-node properties table",
    ),
    (
        "whatsits",
        "texrs's Node::Whatsit carries one string rather than LuaTeX's zoo of \
         whatsit subtypes",
    ),
    (
        "values",
        "this enumerates LuaTeX's internal value tables, which texrs has not got",
    ),
];

/// Which nodes a `traverse` variant yields.
#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All,
    Id,
    Lists,
    Chars,
    Glyphs,
}

/// A `traverse` iterator: a Lua closure over the list, snapshotted at the call.
///
/// LuaTeX's traversers are safe against the current node being freed or
/// relinked mid-loop ("you can remove the current node safely"), which a live
/// `next` walk is not. Taking the chain up front gives the same guarantee.
fn traverse(
    lua: &Lua,
    arena: &Rc<RefCell<Arena>>,
    filter: Filter,
    args: &[Value],
) -> mlua::Result<mlua::Function> {
    let (id, head_at) = match filter {
        Filter::Id => (
            Some(want_id(args.first().unwrap_or(&Value::Nil))?),
            args.get(1).cloned().unwrap_or(Value::Nil),
        ),
        _ => (None, args.first().cloned().unwrap_or(Value::Nil)),
    };
    let chain = match as_node_opt(&head_at)? {
        Some(head) => arena.borrow().chain(head, None)?,
        None => Vec::new(),
    };
    let kept: Vec<usize> = {
        let a = arena.borrow();
        chain
            .into_iter()
            .filter(|at| {
                let Ok(cell) = a.get(*at) else { return false };
                match filter {
                    Filter::All => true,
                    Filter::Id => Some(cell.id) == id,
                    Filter::Lists => matches!(cell.id, HLIST | VLIST),
                    Filter::Chars | Filter::Glyphs => cell.id == GLYPH,
                }
            })
            .collect()
    };
    let arena = Rc::clone(arena);
    let mut i = 0usize;
    lua.create_function_mut(move |lua, ()| {
        while let Some(&at) = kept.get(i) {
            i += 1;
            let Ok((id, subtype)) = arena.borrow().get(at).map(|c| (c.id, c.subtype)) else {
                // Freed inside the loop: skip it, as LuaTeX's own traverser
                // survives a `node.remove` of the node it just handed out.
                continue;
            };
            return Ok(Variadic::from_iter([
                Value::UserData(lua.create_userdata(NodeRef {
                    arena: Rc::clone(&arena),
                    at,
                })?),
                Value::Integer(id as i64),
                Value::Integer(subtype as i64),
            ]));
        }
        Ok(Variadic::from_iter([Value::Nil]))
    })
}

/// `insert_after(head, current, new)` and its mirror. Both answer `head, new`.
fn insert(
    lua: &Lua,
    arena: &Rc<RefCell<Arena>>,
    args: &[Value],
    after: bool,
) -> mlua::Result<Variadic<Value>> {
    let head = as_node(args.first().unwrap_or(&Value::Nil))?;
    let cur = as_node(args.get(1).unwrap_or(&Value::Nil))?;
    let new = as_node(args.get(2).unwrap_or(&Value::Nil))?;
    let mut a = arena.borrow_mut();
    let new_head = match after {
        true => {
            let next = a.get(cur)?.next;
            a.get_mut(cur)?.next = Some(new);
            let cell = a.get_mut(new)?;
            cell.prev = Some(cur);
            cell.next = next;
            if let Some(n) = next {
                a.get_mut(n)?.prev = Some(new);
            }
            head
        }
        false => {
            let prev = a.get(cur)?.prev;
            a.get_mut(cur)?.prev = Some(new);
            let cell = a.get_mut(new)?;
            cell.next = Some(cur);
            cell.prev = prev;
            if let Some(p) = prev {
                a.get_mut(p)?.next = Some(new);
            }
            match cur == head {
                true => new,
                false => head,
            }
        }
    };
    drop(a);
    let hand = |at: usize| -> mlua::Result<Value> {
        Ok(Value::UserData(lua.create_userdata(NodeRef {
            arena: Rc::clone(arena),
            at,
        })?))
    };
    Ok(Variadic::from_iter([hand(new_head)?, hand(new)?]))
}

/// `node.hpack(n[,w,info])` and `node.vpack`, over `crate::pack`'s port of
/// §649-§667 and §668-§679.
///
/// The tolerances are INITEX's, which is not a choice: they decide only what
/// TeX would COMPLAIN about (`crate::pack::Report`), and that report is
/// discarded here because LuaTeX's `node.hpack` does not report either. The
/// numbers a chunk gets back — width, height, depth, glue_set, glue_sign,
/// glue_order and the badness — do not depend on them at all.
fn pack(
    lua: &Lua,
    arena: &Rc<RefCell<Arena>>,
    args: &[Value],
    vertical: bool,
) -> mlua::Result<Variadic<Value>> {
    let head = as_node(args.first().unwrap_or(&Value::Nil))?;
    let w = match args.get(1) {
        Some(v) if !v.is_nil() => want_int(v)?,
        _ => 0,
    };
    let spec = match args.get(2) {
        Some(Value::String(s)) => match &*s.to_str()? {
            "exactly" => crate::pack::Spec::Exactly(w),
            "additional" => crate::pack::Spec::Additional(w),
            other => {
                return Err(mlua::Error::runtime(format!(
                    "node.{}pack's third argument is 'additional' or 'exactly', not '{other}'",
                    match vertical {
                        true => "v",
                        false => "h",
                    }
                )))
            }
        },
        _ => crate::pack::NATURAL,
    };
    if let Some(v) = args.get(3) {
        if !v.is_nil() {
            return Err(mlua::Error::runtime(
                "node.hpack's direction argument is a LuaTeX addition; texrs's \
                 boxes are TeX82's, left to right",
            ));
        }
    }
    let list = {
        let a = arena.borrow();
        list_to_engine(&a, head)?
    };
    let tol = crate::pack::Tolerances::default();
    let packed = match vertical {
        true => crate::pack::vpack(list, spec, tol),
        false => crate::pack::hpack(list, spec, tol, None),
    };
    let b = &packed.node;
    let at = {
        let mut a = arena.borrow_mut();
        let at = a.make(
            match vertical {
                true => VLIST,
                false => HLIST,
            },
            0,
        );
        let cell = a.get_mut(at)?;
        cell.nums.insert("width", b.width);
        cell.nums.insert("height", b.height);
        cell.nums.insert("depth", b.depth);
        cell.nums.insert("glue_sign", glue_sign_number(b.glue_sign));
        cell.nums
            .insert("glue_order", to_luatex_order(b.glue_order));
        cell.glue_set = b.glue_set;
        // "h is the original node list n": the box's head IS the list handed
        // in, not a copy of it.
        cell.lists.insert("head", head);
        at
    };
    Ok(Variadic::from_iter([
        Value::UserData(lua.create_userdata(NodeRef {
            arena: Rc::clone(arena),
            at,
        })?),
        Value::Integer(packed.badness),
    ]))
}

/// `node.dimensions`: the natural size of a list, or the size it takes at a
/// given glue setting.
///
/// Both forms the manual gives, and the four-argument one is the reason this is
/// not just `hpack(...).width`: it asks what a list measures when its glue is
/// already set, which is §625's arithmetic rather than §649's.
fn dimensions(arena: &Rc<RefCell<Arena>>, args: &[Value]) -> mlua::Result<Variadic<Value>> {
    let setting = matches!(args.first(), Some(v) if v.as_number().is_some());
    let (head, stop, set) = match setting {
        true => (
            as_node(args.get(3).unwrap_or(&Value::Nil))?,
            match args.get(4) {
                Some(v) => as_node_opt(v)?,
                None => None,
            },
            Some((
                args[0].as_number().unwrap_or(0.0),
                want_int(args.get(1).unwrap_or(&Value::Nil))?,
                want_int(args.get(2).unwrap_or(&Value::Nil))?,
            )),
        ),
        false => (
            as_node(args.first().unwrap_or(&Value::Nil))?,
            match args.get(1) {
                Some(v) => as_node_opt(v)?,
                None => None,
            },
            None,
        ),
    };
    let list = {
        let a = arena.borrow();
        a.chain(head, stop)?
            .into_iter()
            .map(|at| to_engine(&a, at))
            .collect::<mlua::Result<Vec<_>>>()?
    };
    let packed = crate::pack::hpack(
        list,
        crate::pack::NATURAL,
        crate::pack::Tolerances::default(),
        None,
    );
    let (mut w, h, d) = (packed.node.width, packed.node.height, packed.node.depth);
    if let Some((ratio, sign, order)) = set {
        // The list, re-measured with the glue set the way the caller says the
        // parent box set it: §625's setter over the same nodes.
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
                ENode::Char(c) | ENode::Ligature(c) => c.width,
                ENode::Kern { width, .. } => *width,
                ENode::Math(s) => *s,
                _ => 0,
            })
            .sum();
    }
    Ok(Variadic::from_iter([w, h, d].map(Value::Integer)))
}

/// The subtype names LuaTeX gives a type, read out of `node.subtypes` in
/// luatex 1.24.0. Only the types `node.new` will make: for anything else the
/// answer would be a list of names for a node that cannot exist here.
fn subtypes_of(id: u8) -> &'static [(i64, &'static str)] {
    match id {
        GLUE => &[
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
        KERN => &[
            (0, "fontkern"),
            (1, "userkern"),
            (2, "accentkern"),
            (3, "italiccorrection"),
        ],
        GLYPH => &[
            (0, "unset"),
            (1, "character"),
            (2, "ligature"),
            (4, "ghost"),
            (8, "left"),
            (16, "right"),
        ],
        PENALTY => &[
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
        RULE => &[
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
        MATH => &[(0, "beginmath"), (1, "endmath")],
        DISC => &[
            (0, "discretionary"),
            (1, "explicit"),
            (2, "automatic"),
            (3, "regular"),
            (4, "first"),
            (5, "second"),
        ],
        _ => &[],
    }
}

/// The glue functions want a glue node and say so rather than reading zeros off
/// a penalty.
fn want_glue(cell: &Cell) -> mlua::Result<()> {
    match cell.id == GLUE {
        true => Ok(()),
        false => Err(mlua::Error::runtime(format!(
            "expected a glue node, got a {}",
            type_name(cell.id).unwrap_or("?")
        ))),
    }
}

// ── the glue_spec half of the register interface ─────────────────────────

/// A `glue_spec` node holding a `\skip` register's five numbers.
///
/// The LuaTeX manual: "The skip registers accept and return `glue_spec`
/// userdata node objects." That is `tex.skip[n]`, `tex.getskip` and
/// `tex.setskip`, and it is the whole difference between them and
/// `tex.getglue`, which is the same registers as bare numbers. It refused here
/// while there was no node interface; a `glue_spec` is a node this arena can
/// carry faithfully — five integers and nothing else — so it does not any more.
///
/// The orders are the caller's: they arrive already in LuaTeX's `fi`-based
/// numbering from [`super::glue_get`].
pub(super) fn spec_node(
    lua: &Lua,
    arena: &Rc<RefCell<Arena>>,
    glue: (i64, i64, i64, i64, i64),
) -> mlua::Result<Value> {
    let at = {
        let mut a = arena.borrow_mut();
        let at = a.make(GLUE_SPEC, 0);
        let cell = a.get_mut(at)?;
        for (name, v) in [
            ("width", glue.0),
            ("stretch", glue.1),
            ("shrink", glue.2),
            ("stretch_order", glue.3),
            ("shrink_order", glue.4),
        ] {
            cell.nums.insert(
                fields_of(GLUE_SPEC)
                    .iter()
                    .find(|f| f.name == name)
                    .expect("glue_spec field")
                    .name,
                v,
            );
        }
        at
    };
    Ok(Value::UserData(lua.create_userdata(NodeRef {
        arena: Rc::clone(arena),
        at,
    })?))
}

/// The five numbers a `glue_spec` (or a `glue` node, which carries the same
/// five) holds, for a register write.
pub(super) fn spec_values(
    arena: &Rc<RefCell<Arena>>,
    v: &Value,
) -> mlua::Result<(i64, i64, i64, i64, i64)> {
    let at = as_node(v)?;
    let a = arena.borrow();
    let cell = a.get(at)?;
    if cell.id != GLUE_SPEC && cell.id != GLUE {
        return Err(mlua::Error::runtime(format!(
            "a \\skip register is set from a glue_spec node, not from a {}",
            type_name(cell.id).unwrap_or("?")
        )));
    }
    Ok((
        cell.num("width"),
        cell.num("stretch"),
        cell.num("shrink"),
        cell.num("stretch_order"),
        cell.num("shrink_order"),
    ))
}
