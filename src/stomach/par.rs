//! Paragraphs: §1091-§1096's entry and exit, around §813-§890's breaker.
//!
//! `new_graf` opens a horizontal list on top of the vertical one, and
//! `end_graf` breaks it into lines (`crate::parbreak`), packages them
//! (`crate::postline`) and appends them to the enclosing vertical list.
//! Everything a paragraph reads -- `\hsize`, `\leftskip`, `\parfillskip`,
//! `\baselineskip`, the penalties -- is read as it stands when the paragraph
//! ENDS, which is when tex reads it.

use super::{dimen_par, glue_par, Eqtb, ListState, Mode, Opened, Stomach};
use crate::node::{BoxNode, GlueNode, GlueSource, Node, INF_PENALTY};

impl Stomach {
    /// `new_graf(indented)` (§1091). `\everypar` is the lowerer's: it is a
    /// token list, inserted where the mouth stands.
    pub fn new_graf(&mut self, indented: bool, e: &dyn Eqtb) {
        let cur = self.cur_mut();
        cur.prev_graf = 0;
        let outer = cur.mode == Mode::Vertical;
        if outer || !cur.list.is_empty() {
            let par_skip = glue_par(e, "parskip");
            self.tail_append(Node::Glue(GlueNode::param(par_skip, GlueSource::ParSkip)));
        }
        let mut l = ListState::new(Mode::Horizontal, Opened::Page);
        l.space_factor = 1000;
        l.mode_line = self.line;
        // §1091 stores `\lefthyphenmin`, `\righthyphenmin` and the language
        // in `prev_graf` for §816 to read back; the hyphenation pass is not on
        // this path yet, so only the encoding's shape is kept.
        l.prev_graf = 0;
        if indented {
            l.list.push(Node::Box(BoxNode {
                width: dimen_par(e, "parindent"),
                ..BoxNode::null()
            }));
        }
        self.nest.push(l);
        if self.nest.len() == 2 {
            self.want_build_page();
        }
    }

    /// `indent_in_hmode` (§1093): `\indent` in a horizontal list.
    pub fn indent_in_hmode(&mut self, e: &dyn Eqtb) {
        let b = BoxNode {
            width: dimen_par(e, "parindent"),
            ..BoxNode::null()
        };
        self.cur_mut().space_factor = 1000;
        self.tail_append(Node::Box(b));
    }

    /// `end_graf` (§1096), with §1094's `build_page` after it: break the
    /// paragraph, or drop it if it is empty, and return to the vertical list.
    pub fn end_graf(&mut self, e: &dyn Eqtb) {
        if self.mode() != Mode::Horizontal {
            return;
        }
        if self.cur().list.is_empty() {
            self.pop_list();
        } else {
            let widow = self.int_par("widowpenalty");
            self.line_break(widow, e);
        }
        if self.mode() == Mode::Vertical {
            self.want_build_page();
        }
    }

    /// `line_break(final_widow_penalty)` (§815): §816's ending of the list,
    /// §863's search, and §877's `post_line_break`.
    pub fn line_break(&mut self, final_widow_penalty: i64, e: &dyn Eqtb) {
        let Some(par) = self.pop_list() else {
            return;
        };
        self.pack_begin_line = par.mode_line;
        let mut list = par.list;
        // §816: the last glue becomes `\penalty10000`, or one is added, and
        // `\parfillskip` ends the paragraph.
        match list.last() {
            Some(Node::Glue(_)) => {
                list.pop();
                list.push(Node::Penalty(INF_PENALTY));
            }
            _ => list.push(Node::Penalty(INF_PENALTY)),
        }
        list.push(Node::Glue(GlueNode::param(
            glue_par(e, "parfillskip"),
            GlueSource::ParFillSkip,
        )));
        crate::node::hooks::pre_linebreak_filter(&mut list, "");

        let prev_graf = self.cur().prev_graf;
        let hsize = dimen_par(e, "hsize");
        let hang_indent = dimen_par(e, "hangindent");
        let hang_after = self.int_par("hangafter");
        // §848: `\parshape` is not a register texrs keeps yet, so the shape is
        // `\hangindent`'s or `\hsize`'s.
        let shape = match hang_indent == 0 {
            true => crate::postline::LineShape::fixed(hsize),
            false => crate::postline::LineShape::hanging(hsize, hang_indent, hang_after),
        };
        let left_skip = glue_par(e, "leftskip");
        let right_skip = glue_par(e, "rightskip");
        let params = crate::parbreak::BreakParams {
            pretolerance: self.int_par("pretolerance"),
            tolerance: self.int_par("tolerance"),
            line_penalty: self.int_par("linepenalty"),
            hyphen_penalty: self.int_par("hyphenpenalty"),
            ex_hyphen_penalty: self.int_par("exhyphenpenalty"),
            adj_demerits: self.int_par("adjdemerits"),
            double_hyphen_demerits: self.int_par("doublehyphendemerits"),
            final_hyphen_demerits: self.int_par("finalhyphendemerits"),
            looseness: self.int_par("looseness"),
            emergency_stretch: dimen_par(e, "emergencystretch"),
            left_skip,
            right_skip,
            shape: shape.clone(),
            prev_graf,
        };
        let breaks = crate::parbreak::line_break(
            &mut list,
            &params,
            Some(&mut |l: &mut Vec<Node>| self.hyphenate_all(l)),
        );
        if breaks.infinite_shrinkage {
            self.say(super::Said::Error(
                "Infinite glue shrinkage found in a paragraph".into(),
            ));
        }

        // §877-§890.
        let positions: Vec<usize> = breaks
            .breaks
            .iter()
            .map(|b| b.unwrap_or(usize::MAX))
            .collect();
        let tol = self.tolerances(e);
        let par_params = crate::postline::ParParams {
            shape,
            prev_graf: prev_graf.max(0) as usize,
            skips: crate::postline::Skips {
                left_skip,
                right_skip,
            },
            penalties: crate::page::ParPenalties {
                inter_line: self.int_par("interlinepenalty"),
                club: self.int_par("clubpenalty"),
                widow: final_widow_penalty,
                broken: self.int_par("brokenpenalty"),
            },
            baselines: self.baselines,
            tolerances: tol,
        };
        let mut prev_depth = self.cur().prev_depth;
        let mut out = crate::postline::post_line_break(list, &positions, &par_params, &mut prev_depth);
        // The reports `hpack` would have made, line by line, in order: §663's
        // "in paragraph at lines".
        for (report, &at) in out.reports.iter().zip(out.lines.iter()) {
            if let (Some(r), Some(Node::Box(b))) = (report, out.list.get(at)) {
                let b = b.clone();
                self.box_report(r, false, &b);
            }
        }
        if let Some(b) = out.badness.last() {
            self.last_badness = *b;
        }
        crate::node::hooks::post_linebreak_filter(&mut out.list, "");
        let cur = self.cur_mut();
        cur.prev_depth = prev_depth;
        cur.list.append(&mut out.list);
        cur.prev_graf = breaks.best_line - 1;
        self.pack_begin_line = 0;
    }
}
