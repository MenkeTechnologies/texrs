//! `fontloader-font-ots.lua`: node mode's feature processor. For one font it
//! walks the list once per enabled lookup ("dataset"), in lookup order,
//! applying each lookup's handler where a step covers the character, and
//! deals with discretionaries the way the fontloader does -- `comprun` for
//! substitutions inside them, `testrun` for ligatures that may cross them,
//! `kernrun` for positioning across them, and `chaindisk` for contextual
//! rules that see one.
//!
//! Positioning is not applied here: the handlers record injections in the
//! nodes' properties (`super::otj`), and the injection pass turns them into
//! kerns and offsets after every font has been processed, as
//! `nodes.simple_font_handler` (fontloader-fonts-gbn.lua:288-299) orders it.

use std::collections::{BTreeMap, BTreeSet};

use super::dsp::{BaseAnchor, Coverage, Flags, LigTree, PairValue, Pos, Sequence};
use super::nodes::{Ix, Kind, List, Slot, GLYPH_LIGATURE};
use super::otj::{self, Counts};
use super::oup::RuleData;
use super::Face;

/// What `otf.dataset` hands the main loop for one lookup
/// (`{ valid, autofeatures[kind] or false, sequence, kind }`).
#[derive(Clone, Debug)]
pub struct Dataset {
    /// The feature's value: `true`, a number, a string.
    pub value: FeatureValue,
    pub sequence: usize,
    pub kind: String,
    /// A trigger_space_kerns pseudo-lookup (font-ots.lua:4540-4550).
    pub space_trigger: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FeatureValue {
    True,
    Number(f64),
    Text(String),
}

/// A node-mode font as the processor sees it.
pub struct Font<'a> {
    pub face: &'a Face,
    pub id: usize,
    /// `parameters.factor`: size / units per em.
    pub factor: f64,
    /// `getthreshold`: with `injections.threshold` 0 this is
    /// `max(0, -2)`, i.e. 0 (font-otj.lua:1597-1605).
    pub threshold: f64,
    /// The characters' scaled widths, the Lua floats setmark and setcursive
    /// read.
    pub widths: BTreeMap<u32, f64>,
    /// The features the request enabled (`shared.features`).
    pub features: BTreeMap<String, FeatureValue>,
    pub script: String,
    pub language: String,
    pub checkmarks: bool,
}

impl Font<'_> {
    fn is_mark(&self, c: u32) -> bool {
        self.face.layout.marks.contains(&c)
    }

    fn class(&self, c: u32) -> &str {
        self.face.class(c)
    }

    fn width(&self, c: u32) -> f64 {
        self.widths.get(&c).copied().unwrap_or(0.0)
    }
}

/// `sequence.skiphash` (font-oup.lua:3085-3112): whether a character is
/// skipped by a lookup's flags and mark filter.
#[derive(Clone)]
pub struct Skip<'a> {
    flags: Flags,
    markclass: Option<&'a BTreeSet<u32>>,
    font: &'a Font<'a>,
}

impl Skip<'_> {
    fn of<'a>(
        font: &'a Font<'a>,
        seq: &Sequence,
        markclass: Option<&'a BTreeSet<u32>>,
    ) -> Option<Skip<'a>> {
        let f = seq.flags;
        (f.mark || f.ligature || f.base || markclass.is_some()).then_some(Skip {
            flags: f,
            markclass,
            font,
        })
    }

    fn skips(&self, c: u32) -> bool {
        let class = self.font.class(c);
        (self.flags.mark && class == "mark")
            || (self
                .markclass
                .is_some_and(|m| class == "mark" && !m.contains(&c)))
            || (self.flags.ligature && class == "ligature")
            || (self.flags.base && class == "base")
    }
}

fn skips(skip: &Option<Skip>, c: u32) -> bool {
    skip.as_ref().is_some_and(|s| s.skips(c))
}

/// The state font-ots.lua keeps in upvalues for one call of the processor.
pub struct State<'a> {
    pub list: &'a mut List,
    pub font: &'a Font<'a>,
    pub counts: &'a mut Counts,
    pub triggers: &'a mut BTreeMap<usize, otj::SpaceKerns>,
    /// `sweephead`: a list head to the node to resume at, `None` for the
    /// Lua `false` (resume nowhere).
    sweephead: BTreeMap<Ix, Option<Ix>>,
    sweepnode: Option<Ix>,
    sweeptype: Option<Sweep>,
    notmatchpre: BTreeSet<Ix>,
    notmatchpost: BTreeSet<Ix>,
    notmatchreplace: BTreeSet<Ix>,
    /// `discs`: with more than one font in the list, which discretionaries
    /// use this font (`fastdisc`).
    discs: Option<BTreeMap<Ix, bool>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sweep {
    Pre,
    Post,
    Replace,
}

type R = (Option<Ix>, Option<Ix>, bool);

/// The lookup's data at one character (`lookupcache[char]`).
#[derive(Clone)]
enum Match<'a> {
    Single(u32),
    Seq(&'a Vec<u32>),
    Lig(&'a LigTree),
    PosSingle(&'a Option<Pos>),
    Move1(f64),
    Pair(&'a BTreeMap<u32, PairValue>),
    Move2(&'a BTreeMap<u32, f64>),
    Mark(usize, Option<[f64; 2]>),
    Cursive(Option<[f64; 2]>),
    Rules(&'a Vec<usize>),
}

fn step_match<'a>(step: &'a super::dsp::Step, c: u32) -> Option<Match<'a>> {
    Some(match &step.coverage {
        Coverage::Single(m) => Match::Single(*m.get(&c)?),
        Coverage::Sequence(m) => Match::Seq(m.get(&c)?),
        Coverage::Ligature(m) => Match::Lig(m.get(&c)?),
        Coverage::Single1(m) => Match::PosSingle(m.get(&c)?),
        Coverage::Move1(m) => Match::Move1(*m.get(&c)?),
        Coverage::Pair(m) => Match::Pair(m.get(&c)?),
        Coverage::Move2(m) | Coverage::Kern(m) => Match::Move2(m.get(&c)?),
        Coverage::Mark(m) => {
            let r = m.get(&c)?;
            Match::Mark(r.class, r.anchor)
        }
        Coverage::Cursive(m) => Match::Cursive(m.get(&c)?.exit),
        Coverage::Rules(m) => Match::Rules(m.get(&c)?),
        Coverage::None => return None,
    })
}

impl<'a> State<'a> {
    pub fn new(
        list: &'a mut List,
        font: &'a Font<'a>,
        counts: &'a mut Counts,
        triggers: &'a mut BTreeMap<usize, otj::SpaceKerns>,
        fonts_in_list: usize,
    ) -> State<'a> {
        State {
            list,
            font,
            counts,
            triggers,
            sweephead: BTreeMap::new(),
            sweepnode: None,
            sweeptype: None,
            notmatchpre: BTreeSet::new(),
            notmatchpost: BTreeSet::new(),
            notmatchreplace: BTreeSet::new(),
            discs: (fonts_in_list > 1).then(BTreeMap::new),
        }
    }

    fn fid(&self) -> Option<usize> {
        Some(self.font.id)
    }

    /// `ischar(n, currentfont)`.
    fn ischar(&self, n: Option<Ix>) -> Result<Option<u32>, ()> {
        self.list.ischar(n, self.fid())
    }

    fn char_of(&self, n: Option<Ix>) -> Option<u32> {
        self.ischar(n).ok().flatten()
    }

    fn next(&self, n: Option<Ix>) -> Option<Ix> {
        self.list.next(n)
    }

    fn prev(&self, n: Option<Ix>) -> Option<Ix> {
        self.list.prev(n)
    }

    /// `usesfont(disc, font)`, cached per disc as `getfastdisc` does.
    fn uses_font(&mut self, disc: Ix) -> bool {
        if let Some(d) = self.discs.as_ref().and_then(|m| m.get(&disc)) {
            return *d;
        }
        let (pre, post, replace) = self.list.disc(disc);
        let fid = self.font.id;
        let mut used = false;
        for l in [pre, post, replace] {
            let mut n = l;
            while let Some(i) = n {
                if self.list.font(i) == Some(fid) {
                    used = true;
                }
                n = self.list.nodes[i].next;
            }
        }
        if let Some(m) = self.discs.as_mut() {
            m.insert(disc, used);
        }
        used
    }

    // --- list helpers of font-ots.lua -----------------------------------

    /// `flattendisk` (font-ots.lua:410-445).
    fn flattendisk(&mut self, head: Option<Ix>, disc: Ix) -> (Option<Ix>, Option<Ix>) {
        let (_pre, _post, replace, _pt, _qt, replacetail) = self.list.disc_tails(disc);
        let (prev, next) = (self.list.nodes[disc].prev, self.list.nodes[disc].next);
        let ishead = head == Some(disc);
        self.list.set_disc(disc, None, None, None);
        if ishead {
            if replace.is_some() {
                if next.is_some() {
                    self.list.link(replacetail, next);
                }
                (replace, replace)
            } else if next.is_some() {
                self.list.set_prev(next, None);
                (next, next)
            } else {
                (None, None)
            }
        } else if replace.is_some() {
            if next.is_some() {
                self.list.link(replacetail, next);
            }
            self.list.link(prev, replace);
            (head, replace)
        } else {
            self.list.link(prev, next);
            (head, next)
        }
    }

    /// `appenddisc` (font-ots.lua:447-462).
    fn appenddisc(&mut self, disc: Ix, list: Option<Ix>) {
        let (pre, post, replace, _pt, posttail, replacetail) = self.list.disc_tails(disc);
        let posthead = list;
        let replacehead = self.list.copy_list(list, None);
        let post = match post {
            Some(_) => {
                self.list.link(posttail, posthead);
                post
            }
            None => posthead,
        };
        let replace = match replace {
            Some(_) => {
                self.list.link(replacetail, replacehead);
                replace
            }
            None => replacehead,
        };
        self.list.set_disc(disc, pre, post, replace);
    }

    /// `copynocomponents(g, copyinjection)`.
    fn copy_no_components(&mut self, g: Ix) -> Ix {
        let comps = self.list.components(g);
        self.list.set_components(g, None);
        let n = self.list.copy(g);
        otj::copy(self.list, n, g);
        self.list.set_components(g, comps);
        n
    }

    /// `components.count` (fontloader-basics-nod.lua:264-280).
    fn count_components(&self, start: Ix) -> i64 {
        let Some(c) = self.list.isglyph(Some(start)) else {
            return 0;
        };
        if self.list.subtype(start) == GLYPH_LIGATURE {
            let mut n = 0;
            let mut comp = self.list.components(start);
            while let Some(x) = comp {
                n += self.count_components(x);
                comp = self.list.nodes[x].next;
            }
            n
        } else if !self.font.is_mark(c) {
            1
        } else {
            0
        }
    }

    /// `markstoligature` (font-ots.lua:464-483).
    fn markstoligature(
        &mut self,
        head: Option<Ix>,
        start: Ix,
        stop: Ix,
        char: u32,
    ) -> (Option<Ix>, Ix) {
        if start == stop && self.list.char(start) == char {
            return (head, start);
        }
        let prev = self.list.nodes[start].prev;
        let next = self.list.nodes[stop].next;
        self.list.set_prev(Some(start), None);
        self.list.set_next(Some(stop), None);
        let base = self.copy_no_components(start);
        let head = if head == Some(start) {
            Some(base)
        } else {
            head
        };
        otj::reset(self.list, base);
        self.list.set_char(base, char);
        self.list.set_components(base, Some(start));
        self.list.link3(prev, Some(base), next);
        if prev.is_none() {
            self.list.set_prev(Some(base), None);
        }
        (head, base)
    }

    /// `toligature` (font-ots.lua:510-622), without the attribute-based
    /// `no_right_ligature` option (no attributes reach the processor here).
    #[allow(clippy::too_many_arguments)]
    fn toligature(
        &mut self,
        head: Option<Ix>,
        start: Ix,
        stop: Ix,
        char: u32,
        skip: &Option<Skip>,
        discfound: Option<Ix>,
        hasmarks: bool,
    ) -> (Option<Ix>, Option<Ix>) {
        if start == stop && self.list.char(start) == char {
            otj::reset(self.list, start);
            self.list.set_char(start, char);
            return (head, Some(start));
        }
        let prev = self.list.nodes[start].prev;
        let next = self.list.nodes[stop].next;
        let comp = start;
        self.list.set_prev(Some(start), None);
        self.list.set_next(Some(stop), None);
        let base = self.copy_no_components(start);
        let mut head = head;
        if Some(start) == head {
            head = Some(base);
        }
        otj::reset(self.list, base);
        self.list.set_char(base, char);
        self.list.set_components(base, Some(comp));
        self.list.link3(prev, Some(base), next);
        if prev.is_none() {
            self.list.set_prev(Some(base), None);
        }
        let mut result = Some(base);
        match discfound {
            None => {
                let deletemarks = skip.is_none() || hasmarks;
                let mut baseindex: i64 = 0;
                let mut componentindex: i64 = 0;
                let mut lhead = Some(base);
                let mut current = Some(base);
                let mut s = Some(start);
                while let Some(st) = s {
                    let c = self.list.char(st);
                    if !self.font.is_mark(c) {
                        baseindex += componentindex;
                        componentindex = self.count_components(st);
                    } else if !deletemarks {
                        let li =
                            otj::get_ligaindex(self.list, st, Some(componentindex)).unwrap_or(0);
                        otj::set_ligaindex(self.list, st, baseindex + li);
                        let n = self.list.copy(st);
                        otj::copy(self.list, n, st);
                        let (h, c2) = self.list.insert_after(lhead, current, n);
                        lhead = h;
                        current = c2;
                    }
                    s = self.list.nodes[st].next;
                }
                let mut s = self.next(current);
                while let Some(st) = s {
                    match self.list.ischar(Some(st), None) {
                        Ok(Some(c)) if self.font.is_mark(c) => {
                            let li = otj::get_ligaindex(self.list, st, Some(componentindex))
                                .unwrap_or(0);
                            otj::set_ligaindex(self.list, st, baseindex + li);
                            s = self.list.nodes[st].next;
                        }
                        _ => break,
                    }
                }
            }
            Some(discfound) => {
                let (discprev, discnext) = (
                    self.list.nodes[discfound].prev,
                    self.list.nodes[discfound].next,
                );
                if discprev.is_some() && discnext.is_some() {
                    let (pre, post, replace, _pt, posttail, _rt) = self.list.disc_tails(discfound);
                    if replace.is_none() {
                        let bprev = self.list.nodes[base].prev;
                        let copied = self.copy_only_glyphs(Some(comp));
                        match pre {
                            Some(_) => self.list.link(discprev, pre),
                            None => self.list.set_next(discprev, None),
                        }
                        let pre = Some(comp);
                        let post = match post {
                            Some(_) => {
                                self.list.link(posttail, discnext);
                                self.list.set_prev(post, None);
                                post
                            }
                            None => {
                                self.list.set_prev(discnext, None);
                                discnext
                            }
                        };
                        self.list.link3(bprev, Some(discfound), next);
                        self.list.set_prev(Some(base), None);
                        self.list.set_next(Some(base), None);
                        self.list.set_components(base, copied);
                        self.list.set_disc(discfound, pre, post, Some(base));
                        result = bprev;
                    }
                }
            }
        }
        (head, result)
    }

    fn copy_only_glyphs(&mut self, current: Option<Ix>) -> Option<Ix> {
        let mut head = None;
        let mut previous = None;
        let mut n = current;
        while let Some(i) = n {
            if matches!(self.list.nodes[i].kind, Kind::Glyph { .. }) {
                let c = self.list.copy(i);
                match previous {
                    Some(p) => self.list.link(Some(p), Some(c)),
                    None => head = Some(c),
                }
                previous = Some(c);
            }
            n = self.list.nodes[i].next;
        }
        head
    }

    /// `multiple_glyphs` (font-ots.lua:624-700) without ConTeXt's
    /// `repeatablemultiples`.
    fn multiple_glyphs(&mut self, head: Option<Ix>, start: Ix, multiple: &[u32]) -> R {
        if multiple.is_empty() {
            return (head, Some(start), false);
        }
        otj::reset(self.list, start);
        self.list.set_char(start, multiple[0]);
        let mut start = start;
        for &m in &multiple[1..] {
            let n = self.list.copy(start);
            otj::reset(self.list, n);
            self.list.set_char(n, m);
            self.list.insert_after(head, Some(start), n);
            start = n;
        }
        (head, Some(start), true)
    }

    /// `get_alternative_glyph` (font-ots.lua:702-741), with
    /// `otf.defaultnodealternate = "none"`.
    fn alternative(&self, start: Ix, alternatives: &[u32], value: &FeatureValue) -> Option<u32> {
        let n = alternatives.len();
        if n == 1 {
            return Some(alternatives[0]);
        }
        let v: Option<f64> = match value {
            FeatureValue::Text(t) if t == "first" => return alternatives.first().copied(),
            FeatureValue::Text(t) if t == "last" => return alternatives.last().copied(),
            // "random" would need the run's random state; luaotfload's is
            // math.random, which nothing here reproduces.
            FeatureValue::Text(t) if t == "random" => return alternatives.first().copied(),
            FeatureValue::True => Some(1.0),
            FeatureValue::Number(x) => Some(*x),
            FeatureValue::Text(t) => t.parse().ok(),
        };
        let Some(v) = v else {
            return alternatives.first().copied();
        };
        if v > n as f64 {
            None
        } else if v == 0.0 {
            Some(self.list.char(start))
        } else if v < 1.0 {
            alternatives.first().copied()
        } else if v.fract() != 0.0 {
            // `alternatives[1.5]` is nil
            None
        } else {
            alternatives.get(v as usize - 1).copied()
        }
    }

    // --- handlers -------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn handler(
        &mut self,
        head: Option<Ix>,
        start: Ix,
        dataset: &Dataset,
        seq: &Sequence,
        m: &Match,
        rlmode: i32,
        skip: &Option<Skip>,
        stepi: usize,
        injection: Option<Slot>,
    ) -> (Option<Ix>, Option<Ix>, bool, bool) {
        let typ = seq.typ;
        match (typ, m) {
            ("gsub_single", Match::Single(r)) => {
                otj::reset(self.list, start);
                self.list.set_char(start, *r);
                (head, Some(start), true, false)
            }
            ("gsub_alternate", Match::Seq(alts)) => {
                let value = match &dataset.value {
                    FeatureValue::True => self
                        .font
                        .features
                        .get(&dataset.kind)
                        .cloned()
                        .unwrap_or(FeatureValue::True),
                    v => v.clone(),
                };
                if let Some(choice) = self.alternative(start, alts, &value) {
                    otj::reset(self.list, start);
                    self.list.set_char(start, choice);
                }
                (head, Some(start), true, false)
            }
            ("gsub_multiple", Match::Seq(multiple)) => {
                let (h, s, ok) = self.multiple_glyphs(head, start, multiple);
                (h, s, ok, false)
            }
            ("gsub_ligature", Match::Lig(tree)) => self.gsub_ligature(head, start, tree, skip),
            ("gpos_single", _) => {
                match m {
                    Match::PosSingle(v) => {
                        let spec = match v {
                            Some(Pos::V(v)) => *v,
                            _ => [0.0; 4],
                        };
                        otj::set_position(
                            self.list,
                            self.counts,
                            start,
                            self.font.factor,
                            rlmode,
                            spec,
                            injection,
                        );
                    }
                    Match::Move1(k) => {
                        otj::set_move(
                            self.list,
                            self.counts,
                            start,
                            self.font.factor,
                            rlmode,
                            *k,
                            injection,
                        );
                    }
                    _ => {}
                }
                (head, Some(start), true, false)
            }
            ("gpos_pair", _) => {
                let format = seq.steps.get(stepi).and_then(|s| s.format);
                let (h, s, ok) = self.gpos_pair(head, start, m, format, rlmode, skip, injection);
                (h, s, ok, false)
            }
            ("gpos_mark2base", Match::Mark(class, ma)) => {
                let ok = self.mark2base(start, seq, stepi, *class, *ma, rlmode, false);
                (head, Some(start), ok, false)
            }
            ("gpos_mark2ligature", Match::Mark(class, ma)) => {
                let ok = self.mark2ligature(start, seq, stepi, *class, *ma, rlmode);
                (head, Some(start), ok, false)
            }
            ("gpos_mark2mark", Match::Mark(class, ma)) => {
                let ok = self.mark2mark(start, seq, stepi, *class, *ma, rlmode);
                (head, Some(start), ok, false)
            }
            ("gpos_cursive", Match::Cursive(exit)) => {
                let ok = self.cursive(start, seq, stepi, *exit, rlmode);
                (head, Some(start), ok, false)
            }
            (_, Match::Rules(contexts)) => {
                let (h, s, d) =
                    self.contextchain(head, start, dataset, seq, stepi, contexts, rlmode, skip);
                (h, s, d, false)
            }
            _ => (head, Some(start), false, false),
        }
    }

    /// `handlers.gsub_ligature` (font-ots.lua:784-950).
    fn gsub_ligature(
        &mut self,
        head: Option<Ix>,
        start: Ix,
        tree: &LigTree,
        skip: &Option<Skip>,
    ) -> (Option<Ix>, Option<Ix>, bool, bool) {
        let mut current = self.next(Some(start));
        if current.is_none() {
            return (head, Some(start), false, false);
        }
        let mut stop: Option<Ix> = None;
        let startchar = self.list.char(start);
        let mut node = tree;
        let mut head = head;
        if skips(skip, startchar) {
            while let Some(cur) = current {
                match self.ischar(Some(cur)) {
                    Ok(Some(c)) => match node.next.get(&c) {
                        Some(lg) => {
                            stop = Some(cur);
                            node = lg;
                            current = self.next(Some(cur));
                        }
                        None => break,
                    },
                    _ => break,
                }
            }
            if let Some(stop) = stop {
                if let Some(lig) = node.ligature {
                    let (h, s) = self.markstoligature(head, start, stop, lig);
                    return (h, Some(s), true, false);
                }
            }
        } else {
            let mut discfound: Option<Ix> = None;
            let mut hasmarks = self.font.is_mark(startchar);
            while let Some(cur) = current {
                match self.ischar(Some(cur)) {
                    Ok(Some(c)) => {
                        if skips(skip, c) {
                            current = self.next(Some(cur));
                        } else {
                            match node.next.get(&c) {
                                Some(lg) => {
                                    if self.font.is_mark(c) {
                                        hasmarks = true;
                                    }
                                    stop = Some(cur);
                                    node = lg;
                                    current = self.next(Some(cur));
                                }
                                None => break,
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(()) if self.list.is_disc(Some(cur)) => {
                        discfound = Some(cur);
                        break;
                    }
                    Err(()) => break,
                }
            }
            if let Some(df) = discfound {
                let (pre, post, replace) = self.list.disc(df);
                let _ = post;
                let mut matched = false;
                if replace.is_some() {
                    if let Some(c) = self.char_of(replace) {
                        if node.next.contains_key(&c) {
                            matched = true;
                        }
                    }
                }
                if !matched && pre.is_some() {
                    if let Some(c) = self.char_of(pre) {
                        if node.next.contains_key(&c) {
                            matched = true;
                        }
                    }
                }
                // `not match and not pre or not replace`
                if (!matched && pre.is_none()) || replace.is_none() {
                    let n = self.next(Some(df));
                    if let Some(c) = self.char_of(n) {
                        if node.next.contains_key(&c) {
                            matched = true;
                        }
                    }
                }
                if matched {
                    let ishead = head == Some(start);
                    let prev = self.list.nodes[start].prev;
                    let (mut pre, mut replace) = (pre, replace);
                    match stop {
                        Some(stop) => {
                            self.list.set_next(Some(stop), None);
                            let copy = self.list.copy_list(Some(start), None);
                            let tail = Some(stop);
                            let liat = self.list.tail(copy);
                            if pre.is_some() {
                                self.list.link(liat, pre);
                            }
                            if replace.is_some() {
                                self.list.link(tail, replace);
                            }
                            pre = copy;
                            replace = Some(start);
                        }
                        None => {
                            self.list.set_next(Some(start), None);
                            let copy = self.list.copy(start);
                            if pre.is_some() {
                                self.list.link(Some(copy), pre);
                            }
                            if replace.is_some() {
                                self.list.link(Some(start), replace);
                            }
                            pre = Some(copy);
                            replace = Some(start);
                        }
                    }
                    let post = self.list.disc(df).1;
                    self.list.set_disc(df, pre, post, replace);
                    match prev {
                        Some(_) => self.list.link(prev, Some(df)),
                        None => {
                            self.list.set_prev(Some(df), None);
                            head = Some(df);
                        }
                    }
                    let _ = ishead;
                    return (head, Some(df), true, true);
                }
            }
            if let Some(lig) = node.ligature {
                match stop {
                    Some(stop) => {
                        let (h, s) = self.toligature(head, start, stop, lig, skip, None, hasmarks);
                        return (h, s, true, false);
                    }
                    None => {
                        otj::reset(self.list, start);
                        self.list.set_char(start, lig);
                        return (head, Some(start), true, false);
                    }
                }
            }
        }
        (head, Some(start), false, false)
    }

    /// `handlers.gpos_pair` (font-ots.lua:973-1037) and its chain variant.
    #[allow(clippy::too_many_arguments)]
    fn gpos_pair(
        &mut self,
        head: Option<Ix>,
        start: Ix,
        m: &Match,
        format: Option<&str>,
        rlmode: i32,
        skip: &Option<Skip>,
        injection: Option<Slot>,
    ) -> R {
        let mut snext = self.next(Some(start));
        if snext.is_none() {
            return (head, Some(start), false);
        }
        let mut start = start;
        while let Some(sn) = snext {
            let Some(nextchar) = self.char_of(Some(sn)) else {
                break;
            };
            if skips(skip, nextchar) {
                snext = self.next(Some(sn));
                continue;
            }
            match m {
                Match::Pair(kerns) => {
                    let Some(krn) = kerns.get(&nextchar) else {
                        break;
                    };
                    if format == Some("pair") {
                        if let Some(Pos::V(a)) = &krn.first {
                            otj::set_position(
                                self.list,
                                self.counts,
                                start,
                                self.font.factor,
                                rlmode,
                                *a,
                                injection,
                            );
                        }
                        match &krn.second {
                            Some(Pos::Zero) => start = sn,
                            Some(Pos::V(b)) => {
                                otj::set_position(
                                    self.list,
                                    self.counts,
                                    sn,
                                    self.font.factor,
                                    rlmode,
                                    *b,
                                    injection,
                                );
                                start = sn;
                            }
                            None => {}
                        }
                        return (head, Some(start), true);
                    }
                    break;
                }
                Match::Move2(kerns) => {
                    let Some(&krn) = kerns.get(&nextchar) else {
                        break;
                    };
                    if krn != 0.0 {
                        match format {
                            Some("move") => otj::set_move(
                                self.list,
                                self.counts,
                                sn,
                                self.font.factor,
                                rlmode,
                                krn,
                                injection,
                            ),
                            _ => otj::set_kern(
                                self.list,
                                self.counts,
                                sn,
                                self.font.factor,
                                krn,
                                injection,
                            ),
                        };
                        return (head, Some(start), true);
                    }
                    break;
                }
                _ => break,
            }
        }
        (head, Some(start), false)
    }

    fn baseclasses<'s>(
        &self,
        seq: &'s Sequence,
        stepi: usize,
        class: usize,
    ) -> Option<&'s BTreeMap<u32, BaseAnchor>> {
        seq.steps.get(stepi)?.baseclasses.as_ref()?.get(&class)
    }

    /// The base a mark attaches to: the previous non-mark glyph of the font
    /// (font-ots.lua:1045-1071).
    fn base_before(&self, start: Ix) -> Option<(Ix, u32)> {
        let mut base = self.prev(Some(start));
        let mut basechar = self.char_of(base)?;
        if self.font.is_mark(basechar) {
            loop {
                base = self.prev(base);
                base?;
                match self.ischar(base) {
                    Ok(Some(c)) => {
                        basechar = c;
                        if !self.font.is_mark(c) {
                            break;
                        }
                    }
                    _ => return None,
                }
            }
        }
        Some((base?, basechar))
    }

    #[allow(clippy::too_many_arguments)]
    fn mark2base(
        &mut self,
        start: Ix,
        seq: &Sequence,
        stepi: usize,
        class: usize,
        ma: Option<[f64; 2]>,
        rlmode: i32,
        _chain: bool,
    ) -> bool {
        let markchar = self.list.char(start);
        if !self.font.is_mark(markchar) {
            return false;
        }
        let Some((base, basechar)) = self.base_before(start) else {
            return false;
        };
        let ba = match self
            .baseclasses(seq, stepi, class)
            .and_then(|b| b.get(&basechar))
        {
            Some(BaseAnchor::One(Some(a))) => *a,
            _ => return false,
        };
        let Some(ma) = ma else {
            return false;
        };
        let w = self.font.width(basechar);
        otj::set_mark(
            self.list,
            self.counts,
            start,
            base,
            self.font.factor,
            rlmode,
            ba,
            ma,
            w,
            false,
            self.font.checkmarks,
        );
        true
    }

    fn mark2ligature(
        &mut self,
        start: Ix,
        seq: &Sequence,
        stepi: usize,
        class: usize,
        ma: Option<[f64; 2]>,
        rlmode: i32,
    ) -> bool {
        let markchar = self.list.char(start);
        if !self.font.is_mark(markchar) {
            return false;
        }
        let Some((base, basechar)) = self.base_before(start) else {
            return false;
        };
        let Some(BaseAnchor::Components(comps)) = self
            .baseclasses(seq, stepi, class)
            .and_then(|b| b.get(&basechar))
        else {
            return false;
        };
        let Some(ma) = ma else {
            return false;
        };
        let index = otj::get_ligaindex(self.list, start, None);
        let Some(Some(ba)) = index
            .and_then(|i| usize::try_from(i).ok())
            .and_then(|i| comps.get(&i))
        else {
            return false;
        };
        let ba = *ba;
        let w = self.font.width(basechar);
        otj::set_mark(
            self.list,
            self.counts,
            start,
            base,
            self.font.factor,
            rlmode,
            ba,
            ma,
            w,
            false,
            self.font.checkmarks,
        );
        true
    }

    fn mark2mark(
        &mut self,
        start: Ix,
        seq: &Sequence,
        stepi: usize,
        class: usize,
        ma: Option<[f64; 2]>,
        rlmode: i32,
    ) -> bool {
        let markchar = self.list.char(start);
        if !self.font.is_mark(markchar) {
            return false;
        }
        let mut base = self.prev(Some(start));
        if let Some(slc) = otj::get_ligaindex(self.list, start, None) {
            while let Some(b) = base {
                let blc = otj::get_ligaindex(self.list, b, None);
                if blc.is_some_and(|blc| blc != slc) {
                    base = self.prev(Some(b));
                } else {
                    break;
                }
            }
        }
        let Some(b) = base else {
            return false;
        };
        let Some(basechar) = self.char_of(Some(b)) else {
            return false;
        };
        let ba = match self
            .baseclasses(seq, stepi, class)
            .and_then(|m| m.get(&basechar))
        {
            Some(BaseAnchor::One(Some(a))) => *a,
            _ => return false,
        };
        let Some(ma) = ma else {
            return false;
        };
        let w = self.font.width(basechar);
        otj::set_mark(
            self.list,
            self.counts,
            start,
            b,
            self.font.factor,
            rlmode,
            ba,
            ma,
            w,
            true,
            self.font.checkmarks,
        );
        true
    }

    /// `handlers.gpos_cursive` (font-ots.lua:1198-1233).
    fn cursive(
        &mut self,
        start: Ix,
        seq: &Sequence,
        stepi: usize,
        exit: Option<[f64; 2]>,
        rlmode: i32,
    ) -> bool {
        let startchar = self.list.char(start);
        if self.font.is_mark(startchar) {
            return false;
        }
        let mut nxt = self.next(Some(start));
        while let Some(n) = nxt {
            let Some(nextchar) = self.char_of(Some(n)) else {
                break;
            };
            if self.font.is_mark(nextchar) {
                nxt = self.next(Some(n));
                continue;
            }
            let Some(exit) = exit else {
                break;
            };
            let entry = match seq.steps.get(stepi).map(|s| &s.coverage) {
                Some(Coverage::Cursive(c)) => c.get(&nextchar).and_then(|e| e.entry),
                _ => None,
            };
            if let Some(entry) = entry {
                let ws = self.font.width(startchar);
                let wn = self.font.width(nextchar);
                otj::set_cursive(
                    self.list,
                    self.counts,
                    start,
                    n,
                    self.font.factor,
                    rlmode,
                    exit,
                    entry,
                    ws,
                    wn,
                );
                return true;
            }
            break;
        }
        false
    }

    // --- contextual chains ------------------------------------------------

    fn rules(&self, seq: &Sequence, stepi: usize) -> &'a [RuleData] {
        let face: &'a Face = self.font.face;
        let (list, index) = match face
            .layout
            .sequences
            .iter()
            .position(|s| std::ptr::eq(s, seq))
        {
            Some(i) => (&face.expanded.rules, i),
            None => (
                &face.expanded.subrules,
                face.layout
                    .sublookups
                    .iter()
                    .position(|s| std::ptr::eq(s, seq))
                    .unwrap_or(0),
            ),
        };
        list.get(index)
            .and_then(|steps| steps.get(stepi))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// `handle_contextchain` (font-ots.lua:2480-2931).
    #[allow(clippy::too_many_arguments)]
    fn contextchain(
        &mut self,
        head: Option<Ix>,
        start: Ix,
        dataset: &Dataset,
        seq: &Sequence,
        stepi: usize,
        contexts: &[usize],
        rlmode: i32,
        skip: &Option<Skip>,
    ) -> R {
        let allrules = self.rules(seq, stepi);
        let sweepnode = self.sweepnode;
        let sweeptype = self.sweeptype;
        let (mut postreplace, mut prereplace, mut checkdisc) = (false, false, None);
        if let Some(t) = sweeptype {
            if t == Sweep::Replace {
                postreplace = true;
                prereplace = true;
            } else {
                postreplace = t == Sweep::Post;
                prereplace = t == Sweep::Pre;
            }
            checkdisc = self.prev(head);
        }
        let mut discseen = false;
        let mut skipped = false;
        let (startprev, startnext) = (self.list.nodes[start].prev, self.list.nodes[start].next);
        let mut head = head;
        let mut start = Some(start);
        let mut done = false;
        let startchar = self.char_of(start);
        let space = |s: &Self, n: Ix| -> bool {
            matches!(s.list.nodes[n].kind, Kind::Glue { .. })
                && (s.list.width_of_glue(n) as f64) > s.font.threshold
        };
        'contexts: for &k in contexts {
            let Some(ck) = allrules.get(k) else {
                continue;
            };
            let seqsets = &ck.sequence;
            let f = ck.start;
            let mut last = start;
            let Some(sc) = startchar else {
                continue;
            };
            if !seqsets[f - 1].contains(&sc) {
                continue;
            }
            let s = seqsets.len();
            if s != 1 {
                let l = ck.stop;
                // current match
                if l > f {
                    let mut discfound: Option<Ix> = None;
                    let mut n = f + 1;
                    last = startnext;
                    while n <= l {
                        if postreplace && last.is_none() {
                            // (the Lua also clears its local copy of sweeptype here,
                            // which nothing reads afterwards)
                            last = self.next(sweepnode);
                        }
                        let Some(lst) = last else {
                            continue 'contexts;
                        };
                        match self.ischar(Some(lst)) {
                            Ok(Some(c)) => {
                                if seqsets[n - 1].contains(&c) {
                                    if n < l {
                                        last = self.next(Some(lst));
                                    }
                                    n += 1;
                                } else if skips(skip, c) {
                                    skipped = true;
                                    last = self.next(Some(lst));
                                } else if let Some(df) = discfound {
                                    self.notmatchreplace.insert(df);
                                    if self.notmatchpre.contains(&df) {
                                        continue 'contexts;
                                    }
                                    break;
                                } else {
                                    continue 'contexts;
                                }
                            }
                            Ok(None) => {
                                if let Some(df) = discfound {
                                    self.notmatchreplace.insert(df);
                                    if self.notmatchpre.contains(&df) {
                                        continue 'contexts;
                                    }
                                    break;
                                }
                                continue 'contexts;
                            }
                            Err(()) if self.list.is_disc(Some(lst)) => {
                                discseen = true;
                                discfound = Some(lst);
                                self.notmatchpre.remove(&lst);
                                self.notmatchpost.insert(lst);
                                self.notmatchreplace.remove(&lst);
                                let (mut pre, _post, mut replace) = self.list.disc(lst);
                                if pre.is_some() {
                                    let mut nn = n;
                                    while let Some(p) = pre {
                                        if seqsets[nn - 1].contains(&self.list.char(p)) {
                                            nn += 1;
                                            if nn > l {
                                                break;
                                            }
                                            pre = self.next(Some(p));
                                        } else {
                                            self.notmatchpre.insert(lst);
                                            break;
                                        }
                                    }
                                    if nn <= l {
                                        self.notmatchpre.insert(lst);
                                    }
                                } else {
                                    self.notmatchpre.insert(lst);
                                }
                                if replace.is_some() {
                                    while let Some(r) = replace {
                                        if seqsets[n - 1].contains(&self.list.char(r)) {
                                            n += 1;
                                            if n > l {
                                                break;
                                            }
                                            replace = self.next(Some(r));
                                        } else {
                                            self.notmatchreplace.insert(lst);
                                            if self.notmatchpre.contains(&lst) {
                                                continue 'contexts;
                                            }
                                            break;
                                        }
                                    }
                                    if self.notmatchpre.contains(&lst) {
                                        continue 'contexts;
                                    }
                                }
                                last = self.next(Some(lst));
                            }
                            Err(()) => continue 'contexts,
                        }
                    }
                }
                // before
                if f > 1 {
                    let mut prev = startprev;
                    if prereplace && prev == checkdisc {
                        prev = self.prev(sweepnode);
                    }
                    if prev.is_none() {
                        continue 'contexts;
                    }
                    let mut discfound: Option<Ix> = None;
                    let mut n = f - 1;
                    while n >= 1 {
                        let Some(pv) = prev else {
                            continue 'contexts;
                        };
                        match self.ischar(Some(pv)) {
                            Ok(Some(c)) => {
                                if seqsets[n - 1].contains(&c) {
                                    if n > 1 {
                                        prev = self.prev(Some(pv));
                                    }
                                    n -= 1;
                                } else if skips(skip, c) {
                                    skipped = true;
                                    prev = self.prev(Some(pv));
                                } else if let Some(df) = discfound {
                                    self.notmatchreplace.insert(df);
                                    if self.notmatchpost.contains(&df) {
                                        continue 'contexts;
                                    }
                                    break;
                                } else {
                                    continue 'contexts;
                                }
                            }
                            Ok(None) => {
                                if let Some(df) = discfound {
                                    self.notmatchreplace.insert(df);
                                    if self.notmatchpost.contains(&df) {
                                        continue 'contexts;
                                    }
                                } else {
                                    continue 'contexts;
                                }
                                break;
                            }
                            Err(()) => match self.list.nodes[pv].kind.clone() {
                                Kind::Disc { .. } => {
                                    discseen = true;
                                    discfound = Some(pv);
                                    self.notmatchpre.insert(pv);
                                    self.notmatchpost.remove(&pv);
                                    self.notmatchreplace.remove(&pv);
                                    let (pre, post, replace, _pt, mut posttail, mut replacetail) =
                                        self.list.disc_tails(pv);
                                    if pre != start && post != start && replace != start {
                                        if post.is_some() {
                                            let mut nn = n;
                                            while let Some(pt) = posttail {
                                                if seqsets[nn - 1].contains(&self.list.char(pt)) {
                                                    nn -= 1;
                                                    if Some(pt) == post || nn < 1 {
                                                        break;
                                                    }
                                                    posttail = self.prev(Some(pt));
                                                } else {
                                                    self.notmatchpost.insert(pv);
                                                    break;
                                                }
                                            }
                                            if nn >= 1 {
                                                self.notmatchpost.insert(pv);
                                            }
                                        } else {
                                            self.notmatchpost.insert(pv);
                                        }
                                        if replace.is_some() {
                                            while let Some(rt) = replacetail {
                                                if seqsets[n - 1].contains(&self.list.char(rt)) {
                                                    n -= 1;
                                                    if Some(rt) == replace || n < 1 {
                                                        break;
                                                    }
                                                    replacetail = self.prev(Some(rt));
                                                } else {
                                                    self.notmatchreplace.insert(pv);
                                                    if self.notmatchpost.contains(&pv) {
                                                        continue 'contexts;
                                                    }
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                    prev = self.prev(Some(pv));
                                }
                                Kind::Glue { .. } => {
                                    let sn = &seqsets[n - 1];
                                    if (sn.contains(&32) && space(self, pv)) || sn.contains(&0xFFFC)
                                    {
                                        n -= 1;
                                        prev = self.prev(Some(pv));
                                    } else {
                                        continue 'contexts;
                                    }
                                }
                                _ => {
                                    if seqsets[n - 1].contains(&0xFFFC) {
                                        n -= 1;
                                        prev = self.prev(Some(pv));
                                    } else {
                                        continue 'contexts;
                                    }
                                }
                            },
                        }
                        if n == 0 {
                            break;
                        }
                    }
                }
                // after
                if s > l {
                    let mut current = last.and_then(|x| self.next(Some(x)));
                    if current.is_none() && postreplace {
                        current = self.next(sweepnode);
                    }
                    if current.is_none() {
                        continue 'contexts;
                    }
                    let mut discfound: Option<Ix> = None;
                    let mut n = l + 1;
                    while n <= s {
                        let Some(cur) = current else {
                            continue 'contexts;
                        };
                        match self.ischar(Some(cur)) {
                            Ok(Some(c)) => {
                                if seqsets[n - 1].contains(&c) {
                                    if n < s {
                                        current = self.next(Some(cur));
                                    }
                                    n += 1;
                                } else if skips(skip, c) {
                                    skipped = true;
                                    current = self.next(Some(cur));
                                } else if let Some(df) = discfound {
                                    self.notmatchreplace.insert(df);
                                    if self.notmatchpre.contains(&df) {
                                        continue 'contexts;
                                    }
                                    break;
                                } else {
                                    continue 'contexts;
                                }
                            }
                            Ok(None) => {
                                if let Some(df) = discfound {
                                    self.notmatchreplace.insert(df);
                                    if self.notmatchpre.contains(&df) {
                                        continue 'contexts;
                                    }
                                    break;
                                }
                                continue 'contexts;
                            }
                            Err(()) => match self.list.nodes[cur].kind.clone() {
                                Kind::Disc { .. } => {
                                    discseen = true;
                                    discfound = Some(cur);
                                    self.notmatchpre.remove(&cur);
                                    self.notmatchpost.insert(cur);
                                    self.notmatchreplace.remove(&cur);
                                    let (mut pre, _post, mut replace) = self.list.disc(cur);
                                    if pre.is_some() {
                                        let mut nn = n;
                                        while let Some(p) = pre {
                                            if seqsets[nn - 1].contains(&self.list.char(p)) {
                                                nn += 1;
                                                if nn > s {
                                                    break;
                                                }
                                                pre = self.next(Some(p));
                                            } else {
                                                self.notmatchpre.insert(cur);
                                                break;
                                            }
                                        }
                                        if nn <= s {
                                            self.notmatchpre.insert(cur);
                                        }
                                    } else {
                                        self.notmatchpre.insert(cur);
                                    }
                                    if replace.is_some() {
                                        while let Some(r) = replace {
                                            if seqsets[n - 1].contains(&self.list.char(r)) {
                                                n += 1;
                                                if n > s {
                                                    break;
                                                }
                                                replace = self.next(Some(r));
                                            } else {
                                                self.notmatchreplace.insert(cur);
                                                if self.notmatchpre.contains(&cur) {
                                                    continue 'contexts;
                                                }
                                                break;
                                            }
                                        }
                                    }
                                    current = self.next(Some(cur));
                                }
                                Kind::Glue { .. } => {
                                    let sn = &seqsets[n - 1];
                                    if (sn.contains(&32) && space(self, cur))
                                        || sn.contains(&0xFFFC)
                                    {
                                        n += 1;
                                        current = self.next(Some(cur));
                                    } else {
                                        continue 'contexts;
                                    }
                                }
                                _ => {
                                    if seqsets[n - 1].contains(&0xFFFC) {
                                        n += 1;
                                        current = self.next(Some(cur));
                                    } else {
                                        continue 'contexts;
                                    }
                                }
                            },
                        }
                    }
                }
            }
            let skip = if skipped { skip.clone() } else { None };
            let (h, st, d) = if discseen || self.sweepnode.is_some() {
                self.chaindisk(head, start, dataset, rlmode, &skip, ck)
            } else {
                self.chainrun(head, start, last, dataset, rlmode, &skip, ck)
            };
            head = h;
            start = st;
            done = d;
            if done {
                break;
            }
        }
        if discseen {
            self.notmatchpre.clear();
            self.notmatchpost.clear();
            self.notmatchreplace.clear();
        }
        (head, start, done)
    }

    /// `chainrun` (font-ots.lua:1971-2104).
    #[allow(clippy::too_many_arguments)]
    fn chainrun(
        &mut self,
        head: Option<Ix>,
        start: Option<Ix>,
        last: Option<Ix>,
        dataset: &Dataset,
        rlmode: i32,
        skip: &Option<Skip>,
        ck: &RuleData,
    ) -> R {
        let size = ck.stop + 1 - ck.start;
        let mut head = head;
        let mut start = start;
        let mut done = false;
        match &ck.lookups {
            Some(chainlookups) => {
                if size == 1 {
                    if let Some(Some(chainlookup)) = chainlookups.first() {
                        for &sub in chainlookup {
                            let Some(st) = start else {
                                break;
                            };
                            let (h, s, ok, _n, _d) =
                                self.chainproc(head, st, last, dataset, sub, rlmode, skip, Some(1));
                            head = h;
                            start = s;
                            if ok {
                                done = true;
                            }
                        }
                    }
                } else {
                    let mut i = 1;
                    let mut laststart = start;
                    let nofchainlookups = chainlookups.len();
                    while start.is_some() {
                        if skip.is_some() {
                            while let Some(st) = start {
                                match self.ischar(Some(st)) {
                                    Ok(Some(c)) if skips(skip, c) => start = self.next(Some(st)),
                                    _ => break,
                                }
                            }
                        }
                        if let Some(Some(chainlookup)) = chainlookups.get(i - 1) {
                            for &sub in chainlookup {
                                let Some(st) = start else {
                                    break;
                                };
                                let (h, s, ok, n, _d) = self.chainproc(
                                    head,
                                    st,
                                    last,
                                    dataset,
                                    sub,
                                    rlmode,
                                    skip,
                                    Some(i),
                                );
                                head = h;
                                start = s;
                                if ok {
                                    done = true;
                                    if n > 1 && i + n > nofchainlookups {
                                        i = size;
                                        break;
                                    }
                                }
                            }
                        }
                        i += 1;
                        if i > size || start.is_none() {
                            break;
                        }
                        laststart = start;
                        start = self.next(start);
                    }
                    if start.is_none() {
                        start = laststart;
                    }
                }
            }
            None => match &ck.replacements {
                Some(replacements) => {
                    if let Some(st) = start {
                        let c = self.list.char(st);
                        if let Some(&r) = replacements.get(&c) {
                            otj::reset(self.list, st);
                            self.list.set_char(st, r);
                            done = true;
                        }
                    }
                }
                None => done = true,
            },
        }
        (head, start, done)
    }

    /// `chainprocs[type](...)`: one sublookup applied inside a chain. Returns
    /// head, start, ok, the number of replacements (ligatures) and the disc a
    /// ligature crossed.
    #[allow(clippy::too_many_arguments)]
    fn chainproc(
        &mut self,
        head: Option<Ix>,
        start: Ix,
        stop: Option<Ix>,
        dataset: &Dataset,
        sub: usize,
        rlmode: i32,
        skip: &Option<Skip>,
        chainindex: Option<usize>,
    ) -> (Option<Ix>, Option<Ix>, bool, usize, Option<Ix>) {
        let face: &'a Face = self.font.face;
        let Some(lookup) = face.layout.sublookups.get(sub - 1) else {
            return (head, Some(start), false, 0, None);
        };
        // getmapping: the first step's coverage and format
        let Some(step) = lookup.steps.first() else {
            return (head, Some(start), false, 0, None);
        };
        let format = step.format;
        let typ = lookup.typ;
        match typ {
            "gsub_single" => {
                let mut current = Some(start);
                while let Some(cur) = current {
                    match self.list.ischar(Some(cur), None) {
                        Ok(Some(c)) => {
                            if let Some(Match::Single(r)) = step_match(step, c) {
                                otj::reset(self.list, cur);
                                self.list.set_char(cur, r);
                            }
                            return (head, Some(start), true, 0, None);
                        }
                        Ok(None) => break,
                        Err(()) => {
                            if Some(cur) == stop {
                                break;
                            }
                            current = self.next(Some(cur));
                        }
                    }
                }
                (head, Some(start), false, 0, None)
            }
            "gsub_alternate" => {
                let value = match &dataset.value {
                    FeatureValue::True => self
                        .font
                        .features
                        .get(&dataset.kind)
                        .cloned()
                        .unwrap_or(FeatureValue::True),
                    v => v.clone(),
                };
                let mut current = Some(start);
                while let Some(cur) = current {
                    match self.list.ischar(Some(cur), None) {
                        Ok(Some(c)) => {
                            if let Some(Match::Seq(alts)) = step_match(step, c) {
                                if let Some(choice) = self.alternative(cur, alts, &value) {
                                    otj::reset(self.list, start);
                                    self.list.set_char(start, choice);
                                }
                            }
                            return (head, Some(start), true, 0, None);
                        }
                        Ok(None) => break,
                        Err(()) => {
                            if Some(cur) == stop {
                                break;
                            }
                            current = self.next(Some(cur));
                        }
                    }
                }
                (head, Some(start), false, 0, None)
            }
            "gsub_multiple" => {
                let c = self.list.char(start);
                match step_match(step, c) {
                    Some(Match::Seq(m)) if !m.is_empty() => {
                        let (h, s, ok) = self.multiple_glyphs(head, start, m);
                        (h, s, ok, 0, None)
                    }
                    _ => (head, Some(start), false, 0, None),
                }
            }
            "gsub_ligature" => {
                let startchar = self.list.char(start);
                let Some(Match::Lig(tree)) = step_match(step, startchar) else {
                    return (head, Some(start), false, 0, None);
                };
                let mut hasmarks = self.font.is_mark(startchar);
                let mut current = self.next(Some(start));
                let mut discfound: Option<Ix> = None;
                let mut last = stop;
                let mut nofreplacements = 1;
                let mut node = tree;
                while let Some(cur) = current {
                    if self.list.is_disc(Some(cur)) {
                        if discfound.is_none() {
                            discfound = Some(cur);
                        }
                        if Some(cur) == stop {
                            break;
                        }
                        current = self.next(Some(cur));
                        continue;
                    }
                    let schar = self.list.char(cur);
                    if skips(skip, schar) {
                        current = self.next(Some(cur));
                        continue;
                    }
                    match node.next.get(&schar) {
                        Some(lg) => {
                            node = lg;
                            last = Some(cur);
                            nofreplacements += 1;
                            // `marks[char]` reads an undefined `char` (nil):
                            // never true.
                            let _ = &mut hasmarks;
                            if Some(cur) == stop {
                                break;
                            }
                            current = self.next(Some(cur));
                        }
                        None => break,
                    }
                }
                match node.ligature {
                    Some(lig) => {
                        let stop = match chainindex {
                            Some(_) => last,
                            None => stop,
                        };
                        let stop = stop.unwrap_or(start);
                        let (h, s) =
                            self.toligature(head, start, stop, lig, skip, discfound, hasmarks);
                        (h, s, true, nofreplacements, discfound)
                    }
                    None => (head, Some(start), false, 0, None),
                }
            }
            "gpos_single" => {
                let c = self.list.char(start);
                match step_match(step, c) {
                    Some(Match::PosSingle(v)) if format == Some("single") => {
                        let spec = match v {
                            Some(Pos::V(v)) => *v,
                            _ => [0.0; 4],
                        };
                        otj::set_position(
                            self.list,
                            self.counts,
                            start,
                            self.font.factor,
                            rlmode,
                            spec,
                            None,
                        );
                        (head, Some(start), true, 0, None)
                    }
                    Some(Match::Move1(k)) => {
                        match format {
                            Some("move") => otj::set_move(
                                self.list,
                                self.counts,
                                start,
                                self.font.factor,
                                rlmode,
                                k,
                                None,
                            ),
                            _ => otj::set_kern(
                                self.list,
                                self.counts,
                                start,
                                self.font.factor,
                                k,
                                None,
                            ),
                        };
                        (head, Some(start), true, 0, None)
                    }
                    _ => (head, Some(start), false, 0, None),
                }
            }
            "gpos_pair" => {
                if self.next(Some(start)).is_none() {
                    return (head, Some(start), false, 0, None);
                }
                let c = self.list.char(start);
                let Some(m) = step_match(step, c) else {
                    return (head, Some(start), false, 0, None);
                };
                let (h, s, ok) = self.gpos_pair(
                    head,
                    start,
                    &m,
                    format,
                    rlmode,
                    skip,
                    Some(Slot::Injections),
                );
                (h, s, ok, 0, None)
            }
            "gpos_mark2base" | "gpos_mark2ligature" | "gpos_mark2mark" => {
                let c = self.list.char(start);
                let Some(Match::Mark(class, ma)) = step_match(step, c) else {
                    return (head, Some(start), false, 0, None);
                };
                let ok = match typ {
                    "gpos_mark2base" => self.mark2base(start, lookup, 0, class, ma, rlmode, true),
                    "gpos_mark2ligature" => self.mark2ligature(start, lookup, 0, class, ma, rlmode),
                    _ => self.mark2mark(start, lookup, 0, class, ma, rlmode),
                };
                (head, Some(start), ok, 0, None)
            }
            "gpos_cursive" => {
                let c = self.list.char(start);
                let Some(Match::Cursive(exit)) = step_match(step, c) else {
                    return (head, Some(start), false, 0, None);
                };
                let ok = self.cursive(start, lookup, 0, exit, rlmode);
                (head, Some(start), ok, 0, None)
            }
            _ if typ.contains("context") => {
                // chained_contextchain (font-ots.lua:2956-2979)
                let c = self.list.char(start);
                for (i, s) in lookup.steps.iter().enumerate() {
                    if let Some(Match::Rules(l)) = step_match(s, c) {
                        let (h, st, d) =
                            self.contextchain(head, start, dataset, lookup, i, l, rlmode, skip);
                        if d || lookup.steps.len() == 1 {
                            return (h, st, d, 0, None);
                        }
                    }
                }
                (head, Some(start), false, 0, None)
            }
            _ => (head, Some(start), false, 0, None),
        }
    }

    /// `chaindisk` (font-ots.lua:2106-2433).
    #[allow(clippy::too_many_arguments)]
    fn chaindisk(
        &mut self,
        head: Option<Ix>,
        start: Option<Ix>,
        dataset: &Dataset,
        rlmode: i32,
        skip: &Option<Skip>,
        ck: &RuleData,
    ) -> R {
        let Some(st0) = start else {
            return (head, start, false);
        };
        let mut head = head;
        let mut start = start;
        let mut startishead = start == head;
        let s = ck.sequence.len();
        let f = ck.start;
        let l = ck.stop;
        let mut sweepnode = self.sweepnode;
        let mut sweeptype = self.sweeptype;
        let mut sweepoverflow = false;
        let checkdisc = self.prev(head);
        let mut keepdisc = sweepnode.is_none();
        let mut lookaheaddisc: Option<Ix> = None;
        let mut backtrackdisc: Option<Ix> = None;
        let mut current = start;
        let mut last = start;
        let prev = self.prev(start);
        let mut hasglue = false;
        let mut useddisc: Option<Ix> = None;
        let usedstart = start;

        let mut i = f;
        while i <= l {
            let Some(cur) = current else {
                break;
            };
            match self.list.nodes[cur].kind.clone() {
                Kind::Glyph { .. } => {
                    i += 1;
                    last = current;
                    current = self.next(current);
                }
                Kind::Glue { .. } => {
                    i += 1;
                    last = current;
                    current = self.next(current);
                    hasglue = true;
                }
                Kind::Disc { replace, .. } => {
                    if keepdisc {
                        keepdisc = false;
                        lookaheaddisc = current;
                        if replace.is_none() {
                            sweepoverflow = true;
                            sweepnode = current;
                            current = self.next(current);
                        } else {
                            let mut r = replace;
                            let mut rl = r;
                            while let Some(x) = r {
                                if i > l {
                                    break;
                                }
                                if matches!(self.list.nodes[x].kind, Kind::Glyph { .. }) {
                                    i += 1;
                                }
                                rl = Some(x);
                                r = self.next(Some(x));
                            }
                            // `current = getnext(replace)` with replace past the
                            // counted glyphs: the node after the last one read
                            let _ = rl;
                            current = r.and_then(|x| self.next(Some(x)));
                        }
                        last = current;
                    } else {
                        let (h, c) = self.flattendisk(head, cur);
                        head = h;
                        current = c;
                    }
                }
                _ => {
                    last = current;
                    current = self.next(current);
                }
            }
            if current.is_some() {
            } else if sweepoverflow {
                break;
            } else if sweeptype == Some(Sweep::Post) || sweeptype == Some(Sweep::Replace) {
                current = self.next(sweepnode);
                if current.is_some() {
                    sweeptype = None;
                    sweepoverflow = true;
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        if sweepoverflow {
            let prev = current.and_then(|c| self.list.nodes[c].prev);
            if current.is_none() || prev != sweepnode {
                let sn = sweepnode.expect("sweeping");
                let h = self.next(Some(sn));
                let tail;
                match prev {
                    Some(_) => {
                        tail = prev;
                        self.list.set_prev(current, Some(sn));
                    }
                    None => tail = self.list.tail(h),
                }
                self.list.set_next(Some(sn), current);
                self.list.set_prev(h, None);
                self.list.set_next(tail, None);
                self.appenddisc(sn, h);
            }
        }

        if l < s {
            let mut i = l;
            let t = sweeptype == Some(Sweep::Post) || sweeptype == Some(Sweep::Replace);
            while let Some(cur) = current {
                if i >= s {
                    break;
                }
                match self.list.nodes[cur].kind.clone() {
                    Kind::Glyph { .. } => {
                        i += 1;
                        current = self.next(current);
                    }
                    Kind::Glue { .. } => {
                        i += 1;
                        current = self.next(current);
                        hasglue = true;
                    }
                    Kind::Disc { replace, .. } => {
                        if keepdisc {
                            keepdisc = false;
                            if self.notmatchpre.contains(&cur)
                                != self.notmatchreplace.contains(&cur)
                            {
                                lookaheaddisc = current;
                            }
                            let mut r = replace;
                            while let Some(x) = r {
                                if i >= s {
                                    break;
                                }
                                if matches!(self.list.nodes[x].kind, Kind::Glyph { .. }) {
                                    i += 1;
                                }
                                r = self.next(Some(x));
                            }
                            current = self.next(current);
                        } else if self.notmatchpre.contains(&cur)
                            != self.notmatchreplace.contains(&cur)
                        {
                            let (h, c) = self.flattendisk(head, cur);
                            head = h;
                            current = c;
                        } else {
                            current = self.next(current);
                        }
                    }
                    _ => current = self.next(current),
                }
                if current.is_none() && t {
                    current = self.next(sweepnode);
                    if current.is_some() {
                        sweeptype = None;
                    }
                }
            }
        }

        if f > 1 {
            let mut current = prev;
            let mut i = f;
            let t = sweeptype == Some(Sweep::Pre) || sweeptype == Some(Sweep::Replace);
            if current.is_none() && t && current == checkdisc {
                current = self.prev(sweepnode);
            }
            while let Some(cur) = current {
                if i <= 1 {
                    break;
                }
                match self.list.nodes[cur].kind.clone() {
                    Kind::Glyph { .. } => i -= 1,
                    Kind::Glue { .. } => {
                        i -= 1;
                        hasglue = true;
                    }
                    Kind::Disc { replace, .. } => {
                        if keepdisc {
                            keepdisc = false;
                            if self.notmatchpost.contains(&cur)
                                != self.notmatchreplace.contains(&cur)
                            {
                                backtrackdisc = Some(cur);
                            }
                            let mut r = replace;
                            while let Some(x) = r {
                                if i <= 1 {
                                    break;
                                }
                                if matches!(self.list.nodes[x].kind, Kind::Glyph { .. }) {
                                    i -= 1;
                                }
                                r = self.next(Some(x));
                            }
                        } else if self.notmatchpost.contains(&cur)
                            != self.notmatchreplace.contains(&cur)
                        {
                            let (h, c) = self.flattendisk(head, cur);
                            head = h;
                            current = c;
                        }
                    }
                    _ => {}
                }
                current = self.prev(current);
                if t && current == checkdisc {
                    current = self.prev(sweepnode);
                }
            }
        }

        let mut done = false;
        let new_start;
        if let Some(ld) = lookaheaddisc {
            let mut cf = start;
            let cl = self.prev(Some(ld));
            let mut cprev = self.prev(start);
            let mut insertedmarks = 0;
            while cprev.is_some() {
                match self.ischar(cf) {
                    Ok(Some(c)) if self.font.is_mark(c) => {
                        insertedmarks += 1;
                        cf = cprev;
                        startishead = cf == head;
                        cprev = self.prev(cprev);
                    }
                    _ => break,
                }
            }
            self.list.link(cprev, Some(ld));
            if cprev.is_none() {
                self.list.set_prev(Some(ld), None);
            }
            self.list.set_prev(cf, None);
            self.list.set_next(cl, None);
            if startishead {
                head = Some(ld);
            }
            let (pre, post, replace) = self.list.disc(ld);
            let new = self.list.copy_list(cf, None);
            let mut cnew = new;
            if pre.is_some() {
                let t = self.list.tail(cf);
                self.list.link(t, pre);
            }
            if replace.is_some() {
                let t = self.list.tail(new);
                self.list.link(t, replace);
            }
            for _ in 0..insertedmarks {
                cnew = self.next(cnew);
            }
            let mut cl = start;
            let mut clast = cnew;
            for _ in f..=l {
                cl = self.next(cl);
                clast = self.next(clast);
            }
            let mut cf2 = cf;
            let mut new2 = new;
            if !self.notmatchpre.contains(&ld) {
                // (the start this returns is replaced below, as in the Lua)
                let (h, _, ok) = self.chainrun(cf, start, cl, dataset, rlmode, skip, ck);
                cf2 = h;
                if ok {
                    done = true;
                }
            }
            if !self.notmatchreplace.contains(&ld) {
                let (h, _c, ok) = self.chainrun(new, cnew, clast, dataset, rlmode, skip, ck);
                new2 = h;
                if ok {
                    done = true;
                }
            }
            if hasglue {
                self.setdiscchecked(ld, cf2, post, new2);
            } else {
                self.list.set_disc(ld, cf2, post, new2);
            }
            new_start = self.prev(Some(ld));
            useddisc = Some(ld);
            if let Some(c) = cf2 {
                let v = self.next(clast);
                self.sweephead.insert(c, v);
            }
            if let Some(n) = new2 {
                let v = self.next(cl);
                self.sweephead.insert(n, v);
            }
        } else if let Some(bd) = backtrackdisc {
            let cf = self.next(Some(bd));
            let mut cl = start;
            let mut cnext = self.next(start);
            let mut insertedmarks = 0;
            while let Some(cn) = cnext {
                match self.ischar(Some(cn)) {
                    Ok(Some(c)) if self.font.is_mark(c) => {
                        insertedmarks += 1;
                        cl = cnext;
                        cnext = self.next(Some(cn));
                    }
                    _ => break,
                }
            }
            self.list.link(Some(bd), cnext);
            self.list.set_prev(cf, None);
            self.list.set_next(cl, None);
            let (pre, post, replace, _pt, posttail, replacetail) = self.list.disc_tails(bd);
            let new = self.list.copy_list(cf, None);
            let mut cnew = self.list.tail(new);
            for _ in 0..insertedmarks {
                cnew = self.prev(cnew);
            }
            let mut clast = cnew;
            for _ in f..=l {
                clast = self.next(clast);
            }
            let mut cf2 = cf;
            let mut new2 = new;
            if !self.notmatchpost.contains(&bd) {
                let (h, _, ok) = self.chainrun(cf, start, last, dataset, rlmode, skip, ck);
                cf2 = h;
                if ok {
                    done = true;
                }
            }
            if !self.notmatchreplace.contains(&bd) {
                let (h, _c, ok) = self.chainrun(new, cnew, clast, dataset, rlmode, skip, ck);
                new2 = h;
                if ok {
                    done = true;
                }
            }
            let post = match post {
                Some(_) => {
                    self.list.link(posttail, cf2);
                    post
                }
                None => cf2,
            };
            let replace = match replace {
                Some(_) => {
                    self.list.link(replacetail, new2);
                    replace
                }
                None => new2,
            };
            if hasglue {
                self.setdiscchecked(bd, pre, post, replace);
            } else {
                self.list.set_disc(bd, pre, post, replace);
            }
            new_start = self.prev(Some(bd));
            useddisc = Some(bd);
            if let Some(p) = post {
                let v = self.next(clast);
                self.sweephead.insert(p, v);
            }
            if let Some(r) = replace {
                let v = self.next(last);
                self.sweephead.insert(r, v);
            }
        } else {
            let (h, s2, ok) = self.chainrun(head, start, last, dataset, rlmode, skip, ck);
            head = h;
            new_start = s2;
            if ok {
                done = true;
            }
        }
        start = new_start;
        if useddisc.is_some() && start != usedstart {
            start = self.next(start);
        }
        let _ = st0;
        (head, start, done)
    }

    /// `checked` + `setdiscchecked` (font-ots.lua:1936-1967): glue inside a
    /// discretionary becomes a user kern of its width.
    fn setdiscchecked(&mut self, d: Ix, pre: Option<Ix>, post: Option<Ix>, replace: Option<Ix>) {
        let pre = self.checked(pre);
        let post = self.checked(post);
        let replace = self.checked(replace);
        self.list.set_disc(d, pre, post, replace);
    }

    fn checked(&mut self, head: Option<Ix>) -> Option<Ix> {
        let mut head = head;
        let mut current = head;
        while let Some(cur) = current {
            if let Kind::Glue { width, .. } = self.list.nodes[cur].kind {
                let kern = self.list.alloc(Kind::Kern {
                    subtype: 1,
                    kern: width,
                });
                if head == Some(cur) {
                    let next = self.next(Some(cur));
                    if next.is_some() {
                        self.list.link(Some(kern), next);
                    }
                    head = Some(kern);
                    current = next;
                } else {
                    let (prev, next) = (self.list.nodes[cur].prev, self.list.nodes[cur].next);
                    self.list.link3(prev, Some(kern), next);
                    current = next;
                }
            } else {
                current = self.next(Some(cur));
            }
        }
        head
    }
}

/// `fonts.analyzers.features`: lookups that only apply where the script
/// analyzer has set a state (`getstate(start, attribute)`).
const AUTOFEATURES: [&str; 12] = [
    "init", "medi", "med2", "fina", "fin2", "fin3", "isol", "rphf", "half", "pref", "blwf", "pstf",
];

/// The scripts whose node-mode processing runs an analyzer (font-ota.lua,
/// font-osd.lua) before the lookups; not ported, so refused rather than
/// processed without it.
pub const ANALYZED_SCRIPTS: [&str; 15] = [
    "arab", "syrc", "mong", "nko ", "dev2", "deva", "bng2", "beng", "gjr2", "gujr", "gur2", "guru",
    "knd2", "mlm2", "tml2",
];

/// `hasspacekerns` (font-ots.lua:4371-4431): whether a GPOS kern lookup kerns
/// the space.
fn hasspacekerns(face: &Face) -> bool {
    if !face.layout.features.contains_key("gpos") {
        return false;
    }
    for seq in &face.layout.sequences {
        if !seq.features.contains_key("kern") {
            continue;
        }
        if seq.typ != "gpos_pair" && seq.typ != "gpos_single" {
            continue;
        }
        for step in &seq.steps {
            if step.rules.is_some() || seq.typ == "gpos_single" {
                continue;
            }
            match (&step.format, &step.coverage) {
                (Some("move") | Some("kern"), Coverage::Move2(c) | Coverage::Kern(c)) => {
                    if c.contains_key(&32) || c.values().any(|v| v.contains_key(&32)) {
                        return true;
                    }
                }
                (Some("pair"), Coverage::Pair(c)) => {
                    let first = |v: &PairValue| matches!(v.first, Some(Pos::V(_)));
                    if c.get(&32).is_some_and(|k| k.values().any(first)) {
                        return true;
                    }
                    if c.values().any(|v| v.get(&32).is_some_and(first)) {
                        return true;
                    }
                }
                _ => {}
            }
        }
    }
    false
}

/// A `kern` feature table: script to languages.
type ScriptLangs = BTreeMap<String, BTreeSet<String>>;

/// What `spaceinitializer` (font-ots.lua:4440-4558) leaves behind: the
/// space kerns, where the trigger pseudo-lookup was inserted (before the
/// sequence with this 1-based index, `insert(sequences, last, ...)`), the
/// `kern` table it is enabled under, and the kern tables of real lookups it
/// changed on the way.
///
/// The Lua's `feat` IS the first kern lookup's `features.kern` table; a later
/// lookup's script missing from it is added by reference (`feat[script] =
/// languages`), and a script already there gets the later lookup's languages
/// added (`f[l] = true`) to whichever lookup's table that entry is. So the
/// kern lookups' own script/language tables grow, and `otf.dataset` reads
/// them grown.
struct SpaceInit {
    position: usize,
    kerns: otj::SpaceKerns,
    feat: ScriptLangs,
    changed: BTreeMap<usize, ScriptLangs>,
}

fn spaceinitializer(face: &Face) -> Option<SpaceInit> {
    if !hasspacekerns(face) || !face.layout.features.contains_key("gpos") {
        return None;
    }
    let mut left = BTreeMap::new();
    let mut right = BTreeMap::new();
    let mut last = 0;
    // the kern tables as they become, and for each script of `feat`, the
    // lookup whose languages table that entry is
    let mut tables: BTreeMap<usize, ScriptLangs> = BTreeMap::new();
    let mut first: Option<usize> = None;
    let mut owner: BTreeMap<String, usize> = BTreeMap::new();
    for (i, seq) in face.layout.sequences.iter().enumerate() {
        let Some(kern) = seq.features.get("kern") else {
            continue;
        };
        if seq.typ != "gpos_pair" && seq.typ != "gpos_single" {
            continue;
        }
        match first {
            Some(_) => {
                for (script, languages) in kern {
                    match owner.get(script) {
                        Some(&o) => {
                            let t = tables.entry(o).or_insert_with(|| {
                                face.layout.sequences[o].features["kern"].clone()
                            });
                            t.entry(script.clone())
                                .or_default()
                                .extend(languages.iter().cloned());
                        }
                        None => {
                            owner.insert(script.clone(), i);
                        }
                    }
                }
            }
            None => {
                first = Some(i);
                for script in kern.keys() {
                    owner.insert(script.clone(), i);
                }
            }
        }
        for step in &seq.steps {
            if step.rules.is_some() || seq.typ == "gpos_single" {
                continue;
            }
            match (&step.format, &step.coverage) {
                (Some("move") | Some("kern"), Coverage::Move2(c) | Coverage::Kern(c)) => {
                    if let Some(k) = c.get(&32) {
                        right.extend(k.iter().map(|(a, b)| (*a, *b)));
                    }
                    for (k, v) in c {
                        if let Some(x) = v.get(&32) {
                            left.insert(*k, *x);
                        }
                    }
                }
                (Some("pair"), Coverage::Pair(c)) => {
                    if let Some(k) = c.get(&32) {
                        for (a, v) in k {
                            if let Some(Pos::V(one)) = &v.first {
                                right.insert(*a, one[2]);
                            }
                        }
                    }
                    for (k, v) in c {
                        if let Some(Some(Pos::V(one))) = v.get(&32).map(|p| &p.first) {
                            left.insert(*k, one[2]);
                        }
                    }
                }
                _ => {}
            }
        }
        last = i + 1;
    }
    if left.is_empty() && right.is_empty() {
        return None;
    }
    let table_of = |s: usize| -> ScriptLangs {
        tables
            .get(&s)
            .cloned()
            .unwrap_or_else(|| face.layout.sequences[s].features["kern"].clone())
    };
    // feat: the first lookup's table, with the scripts it gained by reference
    let feat: ScriptLangs = match first {
        Some(_) => owner
            .iter()
            .map(|(script, &o)| {
                (
                    script.clone(),
                    table_of(o).get(script).cloned().unwrap_or_default(),
                )
            })
            .collect(),
        None => BTreeMap::from([("dflt".to_string(), BTreeSet::from(["dflt".to_string()]))]),
    };
    let mut changed = tables;
    if let Some(f) = first {
        changed.insert(f, feat.clone());
    }
    Some(SpaceInit {
        position: last,
        kerns: otj::SpaceKerns {
            left: (!left.is_empty()).then_some(left),
            right: (!right.is_empty()).then_some(right),
        },
        feat,
        changed,
    })
}

/// The lookups `otf.dataset` enables for the font's script and language, in
/// sequence order (font-ots.lua:3044-3112), the space-kern trigger among
/// them where `spaceinitializer` put it, each with the analyzer feature it
/// depends on, if any.
pub struct Plan {
    pub datasets: Vec<(Dataset, Option<&'static str>)>,
    pub spacekerns: Option<otj::SpaceKerns>,
}

pub fn plan(font: &Font) -> Plan {
    let face = font.face;
    let enabled = &font.features;
    let space = match enabled.contains_key("spacekern") {
        true => spaceinitializer(face),
        false => None,
    };
    let languages_of = |scripts: &ScriptLangs| -> bool {
        let languages = scripts.get(&font.script).or_else(|| scripts.get("*"));
        languages.is_some_and(|l| l.contains(&font.language) || l.contains("*"))
    };
    let mut out = Vec::new();
    for (s, seq) in face.layout.sequences.iter().enumerate() {
        if let Some(sp) = &space {
            if sp.position == s + 1 {
                if let Some(value) = enabled.get("kern") {
                    if languages_of(&sp.feat) {
                        out.push((
                            Dataset {
                                value: value.clone(),
                                sequence: usize::MAX,
                                kind: "kern".into(),
                                space_trigger: true,
                            },
                            None,
                        ));
                    }
                }
            }
        }
        for kind in &seq.order {
            let Some(valid) = enabled.get(kind) else {
                continue;
            };
            let Some(scripts) = seq.features.get(kind) else {
                continue;
            };
            let scripts = match (&space, kind.as_str()) {
                (Some(sp), "kern") => sp.changed.get(&s).unwrap_or(scripts),
                _ => scripts,
            };
            if languages_of(scripts) {
                let auto = AUTOFEATURES.iter().find(|f| **f == kind).copied();
                out.push((
                    Dataset {
                        value: valid.clone(),
                        sequence: s,
                        kind: kind.clone(),
                        space_trigger: false,
                    },
                    auto,
                ));
                break;
            }
        }
    }
    Plan {
        datasets: out,
        spacekerns: space.map(|s| s.kerns),
    }
}

/// `txtdirstate` (font-ots.lua:3753-3777).
fn txtdirstate(
    list: &List,
    start: Ix,
    stack: &mut Vec<u8>,
    top: usize,
    rlparmode: i32,
) -> (usize, i32) {
    let Kind::Dir { dir, cancel } = list.nodes[start].kind else {
        return (top, rlparmode);
    };
    if cancel {
        if top == 1 {
            return (0, rlparmode);
        }
        let top = top - 1;
        return (
            top,
            if stack.get(top - 1) == Some(&1) {
                -1
            } else {
                1
            },
        );
    }
    stack.truncate(top);
    stack.push(dir);
    (top + 1, if dir == 1 { -1 } else { 1 })
}

/// `endofmath`: the math node closing the one at `start`.
fn endofmath(list: &List, start: Ix) -> Option<Ix> {
    let mut n = list.nodes[start].next;
    while let Some(i) = n {
        if let Kind::Math { subtype: 1 } = list.nodes[i].kind {
            return Some(i);
        }
        n = list.nodes[i].next;
    }
    None
}

impl State<'_> {
    /// `otf.featuresprocessor` (font-ots.lua:3835-4124) for this state's font.
    pub fn featuresprocessor(
        &mut self,
        head: Option<Ix>,
        direction: i32,
        plan: &Plan,
    ) -> Option<Ix> {
        let mut head = head;
        let initialrl = match head.map(|h| &self.list.nodes[h].kind) {
            Some(Kind::Par { dir }) => {
                if *dir == 1 {
                    -1
                } else {
                    1
                }
            }
            _ if direction == 1 => -1,
            _ => 0,
        };
        let face: &Face = self.font.face;
        self.sweephead.clear();
        for (dataset, attribute) in &plan.datasets {
            if dataset.space_trigger {
                // handlers.trigger_space_kerns (font-ots.lua:4356-4364)
                if self.font.features.contains_key("spacekern")
                    && self.font.features.contains_key("kern")
                {
                    if let Some(k) = &plan.spacekerns {
                        self.triggers.insert(self.font.id, k.clone());
                    }
                }
                continue;
            }
            let seq = &face.layout.sequences[dataset.sequence];
            let font: &Font = self.font;
            let skip = &Skip::of(
                font,
                seq,
                face.expanded
                    .markclass
                    .get(dataset.sequence)
                    .and_then(|m| m.as_ref()),
            );
            let typ = seq.typ;
            let gpossing = typ == "gpos_single" || typ == "gpos_pair";
            let forcetestrun = typ == "gsub_ligature";
            let rlparmode = initialrl;
            let mut topstack = 0;
            let mut dirstack: Vec<u8> = Vec::new();
            if typ == "gsub_reversecontextchain" {
                let mut start = self.list.tail(head);
                while let Some(st) = start {
                    if let Ok(Some(c)) = self.ischar(Some(st)) {
                        if attribute.is_none() {
                            for (i, step) in seq.steps.iter().enumerate() {
                                if let Some(m) = step_match(step, c) {
                                    let (h, s, ok, _) =
                                        self.handler(head, st, dataset, seq, &m, 0, skip, i, None);
                                    head = h;
                                    start = s;
                                    if ok {
                                        break;
                                    }
                                }
                            }
                        }
                        start = self.prev(start);
                    } else {
                        start = self.prev(Some(st));
                    }
                }
                continue;
            }
            let mut start = head;
            let mut rlmode = initialrl;
            while let Some(st) = start {
                match self.ischar(Some(st)) {
                    Ok(Some(c)) => {
                        let first = seq.steps.iter().position(|s| step_match(s, c).is_some());
                        let Some(first) = first else {
                            start = self.next(Some(st));
                            continue;
                        };
                        // an auto feature needs the analyzer's state, which no
                        // node has here
                        if attribute.is_some() {
                            start = self.next(Some(st));
                            continue;
                        }
                        let mut df = false;
                        let mut cur = Some(st);
                        for (i, step) in seq.steps.iter().enumerate().skip(first) {
                            let Some(m) = step_match(step, c) else {
                                continue;
                            };
                            let (h, s, ok, d) = self.handler(
                                head,
                                cur.unwrap(),
                                dataset,
                                seq,
                                &m,
                                rlmode,
                                skip,
                                i,
                                None,
                            );
                            head = h;
                            cur = s;
                            df = d;
                            if df || ok || cur.is_none() || seq.steps.len() == 1 {
                                break;
                            }
                        }
                        start = if df { cur } else { self.next(cur) };
                    }
                    Ok(None) => start = self.next(Some(st)),
                    Err(()) => match self.list.nodes[st].kind.clone() {
                        Kind::Glue { .. } => start = self.next(Some(st)),
                        Kind::Disc { .. } => {
                            if self.discs.is_none() || self.uses_font(st) {
                                start = if gpossing {
                                    self.kernrun(st, dataset, seq, rlmode, skip)
                                } else if forcetestrun {
                                    self.testrun(st, dataset, seq, rlmode, skip)
                                } else {
                                    self.comprun(st, dataset, seq, rlmode, skip)
                                };
                            } else {
                                start = self.next(Some(st));
                            }
                        }
                        Kind::Math { .. } => start = self.next(endofmath(self.list, st)),
                        Kind::Dir { .. } => {
                            (topstack, rlmode) =
                                txtdirstate(self.list, st, &mut dirstack, topstack, rlparmode);
                            start = self.next(Some(st));
                        }
                        _ => start = self.next(Some(st)),
                    },
                }
            }
        }
        head
    }

    /// `c_run_single` / `c_run_multiple` (font-ots.lua:3416-3461, 3578-3633).
    fn c_run(
        &mut self,
        head: Ix,
        dataset: &Dataset,
        seq: &Sequence,
        rlmode: i32,
        skip: &Option<Skip>,
    ) -> (Option<Ix>, bool) {
        let mut done = false;
        let mut head = Some(head);
        // `sweephead[head]` is a node to resume at, or false once used
        let sweep = self.sweephead.get(&head.unwrap()).copied().flatten();
        let mut start = match sweep {
            Some(s) => {
                self.sweephead.insert(head.unwrap(), None);
                Some(s)
            }
            None => head,
        };
        let swept = sweep.is_some();
        while let Some(st) = start {
            match self.ischar(Some(st)) {
                Ok(Some(c)) => {
                    let mut cur = Some(st);
                    for (i, step) in seq.steps.iter().enumerate() {
                        let Some(m) = step_match(step, c) else {
                            continue;
                        };
                        let Some(at) = cur else {
                            break;
                        };
                        let (h, s, ok, _) =
                            self.handler(head, at, dataset, seq, &m, rlmode, skip, i, None);
                        head = h;
                        cur = s;
                        if ok {
                            done = true;
                            break;
                        }
                        if cur.is_none() || seq.steps.len() == 1 {
                            break;
                        }
                    }
                    start = cur.and_then(|c| self.next(Some(c)));
                }
                Ok(None) => return (head, done),
                Err(()) if swept => return (head, done),
                Err(()) => start = self.next(Some(st)),
            }
        }
        (head, done)
    }

    /// `t_run_single` / `t_run_multiple` (font-ots.lua:3465-3552,
    /// 3635-3719). The multiple-step variant tests the following glyphs
    /// without a font (`ischar(s)`), as the Lua does.
    fn t_run(&self, start: Option<Ix>, stop: Option<Ix>, seq: &Sequence) -> usize {
        let multiple = seq.steps.len() > 1;
        let mut start = start;
        while start != stop {
            let Some(st) = start else {
                break;
            };
            let Ok(Some(c)) = self.ischar(Some(st)) else {
                break;
            };
            let startnext = self.next(Some(st));
            let mut lastd = None;
            {
                for step in &seq.steps {
                    let Some(m) = step_match(step, c) else {
                        continue;
                    };
                    let mut lookupmatch = match m {
                        Match::Lig(t) => Some(t),
                        _ => None,
                    };
                    let mut s = startnext;
                    let mut ss: Option<Ix> = None;
                    let mut sstop = s == stop;
                    while self.list.is_disc(s) {
                        ss = self.next(s);
                        s = self.list.disc(s.unwrap()).2;
                        if s.is_none() {
                            s = ss;
                            ss = None;
                        }
                    }
                    let mut l: Option<&LigTree> = None;
                    let mut d = 0;
                    while s.is_some() {
                        let ch = if multiple {
                            self.list.ischar(s, None).ok().flatten()
                        } else {
                            self.char_of(s)
                        };
                        let Some(ch) = ch else {
                            break;
                        };
                        let Some(lg) = lookupmatch.and_then(|t| t.next.get(&ch)) else {
                            break;
                        };
                        if sstop {
                            d = 1;
                        } else if d > 0 {
                            d += 1;
                        }
                        l = Some(lg);
                        s = self.next(s);
                        sstop = s == stop;
                        if s.is_none() {
                            s = ss;
                            ss = None;
                        }
                        while self.list.is_disc(s) {
                            ss = self.next(s);
                            s = self.list.disc(s.unwrap()).2;
                            if s.is_none() {
                                s = ss;
                                ss = None;
                            }
                        }
                        lookupmatch = Some(lg);
                    }
                    if l.is_some_and(|l| l.ligature.is_some()) {
                        lastd = Some(d);
                    }
                    if !multiple {
                        break;
                    }
                }
            }
            if let Some(d) = lastd {
                return d;
            }
            start = startnext;
        }
        0
    }

    /// `k_run_single` / `k_run_multiple` (font-ots.lua:3554-3576,
    /// 3721-3747).
    #[allow(clippy::too_many_arguments)]
    fn k_run(
        &mut self,
        sub: Option<Ix>,
        injection: Slot,
        last: Option<Ix>,
        dataset: &Dataset,
        seq: &Sequence,
        rlmode: i32,
        skip: &Option<Skip>,
    ) -> bool {
        let multiple = seq.steps.len() > 1;
        let mut n = sub;
        while let Some(i) = n {
            if Some(i) == last {
                break;
            }
            let ch = if multiple {
                self.list.ischar(Some(i), None).ok().flatten()
            } else {
                self.char_of(Some(i))
            };
            if let Some(c) = ch {
                for (si, step) in seq.steps.iter().enumerate() {
                    let Some(m) = step_match(step, c) else {
                        continue;
                    };
                    let (_h, _d, ok, _) =
                        self.handler(sub, i, dataset, seq, &m, rlmode, skip, si, Some(injection));
                    if ok {
                        return true;
                    }
                    if !multiple {
                        break;
                    }
                }
            }
            n = self.list.nodes[i].next;
        }
        false
    }

    /// `kernrun` (font-ots.lua:3128-3226).
    fn kernrun(
        &mut self,
        disc: Ix,
        dataset: &Dataset,
        seq: &Sequence,
        rlmode: i32,
        skip: &Option<Skip>,
    ) -> Option<Ix> {
        let (prev, next) = (self.list.nodes[disc].prev, self.list.nodes[disc].next);
        let nextstart = next;
        let (pre, post, replace, _pt, posttail, replacetail) = self.list.disc_tails(disc);
        let mut prevmarks = prev;
        while let Some(pm) = prevmarks {
            match self.char_of(Some(pm)) {
                Some(c) if self.font.is_mark(c) => prevmarks = self.prev(Some(pm)),
                _ => break,
            }
        }
        let prev = prev.filter(|p| self.char_of(Some(*p)).is_some());
        let next = next.filter(|n| self.char_of(Some(*n)).is_some());
        if pre.is_some() {
            self.k_run(pre, Slot::Injections, None, dataset, seq, rlmode, skip);
            if prev.is_some() {
                self.list.link(prev, pre);
                self.k_run(prevmarks, Slot::Pre, pre, dataset, seq, rlmode, skip);
                self.list.set_prev(pre, None);
                self.list.link(prev, Some(disc));
            }
        }
        if post.is_some() {
            self.k_run(post, Slot::Injections, None, dataset, seq, rlmode, skip);
            if next.is_some() {
                self.list.link(posttail, next);
                self.k_run(posttail, Slot::Post, next, dataset, seq, rlmode, skip);
                self.list.set_next(posttail, None);
                self.list.link(Some(disc), next);
            }
        }
        if replace.is_some() {
            self.k_run(replace, Slot::Injections, None, dataset, seq, rlmode, skip);
            if prev.is_some() {
                self.list.link(prev, replace);
                self.k_run(
                    prevmarks,
                    Slot::Replace,
                    replace,
                    dataset,
                    seq,
                    rlmode,
                    skip,
                );
                self.list.set_prev(replace, None);
                self.list.link(prev, Some(disc));
            }
            if next.is_some() {
                self.list.link(replacetail, next);
                self.k_run(replacetail, Slot::Replace, next, dataset, seq, rlmode, skip);
                self.list.set_next(replacetail, None);
                self.list.link(Some(disc), next);
            }
        } else if prev.is_some() && next.is_some() {
            self.list.link(prev, next);
            self.k_run(prevmarks, Slot::Empty, next, dataset, seq, rlmode, skip);
            self.list.link3(prev, Some(disc), next);
        }
        nextstart
    }

    /// The three sub-runs `comprun` and `testrun` end with.
    fn sweep(
        &mut self,
        disc: Ix,
        pre: Option<Ix>,
        post: Option<Ix>,
        replace: Option<Ix>,
        dataset: &Dataset,
        seq: &Sequence,
        rlmode: i32,
        skip: &Option<Skip>,
    ) {
        let (mut pre, mut post, mut replace) = (pre, post, replace);
        let mut renewed = false;
        for (which, t) in [(0, Sweep::Pre), (1, Sweep::Post), (2, Sweep::Replace)] {
            let l = match which {
                0 => pre,
                1 => post,
                _ => replace,
            };
            let Some(h) = l else {
                continue;
            };
            self.sweepnode = Some(disc);
            self.sweeptype = Some(t);
            let (new, done) = self.c_run(h, dataset, seq, rlmode, skip);
            if done {
                match which {
                    0 => pre = new,
                    1 => post = new,
                    _ => replace = new,
                }
                renewed = true;
            }
        }
        self.sweepnode = None;
        self.sweeptype = None;
        if renewed {
            self.list.set_disc(disc, pre, post, replace);
        }
    }

    /// `comprun` (font-ots.lua:3231-3280).
    fn comprun(
        &mut self,
        disc: Ix,
        dataset: &Dataset,
        seq: &Sequence,
        rlmode: i32,
        skip: &Option<Skip>,
    ) -> Option<Ix> {
        let (pre, post, replace) = self.list.disc(disc);
        self.sweep(disc, pre, post, replace, dataset, seq, rlmode, skip);
        self.next(Some(disc))
    }

    /// `testrun` (font-ots.lua:3291-3409).
    fn testrun(
        &mut self,
        disc: Ix,
        dataset: &Dataset,
        seq: &Sequence,
        rlmode: i32,
        skip: &Option<Skip>,
    ) -> Option<Ix> {
        let next = self.list.nodes[disc].next;
        next?;
        let (pre, mut post, mut replace, _pt, posttail, replacetail) = self.list.disc_tails(disc);
        let mut next = next;
        if post.is_some() || replace.is_some() {
            match post {
                Some(_) => self.list.link(posttail, next),
                None => post = next,
            }
            match replace {
                Some(_) => self.list.link(replacetail, next),
                None => replace = next,
            }
            let d_post = self.t_run(post, next, seq);
            let d_replace = self.t_run(replace, next, seq);
            if d_post > 0 || d_replace > 0 {
                let d = d_replace.max(d_post);
                let mut head = self.next(Some(disc));
                let mut tail = head;
                for _ in 2..=d {
                    let nx = self.next(tail);
                    match nx.map(|n| &self.list.nodes[n].kind) {
                        Some(Kind::Disc { .. }) => {
                            let (h, t) = self.flattendisk(head, nx.unwrap());
                            head = h;
                            tail = t;
                        }
                        Some(Kind::Glyph { .. }) => tail = nx,
                        _ => break,
                    }
                }
                next = self.next(tail);
                self.list.set_next(tail, None);
                self.list.set_prev(head, None);
                let new = self.list.copy_list(head, None);
                match posttail {
                    Some(_) => self.list.link(posttail, head),
                    None => post = head,
                }
                match replacetail {
                    Some(_) => self.list.link(replacetail, new),
                    None => replace = new,
                }
            } else {
                match posttail {
                    Some(_) => self.list.set_next(posttail, None),
                    None => post = None,
                }
                match replacetail {
                    Some(_) => self.list.set_next(replacetail, None),
                    None => replace = None,
                }
            }
            self.list.link(Some(disc), next);
            if next.is_none() {
                self.list.set_next(Some(disc), None);
            }
        }
        self.sweep(disc, pre, post, replace, dataset, seq, rlmode, skip);
        self.next(Some(disc))
    }
}

/// `nodes.simple_font_handler` (fontloader-fonts-gbn.lua:288-299) for the
/// node-mode fonts of a list: every font's features, then the injections.
/// `fonts` are the list's node-mode fonts by id; the Lua visits them in
/// table order, here ascending id.
pub fn process(
    list: &mut List,
    head: Option<Ix>,
    fonts: &BTreeMap<usize, Font>,
    direction: i32,
    widths: &dyn otj::Widths,
) -> Option<Ix> {
    let mut used: BTreeSet<usize> = BTreeSet::new();
    let mut n = head;
    while let Some(i) = n {
        if let Kind::Glyph { font, .. } = list.nodes[i].kind {
            if fonts.contains_key(&font) {
                used.insert(font);
            }
        }
        n = list.nodes[i].next;
    }
    let mut d = head;
    while let Some(i) = d {
        if let Kind::Disc { replace, .. } = list.nodes[i].kind {
            let mut r = replace;
            while let Some(x) = r {
                if let Kind::Glyph { font, .. } = list.nodes[x].kind {
                    if fonts.contains_key(&font) {
                        used.insert(font);
                    }
                }
                r = list.nodes[x].next;
            }
        }
        d = list.nodes[i].next;
    }
    let mut counts = Counts::default();
    let mut triggers = BTreeMap::new();
    let mut head = head;
    let nofused = used.len();
    for id in &used {
        let font = &fonts[id];
        // the `normalize` processor comes first (position 1)
        if let Some(how) = font.features.get("normalize") {
            let nfc = match how {
                FeatureValue::Text(t) => !matches!(t.as_str(), "nfd" | "nfkd"),
                _ => true,
            };
            if let (true, Some(t)) = (nfc, super::normalize::Tables::get()) {
                let allowed = |c: u32| font.face.descriptions.contains_key(&c);
                head = super::normalize::nodes_to_nfc(list, head, *id, &allowed, t);
            }
        }
        let plan = plan(font);
        let mut state = State::new(list, font, &mut counts, &mut triggers, nofused);
        head = state.featuresprocessor(head, direction, &plan);
    }
    let thresholds = |id: usize| -> (f64, f64) {
        (
            fonts.get(&id).map_or(0.0, |f| f.threshold),
            fonts.get(&id).map_or(0.0, |f| f.factor),
        )
    };
    otj::handler(list, head, &mut counts, &mut triggers, &thresholds, widths)
}
