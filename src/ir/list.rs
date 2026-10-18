//! The list actions a program carries: `main_control`'s cases (§1045) after
//! the lowerer has dispatched them, for `crate::stomach` to carry out.
//!
//! A [`ListOp`] is generic over what a number is. While lowering it is a
//! [`Num`] -- `\kern\dimen0` is a kern whose width the program decides --
//! and when the program runs it is the `i64` that `Num` evaluated to. One
//! enum for both halves means the compiler's encoding and the runtime's
//! decoding are written against the same fields, and [`ListOp::visit`] and
//! [`ListOp::decode`] are the only two places that know the order of them.

use super::Num;
use crate::node::LeaderKind;
use crate::stomach::build::Removed;
use crate::stomach::{BoxDimen, BoxKind};

/// Where a finished box goes (§1071's box context), with the shift a run-time
/// number.
#[derive(Clone, Debug, PartialEq)]
pub enum Context<N = Num> {
    /// Onto the current list, shifted (`\raise` and its siblings, or 0).
    Shift(N),
    /// `\setbox n`, or `\global\setbox n`.
    SetBox { n: u16, global: bool },
    /// `\shipout`.
    ShipOut,
    /// `\leaders`, `\cleaders`, `\xleaders`.
    Leaders(LeaderKind),
}

/// Where §1079's `begin_box` takes a box from when it does not build one.
#[derive(Clone, Debug, PartialEq)]
pub enum Fetch<N = Num> {
    Box(u16),
    Copy(u16),
    LastBox,
    /// `\vsplit n to h`.
    Vsplit(u16, N),
}

/// One list action.
#[derive(Clone, Debug, PartialEq)]
pub enum ListOp<N = Num> {
    /// §1257's `new_font`: internal font `f` is the file `name` at `size`
    /// scaled points, and `id` is the control sequence that named it.
    DefineFont {
        f: u16,
        name: String,
        id: String,
        size: i64,
    },
    /// §236's integer parameters that changed since the stomach last heard,
    /// by index into `crate::intpar::NAMES`.
    IntPars(Vec<(u16, i64)>),
    /// §230's `\sfcode`s that changed since the stomach last heard.
    SfCodes(Vec<(char, i64)>),
    /// §1034-§1040: a run of characters in font `f`, as the main loop reads
    /// one, with `\noboundary` before (`cancel_left`) or after (`no_right`).
    Chars {
        f: u16,
        text: String,
        cancel_left: bool,
        no_right: bool,
    },
    /// §1041-§1044: an interword space in font `f`; `normal` is
    /// `append_normal_space`, which ignores the space factor (§1045's
    /// `\ ` and `hmode+spacer` when `space_factor=1000`).
    Space { f: u16, normal: bool },
    /// `append_glue` (§1060): natural, stretch, shrink and the packed
    /// orders, as a glue register's slots hold them.
    Glue {
        spec: [N; 4],
        mu: bool,
        leaders: Option<LeaderKind>,
    },
    /// `append_kern` (§1061).
    Kern(N),
    /// `append_penalty` (§1103).
    Penalty(N),
    /// §1056: a rule with the dimensions §463's `scan_rule_spec` found;
    /// `None` is the default for the kind. `leader` puts it where a leader
    /// box is wanted (§1084) rather than on the list.
    Rule {
        vrule: bool,
        width: Option<N>,
        height: Option<N>,
        depth: Option<N>,
        leader: bool,
    },
    /// §1083: open a box's list; `to` is `\hbox to`, else `spread` (zero for
    /// a natural box).
    BeginBox {
        kind: BoxKind,
        exactly: bool,
        size: N,
        context: Context<N>,
    },
    /// §1086's `d:=box_max_depth`, read before the group ends.
    BoxMaxDepth(N),
    /// §1086's `package`, after the group has ended.
    EndBox,
    /// §1079's other four cases.
    Fetch { from: Fetch<N>, context: Context<N> },
    /// `unpackage` (§1110).
    Unpackage { n: u16, copy: bool },
    /// `delete_last` (§1105).
    DeleteLast(Removed),
    /// `make_mark` (§1101), its text expanded.
    Mark(String),
    /// `\/` (§1113).
    ItalicCorrection,
    /// `\-` in font `f` (§1117).
    Hyphen(u16),
    /// `\discretionary` (§1117).
    BeginDiscretionary,
    /// The `}` of one of its three lists (§1119).
    EndDiscList(u8),
    /// `\insert n` or `\vadjust` (255) (§1099).
    BeginInsert(u16),
    /// §1100: its `}`, with `\splittopskip`, `\splitmaxdepth` and
    /// `\floatingpenalty` as they stood before the group ended.
    EndInsert {
        split_top_skip: [N; 4],
        split_max_depth: N,
        floating_penalty: N,
    },
    /// A group's box-register save level (§274, §281).
    GroupBegin,
    GroupEnd,
    /// `\wd n=`, `\ht n=`, `\dp n=` (§1247): the box changes in place.
    SetBoxDimen { n: u16, which: BoxDimen, value: N },
    /// `\prevdepth=` and `\spacefactor=` (§1242-§1243), `\prevgraf=`
    /// (§1244).
    SetPrevDepth(N),
    SetSpaceFactor(N),
    SetPrevGraf(N),
    /// §1253: `\fontdimen n f=`.
    SetFontDimen { f: u16, n: u16, value: N },
    /// §1253: `\hyphenchar f=` and `\skewchar f=`.
    SetHyphenChar { f: u16, value: N },
    SetSkewChar { f: u16, value: N },
    /// `new_graf` (§1091): open a paragraph, indented or not.
    NewGraf(bool),
    /// `end_graf` (§1096): break the paragraph and go back to the vertical
    /// list.
    EndGraf,
    /// `indent_in_hmode` (§1093).
    Indent,
    /// §1094's `if mode>0 then build_page`, after `\par` in vertical mode.
    BuildPage,
    /// `\end` (§1054): ship what is left until `its_all_over`.
    End,
    /// Whether `\output` is non-empty, and the routine to run: the index of
    /// the program's sub-chunk the lowerer compiled it into, or -1.
    OutputRoutine(i64),
    /// The characters `\number`, `\romannumeral` or `\the` makes of a
    /// register only the program holds (§465, §470), set in font `f` as the main
    /// loop sets characters: `form` 0 is a number, 1 roman numerals, 2 a
    /// dimension, 3 a glue (all four values), 4 a math glue.
    Converted { f: u16, form: u8, values: [N; 4] },
    /// §960 and §934: `\patterns{...}` and `\hyphenation{...}`, as the text that
    /// was between the braces, for the hyphenation pass to read.
    Patterns(String),
    Hyphenation(String),
}

/// One argument as the compiler pushes it.
pub enum Arg<'a, N> {
    Num(&'a N),
    Int(i64),
    Str(&'a str),
}

/// One argument as the runtime pops it.
#[derive(Clone, Debug)]
pub enum Popped {
    Int(i64),
    Str(String),
}

fn kind_code(k: BoxKind) -> i64 {
    match k {
        BoxKind::H => 0,
        BoxKind::V => 1,
        BoxKind::Top => 2,
    }
}

fn leader_code(k: LeaderKind) -> i64 {
    match k {
        LeaderKind::Normal => 0,
        LeaderKind::Aligned => 1,
        LeaderKind::Centred => 2,
        LeaderKind::Expanded => 3,
    }
}

fn leader_of(c: i64) -> LeaderKind {
    match c {
        1 => LeaderKind::Aligned,
        2 => LeaderKind::Centred,
        3 => LeaderKind::Expanded,
        _ => LeaderKind::Normal,
    }
}

fn dimen_code(d: BoxDimen) -> i64 {
    match d {
        BoxDimen::Wd => 0,
        BoxDimen::Ht => 1,
        BoxDimen::Dp => 2,
    }
}

fn removed_code(r: Removed) -> i64 {
    match r {
        Removed::Penalty => 0,
        Removed::Kern => 1,
        Removed::Glue => 2,
    }
}

impl<N> ListOp<N> {
    /// The opcode the runtime dispatches on.
    pub fn code(&self) -> i64 {
        match self {
            ListOp::DefineFont { .. } => 0,
            ListOp::IntPars(_) => 1,
            ListOp::SfCodes(_) => 2,
            ListOp::Chars { .. } => 3,
            ListOp::Space { .. } => 4,
            ListOp::Glue { .. } => 5,
            ListOp::Kern(_) => 6,
            ListOp::Penalty(_) => 7,
            ListOp::Rule { .. } => 8,
            ListOp::BeginBox { .. } => 9,
            ListOp::BoxMaxDepth(_) => 10,
            ListOp::EndBox => 11,
            ListOp::Fetch { .. } => 12,
            ListOp::Unpackage { .. } => 13,
            ListOp::DeleteLast(_) => 14,
            ListOp::Mark(_) => 15,
            ListOp::ItalicCorrection => 16,
            ListOp::Hyphen(_) => 17,
            ListOp::BeginDiscretionary => 18,
            ListOp::EndDiscList(_) => 19,
            ListOp::BeginInsert(_) => 20,
            ListOp::EndInsert { .. } => 21,
            ListOp::GroupBegin => 22,
            ListOp::GroupEnd => 23,
            ListOp::SetBoxDimen { .. } => 24,
            ListOp::SetPrevDepth(_) => 25,
            ListOp::SetSpaceFactor(_) => 26,
            ListOp::SetPrevGraf(_) => 27,
            ListOp::SetFontDimen { .. } => 28,
            ListOp::SetHyphenChar { .. } => 29,
            ListOp::SetSkewChar { .. } => 30,
            ListOp::NewGraf(_) => 31,
            ListOp::EndGraf => 32,
            ListOp::Indent => 33,
            ListOp::BuildPage => 34,
            ListOp::End => 35,
            ListOp::OutputRoutine(_) => 36,
            ListOp::Converted { .. } => 37,
            ListOp::Patterns(_) => 38,
            ListOp::Hyphenation(_) => 39,
        }
    }

    /// Every argument, in the order the compiler pushes them and
    /// [`ListOp::decode`] reads them back.
    pub fn visit<'a>(&'a self, f: &mut impl FnMut(Arg<'a, N>)) {
        let context = |c: &'a Context<N>, f: &mut dyn FnMut(Arg<'a, N>)| match c {
            Context::Shift(s) => {
                f(Arg::Int(0));
                f(Arg::Num(s));
            }
            Context::SetBox { n, global } => {
                f(Arg::Int(1));
                f(Arg::Int(i64::from(*n) * 2 + i64::from(*global)));
            }
            Context::ShipOut => {
                f(Arg::Int(2));
                f(Arg::Int(0));
            }
            Context::Leaders(k) => {
                f(Arg::Int(3));
                f(Arg::Int(leader_code(*k)));
            }
        };
        let opt = |o: &'a Option<N>, f: &mut dyn FnMut(Arg<'a, N>)| match o {
            Some(n) => {
                f(Arg::Int(1));
                f(Arg::Num(n));
            }
            None => {
                f(Arg::Int(0));
                f(Arg::Int(0));
            }
        };
        match self {
            ListOp::DefineFont { f: font, name, id, size } => {
                f(Arg::Int(i64::from(*font)));
                f(Arg::Str(name));
                f(Arg::Str(id));
                f(Arg::Int(*size));
            }
            ListOp::IntPars(v) => {
                for (i, x) in v {
                    f(Arg::Int(i64::from(*i)));
                    f(Arg::Int(*x));
                }
            }
            ListOp::SfCodes(v) => {
                for (c, x) in v {
                    f(Arg::Int(i64::from(u32::from(*c))));
                    f(Arg::Int(*x));
                }
            }
            ListOp::Chars {
                f: font,
                text,
                cancel_left,
                no_right,
            } => {
                f(Arg::Int(i64::from(*font)));
                f(Arg::Str(text));
                f(Arg::Int(i64::from(*cancel_left) + 2 * i64::from(*no_right)));
            }
            ListOp::Space { f: font, normal } => {
                f(Arg::Int(i64::from(*font)));
                f(Arg::Int(i64::from(*normal)));
            }
            ListOp::Glue { spec, mu, leaders } => {
                for n in spec {
                    f(Arg::Num(n));
                }
                f(Arg::Int(i64::from(*mu)));
                f(Arg::Int(leaders.map_or(-1, leader_code)));
            }
            ListOp::Kern(n)
            | ListOp::Penalty(n)
            | ListOp::BoxMaxDepth(n)
            | ListOp::SetPrevDepth(n)
            | ListOp::SetSpaceFactor(n)
            | ListOp::SetPrevGraf(n) => f(Arg::Num(n)),
            ListOp::Rule {
                vrule,
                width,
                height,
                depth,
                leader,
            } => {
                f(Arg::Int(i64::from(*vrule) + 2 * i64::from(*leader)));
                opt(width, f);
                opt(height, f);
                opt(depth, f);
            }
            ListOp::BeginBox {
                kind,
                exactly,
                size,
                context: c,
            } => {
                f(Arg::Int(kind_code(*kind) * 2 + i64::from(*exactly)));
                f(Arg::Num(size));
                context(c, f);
            }
            ListOp::Fetch { from, context: c } => {
                match from {
                    Fetch::Box(n) => {
                        f(Arg::Int(0));
                        f(Arg::Int(i64::from(*n)));
                    }
                    Fetch::Copy(n) => {
                        f(Arg::Int(1));
                        f(Arg::Int(i64::from(*n)));
                    }
                    Fetch::LastBox => {
                        f(Arg::Int(2));
                        f(Arg::Int(0));
                    }
                    Fetch::Vsplit(n, _) => {
                        f(Arg::Int(3));
                        f(Arg::Int(i64::from(*n)));
                    }
                }
                match from {
                    Fetch::Vsplit(_, h) => f(Arg::Num(h)),
                    _ => f(Arg::Int(0)),
                }
                context(c, f);
            }
            ListOp::Unpackage { n, copy } => f(Arg::Int(i64::from(*n) * 2 + i64::from(*copy))),
            ListOp::DeleteLast(r) => f(Arg::Int(removed_code(*r))),
            ListOp::Mark(text) | ListOp::Patterns(text) | ListOp::Hyphenation(text) => {
                f(Arg::Str(text))
            }
            ListOp::Hyphen(font) => f(Arg::Int(i64::from(*font))),
            ListOp::EndDiscList(k) => f(Arg::Int(i64::from(*k))),
            ListOp::BeginInsert(n) => f(Arg::Int(i64::from(*n))),
            ListOp::EndInsert {
                split_top_skip,
                split_max_depth,
                floating_penalty,
            } => {
                for n in split_top_skip {
                    f(Arg::Num(n));
                }
                f(Arg::Num(split_max_depth));
                f(Arg::Num(floating_penalty));
            }
            ListOp::SetBoxDimen { n, which, value } => {
                f(Arg::Int(i64::from(*n) * 4 + dimen_code(*which)));
                f(Arg::Num(value));
            }
            ListOp::SetFontDimen { f: font, n, value } => {
                f(Arg::Int(i64::from(*font)));
                f(Arg::Int(i64::from(*n)));
                f(Arg::Num(value));
            }
            ListOp::SetHyphenChar { f: font, value } | ListOp::SetSkewChar { f: font, value } => {
                f(Arg::Int(i64::from(*font)));
                f(Arg::Num(value));
            }
            ListOp::NewGraf(indent) => f(Arg::Int(i64::from(*indent))),
            ListOp::OutputRoutine(k) => f(Arg::Int(*k)),
            ListOp::Converted { f: font, form, values } => {
                f(Arg::Int(i64::from(*font)));
                f(Arg::Int(i64::from(*form)));
                for v in values {
                    f(Arg::Num(v));
                }
            }
            ListOp::ItalicCorrection
            | ListOp::BeginDiscretionary
            | ListOp::EndBox
            | ListOp::GroupBegin
            | ListOp::GroupEnd
            | ListOp::EndGraf
            | ListOp::Indent
            | ListOp::BuildPage
            | ListOp::End => {}
        }
    }
}

/// A reader over the popped arguments, in order.
struct Reader<'a> {
    args: &'a [Popped],
    at: usize,
}

impl Reader<'_> {
    fn int(&mut self) -> i64 {
        let v = match self.args.get(self.at) {
            Some(Popped::Int(v)) => *v,
            _ => 0,
        };
        self.at += 1;
        v
    }

    fn string(&mut self) -> String {
        let v = match self.args.get(self.at) {
            Some(Popped::Str(s)) => s.clone(),
            Some(Popped::Int(v)) => v.to_string(),
            None => String::new(),
        };
        self.at += 1;
        v
    }

    fn u16(&mut self) -> u16 {
        u16::try_from(self.int()).unwrap_or(0)
    }

    fn context(&mut self) -> Context<i64> {
        let tag = self.int();
        let v = self.int();
        match tag {
            1 => Context::SetBox {
                n: u16::try_from(v / 2).unwrap_or(0),
                global: v % 2 == 1,
            },
            2 => Context::ShipOut,
            3 => Context::Leaders(leader_of(v)),
            _ => Context::Shift(v),
        }
    }

    fn opt(&mut self) -> Option<i64> {
        let present = self.int();
        let v = self.int();
        (present == 1).then_some(v)
    }

    fn rest(&self) -> usize {
        self.args.len().saturating_sub(self.at)
    }
}

impl ListOp<i64> {
    /// The action `code` with the arguments the compiler pushed for it, now
    /// evaluated. `None` for a code no compiler wrote.
    pub fn decode(code: i64, args: &[Popped]) -> Option<ListOp<i64>> {
        let mut r = Reader { args, at: 0 };
        Some(match code {
            0 => ListOp::DefineFont {
                f: r.u16(),
                name: r.string(),
                id: r.string(),
                size: r.int(),
            },
            1 => {
                let mut v = Vec::new();
                while r.rest() >= 2 {
                    v.push((r.u16(), r.int()));
                }
                ListOp::IntPars(v)
            }
            2 => {
                let mut v = Vec::new();
                while r.rest() >= 2 {
                    let c = char::from_u32(r.int() as u32).unwrap_or('\0');
                    v.push((c, r.int()));
                }
                ListOp::SfCodes(v)
            }
            3 => {
                let f = r.u16();
                let text = r.string();
                let flags = r.int();
                ListOp::Chars {
                    f,
                    text,
                    cancel_left: flags & 1 == 1,
                    no_right: flags & 2 == 2,
                }
            }
            4 => ListOp::Space {
                f: r.u16(),
                normal: r.int() == 1,
            },
            5 => {
                let spec = [r.int(), r.int(), r.int(), r.int()];
                let mu = r.int() == 1;
                let leaders = match r.int() {
                    -1 => None,
                    k => Some(leader_of(k)),
                };
                ListOp::Glue { spec, mu, leaders }
            }
            6 => ListOp::Kern(r.int()),
            7 => ListOp::Penalty(r.int()),
            8 => {
                let flags = r.int();
                ListOp::Rule {
                    vrule: flags & 1 == 1,
                    leader: flags & 2 == 2,
                    width: r.opt(),
                    height: r.opt(),
                    depth: r.opt(),
                }
            }
            9 => {
                let k = r.int();
                let kind = match k / 2 {
                    1 => BoxKind::V,
                    2 => BoxKind::Top,
                    _ => BoxKind::H,
                };
                ListOp::BeginBox {
                    kind,
                    exactly: k % 2 == 1,
                    size: r.int(),
                    context: r.context(),
                }
            }
            10 => ListOp::BoxMaxDepth(r.int()),
            11 => ListOp::EndBox,
            12 => {
                let tag = r.int();
                let n = r.u16();
                let h = r.int();
                let from = match tag {
                    1 => Fetch::Copy(n),
                    2 => Fetch::LastBox,
                    3 => Fetch::Vsplit(n, h),
                    _ => Fetch::Box(n),
                };
                ListOp::Fetch {
                    from,
                    context: r.context(),
                }
            }
            13 => {
                let v = r.int();
                ListOp::Unpackage {
                    n: u16::try_from(v / 2).unwrap_or(0),
                    copy: v % 2 == 1,
                }
            }
            14 => ListOp::DeleteLast(match r.int() {
                0 => Removed::Penalty,
                1 => Removed::Kern,
                _ => Removed::Glue,
            }),
            15 => ListOp::Mark(r.string()),
            16 => ListOp::ItalicCorrection,
            17 => ListOp::Hyphen(r.u16()),
            18 => ListOp::BeginDiscretionary,
            19 => ListOp::EndDiscList(u8::try_from(r.int()).unwrap_or(2)),
            20 => ListOp::BeginInsert(r.u16()),
            21 => ListOp::EndInsert {
                split_top_skip: [r.int(), r.int(), r.int(), r.int()],
                split_max_depth: r.int(),
                floating_penalty: r.int(),
            },
            22 => ListOp::GroupBegin,
            23 => ListOp::GroupEnd,
            24 => {
                let v = r.int();
                let which = match v % 4 {
                    1 => BoxDimen::Ht,
                    2 => BoxDimen::Dp,
                    _ => BoxDimen::Wd,
                };
                ListOp::SetBoxDimen {
                    n: u16::try_from(v / 4).unwrap_or(0),
                    which,
                    value: r.int(),
                }
            }
            25 => ListOp::SetPrevDepth(r.int()),
            26 => ListOp::SetSpaceFactor(r.int()),
            27 => ListOp::SetPrevGraf(r.int()),
            28 => ListOp::SetFontDimen {
                f: r.u16(),
                n: r.u16(),
                value: r.int(),
            },
            29 => ListOp::SetHyphenChar {
                f: r.u16(),
                value: r.int(),
            },
            30 => ListOp::SetSkewChar {
                f: r.u16(),
                value: r.int(),
            },
            31 => ListOp::NewGraf(r.int() == 1),
            32 => ListOp::EndGraf,
            33 => ListOp::Indent,
            34 => ListOp::BuildPage,
            35 => ListOp::End,
            36 => ListOp::OutputRoutine(r.int()),
            38 => ListOp::Patterns(r.string()),
            39 => ListOp::Hyphenation(r.string()),
            37 => ListOp::Converted {
                f: r.u16(),
                form: u8::try_from(r.int()).unwrap_or(0),
                values: [r.int(), r.int(), r.int(), r.int()],
            },
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the compiler would push for `op`, with every `Num` a literal,
    /// as the runtime pops it.
    fn pushed(op: &ListOp<Num>) -> Vec<Popped> {
        let mut out = Vec::new();
        op.visit(&mut |a| {
            out.push(match a {
                Arg::Num(Num::Literal(v)) => Popped::Int(*v),
                Arg::Num(_) => panic!("only literals here"),
                Arg::Int(v) => Popped::Int(v),
                Arg::Str(s) => Popped::Str(s.to_string()),
            })
        });
        out
    }

    /// The encoding and the decoding are written separately, so every
    /// shape is pushed and read back: a field read out of order would
    /// decode to a different action rather than fail.
    #[test]
    fn every_action_survives_the_trip_through_the_vm() {
        let l = Num::Literal;
        let ops: Vec<(ListOp<Num>, ListOp<i64>)> = vec![
            (
                ListOp::DefineFont {
                    f: 3,
                    name: "cmr10".into(),
                    id: "tenrm".into(),
                    size: 655360,
                },
                ListOp::DefineFont {
                    f: 3,
                    name: "cmr10".into(),
                    id: "tenrm".into(),
                    size: 655360,
                },
            ),
            (ListOp::IntPars(vec![(1, 200), (26, -5)]), ListOp::IntPars(vec![(1, 200), (26, -5)])),
            (
                ListOp::Chars {
                    f: 1,
                    text: "abc".into(),
                    cancel_left: false,
                    no_right: true,
                },
                ListOp::Chars {
                    f: 1,
                    text: "abc".into(),
                    cancel_left: false,
                    no_right: true,
                },
            ),
            (
                ListOp::Glue {
                    spec: [l(1), l(2), l(3), l(5)],
                    mu: false,
                    leaders: Some(LeaderKind::Centred),
                },
                ListOp::Glue {
                    spec: [1, 2, 3, 5],
                    mu: false,
                    leaders: Some(LeaderKind::Centred),
                },
            ),
            (
                ListOp::Rule {
                    vrule: true,
                    width: None,
                    height: Some(l(7)),
                    depth: None,
                    leader: true,
                },
                ListOp::Rule {
                    vrule: true,
                    width: None,
                    height: Some(7),
                    depth: None,
                    leader: true,
                },
            ),
            (
                ListOp::BeginBox {
                    kind: BoxKind::Top,
                    exactly: true,
                    size: l(9),
                    context: Context::SetBox { n: 255, global: true },
                },
                ListOp::BeginBox {
                    kind: BoxKind::Top,
                    exactly: true,
                    size: 9,
                    context: Context::SetBox { n: 255, global: true },
                },
            ),
            (
                ListOp::Fetch {
                    from: Fetch::Vsplit(4, l(11)),
                    context: Context::Shift(l(-3)),
                },
                ListOp::Fetch {
                    from: Fetch::Vsplit(4, 11),
                    context: Context::Shift(-3),
                },
            ),
            (
                ListOp::SetBoxDimen {
                    n: 17,
                    which: BoxDimen::Dp,
                    value: l(12),
                },
                ListOp::SetBoxDimen {
                    n: 17,
                    which: BoxDimen::Dp,
                    value: 12,
                },
            ),
            (ListOp::Unpackage { n: 9, copy: true }, ListOp::Unpackage { n: 9, copy: true }),
        ];
        for (op, want) in ops {
            assert_eq!(ListOp::decode(op.code(), &pushed(&op)), Some(want));
        }
    }
}
