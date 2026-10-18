//! `fontloader-font-otj.lua`: the positioning lookups record what they want
//! done in the node's properties (`setkern`, `setmove`, `setposition`,
//! `setmark`, `setcursive`), and `injections.handler` turns that into font
//! kerns, wider glue and glyph offsets once every font has been processed.
//!
//! The counters (`nofregistered*`) are the Lua's module-level state: they
//! decide which of the three injection passes runs, and they are reset by
//! the pass. Here they live in [`Counts`], owned by one processing run.

use std::collections::BTreeMap;

use super::nodes::{Field, Inj, Ix, Kind, List, Slot};

/// `nofregisteredkerns`, `...positions`, `...marks`, `...cursives`.
#[derive(Clone, Debug, Default)]
pub struct Counts {
    pub kerns: usize,
    pub positions: usize,
    pub marks: usize,
    pub cursives: usize,
}

/// The space kerns a font registered for this run (`triggers`): left kerns
/// by the character before a space, right by the one after.
#[derive(Clone, Debug, Default)]
pub struct SpaceKerns {
    /// `false` in the Lua when the font has none on that side.
    pub left: Option<BTreeMap<u32, f64>>,
    pub right: Option<BTreeMap<u32, f64>>,
}

/// `injections.reset(n)` (font-otj.lua:173-180).
pub fn reset(list: &mut List, n: Ix) {
    match &mut list.nodes[n].props {
        Some(Some(p)) => p.slots[0] = Field::False,
        _ => list.nodes[n].props = Some(None),
    }
}

/// `injections.copy(target, source)` (font-otj.lua:182-209).
pub fn copy(list: &mut List, target: Ix, source: Ix) {
    let source_has = list.has_props(source);
    if source_has {
        let si = list.slot(source, Slot::Injections).cloned();
        let target_has = list.has_props(target);
        match si {
            Some(si) => list.set_slot(target, Slot::Injections, Field::Value(si)),
            None if target_has => list.set_slot(target, Slot::Injections, Field::False),
            None => list.set_slot(target, Slot::Injections, Field::Value(Inj::default())),
        }
    } else {
        match &mut list.nodes[target].props {
            Some(Some(p)) => p.slots[0] = Field::False,
            _ => list.nodes[target].props = Some(None),
        }
    }
}

/// `injections.setligaindex(n, index)`.
pub fn set_ligaindex(list: &mut List, n: Ix, index: i64) {
    match list.slot_own(n, Slot::Injections) {
        Some(i) => i.ligaindex = Some(index),
        None => list.set_slot(
            n,
            Slot::Injections,
            Field::Value(Inj {
                ligaindex: Some(index),
                ..Inj::default()
            }),
        ),
    }
}

/// `injections.getligaindex(n, default)`.
pub fn get_ligaindex(list: &List, n: Ix, default: Option<i64>) -> Option<i64> {
    if !list.has_props(n) {
        return default;
    }
    match list.slot(n, Slot::Injections) {
        Some(i) => i.ligaindex.or(default),
        None => default,
    }
}

fn slot_name(injection: Option<Slot>) -> Slot {
    injection.unwrap_or(Slot::Injections)
}

/// `injections.setcursive` (font-otj.lua:242-304).
#[allow(clippy::too_many_arguments)]
pub fn set_cursive(
    list: &mut List,
    counts: &mut Counts,
    start: Ix,
    nxt: Ix,
    factor: f64,
    rlmode: i32,
    exit: [f64; 2],
    entry: [f64; 2],
    ws: f64,
    wn: f64,
) -> (f64, f64) {
    let mut dx = factor * (exit[0] - entry[0]);
    let dy = -factor * (exit[1] - entry[1]);
    counts.cursives += 1;
    if rlmode < 0 {
        dx = -(dx + wn);
    } else {
        dx -= ws;
    }
    if dx == 0.0 {
        dx = 0.0;
    }
    match list.slot_own(start, Slot::Injections) {
        Some(i) => i.cursiveanchor = true,
        None => list.set_slot(
            start,
            Slot::Injections,
            Field::Value(Inj {
                cursiveanchor: true,
                ..Inj::default()
            }),
        ),
    }
    match list.slot_own(nxt, Slot::Injections) {
        Some(i) => {
            i.cursivex = Some(dx);
            i.cursivey = Some(dy);
        }
        None => list.set_slot(
            nxt,
            Slot::Injections,
            Field::Value(Inj {
                cursivex: Some(dx),
                cursivey: Some(dy),
                ..Inj::default()
            }),
        ),
    }
    (dx, dy)
}

/// `injections.setposition` (font-otj.lua:308-368). `spec` is the value
/// record: x and y placement, x and y advance.
pub fn set_position(
    list: &mut List,
    counts: &mut Counts,
    current: Ix,
    factor: f64,
    rlmode: i32,
    spec: [f64; 4],
    injection: Option<Slot>,
) {
    let x = factor * spec[0];
    let y = factor * spec[1];
    let w = factor * spec[2];
    let h = factor * spec[3];
    if x == 0.0 && w == 0.0 && y == 0.0 && h == 0.0 {
        return;
    }
    let yoffset = y - h;
    let mut leftkern = x;
    let mut rightkern = w - x;
    if leftkern == 0.0 && rightkern == 0.0 && yoffset == 0.0 {
        return;
    }
    counts.positions += 1;
    if rlmode < 0 {
        std::mem::swap(&mut leftkern, &mut rightkern);
    }
    let slot = slot_name(injection);
    if list.has_props(current) {
        match list.slot_own(current, slot) {
            Some(i) => {
                if leftkern != 0.0 {
                    i.leftkern = Some(i.leftkern.unwrap_or(0.0) + leftkern);
                }
                if rightkern != 0.0 {
                    i.rightkern = Some(i.rightkern.unwrap_or(0.0) + rightkern);
                }
                if yoffset != 0.0 {
                    i.yoffset = Some(i.yoffset.unwrap_or(0.0) + yoffset);
                }
            }
            None if leftkern != 0.0 || rightkern != 0.0 => list.set_slot(
                current,
                slot,
                Field::Value(Inj {
                    leftkern: Some(leftkern),
                    rightkern: Some(rightkern),
                    yoffset: Some(yoffset),
                    ..Inj::default()
                }),
            ),
            None => list.set_slot(
                current,
                slot,
                Field::Value(Inj {
                    yoffset: Some(yoffset),
                    ..Inj::default()
                }),
            ),
        }
    } else if leftkern != 0.0 || rightkern != 0.0 {
        list.set_slot(
            current,
            slot,
            Field::Value(Inj {
                leftkern: Some(leftkern),
                rightkern: Some(rightkern),
                yoffset: Some(yoffset),
                ..Inj::default()
            }),
        );
    } else {
        list.set_slot(
            current,
            slot,
            Field::Value(Inj {
                yoffset: Some(yoffset),
                ..Inj::default()
            }),
        );
    }
}

/// `injections.setkern` (font-otj.lua:374-402).
pub fn set_kern(
    list: &mut List,
    counts: &mut Counts,
    current: Ix,
    factor: f64,
    x: f64,
    injection: Option<Slot>,
) -> f64 {
    let dx = factor * x;
    if dx == 0.0 {
        return 0.0;
    }
    counts.kerns += 1;
    let slot = slot_name(injection);
    match list.slot_own(current, slot) {
        Some(i) => i.leftkern = Some(dx + i.leftkern.unwrap_or(0.0)),
        None => list.set_slot(
            current,
            slot,
            Field::Value(Inj {
                leftkern: Some(dx),
                ..Inj::default()
            }),
        ),
    }
    dx
}

/// `injections.setmove` (font-otj.lua:408-457): a kern that becomes the right
/// kern when the text runs right to left.
pub fn set_move(
    list: &mut List,
    counts: &mut Counts,
    current: Ix,
    factor: f64,
    rlmode: i32,
    x: f64,
    injection: Option<Slot>,
) -> f64 {
    let dx = factor * x;
    if dx == 0.0 {
        return 0.0;
    }
    counts.kerns += 1;
    let slot = slot_name(injection);
    let rtl = rlmode < 0;
    match list.slot_own(current, slot) {
        Some(i) => {
            if rtl {
                i.rightkern = Some(dx + i.rightkern.unwrap_or(0.0));
            } else {
                i.leftkern = Some(dx + i.leftkern.unwrap_or(0.0));
            }
        }
        None => list.set_slot(
            current,
            slot,
            Field::Value(match rtl {
                true => Inj {
                    rightkern: Some(dx),
                    ..Inj::default()
                },
                false => Inj {
                    leftkern: Some(dx),
                    ..Inj::default()
                },
            }),
        ),
    }
    dx
}

/// `injections.setmark` (font-otj.lua:459-516).
#[allow(clippy::too_many_arguments)]
pub fn set_mark(
    list: &mut List,
    counts: &mut Counts,
    start: Ix,
    base: Ix,
    factor: f64,
    rlmode: i32,
    ba: [f64; 2],
    ma: [f64; 2],
    basewidth: f64,
    mkmk: bool,
    checkmark: bool,
) -> (f64, f64) {
    let mut dx = factor * (ba[0] - ma[0]);
    let dy = factor * (ba[1] - ma[1]);
    counts.marks += 1;
    let markbase = counts.marks;
    if rlmode >= 0 {
        dx = basewidth - dx;
    }
    let has = list.has_props(start);
    if has {
        match list.slot_own(start, Slot::Injections) {
            Some(i) => {
                if !i.markmark {
                    i.markx = Some(dx);
                    i.marky = Some(dy);
                    i.markdir = Some(rlmode);
                    i.markbase = Some(markbase);
                    i.markbasenode = Some(base);
                    i.markmark = mkmk;
                    i.checkmark = checkmark;
                }
                return (dx, dy);
            }
            None => {}
        }
    }
    list.set_slot(
        start,
        Slot::Injections,
        Field::Value(Inj {
            markx: Some(dx),
            marky: Some(dy),
            markdir: Some(rlmode),
            markbase: Some(markbase),
            markbasenode: Some(base),
            markmark: mkmk,
            checkmark,
            ..Inj::default()
        }),
    );
    (dx, dy)
}

/// A font kern (`fontkern(k)`: subtype 0).
fn fontkern(list: &mut List, k: f64) -> Ix {
    list.alloc(Kind::Kern {
        subtype: 0,
        kern: super::nodes::round(k),
    })
}

/// The characters of a list, in order (`for n in nextchar, list`): the
/// glyphs not yet protected.
fn glyphs_of(list: &List, head: Option<Ix>) -> Vec<Ix> {
    let mut out = Vec::new();
    let mut n = head;
    while let Some(i) = n {
        if matches!(list.ischar(Some(i), None), Ok(Some(_))) {
            out.push(i);
        }
        n = list.nodes[i].next;
    }
    out
}

/// The glyph width the engine reports (`getwidth` of a glyph node).
pub trait Widths {
    fn glyph_width(&self, font: usize, char: u32) -> i64;
}

/// `injectspaces` (font-otj.lua:1649-1762).
fn inject_spaces(
    list: &mut List,
    head: Option<Ix>,
    triggers: &BTreeMap<usize, SpaceKerns>,
    thresholds: &dyn Fn(usize) -> (f64, f64),
) -> Option<Ix> {
    let mut lastfont: Option<usize> = None;
    let mut factor = 0.0;
    let mut threshold = 0.0;
    let mut leftkern: Option<f64> = None;
    let mut rightkern: Option<f64> = None;
    let mut n = head;
    let mut glues = Vec::new();
    while let Some(i) = n {
        if matches!(list.nodes[i].kind, Kind::Glue { .. }) {
            glues.push(i);
        }
        n = list.nodes[i].next;
    }
    for g in glues {
        let (prev, next) = (list.nodes[g].prev, list.nodes[g].next);
        let prevchar = list.ischar(prev, None).ok().flatten();
        let nextchar = list.ischar(next, None).ok().flatten();
        if let (Some(nc), Some(nx)) = (nextchar, next) {
            let font = list.font(nx).unwrap_or(0);
            if let Some(trig) = triggers.get(&font) {
                if lastfont != Some(font) {
                    lastfont = Some(font);
                    (threshold, factor) = thresholds(font);
                }
                if let Some(right) = &trig.right {
                    rightkern = right.get(&nc).copied();
                }
            }
        }
        if let (Some(pc), Some(pv)) = (prevchar, prev) {
            let font = list.font(pv).unwrap_or(0);
            if let Some(trig) = triggers.get(&font) {
                if lastfont != Some(font) {
                    lastfont = Some(font);
                    (threshold, factor) = thresholds(font);
                }
                if let Some(left) = &trig.left {
                    leftkern = left.get(&pc).copied();
                }
            }
        }
        if let Some(lk) = leftkern {
            let old = list.width_of_glue(g) as f64;
            if old > threshold {
                match rightkern {
                    Some(rk) => {
                        list.set_glue_width(g, old + (lk + rk) * factor);
                        rightkern = None;
                    }
                    None => list.set_glue_width(g, old + lk * factor),
                }
            }
            leftkern = None;
        } else if let Some(rk) = rightkern {
            let old = list.width_of_glue(g) as f64;
            if old > threshold {
                list.set_glue_width(g, old + rk * factor);
            }
            rightkern = None;
        }
    }
    head
}

/// `injections.handler` (font-otj.lua:1766-1789).
pub fn handler(
    list: &mut List,
    head: Option<Ix>,
    counts: &mut Counts,
    triggers: &mut BTreeMap<usize, SpaceKerns>,
    thresholds: &dyn Fn(usize) -> (f64, f64),
    widths: &dyn Widths,
) -> Option<Ix> {
    let mut head = head;
    if !triggers.is_empty() {
        head = inject_spaces(list, head, triggers, thresholds);
        triggers.clear();
    }
    if counts.marks > 0 || counts.cursives > 0 {
        head = inject(list, head, counts, Pass::Everything, widths);
        *counts = Counts::default();
    } else if counts.positions > 0 {
        head = inject(list, head, counts, Pass::Positions, widths);
        counts.positions = 0;
    } else if counts.kerns > 0 {
        head = inject(list, head, counts, Pass::Kerns, widths);
        counts.kerns = 0;
    }
    head
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pass {
    Kerns,
    Positions,
    Everything,
}

/// `inject_kerns_only`, `inject_positions_only` and `inject_everything`
/// (font-otj.lua:642-1564), which differ in what they apply: kerns only see
/// the left kern; positions also the right kern and the y offset, and a
/// left/right pair that cancels becomes an x offset; everything adds the
/// marks and the cursive chains.
fn inject(
    list: &mut List,
    head: Option<Ix>,
    counts: &Counts,
    pass: Pass,
    widths: &dyn Widths,
) -> Option<Ix> {
    let mut head = head;
    let hascursives = counts.cursives > 0;
    let hasmarks = counts.marks > 0;
    let positions = pass != Pass::Kerns;
    let mut current = head;
    let mut prev: Option<Ix> = None;
    let mut prevdisc: Option<Ix> = None;
    let mut prevglyph: Option<Ix> = None;
    let (mut pre, mut post, mut replace, mut posttail, mut replacetail): (
        Option<Ix>,
        Option<Ix>,
        Option<Ix>,
        Option<Ix>,
        Option<Ix>,
    ) = (None, None, None, None, None);
    let mut cursiveanchor: Option<Ix> = None;
    let mut minc = 0usize;
    let mut maxc = 0usize;
    let mut cglyphs: Vec<Ix> = Vec::new();
    let mut marks: Vec<Ix> = Vec::new();
    let mut last: Option<Ix> = None;

    let process_mark = |list: &mut List, p: Ix, n: Ix, pn: &Inj| {
        let (px, py) = list.offsets(p);
        let (_nx, ny) = list.offsets(n);
        let mut ox;
        let rightkern = list.slot(p, Slot::Injections).and_then(|i| i.rightkern);
        let markdir = pn.markdir;
        if let Some(rk) = rightkern {
            ox = px as f64 - pn.markx.unwrap_or(0.0) - rk;
            if markdir.is_some_and(|d| d < 0) && !pn.markmark {
                ox += pn.leftkern.unwrap_or(0.0);
            }
        } else {
            ox = px as f64 - pn.markx.unwrap_or(0.0);
            if markdir.is_some_and(|d| d < 0) && !pn.markmark {
                if let Some(lk) = pn.leftkern {
                    ox += lk;
                }
            }
            if pn.checkmark {
                let (font, char) = match &list.nodes[n].kind {
                    Kind::Glyph { font, char, .. } => (*font, *char),
                    _ => (0, 0),
                };
                let wn = widths.glyph_width(font, char);
                if wn != 0 {
                    let wn = wn as f64 / 2.0;
                    // `insertnodebefore(n, n, ...)` and after: the mark is the
                    // "head" of these calls, so the list around is relinked
                    // but the outer head is not updated.
                    let k1 = fontkern(list, -wn);
                    list.insert_before(Some(n), Some(n), k1);
                    let k2 = fontkern(list, -wn);
                    list.insert_after(Some(n), Some(n), k2);
                }
            }
        }
        let mut oy = ny as f64 + py as f64 + pn.marky.unwrap_or(0.0);
        if !pn.markmark {
            if let Some(y) = pn.yoffset {
                oy += y;
            }
        }
        list.set_offsets(n, Some(ox), Some(oy));
    };

    while let Some(cur) = current {
        let next = list.nodes[cur].next;
        let ch = list.ischar(Some(cur), None);
        match ch {
            Ok(Some(_)) => {
                let has = list.has_props(cur);
                if has {
                    let i = list.slot(cur, Slot::Injections).cloned();
                    if let Some(i) = i {
                        if pass == Pass::Everything && i.markbasenode.is_some() {
                            marks.push(cur);
                        } else {
                            if positions {
                                if let Some(y) = i.yoffset.filter(|y| *y != 0.0) {
                                    list.set_offsets(cur, None, Some(y));
                                }
                            }
                            let mut i = i;
                            if pass == Pass::Everything && hascursives {
                                if let Some(cx) = i.cursivex {
                                    if let Some(anchor) = cursiveanchor {
                                        if cx != 0.0 {
                                            let lk = i.leftkern.unwrap_or(0.0) + cx;
                                            i.leftkern = Some(lk);
                                            if let Some(own) = list.slot_own(cur, Slot::Injections)
                                            {
                                                own.leftkern = Some(lk);
                                            }
                                        }
                                        if maxc == 0 {
                                            minc = 1;
                                            maxc = 1;
                                            cglyphs = vec![anchor];
                                        } else {
                                            maxc += 1;
                                            cglyphs.push(anchor);
                                        }
                                        list.props_mut(anchor).cursivedy = match i.cursivey {
                                            Some(v) => Field::Value(v),
                                            None => Field::Unset,
                                        };
                                        last = Some(cur);
                                    } else {
                                        maxc = 0;
                                    }
                                } else if maxc > 0 {
                                    let mut ny = list.offsets(cur).1 as f64;
                                    for k in (minc..=maxc).rev() {
                                        let ti = cglyphs[k - 1];
                                        ny += list.cursivedy(ti).unwrap_or(0.0);
                                        list.set_offsets(ti, None, Some(ny));
                                    }
                                    maxc = 0;
                                    // (the Lua clears cursiveanchor here too; both branches
                                    // below set it again)
                                }
                                if i.cursiveanchor {
                                    cursiveanchor = Some(cur);
                                } else {
                                    if maxc > 0 {
                                        let mut ny = list.offsets(cur).1 as f64;
                                        for k in (minc..=maxc).rev() {
                                            let ti = cglyphs[k - 1];
                                            ny += list.cursivedy(ti).unwrap_or(0.0);
                                            list.set_offsets(ti, None, Some(ny));
                                        }
                                        maxc = 0;
                                    }
                                    cursiveanchor = None;
                                }
                            }
                            let leftkern = i.leftkern;
                            let mut rightkern = if positions { i.rightkern } else { None };
                            if let Some(lk) = leftkern.filter(|k| *k != 0.0) {
                                if positions && rightkern.is_some_and(|rk| lk == -rk) {
                                    list.set_offsets(cur, Some(lk), None);
                                    rightkern = Some(0.0);
                                } else if list.is_glue(prev) {
                                    let p = prev.unwrap();
                                    let w = list.width_of_glue(p) as f64;
                                    list.set_glue_width(p, w + lk);
                                } else {
                                    let k = fontkern(list, lk);
                                    head = list.insert_before(head, Some(cur), k);
                                }
                            }
                            if let Some(rk) = rightkern.filter(|k| *k != 0.0) {
                                if list.is_glue(next) {
                                    let nx = next.unwrap();
                                    let w = list.width_of_glue(nx) as f64;
                                    list.set_glue_width(nx, w + rk);
                                } else {
                                    let k = fontkern(list, rk);
                                    list.insert_after(head, Some(cur), k);
                                }
                            }
                        }
                    } else if positions && next.is_some() {
                        if let Some(e) = list.slot(cur, Slot::Empty).cloned() {
                            if let Some(rk) = e.rightkern.filter(|k| *k != 0.0) {
                                let nx = next.unwrap();
                                if list.is_disc(Some(nx)) && list.disc(nx).2.is_none() {
                                    let k = fontkern(list, rk);
                                    list.set_replace(nx, Some(k));
                                }
                            }
                        }
                    }
                    if let Some(pd) = prevdisc {
                        let mut done = false;
                        if post.is_some() {
                            if let Some(lk) = list
                                .slot(cur, Slot::Post)
                                .and_then(|i| i.leftkern)
                                .filter(|k| *k != 0.0)
                            {
                                let k = fontkern(list, lk);
                                list.link(posttail, Some(k));
                                done = true;
                            }
                        }
                        if replace.is_some() {
                            if let Some(lk) = list
                                .slot(cur, Slot::Replace)
                                .and_then(|i| i.leftkern)
                                .filter(|k| *k != 0.0)
                            {
                                let k = fontkern(list, lk);
                                list.link(replacetail, Some(k));
                                done = true;
                            }
                        } else if let Some(lk) = list
                            .slot(cur, Slot::Empty)
                            .and_then(|i| i.leftkern)
                            .filter(|k| *k != 0.0)
                        {
                            replace = Some(fontkern(list, lk));
                            done = true;
                        }
                        if done {
                            list.set_disc(pd, pre, post, replace);
                        }
                    }
                } else if pass == Pass::Everything && hascursives && maxc > 0 {
                    let mut ny = list.offsets(cur).1 as f64;
                    for k in (minc..=maxc).rev() {
                        let ti = cglyphs[k - 1];
                        ny += list.cursivedy(ti).unwrap_or(0.0);
                        let (xi, yi) = list.offsets(ti);
                        list.set_offsets(ti, Some(xi as f64), Some(yi as f64 + ny));
                    }
                    maxc = 0;
                    cursiveanchor = None;
                }
                prevdisc = None;
                prevglyph = Some(cur);
            }
            Ok(None) => {
                prevdisc = None;
                prevglyph = Some(cur);
            }
            Err(()) if list.is_disc(Some(cur)) => {
                let (p0, q0, r0, _pt, qt, rt) = list.disc_tails(cur);
                pre = p0;
                post = q0;
                replace = r0;
                posttail = qt;
                replacetail = rt;
                let mut done = false;
                for (which, slot) in [(0, Slot::Pre), (1, Slot::Post), (2, Slot::Replace)] {
                    let head_of = match which {
                        0 => pre,
                        1 => post,
                        _ => replace,
                    };
                    if head_of.is_none() {
                        continue;
                    }
                    let mut sub = head_of;
                    for n in glyphs_of(list, head_of) {
                        if !list.has_props(n) {
                            continue;
                        }
                        let i = list
                            .slot(n, Slot::Injections)
                            .or_else(|| list.slot(n, slot))
                            .cloned();
                        let Some(i) = i else {
                            continue;
                        };
                        if positions {
                            if let Some(y) = i.yoffset.filter(|y| *y != 0.0) {
                                list.set_offsets(n, None, Some(y));
                            }
                        }
                        if let Some(lk) = i.leftkern.filter(|k| *k != 0.0) {
                            let k = fontkern(list, lk);
                            sub = list.insert_before(sub, Some(n), k);
                            done = true;
                        }
                        if positions {
                            if let Some(rk) = i.rightkern.filter(|k| *k != 0.0) {
                                let k = fontkern(list, rk);
                                list.insert_after(sub, Some(n), k);
                                done = true;
                            }
                        }
                        if pass == Pass::Everything && hasmarks {
                            if let Some(pm) = i.markbasenode {
                                process_mark(list, pm, n, &i);
                            }
                        }
                    }
                    match which {
                        0 => pre = sub,
                        1 => post = sub,
                        _ => replace = sub,
                    }
                }
                if positions {
                    if let Some(pg) = prevglyph {
                        if pre.is_some() {
                            if let Some(rk) = list
                                .slot(pg, Slot::Pre)
                                .and_then(|i| i.rightkern)
                                .filter(|k| *k != 0.0)
                            {
                                let k = fontkern(list, rk);
                                pre = list.insert_before(pre, pre, k);
                                done = true;
                            }
                        }
                        if replace.is_some() {
                            if let Some(rk) = list
                                .slot(pg, Slot::Replace)
                                .and_then(|i| i.rightkern)
                                .filter(|k| *k != 0.0)
                            {
                                let k = fontkern(list, rk);
                                replace = list.insert_before(replace, replace, k);
                                done = true;
                            }
                        }
                    }
                }
                if done {
                    list.set_disc(cur, pre, post, replace);
                }
                prevglyph = None;
                prevdisc = Some(cur);
            }
            Err(()) => {
                prevglyph = None;
                prevdisc = None;
            }
        }
        prev = Some(cur);
        current = next;
    }
    if pass == Pass::Everything {
        if hascursives && maxc > 0 {
            if let Some(l) = last {
                let mut ny = list.offsets(l).1 as f64;
                for k in (minc..=maxc).rev() {
                    let ti = cglyphs[k - 1];
                    ny += list.cursivedy(ti).unwrap_or(0.0);
                    list.set_offsets(ti, None, Some(ny));
                }
            }
        }
        for m in marks {
            if let Some(i) = list.slot(m, Slot::Injections).cloned() {
                if let Some(b) = i.markbasenode {
                    process_mark(list, b, m, &i);
                }
            }
        }
    }
    head
}
