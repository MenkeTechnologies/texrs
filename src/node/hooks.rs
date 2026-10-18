//! The places LuaTeX hands the stomach's lists to a callback, as hook points
//! a Lua binding can fill.
//!
//! Each hook is called at the point in the stomach where LuaTeX 1.24 calls the
//! callback of the same name (LuaTeX manual, "Callbacks", and `texnodes`'s
//! `lua_node_filter` call sites in `linebreak.c`, `packaging.c`,
//! `buildpage.c`). A hook that is not registered costs one `Option` test and
//! changes nothing, so the run with no Lua is the run with every hook empty.
//!
//! A hook receives the list as `&mut Vec<Node>` and edits it in place. That
//! covers the three answers a LuaTeX node filter can give: `true` (leave the
//! list alone) is not touching it, `false` (throw it away) is clearing it, and
//! a new head is replacing its contents.
//!
//! | hook | where the stomach calls it | LuaTeX's arguments |
//! |---|---|---|
//! | `buildpage_filter` | §994's `build_page`, before anything moves | `extrainfo` |
//! | `contribute_filter` | material reaching the contribution list | `extrainfo` |
//! | `pre_linebreak_filter` | §1096's `line_break`, before the paragraph is broken | `head, groupcode` |
//! | `linebreak_filter` | instead of §813-§890, when set | `head, is_display` |
//! | `post_linebreak_filter` | after §877's `post_line_break` has made the lines | `head, groupcode` |
//! | `hpack_filter` | every `hpack` a box construction makes (§1086, §1076) | `head, groupcode, size, packtype` |
//! | `vpack_filter` | every `vpack`/`vpackage` a box construction makes | `head, groupcode, size, packtype, maxdepth` |
//! | `pre_output_filter` | §1017, before `\box255` is packaged for `\output` | `head, groupcode, size, packtype, maxdepth` |
//! | `hyphenate`, `ligaturing`, `kerning` | LuaTeX's three passes over a paragraph's characters, before `pre_linebreak_filter` | `head, tail` |
//!
//! These run while the PROGRAM runs, not while it is lowered: the lists exist
//! only then. A Lua chunk that registers one at `\directlua` time is
//! registering for the run that follows.

use super::Node;
use crate::pack::Spec;
use std::cell::RefCell;

/// A filter over a list and the group it is being built in.
pub type ListFilter = Box<dyn FnMut(&mut Vec<Node>, &Context)>;
/// A notification with LuaTeX's `extrainfo` string.
pub type Notice = Box<dyn FnMut(&str)>;

/// What a filter is told besides the list: LuaTeX's `groupcode`, and for the
/// packagers the size and pack type and `maxdepth`.
#[derive(Clone, Debug, Default)]
pub struct Context {
    /// LuaTeX's group code: `""` for the main vertical list, `"hbox"`,
    /// `"vbox"`, `"vtop"`, `"insert"`, `"vcenter"`, `"output"`, ...
    pub groupcode: &'static str,
    /// The `to`/`spread` size, in scaled points.
    pub size: i64,
    /// `"exactly"` or `"additional"`.
    pub packtype: &'static str,
    /// `\boxmaxdepth` for a vpack, else zero.
    pub maxdepth: i64,
}

/// Every hook, all empty to begin with.
#[derive(Default)]
pub struct Hooks {
    pub buildpage_filter: Option<Notice>,
    pub contribute_filter: Option<Notice>,
    pub pre_linebreak_filter: Option<ListFilter>,
    pub linebreak_filter: Option<ListFilter>,
    pub post_linebreak_filter: Option<ListFilter>,
    pub hpack_filter: Option<ListFilter>,
    pub vpack_filter: Option<ListFilter>,
    pub pre_output_filter: Option<ListFilter>,
    pub hyphenate: Option<ListFilter>,
    pub ligaturing: Option<ListFilter>,
    pub kerning: Option<ListFilter>,
}

thread_local! {
    static HOOKS: RefCell<Hooks> = RefCell::new(Hooks::default());
}

/// Fill or empty hooks: `node::hooks::set(|h| h.hpack_filter = Some(...))`.
pub fn set(f: impl FnOnce(&mut Hooks)) {
    HOOKS.with(|h| f(&mut h.borrow_mut()));
}

/// Empty every hook, as a new run starts with none.
pub fn clear() {
    set(|h| *h = Hooks::default());
}

fn packtype(spec: Spec) -> (&'static str, i64) {
    match spec {
        Spec::Exactly(w) => ("exactly", w),
        Spec::Additional(w) => ("additional", w),
    }
}

/// Run one filter, if it is registered. The hook is taken out of the table
/// while it runs, so a hook that itself builds a box does not re-enter
/// itself through the borrow.
fn filter(
    which: fn(&mut Hooks) -> &mut Option<ListFilter>,
    list: &mut Vec<Node>,
    cx: &Context,
) {
    let Some(mut hook) = HOOKS.with(|h| which(&mut h.borrow_mut()).take()) else {
        return;
    };
    hook(list, cx);
    HOOKS.with(|h| *which(&mut h.borrow_mut()) = Some(hook));
}

fn notice(which: fn(&mut Hooks) -> &mut Option<Notice>, info: &str) {
    let Some(mut hook) = HOOKS.with(|h| which(&mut h.borrow_mut()).take()) else {
        return;
    };
    hook(info);
    HOOKS.with(|h| *which(&mut h.borrow_mut()) = Some(hook));
}

/// `hpack_filter`, for an hlist about to be packaged.
pub fn hpack_filter(list: &mut Vec<Node>, spec: Spec) {
    let (packtype, size) = packtype(spec);
    let cx = Context {
        groupcode: "hbox",
        size,
        packtype,
        maxdepth: 0,
    };
    filter(|h| &mut h.hpack_filter, list, &cx);
}

/// `vpack_filter`, for a vlist about to be packaged.
pub fn vpack_filter(list: &mut Vec<Node>, spec: Spec, groupcode: &'static str, maxdepth: i64) {
    let (packtype, size) = packtype(spec);
    let cx = Context {
        groupcode,
        size,
        packtype,
        maxdepth,
    };
    filter(|h| &mut h.vpack_filter, list, &cx);
}

/// `pre_linebreak_filter`, for a paragraph about to be broken.
pub fn pre_linebreak_filter(list: &mut Vec<Node>, groupcode: &'static str) {
    let cx = Context {
        groupcode,
        ..Context::default()
    };
    filter(|h| &mut h.pre_linebreak_filter, list, &cx);
}

/// `post_linebreak_filter`, for the lines a paragraph became.
pub fn post_linebreak_filter(list: &mut Vec<Node>, groupcode: &'static str) {
    let cx = Context {
        groupcode,
        ..Context::default()
    };
    filter(|h| &mut h.post_linebreak_filter, list, &cx);
}

/// `pre_output_filter`, for the page about to go into `\box255`.
pub fn pre_output_filter(list: &mut Vec<Node>, size: i64, maxdepth: i64) {
    let cx = Context {
        groupcode: "output",
        size,
        packtype: "exactly",
        maxdepth,
    };
    filter(|h| &mut h.pre_output_filter, list, &cx);
}

/// `buildpage_filter`, as `build_page` starts.
pub fn buildpage_filter(info: &str) {
    notice(|h| &mut h.buildpage_filter, info);
}

/// `contribute_filter`, as material reaches the contribution list.
pub fn contribute_filter(info: &str) {
    notice(|h| &mut h.contribute_filter, info);
}

/// Whether `linebreak_filter` is registered, in which case it replaces the
/// line breaker.
pub fn has_linebreak_filter() -> bool {
    HOOKS.with(|h| h.borrow().linebreak_filter.is_some())
}

/// `linebreak_filter`: the callback breaks the paragraph itself.
pub fn linebreak_filter(list: &mut Vec<Node>) {
    filter(|h| &mut h.linebreak_filter, list, &Context::default());
}

/// `hyphenate`, `ligaturing` and `kerning`, in that order, over a paragraph.
pub fn character_passes(list: &mut Vec<Node>) {
    let cx = Context::default();
    filter(|h| &mut h.hyphenate, list, &cx);
    filter(|h| &mut h.ligaturing, list, &cx);
    filter(|h| &mut h.kerning, list, &cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::BoxNode;

    /// An empty hook leaves the list as it was; a registered one sees it and
    /// can change it, and is still registered afterwards.
    #[test]
    fn a_registered_filter_edits_the_list_in_place() {
        clear();
        let mut list = vec![Node::Penalty(1)];
        hpack_filter(&mut list, crate::pack::NATURAL);
        assert_eq!(list.len(), 1);
        set(|h| {
            h.hpack_filter = Some(Box::new(|l: &mut Vec<Node>, cx: &Context| {
                assert_eq!(cx.packtype, "additional");
                l.push(Node::Box(BoxNode::null()));
            }))
        });
        hpack_filter(&mut list, crate::pack::NATURAL);
        hpack_filter(&mut list, crate::pack::NATURAL);
        assert_eq!(list.len(), 3);
        clear();
    }
}
