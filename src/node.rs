//! The node list: what a TeX list is made of, once the mouth and the expander
//! have finished with it.
//!
//! `tex.web` §133-§161. Everything the stomach packages, breaks and ships is a
//! list of these; a box is a node that HOLDS such a list, which is what makes
//! boxes nest. The types are numbered here exactly as `tex.web` numbers them
//! (§135-§159), because two of TeX's rules are stated as arithmetic on that
//! number rather than as a case analysis:
//!
//! - `precedes_break(p) == type(p) < math_node` (§148): glue is a legal
//!   breakpoint only when what precedes it is not itself discardable.
//! - `if type(p)>=rule_node then s:=0 else s:=shift_amount(p)` (§653): only a
//!   box carries a shift, and `hpack` asks the number rather than the variant.
//!
//! Writing those two as a match over variants would be a rephrasing, and a
//! rephrasing is where a port stops being one. So `type_code` exists and the
//! rules are the comparisons Knuth wrote.
//!
//! Dimensions are scaled points throughout (`crate::dimen`), never floats: a
//! box's width is an integer in TeX and the arithmetic on it is exact. The one
//! float in the whole structure is `glue_set`, and it is a float in `tex.web`
//! too (§109's `glue_ratio`).

use crate::glue::{Glue, Order};

/// A dimension, in scaled points.
pub type Scaled = i64;

/// `null_flag` (§2951): a dimension that is not there.
///
/// A rule written `\hrule` with no height has a RUNNING height, which means
/// "as tall as the box that ends up holding me". It is carried as $-2^{30}$
/// precisely so that the `max` in `hpack` ignores it without a test (§653).
pub const NULL_FLAG: Scaled = -0x4000_0000;

/// `max_dimen` (§421): the largest dimension TeX will hold, $2^{30}-1$.
pub const MAX_DIMEN: Scaled = 0x3FFF_FFFF;

/// `inf_bad` (§108): a badness that means "cannot be set".
pub const INF_BAD: i64 = 10000;

/// `inf_penalty` (§157): a penalty at or above this forbids a break.
pub const INF_PENALTY: i64 = INF_BAD;

/// `eject_penalty` (§157): a penalty at or below this forces one.
pub const EJECT_PENALTY: i64 = -INF_PENALTY;

/// `awful_bad` (§833): more than a billion demerits — worse than any real
/// break.
pub const AWFUL_BAD: i64 = 0x3FFF_FFFF;

/// `deplorable` (§974): more than `inf_bad`, less than `awful_bad`.
pub const DEPLORABLE: i64 = 100_000;

/// `ignore_depth` (§212): the `\prevdepth` that means "no baseline yet", so
/// the first box on a vertical list gets no interline glue above it.
pub const IGNORE_DEPTH: Scaled = -65_536_000;

/// Whether a box's glue is being stretched, shrunk, or neither (§135).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GlueSign {
    #[default]
    Normal,
    Stretching,
    Shrinking,
}

/// What a glue node is: ordinary glue, or one of the three kinds of leaders
/// (§149). The numbering is `tex.web`'s subtype, and the ORDER matters:
/// `subtype(p)>=a_leaders` is how §656 asks whether there is a leader box to
/// take dimensions from.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LeaderKind {
    /// Ordinary glue.
    #[default]
    Normal,
    /// `\leaders`: copies aligned to a grid fixed by the enclosing box.
    Aligned,
    /// `\cleaders`: copies centred in the space.
    Centred,
    /// `\xleaders`: copies with the leftover space spread between them.
    Expanded,
}

impl LeaderKind {
    /// `subtype(p)>=a_leaders` (§149).
    pub fn is_leaders(self) -> bool {
        self != LeaderKind::Normal
    }
}

/// Which `\skip` parameter a glue node came from (§149: "the subtype is set to
/// indicate the source of glue"). Carried because the page builder and
/// `\showlists` both name it, not because the arithmetic reads it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GlueSource {
    #[default]
    Normal,
    LineSkip,
    BaselineSkip,
    ParSkip,
    LeftSkip,
    RightSkip,
    TopSkip,
    SplitTopSkip,
    ParFillSkip,
}

/// A box: an hlist or a vlist, packaged (§135-§137).
#[derive(Clone, Debug, Default)]
pub struct BoxNode {
    pub width: Scaled,
    pub depth: Scaled,
    pub height: Scaled,
    /// How far down (hlist inside an hlist: how far down; vlist inside a
    /// vlist: how far right) this box is displaced from where it would sit.
    pub shift_amount: Scaled,
    pub list: Vec<Node>,
    /// The glue ratio `hpack`/`vpack` computed (§135). A float in `tex.web`
    /// too.
    pub glue_set: f64,
    pub glue_sign: GlueSign,
    pub glue_order: Order,
    /// `false` for an `hlist_node`, `true` for a `vlist_node`.
    pub vertical: bool,
}

impl BoxNode {
    /// `new_null_box` (§136): an empty hbox with every dimension zero.
    pub fn null() -> BoxNode {
        BoxNode::default()
    }

    /// The height plus depth, which is what a vertical list charges for a box.
    pub fn height_plus_depth(&self) -> Scaled {
        self.height + self.depth
    }
}

/// A rule (§138): a solid black rectangle, any of whose dimensions may be
/// RUNNING, meaning it takes that dimension from the box it lands in.
#[derive(Clone, Copy, Debug)]
pub struct RuleNode {
    pub width: Scaled,
    pub depth: Scaled,
    pub height: Scaled,
}

impl RuleNode {
    /// `new_rule` (§139): all three dimensions running.
    pub fn running() -> RuleNode {
        RuleNode {
            width: NULL_FLAG,
            depth: NULL_FLAG,
            height: NULL_FLAG,
        }
    }

    /// `is_running(#) == (#=null_flag)` (§138).
    pub fn is_running(d: Scaled) -> bool {
        d == NULL_FLAG
    }
}

/// An insertion (§140): material destined for a different box on whatever page
/// this list ends up on. `\footnote` is one.
#[derive(Clone, Debug)]
pub struct InsNode {
    /// The box number it goes into: `\insert⟨n⟩`.
    pub number: u8,
    /// The natural height-plus-depth of the material.
    pub height: Scaled,
    /// `\splitmaxdepth` in force when it was made.
    pub depth: Scaled,
    /// `\floatingpenalty` in force when it was made.
    pub float_cost: i64,
    /// `\splittopskip` in force when it was made.
    pub split_top_skip: Glue,
    pub list: Vec<Node>,
}

/// A glue node (§149), possibly leaders.
#[derive(Clone, Debug, Default)]
pub struct GlueNode {
    pub spec: Glue,
    pub kind: LeaderKind,
    pub source: GlueSource,
    /// The box or rule leaders replicate. `None` for ordinary glue.
    pub leader: Option<Box<Node>>,
}

impl GlueNode {
    /// Plain glue with no leaders.
    pub fn new(spec: Glue) -> GlueNode {
        GlueNode {
            spec,
            ..GlueNode::default()
        }
    }

    /// Glue that names the parameter it came from, which `\showlists` prints
    /// and the page builder's `\topskip` handling produces.
    pub fn param(spec: Glue, source: GlueSource) -> GlueNode {
        GlueNode {
            spec,
            source,
            ..GlueNode::default()
        }
    }
}

/// A character, already measured in the font that will set it.
///
/// `tex.web` keeps a `char_node` as a font-and-character pair and asks the
/// font for its dimensions inside `hpack`'s inner loop (§654). The metrics are
/// carried here instead so that packaging needs no font table: the caller has
/// a `.tfm` or an `hmtx` open and looks them up once. The arithmetic §654 does
/// with them is unchanged.
#[derive(Clone, Copy, Debug)]
pub struct CharNode {
    pub font: usize,
    pub character: char,
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
}

/// A discretionary break (§145): what to set before it, after it, and instead
/// of it when no break is taken.
#[derive(Clone, Debug, Default)]
pub struct DiscNode {
    pub pre_break: Vec<Node>,
    pub post_break: Vec<Node>,
    /// How many nodes after this one the break replaces.
    pub replace_count: usize,
}

/// One item of a horizontal or vertical list (§133-§159).
#[derive(Clone, Debug)]
pub enum Node {
    Char(CharNode),
    Box(BoxNode),
    Rule(RuleNode),
    Ins(InsNode),
    /// `\mark{...}`: the token list, already expanded to text.
    Mark(String),
    /// `\vadjust{...}`: material to move to the enclosing vertical list.
    Adjust(Vec<Node>),
    /// A ligature, which behaves as the character it draws.
    Ligature(CharNode),
    Disc(DiscNode),
    /// `\special` and friends: no dimensions, carried through.
    Whatsit(String),
    /// The space a maths formula reserves on each side (§147).
    Math(Scaled),
    Glue(GlueNode),
    /// `explicit` is `\kern`'s subtype (§155): an explicit kern is a legal
    /// breakpoint when glue follows it, an implicit one is not.
    Kern {
        width: Scaled,
        explicit: bool,
    },
    Penalty(i64),
}

impl Node {
    /// `type(p)` — `tex.web`'s own numbering (§135-§159).
    ///
    /// Not decoration: §148 and §653 are stated as comparisons against it.
    pub fn type_code(&self) -> u8 {
        match self {
            Node::Box(b) if b.vertical => 1,
            Node::Box(_) => 0,
            Node::Rule(_) => 2,
            Node::Ins(_) => 3,
            Node::Mark(_) => 4,
            Node::Adjust(_) => 5,
            Node::Ligature(_) => 6,
            Node::Disc(_) => 7,
            Node::Whatsit(_) => 8,
            Node::Math(_) => 9,
            Node::Glue(_) => 10,
            Node::Kern { .. } => 11,
            Node::Penalty(_) => 12,
            // A char node has no `type` field at all in `tex.web` -- the
            // pointer itself says so (`is_char_node`). Everything that asks
            // for a type has already excluded it, and 255 keeps an accidental
            // ask from reading as a box.
            Node::Char(_) => 255,
        }
    }

    /// `is_char_node(p)` (§134).
    pub fn is_char(&self) -> bool {
        matches!(self, Node::Char(_))
    }

    /// `precedes_break(p) == (type(p)<math_node)` (§148): whether glue that
    /// FOLLOWS this node is a legal breakpoint.
    pub fn precedes_break(&self) -> bool {
        self.type_code() < 9
    }

    /// `non_discardable(p)` (§148) — the same test, under the name §148 gives
    /// it when the question is whether a break throws the node away.
    pub fn non_discardable(&self) -> bool {
        self.precedes_break()
    }

    /// The shift `hpack` and `vpack` charge for this node (§653, §670).
    ///
    /// `if type(p)>=rule_node then s:=0 else s:=shift_amount(p)`: the types
    /// below `rule_node` are exactly `hlist_node` and `vlist_node`, so a box
    /// carries a shift and nothing else does.
    pub fn shift_amount(&self) -> Scaled {
        match self {
            Node::Box(b) => b.shift_amount,
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The numbering is `tex.web`'s, and the two rules that read it as
    /// arithmetic are the reason it is the numbering rather than an enum
    /// discriminant that happens to be in some order.
    #[test]
    fn type_codes_are_tex_webs_own() {
        assert_eq!(Node::Box(BoxNode::null()).type_code(), 0);
        let vbox = BoxNode {
            vertical: true,
            ..BoxNode::null()
        };
        assert_eq!(Node::Box(vbox).type_code(), 1);
        assert_eq!(Node::Rule(RuleNode::running()).type_code(), 2);
        assert_eq!(Node::Mark(String::new()).type_code(), 4);
        assert_eq!(Node::Math(0).type_code(), 9);
        assert_eq!(Node::Glue(GlueNode::default()).type_code(), 10);
        assert_eq!(
            Node::Kern {
                width: 0,
                explicit: true
            }
            .type_code(),
            11
        );
        assert_eq!(Node::Penalty(0).type_code(), 12);
    }

    /// §148: glue after a box may be broken at, glue after glue may not.
    #[test]
    fn glue_breaks_only_after_something_that_stays() {
        assert!(Node::Box(BoxNode::null()).precedes_break());
        assert!(Node::Rule(RuleNode::running()).precedes_break());
        assert!(Node::Disc(DiscNode::default()).precedes_break());
        assert!(Node::Whatsit(String::new()).precedes_break());
        assert!(!Node::Math(0).precedes_break());
        assert!(!Node::Glue(GlueNode::default()).precedes_break());
        assert!(!Node::Penalty(0).precedes_break());
    }

    /// §653: a rule has no shift, so a shifted rule is not a thing that can be
    /// asked for by accident.
    #[test]
    fn only_a_box_carries_a_shift() {
        let shifted = BoxNode {
            shift_amount: 7,
            ..BoxNode::null()
        };
        assert_eq!(Node::Box(shifted).shift_amount(), 7);
        assert_eq!(Node::Rule(RuleNode::running()).shift_amount(), 0);
    }
}
