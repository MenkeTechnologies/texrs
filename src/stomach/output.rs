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
