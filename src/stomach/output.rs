//! What leaves the stomach: `ship_out` (§638-§641), and the state the page
//! builder and the output routine share with the rest of the run (§980-§1028).
//!
//! A shipped page is kept as the box it is, with the `\count0..9` it was
//! shipped under and the `\hoffset`/`\voffset` in force, and drawn into the
//! output file when the run is over ([`Output::pages`]). Drawing it at once
//! would buy nothing -- §640's `hlist_out` reads nothing but the box and the
//! fonts -- and keeping it lets `--dvi` and `--pdf` draw the same pages.

use super::{Said, SlotSink, Stomach};
use crate::glue::Glue;
use crate::node::{BoxNode, Scaled, MAX_DIMEN};

/// One page as §640 ships it.
#[derive(Clone, Debug)]
pub struct Shipped {
    pub page: BoxNode,
    pub counts: [i32; 10],
    pub h_offset: Scaled,
    pub v_offset: Scaled,
}

/// The output side of the run.
#[derive(Clone, Debug, Default)]
pub struct Output {
    /// Every page shipped, in order.
    pub pages: Vec<Shipped>,
    /// `max_v` and `max_h` (§641): the postamble's page extent.
    pub max_v: Scaled,
    pub max_h: Scaled,
    /// `dead_cycles` (§592): outputs in a row that shipped nothing.
    pub dead_cycles: i64,
    /// `insert_penalties` (§982).
    pub insert_penalties: i64,
    /// `\outputpenalty`, as `fire_up` last set it (§1013).
    pub output_penalty: i64,
    /// `\splitfirstmark` and `\splitbotmark` (§382, §979).
    pub split_first_mark: Option<String>,
    pub split_bot_mark: Option<String>,
    /// The numbers of the five marks (§382), for the mirror.
    pub mark_ids: [i64; 5],
    /// `last_penalty`, `last_kern` and `last_glue` (§982): what the page
    /// builder last took off the contribution list, which `\lastpenalty`
    /// and its siblings read when the outer list is empty (§424).
    pub last_penalty: i64,
    pub last_kern: Scaled,
    pub last_glue: Option<Glue>,
    /// `page_so_far` (§982), for `\pagegoal` and its siblings.
    pub page_goal: Scaled,
    pub page_total: Scaled,
    pub page_stretch: [Scaled; 4],
    pub page_shrink: Scaled,
    pub page_depth: Scaled,
}

impl Output {
    /// Write the page builder's values a document can read into their slots.
    pub fn mirror(&self, out: &mut dyn SlotSink) {
        use super::Internal::*;
        out.set(PageGoal.slot(), self.page_goal);
        out.set(PageTotal.slot(), self.page_total);
        for (o, v) in self.page_stretch.iter().enumerate() {
            out.set(PageStretch(o as u8).slot(), *v);
        }
        out.set(PageShrink.slot(), self.page_shrink);
        out.set(PageDepth.slot(), self.page_depth);
        out.set(DeadCycles.slot(), self.dead_cycles);
        out.set(InsertPenalties.slot(), self.insert_penalties);
        out.set(OutputPenalty.slot(), self.output_penalty);
        for (k, id) in self.mark_ids.iter().enumerate() {
            out.set(Mark(k as u8).slot(), *id);
        }
    }

    /// §424 for the outer vertical list once the page builder has emptied it.
    pub fn last_item(&self) -> (i64, Scaled, Glue) {
        (
            self.last_penalty,
            self.last_kern,
            self.last_glue.unwrap_or_default(),
        )
    }

    /// `last_glue=max_halfword` (§1106): no glue was the last thing the page
    /// builder took.
    pub fn last_glue_is_none(&self) -> bool {
        self.last_glue.is_none()
    }
}

impl Stomach {
    /// `ship_out(p)` (§638-§641), with `h_offset`/`v_offset` as they stand.
    pub fn ship_out_box(&mut self, p: BoxNode) {
        // §638: `[` and the counts that are not trailing zeros, joined by dots.
        let mut j = 9;
        while self.counts[j] == 0 && j > 0 {
            j -= 1;
        }
        let shown: Vec<String> = self.counts[..=j].iter().map(|c| c.to_string()).collect();
        let note = format!("[{}]", shown.join("."));
        let tracing = self.int_par("tracingoutput") > 0;
        if tracing {
            let mut text = String::from("\nCompleted box being shipped out");
            text.push_str(&note);
            text.push_str(&self.show_box_text(std::slice::from_ref(&crate::node::Node::Box(
                p.clone(),
            ))));
            self.diagnostic(&text);
        }
        let (h_offset, v_offset) = (self.h_offset, self.v_offset);
        // §641: a page more than `max_dimen` in any direction is not shipped.
        if p.height > MAX_DIMEN
            || p.depth > MAX_DIMEN
            || p.height + p.depth + v_offset > MAX_DIMEN
            || p.width + h_offset > MAX_DIMEN
        {
            self.say(Said::Error("Huge page cannot be shipped out".into()));
        } else {
            let out = &mut self.output;
            out.max_v = out.max_v.max(p.height + p.depth + v_offset);
            out.max_h = out.max_h.max(p.width + h_offset);
            let mut counts = [0i32; 10];
            for (c, v) in counts.iter_mut().zip(self.counts.iter()) {
                *c = *v as i32;
            }
            out.pages.push(Shipped {
                page: p,
                counts,
                h_offset,
                v_offset,
            });
        }
        if !tracing {
            self.say(Said::Terminal(note));
        }
        self.output.dead_cycles = 0;
    }

    /// `show_box` (§198) of `list` with `\showboxdepth` and
    /// `\showboxbreadth` as they stand, naming fonts by their identifiers.
    pub fn show_box_text(&self, list: &[crate::node::Node]) -> String {
        let esc = self.esc_text();
        let d = crate::node::show::Display {
            depth: self.int_par("showboxdepth"),
            breadth: self.int_par("showboxbreadth"),
            esc: &esc,
            fonts: self,
        };
        d.show_box(list)
    }
}

impl crate::node::show::FontIds for Stomach {
    fn font_id_text(&self, f: usize) -> String {
        self.font(f).id_text.clone()
    }
}

/// The page builder's half of the run (§980-§1028), over `crate::page`.
#[derive(Default)]
pub struct Pages {
    builder: Option<crate::page::PageBuilder>,
    /// §1025 has fired and the output routine has not run yet.
    pub output_pending: bool,
    /// `\end` is waiting for §1054's `its_all_over`.
    pub end_requested: bool,
    /// Whether `\output` is non-empty: the program says so before every
    /// action that can build the page.
    pub has_output_routine: bool,
}

impl Stomach {
    fn builder(&mut self) -> &mut crate::page::PageBuilder {
        let b = self
            .pages
            .builder
            .get_or_insert_with(|| crate::page::PageBuilder::new(crate::page::PageParams::default()));
        b
    }

    /// Read §980's frozen-at-first-box parameters as they stand now; the
    /// builder freezes them itself when the page gets its first box.
    fn refresh_page_params(&mut self, e: &dyn super::Eqtb) {
        let tol = self.tolerances(e);
        let has = self.pages.has_output_routine;
        let holding = self.int_par("holdinginserts");
        let dead = self.int_par("maxdeadcycles");
        let p = crate::page::PageParams {
            vsize: super::dimen_par(e, "vsize"),
            max_depth: super::dimen_par(e, "maxdepth"),
            top_skip: super::glue_par(e, "topskip"),
            split_top_skip: super::glue_par(e, "splittopskip"),
            split_max_depth: super::dimen_par(e, "splitmaxdepth"),
            holding_inserts: holding,
            max_dead_cycles: dead,
            tolerances: tol,
            inserts: self.insert_classes(e),
        };
        let b = self.builder();
        b.params = p;
        b.has_output_routine = has;
    }

    /// §981: the registers of every insertion class the contributions or the
    /// page hold, read from `\count n`, `\dimen n` and `\skip n`.
    fn insert_classes(
        &self,
        e: &dyn super::Eqtb,
    ) -> std::collections::BTreeMap<u8, crate::page::InsertClass> {
        let mut classes = std::collections::BTreeMap::new();
        for node in self.nest[0].list.iter() {
            if let crate::node::Node::Ins(ins) = node {
                let n = ins.number;
                classes.entry(n).or_insert_with(|| crate::page::InsertClass {
                    count: e.slot(i64::from(n)),
                    dimen: e.slot(crate::compiler::DIMEN_BASE + i64::from(n)),
                    skip: super::glue_at(
                        e,
                        crate::compiler::SKIP_BASE + i64::from(n) * crate::compiler::SKIP_STRIDE,
                    ),
                });
            }
        }
        if let Some(b) = &self.pages.builder {
            for (n, c) in &b.params.inserts {
                classes.entry(*n).or_insert(*c);
            }
        }
        classes
    }

    /// §994's `build_page`: move the outer list onto the page, and ship or
    /// hand to `\output` what the builder fires.
    pub fn build_page(&mut self, e: &dyn super::Eqtb) {
        if self.output_active() {
            return;
        }
        crate::node::hooks::buildpage_filter(self.build_page_reason);
        self.refresh_page_params(e);
        let contributions = std::mem::take(&mut self.nest[0].list);
        // The insertion boxes are the document's registers; the builder
        // reads and writes them through its own table while it runs.
        self.lend_insertion_boxes();
        let fired = self.builder().contribute(contributions);
        self.handle_fired(fired);
    }

    fn lend_insertion_boxes(&mut self) {
        let classes: Vec<u8> = self
            .pages
            .builder
            .as_ref()
            .map(|b| b.params.inserts.keys().copied().collect())
            .unwrap_or_default();
        for n in classes {
            let value = self.box_register(u16::from(n)).cloned();
            let b = self.builder();
            match value {
                Some(v) => {
                    b.boxes.insert(n, v);
                }
                None => {
                    b.boxes.remove(&n);
                }
            }
        }
    }

    /// The builder's boxes back into the registers: `\box255` and every
    /// insertion class (§1017-§1022 assign them directly, as `box(n):=`).
    fn take_back_boxes(&mut self) {
        let Some(b) = self.pages.builder.as_mut() else {
            return;
        };
        let boxes = std::mem::take(&mut b.boxes);
        let classes: Vec<u8> = b.params.inserts.keys().copied().collect();
        for n in classes {
            if !boxes.contains_key(&n) {
                self.set_box(u16::from(n), None, true);
            }
        }
        for (n, v) in boxes {
            self.set_box(u16::from(n), Some(v), true);
        }
    }

    fn handle_fired(&mut self, fired: Vec<crate::page::Fired>) {
        self.take_back_boxes();
        self.mirror_page();
        for f in fired {
            match f {
                crate::page::Fired::ShipOut(page) => {
                    self.set_box(255, None, true);
                    self.ship_out_box(page);
                }
                crate::page::Fired::Output {
                    output_penalty,
                    dead_cycles,
                    ..
                } => {
                    // §1025: the routine runs with `\box255` full, on a list
                    // of its own.
                    self.output.output_penalty = output_penalty;
                    self.output.dead_cycles = dead_cycles;
                    let mut l = super::ListState::new(
                        super::Mode::InternalVertical,
                        super::Opened::Output,
                    );
                    l.mode_line = -self.line;
                    self.nest.push(l);
                    self.group_begin();
                    self.pages.output_pending = true;
                    return;
                }
            }
        }
    }

    /// §1026: the output routine has ended; what it left on its list goes
    /// back in front of the contributions, and the builder looks again.
    pub fn end_output(&mut self, e: &dyn super::Eqtb) {
        if self.mode() == super::Mode::Horizontal {
            self.end_graf(e);
        }
        self.group_end();
        if !matches!(self.cur().opened, super::Opened::Output) {
            return;
        }
        // §1028.
        if self.box_register(255).is_some() {
            self.say(Said::Error("Output routine didn't use all of \\box255".into()));
            self.set_box(255, None, true);
        }
        let Some(l) = self.pop_list() else {
            return;
        };
        self.pages.output_pending = false;
        self.output.insert_penalties = 0;
        self.refresh_page_params(e);
        self.lend_insertion_boxes();
        crate::node::hooks::buildpage_filter("after_output");
        let fired = self.builder().resume_output(l.list);
        self.handle_fired(fired);
    }

    /// §1054's `its_all_over`, after `\end`: true when there is nothing left
    /// to ship; otherwise the residue is pushed out with `\hbox to\hsize{}
    /// \vfill\penalty-'10000000000` and the page built again.
    pub fn its_all_over(&mut self, e: &dyn super::Eqtb) -> bool {
        let page_empty = self
            .pages
            .builder
            .as_ref()
            .is_none_or(|b| b.current_page().is_empty() && !b.contributions_pending());
        if page_empty && self.nest[0].list.is_empty() && self.output.dead_cycles == 0 {
            return true;
        }
        let hsize = super::dimen_par(e, "hsize");
        self.tail_append(crate::node::Node::Box(BoxNode {
            width: hsize,
            ..BoxNode::null()
        }));
        self.tail_append(crate::node::Node::Glue(crate::node::GlueNode::new(
            crate::box_::fill::fill(),
        )));
        self.tail_append(crate::node::Node::Penalty(-0o10000000000));
        self.build_page_reason = "end";
        self.build_page(e);
        false
    }

    /// The page builder's readable values into [`Output`], for the mirror.
    pub(super) fn mirror_page(&mut self) {
        let id = |m: &Option<String>| {
            m.as_ref()
                .and_then(|t| self.mark_ids.get(t))
                .map_or(0, |&n| i64::from(n))
        };
        let split = [id(&self.output.split_first_mark), id(&self.output.split_bot_mark)];
        let page = match self.pages.builder.as_ref() {
            Some(b) => [id(&b.top_mark), id(&b.first_mark), id(&b.bot_mark)],
            None => [0; 3],
        };
        self.output.mark_ids = [page[0], page[1], page[2], split[0], split[1]];
        let Some(b) = self.pages.builder.as_ref() else {
            return;
        };
        use crate::page::PageDimen;
        let out = &mut self.output;
        out.page_goal = b.page_so_far(PageDimen::Goal);
        out.page_total = b.page_so_far(PageDimen::Total);
        for o in 0..4 {
            out.page_stretch[o] = b.page_so_far(PageDimen::Stretch(o));
        }
        out.page_shrink = b.page_so_far(PageDimen::Shrink);
        out.page_depth = b.page_so_far(PageDimen::Depth);
        out.insert_penalties = b.insert_penalties();
        out.last_penalty = b.last_penalty;
        out.last_kern = b.last_kern;
        out.last_glue = b.last_glue;
    }
}

impl Stomach {
    /// The DVI file of the pages shipped (§617-§643), as tex writes it; `None`
    /// when no page was shipped, for which tex writes no file (§642).
    pub fn dvi(&self) -> Option<Vec<u8>> {
        if self.output.pages.is_empty() {
            return None;
        }
        // §617: the comment is stamped from `\year`, `\month`, `\day` and
        // `\time` as the run left them.
        let time = self.int_par("time");
        let comment = format!(
            " TeX output {}.{:02}.{:02}:{:02}{:02}",
            self.int_par("year"),
            self.int_par("month"),
            self.int_par("day"),
            time / 60,
            time % 60
        );
        let fonts = self
            .fonts
            .iter()
            .enumerate()
            .map(|(f, font)| match f {
                0 => None,
                _ => Some(crate::dvi::out::FontDef {
                    checksum: font.checksum,
                    size: font.size as i32,
                    design_size: font.design_size as i32,
                    area: String::new(),
                    name: font.name.clone(),
                }),
            })
            .collect();
        let mag = self.int_par("mag") as i32;
        let mut out = crate::dvi::out::Out::new(&comment, mag, fonts);
        for p in &self.output.pages {
            crate::shipout::ship_out(&mut out, &p.page, p.counts, p.h_offset, p.v_offset);
        }
        out.max_v = self.output.max_v as i32;
        out.max_h = self.output.max_h as i32;
        Some(out.finish())
    }
}
