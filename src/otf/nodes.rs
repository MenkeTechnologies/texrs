//! The node list node mode processes, as LuaTeX's `node.direct` presents it
//! to the fontloader: glyphs with their font, subtype and components,
//! discretionaries with three lists, glue, kerns, direction and paragraph
//! nodes, and the `properties` table the injections live in.
//!
//! Two LuaTeX behaviours the fontloader leans on are kept:
//!
//! - `is_char(n, font)` answers the character of an unprotected glyph of that
//!   font, `false` for a glyph that is not (another font, or protected: subtype
//!   256 and up), and nothing for a node that is not a glyph; the processing
//!   loops branch three ways on it.
//! - `set_properties_mode(true, true)` (fontloader-basics-nod.lua:165-167):
//!   a copied node gets a fresh properties table whose metatable falls back on
//!   the original's, so reading a field through a copy sees the original's
//!   CURRENT value until the copy sets its own. `rawget(properties, n)` is the
//!   copy's own table; a field read is an ordinary index and inherits.
//!
//! Dimensions the Lua sets as floats (`setkern`, `setoffsets`, `setwidth`)
//! are rounded as the engine rounds them when they are stored.

pub type Ix = usize;

/// The engine's rounding of a float assigned to a dimension field.
pub fn round(x: f64) -> i64 {
    (x + 0.5).floor() as i64
}

/// luatex's glyph subtype bits.
pub const GLYPH_CHARACTER: u16 = 1;
pub const GLYPH_LIGATURE: u16 = 2;
pub const GLYPH_PROTECTED: u16 = 256;

/// Glue subtypes the processing tests.
pub const GLUE_SPACESKIP: u16 = 13;
pub const GLUE_XSPACESKIP: u16 = 14;

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Glyph {
        char: u32,
        font: usize,
        subtype: u16,
        components: Option<Ix>,
        xoffset: i64,
        yoffset: i64,
    },
    Glue {
        subtype: u16,
        width: i64,
        stretch: i64,
        shrink: i64,
    },
    Kern {
        subtype: u16,
        kern: i64,
    },
    Disc {
        subtype: u16,
        pre: Option<Ix>,
        post: Option<Ix>,
        replace: Option<Ix>,
    },
    /// `dir 0|1`, the cancelling node of a pair flagged `cancel`.
    Dir {
        dir: u8,
        cancel: bool,
    },
    /// The `local_par` a paragraph begins with.
    Par {
        dir: u8,
    },
    /// A math node: `subtype` 0 opens, 1 closes (`endofmath` looks for it).
    Math {
        subtype: u16,
    },
    Other {
        id: u32,
    },
}

/// One injection table (font-otj.lua's `injections`, `preinjections`, ...).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inj {
    pub leftkern: Option<f64>,
    pub rightkern: Option<f64>,
    pub yoffset: Option<f64>,
    pub markx: Option<f64>,
    pub marky: Option<f64>,
    pub markdir: Option<i32>,
    pub markbase: Option<usize>,
    pub markbasenode: Option<Ix>,
    pub markmark: bool,
    pub checkmark: bool,
    pub cursivex: Option<f64>,
    pub cursivey: Option<f64>,
    pub cursiveanchor: bool,
    pub ligaindex: Option<i64>,
}

/// A field of a properties table: unset (fall back on the original through
/// the metatable), explicitly `false`, or a value.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Field<T> {
    #[default]
    Unset,
    False,
    Value(T),
}

/// Which injection table: the five keys font-otj.lua reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Injections,
    Pre,
    Post,
    Replace,
    Empty,
}

/// [`List::ischar`]'s answer for a node that is not a glyph: the Lua `nil`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotGlyph;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Props {
    pub slots: [Field<Inj>; 5],
    pub cursivedy: Field<f64>,
    /// The analyzer's `state` (font-ota.lua, set with `setprop`).
    pub state: Field<u8>,
    /// The node this one was copied from, whose table this one's metatable
    /// falls back on.
    pub parent: Option<Ix>,
}

fn slot_index(s: Slot) -> usize {
    match s {
        Slot::Injections => 0,
        Slot::Pre => 1,
        Slot::Post => 2,
        Slot::Replace => 3,
        Slot::Empty => 4,
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub kind: Kind,
    pub prev: Option<Ix>,
    pub next: Option<Ix>,
    /// `properties[n]`: `None` is nil, `Some(None)` the `false` that
    /// `injections.reset` stores when there was no table, `Some(Some(t))` a
    /// table.
    pub props: Option<Option<Props>>,
}

#[derive(Clone, Debug, Default)]
pub struct List {
    pub nodes: Vec<Node>,
}

impl List {
    pub fn alloc(&mut self, kind: Kind) -> Ix {
        self.nodes.push(Node {
            kind,
            prev: None,
            next: None,
            props: None,
        });
        self.nodes.len() - 1
    }

    pub fn next(&self, n: Option<Ix>) -> Option<Ix> {
        n.and_then(|i| self.nodes[i].next)
    }

    pub fn prev(&self, n: Option<Ix>) -> Option<Ix> {
        n.and_then(|i| self.nodes[i].prev)
    }

    pub fn set_next(&mut self, n: Option<Ix>, to: Option<Ix>) {
        if let Some(n) = n {
            self.nodes[n].next = to;
        }
    }

    pub fn set_prev(&mut self, n: Option<Ix>, to: Option<Ix>) {
        if let Some(n) = n {
            self.nodes[n].prev = to;
        }
    }

    /// `setlink(...)` as lnodelib.c:1912-1948 does it: each node is coupled
    /// to the last one before it; a nil before any node is skipped, a nil
    /// after one ends the list there (its next becomes nil); a node after a
    /// leading nil loses its prev; a repeated node is ignored.
    pub fn links(&mut self, args: &[Option<Ix>]) {
        let mut t: Option<Ix> = None;
        for (i, a) in args.iter().enumerate() {
            match (*a, t) {
                (Some(c), Some(tt)) if c != tt => {
                    self.nodes[tt].next = Some(c);
                    self.nodes[c].prev = Some(tt);
                    t = Some(c);
                }
                (Some(_), Some(_)) => {}
                (Some(c), None) => {
                    if i > 0 {
                        self.nodes[c].prev = None;
                    }
                    t = Some(c);
                }
                (None, Some(tt)) => self.nodes[tt].next = None,
                (None, None) => {}
            }
        }
    }

    /// `setlink(a, b)`.
    pub fn link(&mut self, a: Option<Ix>, b: Option<Ix>) {
        self.links(&[a, b]);
    }

    /// `setlink(a, b, c)`.
    pub fn link3(&mut self, a: Option<Ix>, b: Option<Ix>, c: Option<Ix>) {
        self.links(&[a, b, c]);
    }

    pub fn tail(&self, head: Option<Ix>) -> Option<Ix> {
        let mut n = head?;
        while let Some(x) = self.nodes[n].next {
            n = x;
        }
        Some(n)
    }

    pub fn id(&self, n: Ix) -> &Kind {
        &self.nodes[n].kind
    }

    pub fn is_glyph(&self, n: Option<Ix>) -> bool {
        n.is_some_and(|n| matches!(self.nodes[n].kind, Kind::Glyph { .. }))
    }

    pub fn is_disc(&self, n: Option<Ix>) -> bool {
        n.is_some_and(|n| matches!(self.nodes[n].kind, Kind::Disc { .. }))
    }

    pub fn is_glue(&self, n: Option<Ix>) -> bool {
        n.is_some_and(|n| matches!(self.nodes[n].kind, Kind::Glue { .. }))
    }

    /// `ischar(n, font)`: `Ok(Some(c))` the character, `Ok(None)` the Lua
    /// `false` (a glyph that does not qualify), `Err(NotGlyph)` not a glyph (nil).
    pub fn ischar(&self, n: Option<Ix>, font: Option<usize>) -> Result<Option<u32>, NotGlyph> {
        let Some(n) = n else {
            return Err(NotGlyph);
        };
        match &self.nodes[n].kind {
            Kind::Glyph {
                char,
                font: f,
                subtype,
                ..
            } => {
                if *subtype >= GLYPH_PROTECTED {
                    return Ok(None);
                }
                match font {
                    Some(font) if font != *f => Ok(None),
                    _ => Ok(Some(*char)),
                }
            }
            _ => Err(NotGlyph),
        }
    }

    /// `isglyph(n)`: the character of any glyph.
    pub fn isglyph(&self, n: Option<Ix>) -> Option<u32> {
        match &self.nodes[n?].kind {
            Kind::Glyph { char, .. } => Some(*char),
            _ => None,
        }
    }

    pub fn char(&self, n: Ix) -> u32 {
        match &self.nodes[n].kind {
            Kind::Glyph { char, .. } => *char,
            _ => 0,
        }
    }

    pub fn set_char(&mut self, n: Ix, c: u32) {
        if let Kind::Glyph { char, .. } = &mut self.nodes[n].kind {
            *char = c;
        }
    }

    pub fn font(&self, n: Ix) -> Option<usize> {
        match &self.nodes[n].kind {
            Kind::Glyph { font, .. } => Some(*font),
            _ => None,
        }
    }

    pub fn subtype(&self, n: Ix) -> u16 {
        match &self.nodes[n].kind {
            Kind::Glyph { subtype, .. }
            | Kind::Glue { subtype, .. }
            | Kind::Kern { subtype, .. }
            | Kind::Disc { subtype, .. }
            | Kind::Math { subtype } => *subtype,
            _ => 0,
        }
    }

    pub fn components(&self, n: Ix) -> Option<Ix> {
        match &self.nodes[n].kind {
            Kind::Glyph { components, .. } => *components,
            _ => None,
        }
    }

    pub fn set_components(&mut self, n: Ix, c: Option<Ix>) {
        if let Kind::Glyph { components, .. } = &mut self.nodes[n].kind {
            *components = c;
        }
    }

    pub fn offsets(&self, n: Ix) -> (i64, i64) {
        match &self.nodes[n].kind {
            Kind::Glyph {
                xoffset, yoffset, ..
            } => (*xoffset, *yoffset),
            _ => (0, 0),
        }
    }

    /// `setoffsets(n, x, y)`: a `None` leaves that offset alone (the Lua
    /// `false`).
    pub fn set_offsets(&mut self, n: Ix, x: Option<f64>, y: Option<f64>) {
        if let Kind::Glyph {
            xoffset, yoffset, ..
        } = &mut self.nodes[n].kind
        {
            if let Some(x) = x {
                *xoffset = round(x);
            }
            if let Some(y) = y {
                *yoffset = round(y);
            }
        }
    }

    pub fn disc(&self, n: Ix) -> (Option<Ix>, Option<Ix>, Option<Ix>) {
        match &self.nodes[n].kind {
            Kind::Disc {
                pre, post, replace, ..
            } => (*pre, *post, *replace),
            _ => (None, None, None),
        }
    }

    /// `getdisc(n, true)`: the lists and their tails.
    #[allow(clippy::type_complexity)]
    pub fn disc_tails(
        &self,
        n: Ix,
    ) -> (
        Option<Ix>,
        Option<Ix>,
        Option<Ix>,
        Option<Ix>,
        Option<Ix>,
        Option<Ix>,
    ) {
        let (pre, post, replace) = self.disc(n);
        (
            pre,
            post,
            replace,
            self.tail(pre),
            self.tail(post),
            self.tail(replace),
        )
    }

    /// `setdisc(n, pre, post, replace)`: the lists' heads lose any prev.
    pub fn set_disc(&mut self, n: Ix, pre: Option<Ix>, post: Option<Ix>, replace: Option<Ix>) {
        for l in [pre, post, replace].into_iter().flatten() {
            self.nodes[l].prev = None;
        }
        if let Kind::Disc {
            pre: p,
            post: q,
            replace: r,
            ..
        } = &mut self.nodes[n].kind
        {
            *p = pre;
            *q = post;
            *r = replace;
        }
    }

    pub fn set_replace(&mut self, n: Ix, replace: Option<Ix>) {
        let (pre, post, _) = self.disc(n);
        self.set_disc(n, pre, post, replace);
    }

    pub fn width_of_glue(&self, n: Ix) -> i64 {
        match &self.nodes[n].kind {
            Kind::Glue { width, .. } => *width,
            _ => 0,
        }
    }

    pub fn set_glue_width(&mut self, n: Ix, w: f64) {
        if let Kind::Glue { width, .. } = &mut self.nodes[n].kind {
            *width = round(w);
        }
    }

    /// `copy(n)`: the node, its lists copied with it; the copy's properties
    /// fall back on the original's (set_properties_mode(true, true)).
    pub fn copy(&mut self, n: Ix) -> Ix {
        let kind = self.nodes[n].kind.clone();
        let kind = match kind {
            Kind::Disc {
                subtype,
                pre,
                post,
                replace,
            } => {
                let pre = self.copy_list(pre, None);
                let post = self.copy_list(post, None);
                let replace = self.copy_list(replace, None);
                Kind::Disc {
                    subtype,
                    pre,
                    post,
                    replace,
                }
            }
            Kind::Glyph {
                char,
                font,
                subtype,
                components,
                xoffset,
                yoffset,
            } => Kind::Glyph {
                char,
                font,
                subtype,
                components: self.copy_list(components, None),
                xoffset,
                yoffset,
            },
            k => k,
        };
        let c = self.alloc(kind);
        if self.has_props(n) {
            self.nodes[c].props = Some(Some(Props {
                parent: Some(n),
                ..Props::default()
            }));
        }
        c
    }

    /// `copy_list(from, upto)`.
    pub fn copy_list(&mut self, from: Option<Ix>, upto: Option<Ix>) -> Option<Ix> {
        let mut head = None;
        let mut last = None;
        let mut n = from;
        while let Some(i) = n {
            if Some(i) == upto {
                break;
            }
            let c = self.copy(i);
            match last {
                Some(l) => self.link(Some(l), Some(c)),
                None => head = Some(c),
            }
            last = Some(c);
            n = self.nodes[i].next;
        }
        head
    }

    /// `insert_before(head, current, new)` (lnodelib.c:2319-2357): returns
    /// the head.
    pub fn insert_before(&mut self, head: Option<Ix>, current: Option<Ix>, new: Ix) -> Option<Ix> {
        let Some(h) = head else {
            self.nodes[new].next = None;
            self.nodes[new].prev = None;
            return Some(new);
        };
        let cur = current.or_else(|| self.tail(head)).unwrap_or(h);
        if h != cur {
            let mut t = self.nodes[cur].prev;
            if t.is_none_or(|t| self.nodes[t].next != Some(cur)) {
                let mut p = h;
                while self.nodes[p].next != Some(cur) {
                    p = self.nodes[p].next.expect("current is in the list");
                }
                t = Some(p);
            }
            let t = t.unwrap();
            self.nodes[t].next = Some(new);
            self.nodes[new].prev = Some(t);
        }
        self.nodes[new].next = Some(cur);
        self.nodes[cur].prev = Some(new);
        Some(if h == cur { new } else { h })
    }

    /// `insert_after(head, current, new)` (lnodelib.c:2399-2429): returns the
    /// head and `new`.
    pub fn insert_after(
        &mut self,
        head: Option<Ix>,
        current: Option<Ix>,
        new: Ix,
    ) -> (Option<Ix>, Option<Ix>) {
        let Some(h) = head else {
            self.nodes[new].next = None;
            self.nodes[new].prev = None;
            return (Some(new), Some(new));
        };
        let cur = current.or_else(|| self.tail(head)).unwrap_or(h);
        match self.nodes[cur].next {
            Some(nx) => {
                self.nodes[new].next = Some(nx);
                self.nodes[nx].prev = Some(new);
            }
            None => self.nodes[new].next = None,
        }
        self.nodes[cur].next = Some(new);
        self.nodes[new].prev = Some(cur);
        (head, Some(new))
    }

    /// `remove(head, current)`: unlinks it; returns the head and the node
    /// after it.
    pub fn remove(&mut self, head: Option<Ix>, current: Option<Ix>) -> (Option<Ix>, Option<Ix>) {
        let Some(cur) = current else {
            return (head, None);
        };
        let (prev, next) = (self.nodes[cur].prev, self.nodes[cur].next);
        self.link(prev, next);
        if prev.is_none() {
            self.set_prev(next, None);
        }
        self.nodes[cur].prev = None;
        self.nodes[cur].next = None;
        (if head == Some(cur) { next } else { head }, next)
    }

    // --- properties ---------------------------------------------------------

    /// `rawget(properties, n)` is a table.
    pub fn has_props(&self, n: Ix) -> bool {
        matches!(self.nodes[n].props, Some(Some(_)))
    }

    /// A slot read through the metatable chain: `p[slot]`.
    pub fn slot(&self, n: Ix, s: Slot) -> Option<&Inj> {
        let mut at = n;
        loop {
            let Some(Some(p)) = &self.nodes[at].props else {
                return None;
            };
            match &p.slots[slot_index(s)] {
                Field::Value(v) => return Some(v),
                Field::False => return None,
                Field::Unset => {
                    let parent = p.parent?;
                    at = parent
                }
            }
        }
    }

    /// `p.cursivedy` through the chain.
    pub fn cursivedy(&self, n: Ix) -> Option<f64> {
        let mut at = n;
        loop {
            let Some(Some(p)) = &self.nodes[at].props else {
                return None;
            };
            match &p.cursivedy {
                Field::Value(v) => return Some(*v),
                Field::False => return None,
                Field::Unset => {
                    let parent = p.parent?;
                    at = parent
                }
            }
        }
    }

    /// `getprop(n, "state")` through the chain.
    pub fn state(&self, n: Ix) -> Option<u8> {
        let mut at = n;
        loop {
            let Some(Some(p)) = &self.nodes[at].props else {
                return None;
            };
            match &p.state {
                Field::Value(v) => return Some(*v),
                Field::False => return None,
                Field::Unset => {
                    let parent = p.parent?;
                    at = parent
                }
            }
        }
    }

    /// `setprop(n, "state", v)` (fontloader-basics-nod.lua:177-186): nothing
    /// for a nil `v`, else into the node's own table, made when it has none.
    pub fn set_state(&mut self, n: Ix, v: Option<u8>) {
        if let Some(v) = v {
            self.props_mut(n).state = Field::Value(v);
        }
    }

    /// The node's own table, created when absent (or `false`).
    pub fn props_mut(&mut self, n: Ix) -> &mut Props {
        if !matches!(self.nodes[n].props, Some(Some(_))) {
            self.nodes[n].props = Some(Some(Props::default()));
        }
        match &mut self.nodes[n].props {
            Some(Some(p)) => p,
            _ => unreachable!(),
        }
    }

    /// The slot's table to write into: an existing value (copied out of the
    /// chain into the node's own table first if inherited) or none.
    pub fn slot_own(&mut self, n: Ix, s: Slot) -> Option<&mut Inj> {
        let inherited = match &self.nodes[n].props {
            Some(Some(p)) => matches!(p.slots[slot_index(s)], Field::Unset),
            _ => false,
        };
        if inherited {
            // `local i = p[injection]; i.leftkern = ...` writes into the
            // ORIGINAL's table when i came through the metatable: the same
            // Lua table object. Writing goes to the owner of the value.
            let mut at = n;
            loop {
                let Some(Some(p)) = &self.nodes[at].props else {
                    return None;
                };
                match &p.slots[slot_index(s)] {
                    Field::Value(_) => break,
                    Field::False => return None,
                    Field::Unset => {
                        let parent = p.parent?;
                        at = parent
                    }
                }
            }
            return match &mut self.nodes[at].props {
                Some(Some(p)) => match &mut p.slots[slot_index(s)] {
                    Field::Value(v) => Some(v),
                    _ => None,
                },
                _ => None,
            };
        }
        match &mut self.nodes[n].props {
            Some(Some(p)) => match &mut p.slots[slot_index(s)] {
                Field::Value(v) => Some(v),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn set_slot(&mut self, n: Ix, s: Slot, v: Field<Inj>) {
        self.props_mut(n).slots[slot_index(s)] = v;
    }
}
