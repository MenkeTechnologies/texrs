//! Drawing a box tree into a DVI file: `tex.web` §619-§640.
//!
//! This is the piece that had been missing between the two halves of the
//! stomach. `src/pack.rs` sets a box's glue, `src/postline.rs` assembles
//! breakpoints into lines and `src/page.rs` stacks them onto pages -- and all
//! three produced a `BoxNode` tree that nothing could draw, because the DVI
//! writer was fed runs of strings instead. So they were a library beside the
//! path a run takes rather than the path itself.
//!
//! `hlist_out` and `vlist_out` are what TeX itself calls to turn that tree into
//! a file, and they are the reason a set box can be shipped at all: glue that
//! `hpack` decided to STRETCH or SHRINK is written out here as a movement of
//! the width it was set to (§625), not as the width it was declared with. A
//! driver never has to be told to set a run to a measure -- the measure has
//! already been distributed over the glue by the time this runs, and each piece
//! of it is a `right` command.
//!
//! Two mutually recursive procedures, each holding one invariant, and the
//! invariants are the whole design:
//!
//! - `hlist_out` walks a horizontal list keeping `cur_v = base_line`, so
//!   everything in it sits on one baseline and only `cur_h` moves.
//! - `vlist_out` walks a vertical list keeping `cur_h = left_edge`, so
//!   everything in it starts at one left edge and only `cur_v` moves.
//!
//! `cur_h`/`cur_v` are where TeX thinks it is; `dvi_h`/`dvi_v` are where a DVI
//! reader thinks it is (§617). They differ whenever a movement has been
//! decided but not yet written, and `synch_h`/`synch_v` (§616) close the gap
//! only when something is about to be drawn. That is not a micro-optimisation:
//! it is what keeps a page of text from carrying a `right` command for every
//! word space that lands at the end of a line.

use crate::dvi::Writer;
use crate::glue::{Glue, Order};
use crate::node::{BoxNode, CharNode, GlueSign, LeaderKind, Node, RuleNode, Scaled};

/// A DVI file's dimensions are 32-bit (`tex.web` §110's `scaled`), and every
/// dimension TeX will hold fits (§421's `max_dimen` is $2^{30}-1$). A tree
/// built outside the packagers could still carry more, and a wrapped
/// dimension draws in the wrong place silently, so it is clamped instead.
fn dvi_scaled(x: Scaled) -> i32 {
    x.clamp(i32::MIN as Scaled, i32::MAX as Scaled) as i32
}

/// §625's `billion` and `vet_glue`: the glue arithmetic is done in floating
/// point, and a ratio that has gone wild is clamped rather than rounded into a
/// dimension that would wrap.
fn vet_glue(x: f64) -> f64 {
    x.clamp(-1_000_000_000.0, 1_000_000_000.0)
}

/// Where the shipper is, and where the file's reader is.
///
/// One of these lives for one page. `cur_s` is the depth of nesting, which is
/// exactly the depth of `push` commands in the output (§615).
pub struct Shipper<'a> {
    w: &'a mut Writer,
    /// §617: "a DVI reader program thinks we are here".
    dvi_h: Scaled,
    dvi_v: Scaled,
    /// §617: "TeX thinks we are here".
    cur_h: Scaled,
    cur_v: Scaled,
    /// The font last selected in the file. `None` is §617's `null_font`, which
    /// no character can be in, so the first character always selects.
    dvi_f: Option<usize>,
    /// §615: the current depth of output box nesting, initially -1.
    cur_s: i32,
    /// §628: whether a leader box is being replicated, which forbids nothing
    /// here but is what `\shipout` reports and what `page.rs` would read.
    doing_leaders: bool,
}

/// Ship one page: `tex.web` §640's `ship_out`.
///
/// `counts` are `\count0..9`, which is how a driver names a page. `h_offset`
/// and `v_offset` are `\hoffset` and `\voffset`; TeX's own origin is one inch
/// in from the top left of the paper and the driver applies that, so both are
/// zero for a page laid out the ordinary way.
///
/// The fonts the page uses must already be defined in `w`. §620 defines each
/// one the first time a character in it is drawn; the callers here know the
/// whole chain up front and define it before the first page, which produces
/// the same file with the definitions in a different place.
pub fn ship_out(
    w: &mut Writer,
    page: &BoxNode,
    counts: [i32; 10],
    h_offset: Scaled,
    v_offset: Scaled,
) {
    w.begin_page(counts);
    // §617: `dvi_h:=0; dvi_v:=0; cur_h:=h_offset; dvi_f:=null_font`.
    let mut ship = Shipper {
        w,
        dvi_h: 0,
        dvi_v: 0,
        cur_h: h_offset,
        cur_v: 0,
        dvi_f: None,
        cur_s: -1,
        doing_leaders: false,
    };
    // §640: `cur_v:=height(p)+v_offset` -- a page's reference point is its
    // BASELINE, and the box hangs below the top of the paper by its height.
    ship.cur_v = page.height + v_offset;
    match page.vertical {
        true => ship.vlist_out(page),
        false => ship.hlist_out(page),
    }
    ship.w.end_page();
}

impl Shipper<'_> {
    /// §616's `synch_h`: write the horizontal movement that has been decided
    /// but not yet put in the file.
    fn synch_h(&mut self) {
        if self.cur_h != self.dvi_h {
            self.w.right(dvi_scaled(self.cur_h - self.dvi_h));
            self.dvi_h = self.cur_h;
        }
    }

    /// §616's `synch_v`.
    fn synch_v(&mut self) {
        if self.cur_v != self.dvi_v {
            self.w.down(dvi_scaled(self.cur_v - self.dvi_v));
            self.dvi_v = self.cur_v;
        }
    }

    /// §620's `@<Change font dvi_f to f@>`.
    fn synch_font(&mut self, f: usize) {
        if self.dvi_f != Some(f) {
            self.w.font(f as u32);
            self.dvi_f = Some(f);
        }
    }

    /// §621's `@<Change font…@>` for one character node, followed by the
    /// character itself. `character` carries the FONT SLOT the chain resolved,
    /// which is what a DVI file names, and `width` is the width the `.tfm`
    /// gave it (§654 looks it up; the node carries it here).
    fn out_char(&mut self, c: &CharNode) {
        self.synch_font(c.font);
        self.w.set_char(c.character as u32, dvi_scaled(c.width));
        // §620: `cur_h:=cur_h+char_width(f)(char_info(f)(c))`. The writer has
        // advanced its own idea of h by the same amount, which is why the
        // fast path can set `dvi_h:=cur_h` at the end rather than synching.
        self.cur_h += c.width;
    }

    /// `hlist_out` (§619-§628): output an `hlist_node` box whose reference
    /// point is at `(cur_h, cur_v)`.
    pub fn hlist_out(&mut self, this_box: &BoxNode) {
        let g_order = this_box.glue_order;
        let g_sign = this_box.glue_sign;
        // §625: the glue seen so far, and its rounded equivalent. TeX
        // accumulates the UNSET total and rounds the running product, so that
        // rounding error cannot pile up across a long line.
        let mut cur_glue = 0.0f64;
        let mut cur_g: Scaled = 0;

        self.cur_s += 1;
        if self.cur_s > 0 {
            self.w.push();
        }
        let base_line = self.cur_v;
        let left_edge = self.cur_h;

        let mut i = 0;
        while i < this_box.list.len() {
            let p = &this_box.list[i];
            // §620: a run of character nodes is drawn in one go, which is what
            // makes this TeX's inner loop rather than a synch per letter.
            if let Node::Char(c) | Node::Ligature(c) = p {
                self.synch_h();
                self.synch_v();
                let mut c = c;
                loop {
                    self.out_char(c);
                    i += 1;
                    match this_box.list.get(i) {
                        Some(Node::Char(next) | Node::Ligature(next)) => c = next,
                        _ => break,
                    }
                }
                self.dvi_h = self.cur_h;
                continue;
            }
            // §622: the rule's three dimensions, or the glue's set width, both
            // end at `move_past` -- `cur_h:=cur_h+rule_wd`.
            let rule_wd = match p {
                // §623: output a box in an hlist.
                Node::Box(b) => {
                    if b.list.is_empty() {
                        self.cur_h += b.width;
                    } else {
                        let (save_h, save_v) = (self.dvi_h, self.dvi_v);
                        self.cur_v = base_line + b.shift_amount;
                        let edge = self.cur_h;
                        match b.vertical {
                            true => self.vlist_out(b),
                            false => self.hlist_out(b),
                        }
                        self.dvi_h = save_h;
                        self.dvi_v = save_v;
                        self.cur_h = edge + b.width;
                        self.cur_v = base_line;
                    }
                    i += 1;
                    continue;
                }
                Node::Rule(r) => self.hrule_out(r, this_box, base_line),
                // §1368's `special_out`: a `\special` is placed where it
                // stands, so both coordinates are synchronised first.
                Node::Whatsit(text) => {
                    self.synch_h();
                    self.synch_v();
                    self.w.special(text);
                    i += 1;
                    continue;
                }
                // §625: move right, or output leaders.
                Node::Glue(g) => {
                    let mut rule_wd = g.spec.natural - cur_g;
                    set_glue(
                        &g.spec,
                        g_sign,
                        g_order,
                        this_box.glue_set,
                        &mut cur_glue,
                        &mut cur_g,
                    );
                    rule_wd += cur_g;
                    match (g.kind.is_leaders(), g.leader.as_deref()) {
                        // §626: leaders drawn from a rule become that rule,
                        // stretched over the glue's set width.
                        (true, Some(Node::Rule(r))) => {
                            let over = RuleNode {
                                width: rule_wd,
                                height: r.height,
                                depth: r.depth,
                            };
                            self.hrule_out(&over, this_box, base_line)
                        }
                        (true, Some(Node::Box(b))) => {
                            self.hleaders_out(g.kind, b, rule_wd, left_edge, base_line)
                        }
                        _ => rule_wd,
                    }
                }
                // §621: a kern and the space around a formula are both a
                // movement and nothing else.
                Node::Kern { width, .. } | Node::Math(width) => *width,
                // §621's `othercases do_nothing`: a penalty, a mark, an
                // insertion and a discretionary that was not taken carry no
                // ink and no width.
                _ => 0,
            };
            // §622's `move_past`.
            self.cur_h += rule_wd;
            i += 1;
        }

        if self.cur_s > 0 {
            self.w.pop();
        }
        self.cur_s -= 1;
    }

    /// §624: output a rule in an hlist, and return what `cur_h` advances by.
    ///
    /// A running dimension takes the enclosing box's, which is what `\hrulefill`
    /// and a table's vertical rules are made of.
    fn hrule_out(&mut self, r: &RuleNode, this_box: &BoxNode, base_line: Scaled) -> Scaled {
        let rule_wd = r.width;
        let mut rule_ht = r.height;
        let mut rule_dp = r.depth;
        if RuleNode::is_running(rule_ht) {
            rule_ht = this_box.height;
        }
        if RuleNode::is_running(rule_dp) {
            rule_dp = this_box.depth;
        }
        // "this is the rule thickness"
        rule_ht += rule_dp;
        if rule_ht > 0 && rule_wd > 0 {
            self.synch_h();
            self.cur_v = base_line + rule_dp;
            self.synch_v();
            self.w.rule(dvi_scaled(rule_ht), dvi_scaled(rule_wd), true);
            self.cur_v = base_line;
            // `set_rule` moves the reader right as well as drawing.
            self.dvi_h += rule_wd;
        }
        rule_wd
    }

    /// §626-§628: output leaders in an hlist, and return what `cur_h` advances
    /// by.
    ///
    /// Leaders are a box replicated across a glue's set width. The three kinds
    /// differ only in where the first copy goes: `\leaders` aligns them to a
    /// grid fixed by the enclosing box, so two lines of leaders line up
    /// vertically; `\cleaders` centres the copies in the space; `\xleaders`
    /// spreads the leftover between them.
    fn hleaders_out(
        &mut self,
        kind: LeaderKind,
        leader_box: &BoxNode,
        rule_wd: Scaled,
        left_edge: Scaled,
        base_line: Scaled,
    ) -> Scaled {
        let leader_wd = leader_box.width;
        if leader_wd <= 0 || rule_wd <= 0 {
            return rule_wd;
        }
        // §626: "compensate for floating-point rounding".
        let rule_wd = rule_wd + 10;
        let edge = self.cur_h + rule_wd;
        let mut lx: Scaled = 0;
        // §627: let cur_h be the position of the first box.
        match kind {
            LeaderKind::Aligned => {
                let save_h = self.cur_h;
                self.cur_h = left_edge + leader_wd * ((self.cur_h - left_edge) / leader_wd);
                if self.cur_h < save_h {
                    self.cur_h += leader_wd;
                }
            }
            _ => {
                let lq = rule_wd / leader_wd;
                let lr = rule_wd % leader_wd;
                match kind {
                    LeaderKind::Centred => self.cur_h += lr / 2,
                    _ => {
                        lx = lr / (lq + 1);
                        self.cur_h += (lr - (lq - 1) * lx) / 2;
                    }
                }
            }
        }
        // §628: output a leader box at cur_h, then advance.
        while self.cur_h + leader_wd <= edge {
            self.cur_v = base_line + leader_box.shift_amount;
            self.synch_v();
            let save_v = self.dvi_v;
            self.synch_h();
            let save_h = self.dvi_h;
            let outer = self.doing_leaders;
            self.doing_leaders = true;
            match leader_box.vertical {
                true => self.vlist_out(leader_box),
                false => self.hlist_out(leader_box),
            }
            self.doing_leaders = outer;
            self.dvi_v = save_v;
            self.dvi_h = save_h;
            self.cur_v = base_line;
            self.cur_h = save_h + leader_wd + lx;
        }
        // §626: `cur_h:=edge-10; goto next_p` -- the caller's `move_past` is
        // skipped, so this reports the advance that puts cur_h exactly there.
        edge - 10 - self.cur_h
    }

    /// `vlist_out` (§629-§637): output a `vlist_node` box whose reference point
    /// is at `(cur_h, cur_v)`.
    pub fn vlist_out(&mut self, this_box: &BoxNode) {
        let g_order = this_box.glue_order;
        let g_sign = this_box.glue_sign;
        let mut cur_glue = 0.0f64;
        let mut cur_g: Scaled = 0;

        self.cur_s += 1;
        if self.cur_s > 0 {
            self.w.push();
        }
        let left_edge = self.cur_h;
        // §629: a vlist's reference point is its BASELINE, and its contents
        // start `height` above it.
        self.cur_v -= this_box.height;
        let top_edge = self.cur_v;

        for p in &this_box.list {
            // §630: `if is_char_node(p) then confusion("vlistout")` -- a
            // character in a vertical list is a bug in whatever built it, not
            // something to draw, so it is skipped rather than misplaced.
            let rule_ht = match p {
                // §632: output a box in a vlist.
                Node::Box(b) => {
                    if b.list.is_empty() {
                        self.cur_v += b.height + b.depth;
                    } else {
                        self.cur_v += b.height;
                        self.synch_v();
                        let (save_h, save_v) = (self.dvi_h, self.dvi_v);
                        self.cur_h = left_edge + b.shift_amount;
                        match b.vertical {
                            true => self.vlist_out(b),
                            false => self.hlist_out(b),
                        }
                        self.dvi_h = save_h;
                        self.dvi_v = save_v;
                        self.cur_v = save_v + b.depth;
                        self.cur_h = left_edge;
                    }
                    continue;
                }
                // §633: a rule in a vlist is a `put_rule` -- it does not move
                // the reader, because the list's own arithmetic already has.
                Node::Rule(r) => {
                    self.vrule_out(r, this_box);
                    continue;
                }
                Node::Whatsit(text) => {
                    self.synch_h();
                    self.synch_v();
                    self.w.special(text);
                    continue;
                }
                // §634: move down or output leaders.
                Node::Glue(g) => {
                    let mut rule_ht = g.spec.natural - cur_g;
                    set_glue(
                        &g.spec,
                        g_sign,
                        g_order,
                        this_box.glue_set,
                        &mut cur_glue,
                        &mut cur_g,
                    );
                    rule_ht += cur_g;
                    match (g.kind.is_leaders(), g.leader.as_deref()) {
                        (true, Some(Node::Rule(r))) => {
                            let over = RuleNode {
                                width: r.width,
                                height: rule_ht,
                                depth: 0,
                            };
                            self.vrule_out(&over, this_box);
                            continue;
                        }
                        (true, Some(Node::Box(b))) => {
                            self.vleaders_out(g.kind, b, rule_ht, left_edge, top_edge)
                        }
                        _ => rule_ht,
                    }
                }
                Node::Kern { width, .. } => *width,
                _ => 0,
            };
            // §631's `move_past`.
            self.cur_v += rule_ht;
        }

        if self.cur_s > 0 {
            self.w.pop();
        }
        self.cur_s -= 1;
    }

    /// §633: output a rule in a vlist. Unlike §624 this advances `cur_v` itself
    /// and does not fall through to `move_past`.
    fn vrule_out(&mut self, r: &RuleNode, this_box: &BoxNode) {
        let mut rule_wd = r.width;
        if RuleNode::is_running(rule_wd) {
            rule_wd = this_box.width;
        }
        let rule_ht = r.height + r.depth;
        self.cur_v += rule_ht;
        if rule_ht > 0 && rule_wd > 0 {
            self.synch_h();
            self.synch_v();
            self.w.rule(dvi_scaled(rule_ht), dvi_scaled(rule_wd), false);
        }
    }

    /// §635-§637: output leaders in a vlist, and return what `cur_v` advances
    /// by.
    fn vleaders_out(
        &mut self,
        kind: LeaderKind,
        leader_box: &BoxNode,
        rule_ht: Scaled,
        left_edge: Scaled,
        top_edge: Scaled,
    ) -> Scaled {
        let leader_ht = leader_box.height + leader_box.depth;
        if leader_ht <= 0 || rule_ht <= 0 {
            return rule_ht;
        }
        let rule_ht = rule_ht + 10;
        let edge = self.cur_v + rule_ht;
        let mut lx: Scaled = 0;
        // §636: let cur_v be the position of the first box.
        match kind {
            LeaderKind::Aligned => {
                let save_v = self.cur_v;
                self.cur_v = top_edge + leader_ht * ((self.cur_v - top_edge) / leader_ht);
                if self.cur_v < save_v {
                    self.cur_v += leader_ht;
                }
            }
            _ => {
                let lq = rule_ht / leader_ht;
                let lr = rule_ht % leader_ht;
                match kind {
                    LeaderKind::Centred => self.cur_v += lr / 2,
                    _ => {
                        lx = lr / (lq + 1);
                        self.cur_v += (lr - (lq - 1) * lx) / 2;
                    }
                }
            }
        }
        // §637: "cur_v indicates the top of a leader box, not its baseline".
        while self.cur_v + leader_ht <= edge {
            self.cur_h = left_edge + leader_box.shift_amount;
            self.synch_h();
            let save_h = self.dvi_h;
            self.cur_v += leader_box.height;
            self.synch_v();
            let save_v = self.dvi_v;
            let outer = self.doing_leaders;
            self.doing_leaders = true;
            match leader_box.vertical {
                true => self.vlist_out(leader_box),
                false => self.hlist_out(leader_box),
            }
            self.doing_leaders = outer;
            self.dvi_v = save_v;
            self.dvi_h = save_h;
            self.cur_h = left_edge;
            self.cur_v = save_v - leader_box.height + leader_ht + lx;
        }
        edge - 10 - self.cur_v
    }
}

/// §625 and §634's glue arithmetic, which is the same in both directions.
///
/// The set width of a glue node is not a property of the node: it depends on
/// the RATIO the enclosing box was packed at and on how much glue of the box's
/// own order has been passed already. Only glue of that order moves -- a
/// `1fil` in a box being stretched at order 0 contributes nothing -- which is
/// the whole of `\hss`'s behaviour and of `\hfil` beating a plain `plus 1pt`.
fn set_glue(
    spec: &Glue,
    g_sign: GlueSign,
    g_order: Order,
    glue_set: f64,
    cur_glue: &mut f64,
    cur_g: &mut Scaled,
) {
    match g_sign {
        GlueSign::Normal => {}
        GlueSign::Stretching if spec.stretch_order == g_order => {
            *cur_glue += spec.stretch as f64;
            *cur_g = vet_glue(glue_set * *cur_glue).round() as Scaled;
        }
        GlueSign::Shrinking if spec.shrink_order == g_order => {
            *cur_glue -= spec.shrink as f64;
            *cur_g = vet_glue(glue_set * *cur_glue).round() as Scaled;
        }
        _ => {}
    }
}
