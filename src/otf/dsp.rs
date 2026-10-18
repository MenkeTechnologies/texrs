//! `fontloader-font-dsp.lua`: GDEF's glyph classes and mark sets, the GSUB and
//! GPOS lookups turned into the fontloader's `sequences` and `sublookups`,
//! the old `kern` table when there is no GPOS `kern`, and the MATH table.
//!
//! The data is the fontloader's model, not the OpenType one: a ligature
//! subtable is a tree from first component to ligature, a pair adjustment a
//! table of second glyphs, a contextual rule three lists of glyph SETS
//! (before reversed, current, after) whatever format the font used, and a
//! lookup's features are recorded per tag, script and language. Everything
//! here is keyed by glyph index; `oup::unify_resources` rekeys it by
//! Unicode afterwards, as `unifyresources` does.
//!
//! The tags are lower-cased and stripped (`readtag`), so `DFLT` is `dflt`
//! and `TRK ` is `trk`. Where the Lua walks a hash with `next` and the order
//! could be seen -- the order of a lookup's feature tags, which decides which
//! feature a lookup is attributed to when several are on -- the order here is
//! sorted by tag.

use std::collections::{BTreeMap, BTreeSet};

use super::otr::FontData;
use super::stream::Stream;

/// A set of glyphs (the fontloader's `tohash` of a coverage).
pub type Set = BTreeSet<u32>;

/// A value record (`readposition`, font-dsp.lua:701-796): `true` when every
/// field is zero, else x placement, y placement, x advance, y advance.
#[derive(Clone, Debug, PartialEq)]
pub enum Pos {
    Zero,
    V([f64; 4]),
}

/// A pair's two value records (`{ first, second or nil }`); `None` for the
/// Lua `false`/`nil`.
#[derive(Clone, Debug, PartialEq)]
pub struct PairValue {
    pub first: Option<Pos>,
    pub second: Option<Pos>,
}

pub type Anchor = [f64; 2];

/// A ligature subtable's tree (font-dsp.lua:1336-1356).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LigTree {
    pub ligature: Option<u32>,
    pub next: BTreeMap<u32, LigTree>,
}

/// A mark's record: the class it attaches with and its own anchor.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkRecord {
    pub class: usize,
    pub anchor: Option<Anchor>,
}

/// A base's anchor per mark class, or per ligature component.
#[derive(Clone, Debug, PartialEq)]
pub enum BaseAnchor {
    One(Option<Anchor>),
    /// component index (1-based) -> anchor
    Components(BTreeMap<usize, Option<Anchor>>),
}

/// A cursive record: entry and exit anchors.
#[derive(Clone, Debug, PartialEq)]
pub struct Cursive {
    pub entry: Option<Anchor>,
    pub exit: Option<Anchor>,
}

/// A step's coverage, by lookup type.
#[derive(Clone, Debug, PartialEq)]
pub enum Coverage {
    /// gsub_single
    Single(BTreeMap<u32, u32>),
    /// gsub_multiple, gsub_alternate
    Sequence(BTreeMap<u32, Vec<u32>>),
    /// gsub_ligature
    Ligature(BTreeMap<u32, LigTree>),
    /// gpos_single, format "single"
    Single1(BTreeMap<u32, Option<Pos>>),
    /// gpos_single after `checkkerns`: format "move", the x advance
    Move1(BTreeMap<u32, f64>),
    /// gpos_pair, format "pair"
    Pair(BTreeMap<u32, BTreeMap<u32, PairValue>>),
    /// gpos_pair after `checkpairs`: format "move", the first x advance
    Move2(BTreeMap<u32, BTreeMap<u32, f64>>),
    /// gpos_pair from the old kern table, format "kern"
    Kern(BTreeMap<u32, BTreeMap<u32, f64>>),
    Cursive(BTreeMap<u32, Cursive>),
    Mark(BTreeMap<u32, MarkRecord>),
    /// a contextual lookup's combined first-glyph coverage, filled by
    /// `expand`: glyph -> the rules it may start
    Rules(BTreeMap<u32, Vec<usize>>),
    None,
}

/// A contextual rule (font-dsp.lua:942-1208, after `resolvelookups`).
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    /// reversed, as the fontloader stores it
    pub before: Option<Vec<Set>>,
    pub current: Vec<Set>,
    pub after: Option<Vec<Set>>,
    /// per position in `current` (1-based in the Lua): the sublookups to
    /// apply there, `None` for a hole
    pub lookups: Option<Vec<Option<Vec<usize>>>>,
    /// a reverse chain's single substitution
    pub replacements: Option<BTreeMap<u32, u32>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub coverage: Coverage,
    pub format: Option<&'static str>,
    pub rules: Option<Vec<Rule>>,
    /// mark attachment: class -> base glyph -> anchor
    pub baseclasses: Option<BTreeMap<usize, BTreeMap<u32, BaseAnchor>>>,
}

/// The four lookup flags (`lookupflags`, font-dsp.lua:306-315), already
/// turned into the class names a skip compares with.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Flags {
    pub mark: bool,
    pub ligature: bool,
    pub base: bool,
    pub r2l: bool,
}

/// A lookup (`sequences[i]` or `sublookups[i]`).
#[derive(Clone, Debug, PartialEq)]
pub struct Sequence {
    pub index: usize,
    pub name: String,
    pub typ: &'static str,
    pub steps: Vec<Step>,
    /// a mark class or mark set number until `expand` resolves it
    pub markclass: Option<usize>,
    pub flags: Flags,
    /// the feature tags that reference the lookup, in reference order
    pub order: Vec<String>,
    /// tag -> script -> languages
    pub features: BTreeMap<String, BTreeMap<String, BTreeSet<String>>>,
    pub merged: bool,
}

/// MATH variants and assembly of one glyph (font-dsp.lua:2982-3082).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MathGlyph {
    pub italic: Option<f64>,
    pub accent: Option<f64>,
    /// topright, topleft, bottomright, bottomleft: (height, kern) pairs
    pub kerns: Option<MathKerns>,
    pub vvariants: Option<Vec<u32>>,
    pub hvariants: Option<Vec<u32>>,
    pub vparts: Option<Vec<MathPart>>,
    pub hparts: Option<Vec<MathPart>>,
    pub vitalic: Option<f64>,
    pub hitalic: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MathKerns {
    pub topright: Option<Vec<MathKern>>,
    pub topleft: Option<Vec<MathKern>>,
    pub bottomright: Option<Vec<MathKern>>,
    pub bottomleft: Option<Vec<MathKern>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MathKern {
    pub height: Option<f64>,
    pub kern: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MathPart {
    pub glyph: u32,
    pub start: f64,
    pub end: f64,
    pub advance: f64,
    pub extender: bool,
}

/// What the GDEF/GSUB/GPOS/MATH readers add to `fontdata`.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub marks: Set,
    /// mark attachment classes then mark sets, 1-based by class number
    pub markclasses: BTreeMap<usize, Set>,
    pub nofmarksets: usize,
    pub sequences: Vec<Sequence>,
    pub sublookups: Vec<Sequence>,
    /// what -> tag -> script -> languages
    pub features: BTreeMap<String, BTreeMap<String, BTreeMap<String, BTreeSet<String>>>>,
    pub designsize: Option<f64>,
    pub minsize: Option<f64>,
    pub maxsize: Option<f64>,
    pub mathconstants: Option<BTreeMap<&'static str, f64>>,
    pub math: BTreeMap<u32, MathGlyph>,
    pub hasitalics: bool,
}

/// `readtag`: four bytes, stripped and lower-cased.
fn tag(f: &mut Stream) -> String {
    String::from_utf8_lossy(&f.bytes(4)).trim().to_lowercase()
}

/// `readcoverage` (font-dsp.lua:559-609) as an index -> position map.
fn coverage_map(f: &mut Stream, offset: usize) -> BTreeMap<u32, u32> {
    f.set(offset);
    let mut out = BTreeMap::new();
    match f.u16() {
        1 => {
            let n = f.u16();
            for i in 0..n {
                out.insert(f.u16(), i);
            }
        }
        2 => {
            let ranges = f.u16();
            let mut n = 0;
            for _ in 0..ranges {
                let first = f.u16();
                let last = f.u16();
                let _cover = f.u16();
                for g in first..=last {
                    out.insert(g, n);
                    n += 1;
                }
            }
        }
        _ => {}
    }
    out
}

/// `readcoverage(f, offset, true)`: the covered glyphs in coverage order.
fn coverage_list(f: &mut Stream, offset: usize) -> Vec<u32> {
    f.set(offset);
    let mut out = Vec::new();
    match f.u16() {
        1 => {
            let n = f.u16() as usize;
            out = f.u16s(n);
        }
        2 => {
            let ranges = f.u16();
            for _ in 0..ranges {
                let first = f.u16();
                let last = f.u16();
                let _cover = f.u16();
                out.extend(first..=last);
            }
        }
        _ => {}
    }
    out
}

/// `readclassdef` (font-dsp.lua:611-649): glyph -> class + 1, with every glyph
/// of `preset` (a count or a coverage) defaulting to class 1.
enum Preset<'a> {
    Count(u32),
    Coverage(&'a BTreeMap<u32, u32>),
}

fn classdef(f: &mut Stream, offset: usize, preset: Preset) -> BTreeMap<u32, u32> {
    f.set(offset);
    let format = f.u16();
    let mut def = BTreeMap::new();
    if let Preset::Count(n) = preset {
        for k in 0..n {
            def.insert(k, 1);
        }
    }
    if format == 1 {
        let mut index = f.u16();
        let n = f.u16();
        for _ in 0..n {
            def.insert(index, f.u16() + 1);
            index += 1;
        }
    } else if format == 2 {
        let ranges = f.u16();
        for _ in 0..ranges {
            let first = f.u16();
            let last = f.u16();
            let class = f.u16() + 1;
            for g in first..=last {
                def.insert(g, class);
            }
        }
    }
    if let Preset::Coverage(c) = preset {
        for &k in c.keys() {
            def.entry(k).or_insert(1);
        }
    }
    def
}

/// `classtocoverage`: class -> its glyphs.
fn classes_of(def: &BTreeMap<u32, u32>) -> BTreeMap<u32, Vec<u32>> {
    let mut out: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (&g, &c) in def {
        out.entry(c).or_default().push(g);
    }
    out
}

/// `readposition` (font-dsp.lua:701-796) without variation deltas.
fn position(f: &mut Stream, format: u32) -> Option<Pos> {
    if format == 0 {
        return None;
    }
    let mut x = 0.0;
    let mut y = 0.0;
    let mut h = 0.0;
    let mut v = 0.0;
    if format == 0x04 {
        h = f.i16() as f64;
        return Some(match h == 0.0 {
            true => Pos::Zero,
            false => Pos::V([0.0, 0.0, h, 0.0]),
        });
    }
    if format == 0x05 {
        x = f.i16() as f64;
        h = f.i16() as f64;
        return Some(match x == 0.0 && h == 0.0 {
            true => Pos::Zero,
            false => Pos::V([x, 0.0, h, 0.0]),
        });
    }
    if format == 0x44 {
        h = f.i16() as f64;
        f.skip_u16(1);
        return Some(match h == 0.0 {
            true => Pos::Zero,
            false => Pos::V([0.0, 0.0, h, 0.0]),
        });
    }
    if format & 0x1 != 0 {
        x = f.i16() as f64;
    }
    if format & 0x2 != 0 {
        y = f.i16() as f64;
    }
    if format & 0x4 != 0 {
        h = f.i16() as f64;
    }
    if format & 0x8 != 0 {
        v = f.i16() as f64;
    }
    if format >= 0x10 {
        // the device/variation offsets: read past (`skipshort` returns
        // nothing, so the X/Y/H/V locals are nil and no delta applies)
        for bit in [0x10, 0x20, 0x40, 0x80] {
            if format & bit != 0 {
                f.skip_u16(1);
            }
        }
        // font-dsp.lua:760: and then `skips[extract(format,4,4)]` more
        const SKIPS: [usize; 16] = [0, 1, 1, 2, 1, 2, 2, 3, 2, 2, 3, 2, 3, 3, 4, 0];
        let s = SKIPS[((format >> 4) & 0xF) as usize];
        f.skip_u16(s);
        return Some(Pos::V([x, y, h, v]));
    }
    Some(match x == 0.0 && y == 0.0 && h == 0.0 && v == 0.0 {
        true => Pos::Zero,
        false => Pos::V([x, y, h, v]),
    })
}

/// `readanchor` (font-dsp.lua:798-830).
fn anchor(f: &mut Stream, offset: usize) -> Option<Anchor> {
    if offset == 0 {
        return None;
    }
    f.set(offset);
    let _format = f.i16();
    let x = f.i16() as f64;
    let y = f.i16() as f64;
    Some([x, y])
}

/// `readarray`: a count and that many shorts; `None` for zero.
fn array(f: &mut Stream, offset: Option<usize>) -> Option<Vec<u32>> {
    if let Some(o) = offset {
        f.set(o);
    }
    let n = f.u16() as usize;
    (n > 0).then(|| f.u16s(n))
}

/// `readlookuparray` (font-dsp.lua:892-919): position (1-based) -> lookups
/// (1-based ids), holes `None`.
fn lookup_array(f: &mut Stream, n: u32) -> Vec<Option<Vec<usize>>> {
    let mut out: Vec<Option<Vec<usize>>> = Vec::new();
    for _ in 0..n {
        let index = f.u16() as usize + 1;
        let lookup = f.u16() as usize + 1;
        if out.len() < index {
            out.resize(index, None);
        }
        out[index - 1].get_or_insert_with(Vec::new).push(lookup);
    }
    out
}

fn rule(
    before: Option<Vec<Vec<u32>>>,
    current: Vec<Vec<u32>>,
    after: Option<Vec<Vec<u32>>>,
    lookups: Vec<Option<Vec<usize>>>,
) -> Rule {
    let to_sets =
        |v: Vec<Vec<u32>>| -> Vec<Set> { v.into_iter().map(|l| l.into_iter().collect()).collect() };
    Rule {
        // `rule.before = reversed(before)` (font-dsp.lua:2249)
        before: before.map(|b| to_sets(b).into_iter().rev().collect()),
        current: to_sets(current),
        after: after.map(to_sets),
        lookups: Some(lookups),
        replacements: None,
    }
}

/// `unchainedcontext` and `chainedcontext` (font-dsp.lua:942-1208).
fn context(f: &mut Stream, tableoffset: usize, nofglyphs: u32, chained: bool) -> Option<Step> {
    f.set(tableoffset);
    let subtype = f.u16();
    let mut rules = Vec::new();
    let format: &'static str;
    if subtype == 1 {
        format = "glyphs";
        let coverage = f.u16() as usize;
        let sets = array(f, None);
        if let Some(sets) = sets {
            let coverage = coverage_list(f, tableoffset + coverage);
            for (i, &offset) in sets.iter().enumerate() {
                if offset == 0 {
                    continue;
                }
                let first = coverage.get(i).copied();
                let rulesoffset = tableoffset + offset as usize;
                let ruleoffsets = array(f, Some(rulesoffset)).unwrap_or_default();
                for r in ruleoffsets {
                    f.set(rulesoffset + r as usize);
                    let read_list = |f: &mut Stream| -> Option<Vec<Vec<u32>>> {
                        let n = f.u16();
                        (n > 0).then(|| (0..n).map(|_| vec![f.u16()]).collect())
                    };
                    let (before, current, after, lookups);
                    if chained {
                        before = read_list(f);
                        let nofcurrent = f.u16();
                        let mut c = vec![first.into_iter().collect::<Vec<u32>>()];
                        for _ in 1..nofcurrent {
                            c.push(vec![f.u16()]);
                        }
                        current = c;
                        after = read_list(f);
                        let n = f.u16();
                        lookups = lookup_array(f, n);
                    } else {
                        let nofcurrent = f.u16();
                        let noflookups = f.u16();
                        let mut c = vec![first.into_iter().collect::<Vec<u32>>()];
                        for _ in 1..nofcurrent {
                            c.push(vec![f.u16()]);
                        }
                        current = c;
                        before = None;
                        after = None;
                        lookups = lookup_array(f, noflookups);
                    }
                    rules.push(rule(before, current, after, lookups));
                }
            }
        }
    } else if subtype == 2 {
        format = "class";
        let coverage = f.u16() as usize;
        let (beforedef, currentdef, afterdef);
        if chained {
            beforedef = f.u16() as usize;
            currentdef = f.u16() as usize;
            afterdef = f.u16() as usize;
        } else {
            beforedef = 0;
            currentdef = f.u16() as usize;
            afterdef = 0;
        }
        let sets = array(f, None);
        if let Some(sets) = sets {
            let coverage = coverage_map(f, tableoffset + coverage);
            let beforeclasses = match chained {
                true => classes_of(&classdef(
                    f,
                    tableoffset + beforedef,
                    Preset::Count(nofglyphs),
                )),
                false => BTreeMap::new(),
            };
            let currentclasses = classes_of(&classdef(
                f,
                tableoffset + currentdef,
                Preset::Coverage(&coverage),
            ));
            let afterclasses = match chained {
                true => classes_of(&classdef(
                    f,
                    tableoffset + afterdef,
                    Preset::Count(nofglyphs),
                )),
                false => BTreeMap::new(),
            };
            for (i, &offset) in sets.iter().enumerate() {
                if offset == 0 {
                    continue;
                }
                let class = i as u32 + 1;
                let Some(first) = currentclasses.get(&class) else {
                    continue;
                };
                // `covered(firstcoverage, coverage)`: only the covered ones
                let first: Vec<u32> = first
                    .iter()
                    .copied()
                    .filter(|g| coverage.contains_key(g))
                    .collect();
                if first.is_empty() {
                    continue;
                }
                let rulesoffset = tableoffset + offset as usize;
                let ruleoffsets = array(f, Some(rulesoffset)).unwrap_or_default();
                for r in ruleoffsets {
                    f.set(rulesoffset + r as usize);
                    let classlist =
                        |f: &mut Stream, classes: &BTreeMap<u32, Vec<u32>>| -> Vec<u32> {
                            classes.get(&(f.u16() + 1)).cloned().unwrap_or_default()
                        };
                    let (before, mut current, after, lookups);
                    if chained {
                        let n = f.u16();
                        before =
                            (n > 0).then(|| (0..n).map(|_| classlist(f, &beforeclasses)).collect());
                        let nofcurrent = f.u16();
                        current = vec![first.clone()];
                        for _ in 1..nofcurrent {
                            current.push(classlist(f, &currentclasses));
                        }
                        let n = f.u16();
                        after =
                            (n > 0).then(|| (0..n).map(|_| classlist(f, &afterclasses)).collect());
                        let n = f.u16();
                        lookups = lookup_array(f, n);
                    } else {
                        let nofcurrent = f.u16();
                        let noflookups = f.u16();
                        current = vec![first.clone()];
                        for _ in 1..nofcurrent {
                            current.push(classlist(f, &currentclasses));
                        }
                        before = None;
                        after = None;
                        lookups = lookup_array(f, noflookups);
                    }
                    rules.push(rule(before, current, after, lookups));
                }
            }
        }
    } else if subtype == 3 {
        format = "coverage";
        let (before, current, after, lookups);
        if chained {
            let b = array(f, None);
            let c = array(f, None);
            let a = array(f, None);
            let n = f.u16();
            lookups = lookup_array(f, n);
            let read = |f: &mut Stream, v: Option<Vec<u32>>| -> Option<Vec<Vec<u32>>> {
                v.map(|v| {
                    v.into_iter()
                        .map(|o| coverage_list(f, tableoffset + o as usize))
                        .collect()
                })
            };
            before = read(f, b);
            current = read(f, c).unwrap_or_default();
            after = read(f, a);
        } else {
            let nofglyphs = f.u16() as usize;
            let noflookups = f.u16();
            let c = f.u16s(nofglyphs);
            lookups = lookup_array(f, noflookups);
            current = c
                .into_iter()
                .map(|o| coverage_list(f, tableoffset + o as usize))
                .collect();
            before = None;
            after = None;
        }
        rules.push(rule(before, current, after, lookups));
    } else {
        return None;
    }
    Some(Step {
        coverage: Coverage::None,
        format: Some(format),
        rules: Some(rules),
        baseclasses: None,
    })
}

/// One GSUB subtable (font-dsp.lua:1231-1404). Returns the step and, for an
/// extension, the type it wraps.
fn gsub_step(
    f: &mut Stream,
    typ: &str,
    tableoffset: usize,
    nofglyphs: u32,
) -> Option<(Step, &'static str)> {
    let plain = |coverage: Coverage| Step {
        coverage,
        format: None,
        rules: None,
        baseclasses: None,
    };
    match typ {
        "single" => {
            f.set(tableoffset);
            let subtype = f.u16();
            let cov = f.u16() as usize;
            let mut out = BTreeMap::new();
            if subtype == 1 {
                let delta = f.i16();
                for (&index, _) in &coverage_map(f, tableoffset + cov) {
                    let newindex = ((index as i64 + delta as i64).rem_euclid(65536)) as u32;
                    if index <= nofglyphs && newindex <= nofglyphs {
                        out.insert(index, newindex);
                    }
                }
            } else if subtype == 2 {
                let n = f.u16() as usize;
                let replacements = f.u16s(n);
                for (&index, &pos) in &coverage_map(f, tableoffset + cov) {
                    let newindex = pos + 1;
                    if index <= nofglyphs && newindex <= nofglyphs {
                        if let Some(&r) = replacements.get(pos as usize) {
                            out.insert(index, r);
                        }
                    }
                }
            } else {
                return None;
            }
            Some((plain(Coverage::Single(out)), "single"))
        }
        "multiple" | "alternate" => {
            f.set(tableoffset);
            let subtype = f.u16();
            if subtype != 1 {
                return None;
            }
            let cov = f.u16() as usize;
            let n = f.u16() as usize;
            let offsets = f.u16s(n);
            let sequences: Vec<Vec<u32>> = offsets
                .iter()
                .map(|&o| {
                    f.set(tableoffset + o as usize);
                    let k = f.u16() as usize;
                    f.u16s(k)
                })
                .collect();
            let mut out = BTreeMap::new();
            for (&index, &pos) in &coverage_map(f, tableoffset + cov) {
                let newindex = pos + 1;
                if index <= nofglyphs && newindex <= nofglyphs {
                    if let Some(s) = sequences.get(pos as usize) {
                        out.insert(index, s.clone());
                    }
                }
            }
            let t = match typ {
                "multiple" => "multiple",
                _ => "alternate",
            };
            Some((plain(Coverage::Sequence(out)), t))
        }
        "ligature" => {
            f.set(tableoffset);
            if f.u16() != 1 {
                return None;
            }
            let cov = f.u16() as usize;
            let nofsets = f.u16() as usize;
            let sets = f.u16s(nofsets);
            let ligatures: Vec<Vec<usize>> = sets
                .iter()
                .map(|&s| {
                    let offset = tableoffset + s as usize;
                    f.set(offset);
                    let n = f.u16() as usize;
                    f.u16s(n).into_iter().map(|o| offset + o as usize).collect()
                })
                .collect();
            let mut out = BTreeMap::new();
            for (&index, &pos) in &coverage_map(f, tableoffset + cov) {
                let mut root = LigTree::default();
                if let Some(list) = ligatures.get(pos as usize) {
                    for &offset in list {
                        f.set(offset);
                        let lig = f.u16();
                        let cnt = f.u16();
                        let mut node = &mut root;
                        for _ in 1..cnt {
                            let c = f.u16();
                            node = node.next.entry(c).or_default();
                        }
                        node.ligature = Some(lig);
                    }
                }
                out.insert(index, root);
            }
            Some((plain(Coverage::Ligature(out)), "ligature"))
        }
        "context" => context(f, tableoffset, nofglyphs, false).map(|s| (s, "context")),
        "chainedcontext" => context(f, tableoffset, nofglyphs, true).map(|s| (s, "chainedcontext")),
        "reversechainedcontextsingle" => {
            f.set(tableoffset);
            if f.u16() != 1 {
                return None;
            }
            let current = vec![f.u16()];
            let before = array(f, None);
            let after = array(f, None);
            let replacements = array(f, None);
            let current: Vec<Vec<u32>> = current
                .into_iter()
                .map(|o| coverage_list(f, tableoffset + o as usize))
                .collect();
            let read = |f: &mut Stream, v: Option<Vec<u32>>| -> Option<Vec<Vec<u32>>> {
                v.map(|v| {
                    v.into_iter()
                        .map(|o| coverage_list(f, tableoffset + o as usize))
                        .collect()
                })
            };
            let before = read(f, before);
            let after = read(f, after);
            // font-dsp.lua:2252-2264: one current set, its replacements paired
            // up by position
            let first = current.first().cloned().unwrap_or_default();
            let mut repl = BTreeMap::new();
            for (i, &c) in first.iter().enumerate() {
                if let Some(r) = replacements.as_ref().and_then(|r| r.get(i)) {
                    repl.insert(c, *r);
                }
            }
            let mut r = rule(before, vec![first], after, Vec::new());
            r.lookups = None;
            r.replacements = Some(repl);
            Some((
                Step {
                    coverage: Coverage::None,
                    format: Some("reversecoverage"),
                    rules: Some(vec![r]),
                    baseclasses: None,
                },
                "reversechainedcontextsingle",
            ))
        }
        "extension" => {
            f.set(tableoffset);
            if f.u16() != 1 {
                return None;
            }
            let t = f.u16();
            let far = f.u32() as usize;
            let inner = GSUB_TYPES.get(t as usize - 1).copied()?;
            gsub_step(f, inner, tableoffset + far, nofglyphs)
        }
        _ => None,
    }
}

/// One GPOS subtable (font-dsp.lua:1408-1928).
fn gpos_step(
    f: &mut Stream,
    typ: &str,
    tableoffset: usize,
    nofglyphs: u32,
) -> Option<(Step, &'static str)> {
    match typ {
        "single" => {
            f.set(tableoffset);
            let subtype = f.u16();
            let cov = f.u16() as usize;
            let format = f.u16();
            let mut out = BTreeMap::new();
            if subtype == 1 {
                let value = position(f, format);
                for &index in coverage_map(f, tableoffset + cov).keys() {
                    out.insert(index, value.clone());
                }
            } else if subtype == 2 {
                let n = f.u16();
                let values: Vec<Option<Pos>> = (0..n).map(|_| position(f, format)).collect();
                for (&index, &pos) in &coverage_map(f, tableoffset + cov) {
                    out.insert(index, values.get(pos as usize).cloned().flatten());
                }
            } else {
                return None;
            }
            Some((
                Step {
                    coverage: Coverage::Single1(out),
                    format: Some("single"),
                    rules: None,
                    baseclasses: None,
                },
                "single",
            ))
        }
        "pair" => {
            f.set(tableoffset);
            let subtype = f.u16();
            let mut out: BTreeMap<u32, BTreeMap<u32, PairValue>> = BTreeMap::new();
            if subtype == 1 {
                let cov = f.u16() as usize;
                let format1 = f.u16();
                let format2 = f.u16();
                let sets = array(f, None).unwrap_or_default();
                let sets: Vec<Vec<(u32, Option<Pos>, Option<Pos>)>> = sets
                    .iter()
                    .map(|&o| {
                        let offset = tableoffset + o as usize;
                        f.set(offset);
                        let n = f.u16();
                        (0..n)
                            .map(|_| (f.u16(), position(f, format1), position(f, format2)))
                            .collect()
                    })
                    .collect();
                for (&index, &pos) in &coverage_map(f, tableoffset + cov) {
                    let mut hash = BTreeMap::new();
                    if let Some(set) = sets.get(pos as usize) {
                        for (other, first, second) in set {
                            // `share = { first, second or nil }`, false when
                            // neither: `hash[other] = share or nil`
                            if first.is_some() || second.is_some() {
                                hash.insert(
                                    *other,
                                    PairValue {
                                        first: first.clone(),
                                        second: second.clone(),
                                    },
                                );
                            } else {
                                hash.remove(other);
                            }
                        }
                    }
                    out.insert(index, hash);
                }
            } else if subtype == 2 {
                let cov = f.u16() as usize;
                let format1 = f.u16();
                let format2 = f.u16();
                let classdef1 = f.u16() as usize;
                let classdef2 = f.u16() as usize;
                let nofclasses1 = f.u16();
                let nofclasses2 = f.u16();
                let classlist: Vec<Vec<Option<PairValue>>> = (0..nofclasses1)
                    .map(|_| {
                        (0..nofclasses2)
                            .map(|_| {
                                let one = position(f, format1);
                                let two = position(f, format2);
                                (one.is_some() || two.is_some()).then_some(PairValue {
                                    first: one,
                                    second: two,
                                })
                            })
                            .collect()
                    })
                    .collect();
                let coverage = coverage_map(f, tableoffset + cov);
                let def1 = classdef(f, tableoffset + classdef1, Preset::Coverage(&coverage));
                let def2 = classdef(f, tableoffset + classdef2, Preset::Count(nofglyphs));
                for (&g1, &c1) in &def1 {
                    if !coverage.contains_key(&g1) {
                        continue;
                    }
                    let Some(l1) = classlist.get(c1 as usize - 1) else {
                        continue;
                    };
                    let mut hash = BTreeMap::new();
                    for (&paired, &class) in &def2 {
                        if let Some(Some(v)) = l1.get(class as usize - 1) {
                            hash.insert(paired, v.clone());
                        }
                    }
                    out.insert(g1, hash);
                }
            } else {
                return None;
            }
            Some((
                Step {
                    coverage: Coverage::Pair(out),
                    format: Some("pair"),
                    rules: None,
                    baseclasses: None,
                },
                "pair",
            ))
        }
        "cursive" => {
            f.set(tableoffset);
            if f.u16() != 1 {
                return None;
            }
            let cov = tableoffset + f.u16() as usize;
            let n = f.u16();
            let records: Vec<(u32, u32)> = (0..n).map(|_| (f.u16(), f.u16())).collect();
            let coverage = coverage_map(f, cov);
            let records: Vec<Cursive> = records
                .into_iter()
                .map(|(entry, exit)| Cursive {
                    entry: match entry {
                        0 => None,
                        e => anchor(f, tableoffset + e as usize),
                    },
                    exit: match exit {
                        0 => None,
                        e => anchor(f, tableoffset + e as usize),
                    },
                })
                .collect();
            let mut out = BTreeMap::new();
            for (&index, &pos) in &coverage {
                if let Some(r) = records.get(pos as usize) {
                    out.insert(index, r.clone());
                }
            }
            Some((
                Step {
                    coverage: Coverage::Cursive(out),
                    format: None,
                    rules: None,
                    baseclasses: None,
                },
                "cursive",
            ))
        }
        "marktobase" | "marktoligature" | "marktomark" => {
            let ligature = typ == "marktoligature";
            f.set(tableoffset);
            if f.u16() != 1 {
                return None;
            }
            let markcov = tableoffset + f.u16() as usize;
            let basecov = tableoffset + f.u16() as usize;
            let nofclasses = f.u16() as usize;
            let markoffset = tableoffset + f.u16() as usize;
            let baseoffset = tableoffset + f.u16() as usize;
            let markcoverage = coverage_map(f, markcov);
            let basecoverage = coverage_list(f, basecov);
            f.set(markoffset);
            let nofmarks = f.u16();
            let raw: Vec<(usize, u32)> = (0..nofmarks)
                .map(|_| (f.u16() as usize + 1, f.u16()))
                .collect();
            let markclasses: Vec<Option<MarkRecord>> = raw
                .into_iter()
                .map(|(class, offset)| match offset {
                    0 => None,
                    o => Some(MarkRecord {
                        class,
                        anchor: anchor(f, markoffset + o as usize),
                    }),
                })
                .collect();
            f.set(baseoffset);
            let nofbases = f.u16() as usize;
            let mut baseclasses: BTreeMap<usize, BTreeMap<u32, BaseAnchor>> =
                (1..=nofclasses).map(|i| (i, BTreeMap::new())).collect();
            if ligature {
                let records: Vec<u32> = f.u16s(nofbases);
                for (i, &rec) in records.iter().enumerate() {
                    if rec == 0 {
                        continue;
                    }
                    let recordoffset = baseoffset + rec as usize;
                    f.set(recordoffset);
                    let nofcomponents = f.u16() as usize;
                    let components: Vec<Vec<u32>> =
                        (0..nofcomponents).map(|_| f.u16s(nofclasses)).collect();
                    let Some(&b) = basecoverage.get(i) else {
                        continue;
                    };
                    for (c, classes) in components.iter().enumerate() {
                        for (k, &o) in classes.iter().enumerate() {
                            let a = match o {
                                0 => None,
                                o => anchor(f, recordoffset + o as usize),
                            };
                            let entry = baseclasses
                                .entry(k + 1)
                                .or_default()
                                .entry(b)
                                .or_insert_with(|| BaseAnchor::Components(BTreeMap::new()));
                            if let (BaseAnchor::Components(m), Some(_)) = (entry, a) {
                                m.insert(c + 1, a);
                            }
                        }
                    }
                }
            } else {
                let records: Vec<Vec<u32>> = (0..nofbases).map(|_| f.u16s(nofclasses)).collect();
                for (i, r) in records.iter().enumerate() {
                    let Some(&b) = basecoverage.get(i) else {
                        continue;
                    };
                    for (j, &o) in r.iter().enumerate() {
                        let a = match o {
                            0 => None,
                            o => anchor(f, baseoffset + o as usize),
                        };
                        // `baseclasses[j][b] = readanchor(...)`: nil leaves no entry
                        if a.is_some() {
                            baseclasses
                                .entry(j + 1)
                                .or_default()
                                .insert(b, BaseAnchor::One(a));
                        }
                    }
                }
            }
            let mut out = BTreeMap::new();
            for (&index, &pos) in &markcoverage {
                if let Some(Some(m)) = markclasses.get(pos as usize) {
                    out.insert(index, m.clone());
                }
            }
            let t: &'static str = match typ {
                "marktobase" => "marktobase",
                "marktoligature" => "marktoligature",
                _ => "marktomark",
            };
            Some((
                Step {
                    coverage: Coverage::Mark(out),
                    format: Some(if ligature { "ligature" } else { "base" }),
                    rules: None,
                    baseclasses: Some(baseclasses),
                },
                t,
            ))
        }
        "context" => context(f, tableoffset, nofglyphs, false).map(|s| (s, "context")),
        "chainedcontext" => context(f, tableoffset, nofglyphs, true).map(|s| (s, "chainedcontext")),
        "extension" => {
            f.set(tableoffset);
            if f.u16() != 1 {
                return None;
            }
            let t = f.u16();
            let far = f.u32() as usize;
            let inner = GPOS_TYPES.get(t as usize - 1).copied()?;
            gpos_step(f, inner, tableoffset + far, nofglyphs)
        }
        _ => None,
    }
}

const GSUB_TYPES: [&str; 8] = [
    "single",
    "multiple",
    "alternate",
    "ligature",
    "context",
    "chainedcontext",
    "extension",
    "reversechainedcontextsingle",
];

const GPOS_TYPES: [&str; 9] = [
    "single",
    "pair",
    "cursive",
    "marktobase",
    "marktoligature",
    "marktomark",
    "context",
    "chainedcontext",
    "extension",
];

/// `lookupnames` (font-dsp.lua:261-281).
fn lookup_name(what: &str, t: &str) -> &'static str {
    match (what, t) {
        ("gsub", "single") => "gsub_single",
        ("gsub", "multiple") => "gsub_multiple",
        ("gsub", "alternate") => "gsub_alternate",
        ("gsub", "ligature") => "gsub_ligature",
        ("gsub", "context") => "gsub_context",
        ("gsub", "chainedcontext") => "gsub_contextchain",
        ("gsub", "reversechainedcontextsingle") => "gsub_reversecontextchain",
        ("gpos", "single") => "gpos_single",
        ("gpos", "pair") => "gpos_pair",
        ("gpos", "cursive") => "gpos_cursive",
        ("gpos", "marktobase") => "gpos_mark2base",
        ("gpos", "marktoligature") => "gpos_mark2ligature",
        ("gpos", "marktomark") => "gpos_mark2mark",
        ("gpos", "context") => "gpos_context",
        ("gpos", "chainedcontext") => "gpos_contextchain",
        _ => "unknown",
    }
}

/// `readers.gdef` (font-dsp.lua:2669-2796), without the variation store.
pub fn read_gdef(f: &mut Stream, fontdata: &mut FontData, layout: &mut Layout) {
    let Some(t) = fontdata.tables.get("gdef").copied() else {
        return;
    };
    let tableoffset = t.offset;
    f.set(tableoffset);
    let version = f.u32();
    let classoffset = f.u16() as usize;
    let _attachment = f.u16();
    let _carets = f.u16();
    let markclassoffset = f.u16() as usize;
    let marksetsoffset = match version >= 0x0001_0002 {
        true => f.u16() as usize,
        false => 0,
    };
    const CLASSES: [&str; 4] = ["base", "ligature", "mark", "component"];
    let class_name =
        |c: u32| -> Option<&'static str> { (1..=4).contains(&c).then(|| CLASSES[c as usize - 1]) };
    if classoffset != 0 {
        f.set(tableoffset + classoffset);
        let format = f.u16();
        if format == 1 {
            let first = f.u16();
            let count = f.u16();
            for index in first..first + count {
                let class = class_name(f.u16());
                if class == Some("mark") {
                    layout.marks.insert(index);
                }
                fontdata.glyph(index).class = class.map(str::to_string);
            }
        } else if format == 2 {
            let ranges = f.u16();
            for _ in 0..ranges {
                let first = f.u16();
                let last = f.u16();
                if let Some(class) = class_name(f.u16()) {
                    for index in first..=last {
                        fontdata.glyph(index).class = Some(class.to_string());
                        if class == "mark" {
                            layout.marks.insert(index);
                        }
                    }
                }
            }
        }
    }
    if markclassoffset != 0 {
        f.set(tableoffset + markclassoffset);
        let format = f.u16();
        if format == 1 {
            let first = f.u16();
            let count = f.u16();
            for index in first..first + count {
                let c = f.u16() as usize;
                layout.markclasses.entry(c).or_default().insert(index);
            }
        } else if format == 2 {
            let ranges = f.u16();
            for _ in 0..ranges {
                let first = f.u16();
                let last = f.u16();
                let c = f.u16() as usize;
                let class = layout.markclasses.entry(c).or_default();
                for index in first..=last {
                    class.insert(index);
                }
            }
        }
    }
    if marksetsoffset != 0 {
        // `#fontdata.markclasses`: the border of a table keyed by class
        // number, which for the dense 1..n a font has is n.
        let mut nofmarkclasses = 0;
        while layout.markclasses.contains_key(&(nofmarkclasses + 1)) {
            nofmarkclasses += 1;
        }
        let base = tableoffset + marksetsoffset;
        f.set(base);
        if f.u16() == 1 {
            let n = f.u16() as usize;
            let sets = f.u32s(n);
            for (i, &o) in sets.iter().enumerate() {
                if o != 0 {
                    let cov: Set = coverage_list(f, base + o as usize).into_iter().collect();
                    layout.markclasses.insert(nofmarkclasses + i + 1, cov);
                    layout.nofmarksets = layout.nofmarksets.max(i + 1);
                }
            }
        }
    }
}

/// `readscripts` (font-dsp.lua:2534-2574) for "gsub" or "gpos".
pub fn read_scripts(f: &mut Stream, fontdata: &mut FontData, layout: &mut Layout, what: &str) {
    let Some(t) = fontdata.tables.get(what).copied() else {
        return;
    };
    let tableoffset = t.offset;
    f.set(tableoffset);
    let _version = f.u32();
    let scriptoffset = tableoffset + f.u16() as usize;
    let featureoffset = tableoffset + f.u16() as usize;
    let lookupoffset = tableoffset + f.u16() as usize;

    // readscriplan: script -> language -> feature indices (1-based)
    f.set(scriptoffset);
    let nofscripts = f.u16();
    let mut scripts: Vec<(String, usize)> = Vec::new();
    for _ in 0..nofscripts {
        let s = tag(f);
        let o = scriptoffset + f.u16() as usize;
        scripts.push((s, o));
    }
    // a later duplicate tag replaces an earlier one, as the Lua table does
    let scripts: BTreeMap<String, usize> = scripts.into_iter().collect();
    let mut languages: BTreeMap<String, BTreeMap<String, Vec<usize>>> = BTreeMap::new();
    for (script, &offset) in &scripts {
        f.set(offset);
        let defaultoffset = f.u16() as usize;
        let noflanguages = f.u16();
        let mut langs: Vec<(String, usize)> = Vec::new();
        if defaultoffset > 0 {
            langs.push(("dflt".into(), offset + defaultoffset));
        }
        for _ in 0..noflanguages {
            let l = tag(f);
            let o = offset + f.u16() as usize;
            langs.push((l, o));
        }
        let mut map = BTreeMap::new();
        for (l, o) in langs {
            f.set(o);
            let _lookuporder = f.u16();
            let _required = f.u16();
            let n = f.u16();
            let indices: Vec<usize> = (0..n).map(|_| f.u16() as usize + 1).collect();
            map.insert(l, indices);
        }
        languages.insert(script.clone(), map);
    }

    // readfeatures
    f.set(featureoffset);
    let noffeatures = f.u16();
    let records: Vec<(String, usize)> = (0..noffeatures)
        .map(|_| (tag(f), f.u16() as usize))
        .collect();
    let mut features: Vec<(String, Vec<usize>)> = Vec::new();
    for (ftag, offset) in &records {
        let offset = featureoffset + offset;
        f.set(offset);
        let parameters = f.u16() as usize;
        let n = f.u16() as usize;
        let lookups: Vec<usize> = f.u16s(n).into_iter().map(|l| l as usize + 1).collect();
        if parameters > 0 && ftag == "size" && layout.designsize.is_none() {
            // plugins.size (font-dsp.lua:1936-1976)
            let check = |f: &mut Stream, at: usize| -> Option<(f64, f64, f64)> {
                f.set(at);
                let designsize = f.u16() as f64;
                if designsize > 0.0 {
                    let style = f.u16();
                    let menu = f.u16();
                    let mut minsize = f.u16() as f64;
                    let mut maxsize = f.u16() as f64;
                    if minsize == 0.0 && maxsize == 0.0 && style == 0 && menu == 0 {
                        minsize = designsize;
                        maxsize = designsize;
                    }
                    if designsize >= minsize && designsize <= maxsize {
                        return Some((minsize, maxsize, designsize));
                    }
                }
                None
            };
            let found =
                check(f, offset + parameters).or_else(|| check(f, featureoffset + parameters));
            if let Some((min, max, ds)) = found {
                layout.minsize = Some(min);
                layout.maxsize = Some(max);
                layout.designsize = Some(ds);
            }
        }
        features.push((ftag.clone(), lookups));
    }

    // reorderfeatures (font-dsp.lua:1985-2053)
    let mut scriptlangs: BTreeMap<String, BTreeMap<String, BTreeSet<String>>> = BTreeMap::new();
    let mut featurehash: BTreeMap<usize, BTreeMap<String, BTreeMap<String, BTreeSet<String>>>> =
        BTreeMap::new();
    let mut featureorder: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for (script, langs) in &languages {
        for (language, indices) in langs {
            for &index in indices {
                let Some((ftag, lookups)) = features.get(index - 1) else {
                    continue;
                };
                for &lookup in lookups {
                    let o = featureorder.entry(lookup).or_default();
                    if !o.contains(ftag) {
                        o.push(ftag.clone());
                    }
                    featurehash
                        .entry(lookup)
                        .or_default()
                        .entry(ftag.clone())
                        .or_default()
                        .entry(script.clone())
                        .or_default()
                        .insert(language.clone());
                    scriptlangs
                        .entry(ftag.clone())
                        .or_default()
                        .entry(script.clone())
                        .or_default()
                        .insert(language.clone());
                }
            }
        }
    }
    layout.features.insert(what.to_string(), scriptlangs);

    // readlookups (font-dsp.lua:2137-2174)
    let nofmarkclasses = {
        let mut n = 0;
        while layout.markclasses.contains_key(&(n + 1)) {
            n += 1;
        }
        n - layout.nofmarksets
    };
    f.set(lookupoffset);
    let noflookups = f.u16() as usize;
    let lookupoffsets = f.u16s(noflookups);
    struct Raw {
        typ: &'static str,
        flags: Flags,
        subtables: Vec<usize>,
        markclass: Option<usize>,
    }
    let types: &[&'static str] = match what {
        "gsub" => &GSUB_TYPES,
        _ => &GPOS_TYPES,
    };
    let raws: Vec<Raw> = lookupoffsets
        .iter()
        .map(|&o| {
            let offset = lookupoffset + o as usize;
            f.set(offset);
            let typebits = f.u16();
            let flagbits = f.u16();
            let n = f.u16();
            let subtables: Vec<usize> = (0..n).map(|_| offset + f.u16() as usize).collect();
            let mut markclass = None;
            if flagbits & 0x0010 != 0 {
                markclass = Some(f.u16() as usize);
            }
            let markset = (flagbits >> 8) as usize;
            if markset > 0 {
                markclass = Some(nofmarkclasses + markset);
            }
            Raw {
                typ: types
                    .get((typebits as usize).wrapping_sub(1))
                    .copied()
                    .unwrap_or("unknown"),
                flags: Flags {
                    mark: flagbits & 0x0008 != 0,
                    ligature: flagbits & 0x0004 != 0,
                    base: flagbits & 0x0002 != 0,
                    r2l: flagbits & 0x0001 != 0,
                },
                subtables,
                markclass,
            }
        })
        .collect();

    // resolvelookups (font-dsp.lua:2178-2456)
    let nofglyphs = match fontdata.nofglyphs {
        0 => fontdata.glyphs.len() as u32,
        n => n as u32,
    };
    let prefix = &what[1..2];
    let mut sublookuphash: BTreeMap<usize, usize> = BTreeMap::new();
    // lookup id -> (is sequence, index into the list)
    let mut done: BTreeMap<usize, (bool, usize)> = BTreeMap::new();
    for (k, raw) in raws.iter().enumerate() {
        let lookupid = k + 1;
        if raw.subtables.is_empty() {
            continue;
        }
        let mut typ = raw.typ;
        let mut steps = Vec::new();
        for &sub in &raw.subtables {
            let got = match what {
                "gsub" => gsub_step(f, raw.typ, sub, nofglyphs),
                _ => gpos_step(f, raw.typ, sub, nofglyphs),
            };
            if let Some((step, lt)) = got {
                typ = lt;
                steps.push(step);
            }
        }
        let typ_name = lookup_name(what, typ);
        let nofsteps = steps.len();
        match featurehash.get(&lookupid) {
            Some(features) => {
                let index = layout_list(layout, what, true).len() + 1;
                let s = Sequence {
                    index,
                    name: format!("{prefix}_s_{}", lookupid - 1),
                    typ: typ_name,
                    steps,
                    markclass: raw.markclass,
                    flags: raw.flags,
                    order: featureorder.get(&lookupid).cloned().unwrap_or_default(),
                    features: features.clone(),
                    merged: false,
                };
                let _ = nofsteps;
                layout.sequences.push(s);
                done.insert(lookupid, (true, layout.sequences.len() - 1));
            }
            None => {
                let index = layout.sublookups.len() + 1;
                layout.sublookups.push(Sequence {
                    index,
                    name: format!("{prefix}_l_{}", lookupid - 1),
                    typ: typ_name,
                    steps,
                    markclass: raw.markclass,
                    flags: raw.flags,
                    order: Vec::new(),
                    features: BTreeMap::new(),
                    merged: false,
                });
                sublookuphash.insert(lookupid, index);
                done.insert(lookupid, (false, layout.sublookups.len() - 1));
            }
        }
    }
    // The rules' lookup ids become sublookup numbers; a lookup used both on
    // its own and inside a rule gets a derived sublookup copy.
    // In lookup order, as `allsteps` was filled: the order decides the numbers
    // derived sublookups get.
    let targets: Vec<(bool, usize)> = done.values().copied().collect();
    for (is_seq, i) in targets {
        let nsteps = match is_seq {
            true => layout.sequences[i].steps.len(),
            false => layout.sublookups[i].steps.len(),
        };
        for s in 0..nsteps {
            let rules = match is_seq {
                true => layout.sequences[i].steps[s].rules.clone(),
                false => layout.sublookups[i].steps[s].rules.clone(),
            };
            let Some(mut rules) = rules else {
                continue;
            };
            for rule in rules.iter_mut() {
                let Some(rlookups) = rule.lookups.take() else {
                    continue;
                };
                if rlookups.is_empty() {
                    rule.lookups = None;
                    continue;
                }
                let mut resolved = Vec::new();
                let mut bad = false;
                for list in rlookups {
                    let Some(list) = list else {
                        resolved.push(None);
                        continue;
                    };
                    let mut found = Vec::new();
                    for lookupid in list {
                        let h = match sublookuphash.get(&lookupid) {
                            Some(&h) => Some(h),
                            None => match done.get(&lookupid) {
                                Some(&(true, si)) => {
                                    let d = layout.sequences[si].clone();
                                    let index = layout.sublookups.len() + 1;
                                    layout.sublookups.push(Sequence {
                                        index,
                                        name: format!("{prefix}_d_{}", lookupid - 1),
                                        // `d.lookuptype or "gsub_single"`: a sequence
                                        // has no `lookuptype` field, so a derived
                                        // sublookup is always typed gsub_single.
                                        typ: "gsub_single",
                                        steps: d.steps,
                                        markclass: d.markclass,
                                        flags: d.flags,
                                        order: Vec::new(),
                                        features: BTreeMap::new(),
                                        merged: false,
                                    });
                                    sublookuphash.insert(lookupid, index);
                                    Some(index)
                                }
                                _ => {
                                    bad = true;
                                    break;
                                }
                            },
                        };
                        if let Some(h) = h {
                            found.push(h);
                        }
                    }
                    if bad {
                        break;
                    }
                    resolved.push((!found.is_empty()).then_some(found));
                }
                rule.lookups = match bad {
                    true => None,
                    false => Some(resolved),
                };
            }
            match is_seq {
                true => layout.sequences[i].steps[s].rules = Some(rules),
                false => layout.sublookups[i].steps[s].rules = Some(rules),
            }
        }
    }
}

fn layout_list<'a>(layout: &'a Layout, _what: &str, seq: bool) -> &'a Vec<Sequence> {
    match seq {
        true => &layout.sequences,
        false => &layout.sublookups,
    }
}

/// `checkkerns` (font-dsp.lua:2576-2650): the old `kern` table becomes a
/// `kern` (or `globalkern`) feature -- but only when it has MORE than one
/// subtable, which is the test the Lua makes.
pub fn read_kern(f: &mut Stream, fontdata: &mut FontData, layout: &mut Layout) {
    let Some(t) = fontdata.tables.get("kern").copied() else {
        return;
    };
    let has_gpos_kern = layout
        .features
        .get("gpos")
        .is_some_and(|g| g.contains_key("kern"));
    if has_gpos_kern {
        return;
    }
    f.set(t.offset);
    let _version = f.u16();
    let noftables = f.u16();
    if noftables <= 1 {
        return;
    }
    let mut kerns: BTreeMap<u32, BTreeMap<u32, f64>> = BTreeMap::new();
    for _ in 0..noftables {
        let _v = f.u16();
        let _length = f.u16();
        let coverage = f.u16();
        if coverage >> 8 == 0 {
            let n = f.u16();
            f.skip_u16(3);
            for _ in 0..n {
                let l = f.u16();
                let r = f.u16();
                let k = f.i16() as f64;
                kerns.entry(l).or_default().insert(r, k);
            }
        }
    }
    let feature: BTreeMap<String, BTreeSet<String>> =
        [("dflt".to_string(), BTreeSet::from(["dflt".to_string()]))].into();
    layout
        .features
        .entry("gpos".into())
        .or_default()
        .insert("kern".into(), feature.clone());
    let index = layout.sequences.len() + 1;
    layout.sequences.push(Sequence {
        index,
        name: "kern".into(),
        typ: "gpos_pair",
        steps: vec![Step {
            coverage: Coverage::Kern(kerns),
            format: Some("kern"),
            rules: None,
            baseclasses: None,
        }],
        markclass: None,
        flags: Flags::default(),
        order: vec!["kern".into()],
        features: [("kern".to_string(), feature)].into(),
        merged: false,
    });
}

/// `readmathvalue`: a value and a device offset skipped.
fn mathvalue(f: &mut Stream) -> f64 {
    let v = f.i16() as f64;
    f.skip_u16(1);
    v
}

/// The MATH constants in table order (font-dsp.lua:2807-2867).
const MATH_CONSTANTS: [&str; 56] = [
    "ScriptPercentScaleDown",
    "ScriptScriptPercentScaleDown",
    "DelimitedSubFormulaMinHeight",
    "DisplayOperatorMinHeight",
    "MathLeading",
    "AxisHeight",
    "AccentBaseHeight",
    "FlattenedAccentBaseHeight",
    "SubscriptShiftDown",
    "SubscriptTopMax",
    "SubscriptBaselineDropMin",
    "SuperscriptShiftUp",
    "SuperscriptShiftUpCramped",
    "SuperscriptBottomMin",
    "SuperscriptBaselineDropMax",
    "SubSuperscriptGapMin",
    "SuperscriptBottomMaxWithSubscript",
    "SpaceAfterScript",
    "UpperLimitGapMin",
    "UpperLimitBaselineRiseMin",
    "LowerLimitGapMin",
    "LowerLimitBaselineDropMin",
    "StackTopShiftUp",
    "StackTopDisplayStyleShiftUp",
    "StackBottomShiftDown",
    "StackBottomDisplayStyleShiftDown",
    "StackGapMin",
    "StackDisplayStyleGapMin",
    "StretchStackTopShiftUp",
    "StretchStackBottomShiftDown",
    "StretchStackGapAboveMin",
    "StretchStackGapBelowMin",
    "FractionNumeratorShiftUp",
    "FractionNumeratorDisplayStyleShiftUp",
    "FractionDenominatorShiftDown",
    "FractionDenominatorDisplayStyleShiftDown",
    "FractionNumeratorGapMin",
    "FractionNumeratorDisplayStyleGapMin",
    "FractionRuleThickness",
    "FractionDenominatorGapMin",
    "FractionDenominatorDisplayStyleGapMin",
    "SkewedFractionHorizontalGap",
    "SkewedFractionVerticalGap",
    "OverbarVerticalGap",
    "OverbarRuleThickness",
    "OverbarExtraAscender",
    "UnderbarVerticalGap",
    "UnderbarRuleThickness",
    "UnderbarExtraDescender",
    "RadicalVerticalGap",
    "RadicalDisplayStyleVerticalGap",
    "RadicalRuleThickness",
    "RadicalExtraAscender",
    "RadicalKernBeforeDegree",
    "RadicalKernAfterDegree",
    "RadicalDegreeBottomRaisePercent",
];

/// `readers.math` (font-dsp.lua:3084-3107).
pub fn read_math(f: &mut Stream, fontdata: &mut FontData, layout: &mut Layout) {
    let Some(t) = fontdata.tables.get("math").copied() else {
        return;
    };
    let tableoffset = t.offset;
    f.set(tableoffset);
    let _version = f.u32();
    let constants = f.u16() as usize;
    let glyphinfo = f.u16() as usize;
    let variants = f.u16() as usize;
    if constants != 0 {
        f.set(tableoffset + constants);
        let mut c = BTreeMap::new();
        for (k, name) in MATH_CONSTANTS.iter().enumerate() {
            let v = match k {
                0 | 1 | 55 => f.i16() as f64,
                2 | 3 => f.u16() as f64,
                _ => mathvalue(f),
            };
            c.insert(*name, v);
        }
        layout.mathconstants = Some(c);
    }
    if glyphinfo != 0 {
        let offset = tableoffset + glyphinfo;
        f.set(offset);
        let italics = f.u16() as usize;
        let accents = f.u16() as usize;
        let _extensions = f.u16();
        let kerns = f.u16() as usize;
        for (which, sub) in [(0, italics), (1, accents)] {
            if sub == 0 {
                continue;
            }
            f.set(offset + sub);
            let cov = f.u16() as usize;
            let n = f.u16();
            let coverage = coverage_list(f, offset + sub + cov);
            f.set(offset + sub + 4);
            for i in 0..n as usize {
                let v = mathvalue(f);
                if v != 0.0 {
                    if let Some(&g) = coverage.get(i) {
                        let m = layout.math.entry(g).or_default();
                        match which {
                            0 => m.italic = Some(v),
                            _ => m.accent = Some(v),
                        }
                    }
                }
            }
            if which == 0 {
                layout.hasitalics = true;
            }
        }
        if kerns != 0 {
            let kernoffset = offset + kerns;
            f.set(kernoffset);
            let cov = f.u16() as usize;
            let n = f.u16() as usize;
            if n > 0 {
                let sets: Vec<[u32; 4]> = (0..n)
                    .map(|_| [f.u16(), f.u16(), f.u16(), f.u16()])
                    .collect();
                let coverage = coverage_list(f, kernoffset + cov);
                let get = |f: &mut Stream, o: u32| -> Option<Vec<MathKern>> {
                    if o == 0 {
                        return None;
                    }
                    f.set(kernoffset + o as usize);
                    let n = f.u16() as usize;
                    if n == 0 {
                        let k = mathvalue(f);
                        return (k != 0.0).then(|| {
                            vec![MathKern {
                                height: None,
                                kern: k,
                            }]
                        });
                    }
                    let heights: Vec<f64> = (0..n).map(|_| mathvalue(f)).collect();
                    let mut l: Vec<MathKern> = heights
                        .into_iter()
                        .map(|h| MathKern {
                            height: Some(h),
                            kern: 0.0,
                        })
                        .collect();
                    for k in l.iter_mut() {
                        k.kern = mathvalue(f);
                    }
                    l.push(MathKern {
                        height: None,
                        kern: mathvalue(f),
                    });
                    Some(l)
                };
                for (i, s) in sets.iter().enumerate() {
                    if s.iter().all(|o| *o == 0) {
                        continue;
                    }
                    let k = MathKerns {
                        topright: get(f, s[0]),
                        topleft: get(f, s[1]),
                        bottomright: get(f, s[2]),
                        bottomleft: get(f, s[3]),
                    };
                    let any = k.topright.is_some()
                        || k.topleft.is_some()
                        || k.bottomright.is_some()
                        || k.bottomleft.is_some();
                    if any {
                        if let Some(&g) = coverage.get(i) {
                            layout.math.entry(g).or_default().kerns = Some(k);
                        }
                    }
                }
            }
        }
    }
    if variants != 0 {
        let offset = tableoffset + variants;
        f.set(offset);
        let minoverlap = f.u16() as f64;
        let vcoverage = f.u16() as usize;
        let hcoverage = f.u16() as usize;
        let vn = f.u16() as usize;
        let hn = f.u16() as usize;
        let vcons = f.u16s(vn);
        let hcons = f.u16s(hn);
        if let Some(c) = layout.mathconstants.as_mut() {
            c.insert("MinConnectorOverlap", minoverlap);
        }
        for (vertical, cov, n, cons) in [
            (true, vcoverage, vn, &vcons),
            (false, hcoverage, hn, &hcons),
        ] {
            if cov == 0 || n == 0 {
                continue;
            }
            let coverage = coverage_list(f, offset + cov);
            for i in 0..n {
                let c = cons[i] as usize;
                if c == 0 {
                    continue;
                }
                let Some(&index) = coverage.get(i) else {
                    continue;
                };
                f.set(offset + c);
                let assembly = f.u16() as usize;
                let nofvariants = f.u16();
                let mut variants = Vec::new();
                for _ in 0..nofvariants {
                    let v = f.u16();
                    if v != index {
                        variants.push(v);
                    }
                    f.skip_u16(1);
                }
                if !variants.is_empty() {
                    let m = layout.math.entry(index).or_default();
                    match vertical {
                        true => m.vvariants = Some(variants),
                        false => m.hvariants = Some(variants),
                    }
                }
                if assembly != 0 {
                    f.set(offset + c + assembly);
                    let italic = mathvalue(f);
                    let nofparts = f.u16();
                    let parts: Vec<MathPart> = (0..nofparts)
                        .map(|_| {
                            let glyph = f.u16();
                            let start = f.u16() as f64;
                            let end = f.u16() as f64;
                            let advance = f.u16() as f64;
                            let flags = f.u16();
                            MathPart {
                                glyph,
                                start,
                                end,
                                advance,
                                extender: flags & 1 != 0,
                            }
                        })
                        .collect();
                    let m = layout.math.entry(index).or_default();
                    match vertical {
                        true => {
                            m.vparts = Some(parts);
                            if italic != 0.0 {
                                m.vitalic = Some(italic);
                            }
                        }
                        false => {
                            m.hparts = Some(parts);
                            if italic != 0.0 {
                                m.hitalic = Some(italic);
                            }
                        }
                    }
                }
            }
        }
    }
}
