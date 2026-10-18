//! The stomach inside a run: one [`Stomach`] per thread, the builtins the
//! compiled program calls, and the dispatch of each [`ListOp`] to the action
//! routine it names.
//!
//! Every list action goes through [`b_list`]: the arguments come off the VM
//! stack, the parameters the action may read are refreshed from the slot
//! file, the action runs, and what it changed that a document can read is
//! written back into the slot file ([`super::Mirror`]). What it had to say is
//! handed to the run's terminal stream in the order it was said.

use super::{BoxContext, Eqtb, SlotSink, Stomach};
use crate::ir::list::{Context, Fetch, ListOp, Popped};
use fusevm::{Value, VM};
use std::cell::RefCell;

thread_local! {
    static STOMACH: RefCell<Stomach> = RefCell::new(Stomach::new());
}

/// Start a run with a fresh stomach: the outer list empty, every register
/// void, no fonts but `\nullfont`, no hooks.
pub fn reset() {
    STOMACH.with(|s| *s.borrow_mut() = Stomach::new());
}

/// Hand the run's stomach to the caller -- the shipped pages and the fonts
/// they are set in -- leaving a fresh one.
pub fn take() -> Stomach {
    STOMACH.with(|s| std::mem::take(&mut *s.borrow_mut()))
}

/// Run `f` on the stomach. A Lua binding reaches the current list and the
/// box registers through this while the program runs (see
/// `crate::node::hooks`).
pub fn with<T>(f: impl FnOnce(&mut Stomach) -> T) -> T {
    STOMACH.with(|s| f(&mut s.borrow_mut()))
}

/// The VM's slot file, as the stomach reads it.
struct Slots<'a>(&'a VM);

impl Eqtb for Slots<'_> {
    fn slot(&self, slot: i64) -> i64 {
        match u16::try_from(slot) {
            Ok(s) => self.0.get_slot(s).to_int(),
            Err(_) => 0,
        }
    }
}

/// The VM's slot file, as the mirror writes it.
struct SlotsMut<'a>(&'a mut VM);

impl SlotSink for SlotsMut<'_> {
    fn set(&mut self, slot: i64, value: i64) {
        if let Ok(s) = u16::try_from(slot) {
            self.0.set_slot(s, Value::Int(value));
        }
    }
}

/// The source line of the op the VM is executing, for a message that names
/// one.
fn current_line(vm: &VM) -> i64 {
    let ip = vm.ip.saturating_sub(1);
    vm.chunk.lines.get(ip).map_or(0, |l| i64::from(*l))
}

/// `ops::LIST`: the opcode, then the action's arguments.
pub fn b_list(vm: &mut VM, argc: u8) -> Value {
    let mut args: Vec<Popped> = (0..argc.saturating_sub(1))
        .map(|_| match vm.pop() {
            Value::Str(s) => Popped::Str(s.to_string()),
            other => Popped::Int(other.to_int()),
        })
        .collect();
    args.reverse();
    let code = vm.pop().to_int();
    let Some(op) = ListOp::decode(code, &args) else {
        return Value::Int(0);
    };
    // The output routine is the program's own sub-chunk: keep it where the
    // page builder can run it.
    if let ListOp::OutputRoutine(k) = op {
        let chunk = usize::try_from(k)
            .ok()
            .and_then(|k| vm.chunk.sub_chunks.get(k))
            .map(|c| std::rc::Rc::new(c.clone()));
        let has = chunk.is_some();
        OUTPUT.with(|o| *o.borrow_mut() = chunk);
        STOMACH.with(|s| s.borrow_mut().pages.has_output_routine = has);
        return Value::Int(0);
    }
    let line = current_line(vm);
    step(vm, |s, e| {
        s.line = line;
        s.refresh(e);
        s.apply(op, e);
    });
    drive(vm);
    Value::Int(0)
}

thread_local! {
    /// The output routine (§230's `output_routine`), compiled.
    static OUTPUT: RefCell<Option<std::rc::Rc<fusevm::Chunk>>> = const { RefCell::new(None) };
}

/// Run `f` on the stomach with the VM's slots, then write back what a
/// document can read and print what the stomach said.
fn step(vm: &mut VM, f: impl FnOnce(&mut Stomach, &dyn Eqtb)) {
    let said = STOMACH.with(|s| {
        let mut s = s.borrow_mut();
        f(&mut s, &Slots(vm));
        super::Mirror::flush(&mut s, &mut SlotsMut(vm));
        std::mem::take(&mut s.said)
    });
    let online = STOMACH.with(|s| s.borrow().int_par("tracingonline") > 0);
    crate::runtime::tell(said, online);
}

/// What follows an action: §994's `build_page` when it asked for one, the
/// output routine when the builder fired it (§1025-§1026), and §1054's
/// `its_all_over` loop after `\end`, in that order until none is wanted.
fn drive(vm: &mut VM) {
    loop {
        let (pending, build, end) = STOMACH.with(|s| {
            let s = s.borrow();
            // Inside the output routine `\end` waits: §1054 runs on the page's own
            // list, after the routine has given it back.
            let end = s.pages.end_requested && !s.output_active();
            (s.pages.output_pending, s.build_page_wanted, end)
        });
        if pending {
            run_output(vm);
            step(vm, |s, e| s.end_output(e));
            continue;
        }
        if build {
            step(vm, |s, e| {
                s.take_build_page();
                s.build_page(e);
            });
            continue;
        }
        if end {
            step(vm, |s, e| {
                if s.its_all_over(e) {
                    s.pages.end_requested = false;
                }
            });
            continue;
        }
        return;
    }
}

/// Run the output routine on a VM of its own, over the same registers: the
/// slot file goes in, and comes back out with what the routine assigned.
///
/// A routine that is not set is §1023's default, which the builder has
/// already carried out by shipping `\box255`; this is reached only when one
/// is.
fn run_output(vm: &mut VM) {
    let Some(chunk) = OUTPUT.with(|o| o.borrow().clone()) else {
        STOMACH.with(|s| s.borrow_mut().pages.output_pending = false);
        return;
    };
    let mut inner = VM::new((*chunk).clone());
    crate::runtime::register_message_builtins(&mut inner);
    let total = crate::compiler::TOTAL_SLOTS;
    for slot in 0..total {
        inner.set_slot(slot, vm.get_slot(slot));
    }
    // The page is the routine's to ship; the builder is suspended while it
    // runs (`output_active`), so nothing below fires it again.
    STOMACH.with(|s| s.borrow_mut().pages.output_pending = false);
    let _ = inner.run();
    for slot in 0..total {
        vm.set_slot(slot, inner.get_slot(slot));
    }
}

/// `ops::LIST_SHOWBOX`: append §1296's `\box n=` display to the message
/// being built.
pub fn b_list_showbox(vm: &mut VM, _argc: u8) -> Value {
    let n = u16::try_from(vm.pop().to_int()).unwrap_or(0);
    let text = STOMACH.with(|s| {
        let s = s.borrow();
        match s.box_register(n) {
            None => "void".to_string(),
            Some(b) => s.show_box_text(std::slice::from_ref(&crate::node::Node::Box(b.clone()))),
        }
    });
    crate::runtime::append_to_message(&text.replace('\n', ""));
    Value::Int(0)
}

impl Stomach {
    /// Carry out one list action.
    pub fn apply(&mut self, op: ListOp<i64>, e: &dyn Eqtb) {
        let tol = self.tolerances(e);
        match op {
            ListOp::DefineFont { f, name, id, size } => self.define_font(f, &name, &id, size),
            ListOp::IntPars(v) => {
                for (i, value) in v {
                    if let Some(slot) = self.int_pars.get_mut(i as usize) {
                        *slot = value;
                    }
                }
            }
            ListOp::SfCodes(v) => {
                for (c, value) in v {
                    self.sf_codes.insert(c, value);
                }
            }
            ListOp::Chars {
                f,
                text,
                cancel_left,
                no_right,
            } => {
                let chars: Vec<char> = text.chars().collect();
                self.chars(f as usize, &chars, cancel_left, no_right);
            }
            ListOp::Space { f, normal } => {
                let space_skip = super::glue_par(e, "spaceskip");
                let xspace_skip = super::glue_par(e, "xspaceskip");
                self.app_space(f as usize, space_skip, xspace_skip, normal);
            }
            ListOp::Glue { spec, mu, leaders } => {
                let g = crate::glue::Glue {
                    natural: spec[0],
                    stretch: spec[1],
                    shrink: spec[2],
                    stretch_order: spec[3] / 4,
                    shrink_order: spec[3] % 4,
                };
                self.append_glue(g, mu, leaders);
            }
            ListOp::Kern(w) => self.append_kern(w),
            ListOp::Penalty(p) => self.append_penalty(p),
            ListOp::Rule {
                vrule,
                width,
                height,
                depth,
                leader,
            } => {
                let mut r = match vrule {
                    true => crate::box_::vrule(),
                    false => crate::box_::hrule(),
                };
                if let Some(w) = width {
                    r.width = w;
                }
                if let Some(h) = height {
                    r.height = h;
                }
                if let Some(d) = depth {
                    r.depth = d;
                }
                match leader {
                    true => self.leader_rule(r),
                    false => self.append_rule(r),
                }
            }
            ListOp::BeginBox {
                kind,
                exactly,
                size,
                context,
            } => {
                let spec = match exactly {
                    true => crate::pack::Spec::Exactly(size),
                    false => crate::pack::Spec::Additional(size),
                };
                self.begin_box(kind, spec, box_context(context));
            }
            ListOp::BoxMaxDepth(d) => self.note_box_max_depth(d),
            ListOp::EndBox => self.package(tol),
            ListOp::Fetch { from, context } => {
                let source = match from {
                    Fetch::Box(n) => super::build::Source::Box(n),
                    Fetch::Copy(n) => super::build::Source::Copy(n),
                    Fetch::LastBox => super::build::Source::LastBox,
                    Fetch::Vsplit(n, h) => super::build::Source::Vsplit(n, h),
                };
                self.fetch_box(source, box_context(context), tol);
            }
            ListOp::Unpackage { n, copy } => self.unpackage(n, copy),
            ListOp::DeleteLast(r) => self.delete_last(r),
            ListOp::Mark(text) => self.make_mark(text),
            ListOp::ItalicCorrection => self.italic_correction(),
            ListOp::Hyphen(f) => self.append_hyphen(f as usize),
            ListOp::BeginDiscretionary => self.begin_discretionary(),
            ListOp::EndDiscList(k) => {
                self.end_disc_list(k);
            }
            ListOp::BeginInsert(n) => self.begin_insert(n),
            ListOp::EndInsert {
                split_top_skip,
                split_max_depth,
                floating_penalty,
            } => {
                let g = crate::glue::Glue {
                    natural: split_top_skip[0],
                    stretch: split_top_skip[1],
                    shrink: split_top_skip[2],
                    stretch_order: split_top_skip[3] / 4,
                    shrink_order: split_top_skip[3] % 4,
                };
                self.end_insert(g, split_max_depth, floating_penalty, tol);
            }
            ListOp::GroupBegin => self.group_begin(),
            ListOp::GroupEnd => self.group_end(),
            ListOp::SetBoxDimen { n, which, value } => {
                if let Some(b) = self.box_register_mut(n) {
                    match which {
                        super::BoxDimen::Wd => b.width = value,
                        super::BoxDimen::Ht => b.height = value,
                        super::BoxDimen::Dp => b.depth = value,
                    }
                }
            }
            ListOp::SetPrevDepth(v) => self.cur_mut().prev_depth = v,
            ListOp::SetSpaceFactor(v) => self.cur_mut().space_factor = v,
            ListOp::SetPrevGraf(v) => self.set_prev_graf(v),
            ListOp::SetFontDimen { f, n, value } => {
                if let Some(font) = self.fonts.get_mut(f as usize) {
                    let i = (n as usize).saturating_sub(1);
                    if i >= font.params.len() {
                        font.params.resize(i + 1, 0);
                    }
                    font.params[i] = value;
                }
            }
            ListOp::SetHyphenChar { f, value } => {
                if let Some(font) = self.fonts.get_mut(f as usize) {
                    font.hyphen_char = value;
                }
            }
            ListOp::SetSkewChar { f, value } => {
                if let Some(font) = self.fonts.get_mut(f as usize) {
                    font.skew_char = value;
                }
            }
            ListOp::NewGraf(indented) => self.new_graf(indented, e),
            ListOp::EndGraf => self.end_graf(e),
            ListOp::Indent => self.indent_in_hmode(e),
            ListOp::BuildPage => self.want_build_page("vmode_par"),
            ListOp::End => self.pages.end_requested = true,
            // Kept by `b_list` before it reaches here.
            ListOp::OutputRoutine(_) => {}
            ListOp::Patterns(text) => {
                self.hyphenation_source
                    .push_str(&format!("\\patterns{{\n{text}\n}}\n"));
                self.hyphenator = None;
            }
            ListOp::Hyphenation(text) => {
                self.hyphenation_source
                    .push_str(&format!("\\hyphenation{{\n{text}\n}}\n"));
                self.hyphenator = None;
            }
            ListOp::Converted { f, form, values } => {
                let text = match form {
                    0 => values[0].to_string(),
                    1 => crate::expand::roman_int(values[0]),
                    2 => format!("{}pt", crate::dimen::print_scaled(values[0])),
                    _ => crate::glue::print_glue_in(
                        if form == 4 { "mu" } else { "pt" },
                        values[0],
                        values[1],
                        values[3] / 4,
                        values[2],
                        values[3] % 4,
                    ),
                };
                // §465: `\the` gives category-12 characters and category-10
                // spaces, so a space here is an interword space.
                let space_skip = super::glue_par(e, "spaceskip");
                let xspace_skip = super::glue_par(e, "xspaceskip");
                for (i, run) in text.split(' ').enumerate() {
                    if i > 0 {
                        self.app_space(f as usize, space_skip, xspace_skip, false);
                    }
                    let chars: Vec<char> = run.chars().collect();
                    if !chars.is_empty() {
                        self.chars(f as usize, &chars, false, false);
                    }
                }
            }
        }
    }

    /// §560's `read_font_info`, run again at run time for a font the lowerer
    /// defined: font memory is the program's, so the chunk says which fonts
    /// to load and this loads them.
    fn define_font(&mut self, f: u16, name: &str, id: &str, size: i64) {
        let f = f as usize;
        if self.fonts.len() <= f {
            self.fonts.resize(f + 1, super::Font::null());
        }
        let loaded = crate::typeset::find_font(name)
            .and_then(|p| crate::tfm::Tfm::open(p).ok())
            .map(|t| super::Font::load(std::sync::Arc::new(t), name, id, size));
        self.fonts[f] = match loaded {
            Some(font) => font,
            // §561: a font that cannot be loaded leaves the identifier
            // selecting `\nullfont`'s metrics under the name it was given.
            None => {
                let mut null = super::Font::null();
                null.id_text = id.to_string();
                null.name = name.to_string();
                null
            }
        };
    }
}

fn box_context(c: Context<i64>) -> BoxContext {
    match c {
        Context::Shift(s) => BoxContext::Shift(s),
        Context::SetBox { n, global } => BoxContext::SetBox { n, global },
        Context::ShipOut => BoxContext::ShipOut,
        Context::Leaders(k) => BoxContext::Leaders(k),
    }
}
