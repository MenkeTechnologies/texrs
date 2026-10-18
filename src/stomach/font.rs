//! Fonts as the stomach holds them, and the characters they set.
//!
//! §539-§582 for what a loaded font is -- every dimension already scaled to
//! the size it was loaded at, which is what `\font\x=cmr10 at 12pt` decides
//! once and every character after it reads -- and §1034-§1044 for the main
//! loop that turns a run of characters into character, ligature and kern
//! nodes and an interword space into glue.

use super::{Kind, Mode, Stomach};
use crate::glue::Glue;
use crate::node::{CharNode, DiscNode, GlueNode, GlueSource, Ligature, Node, Scaled};
use crate::tfm::{store_scaled, Tfm};

/// §549's `non_char`: a character code no font has.
const NON_CHAR: u16 = 256;
/// §545: an instruction at or above this skip byte ends the program.
const STOP_FLAG: u8 = 128;
/// §545: an op byte at or above this is a kern.
const KERN_FLAG: u8 = 128;

/// One font in font memory (§549-§551), at the size it was loaded at.
#[derive(Clone, Debug)]
pub struct Font {
    /// `font_name`: the file, as `\fontname` and a DVI `fnt_def` write it.
    pub name: String,
    /// `font_id_text`: the control sequence that last named it (§1257).
    pub id_text: String,
    /// `font_size` and `font_dsize`, in scaled points.
    pub size: Scaled,
    pub design_size: Scaled,
    /// `font_check` (§549): the TFM checksum a DVI `fnt_def` carries.
    pub checksum: u32,
    /// `\fontdimen1..np` at this size (§558), at least seven of them.
    pub params: Vec<Scaled>,
    /// `hyphen_char` and `skew_char` (§549), `\defaulthyphenchar` and
    /// `\defaultskewchar` at load time.
    pub hyphen_char: i64,
    pub skew_char: i64,
    /// The metrics, per code: `None` where the font has no character.
    chars: Vec<Option<Metrics>>,
    /// The characters above 255, which only a font made from metrics (a
    /// `define_font` callback's) has.
    wide: rustc_hash::FxHashMap<u32, Metrics>,
    /// The ligature/kern program, kept for §1039's walk.
    tfm: Option<std::sync::Arc<Tfm>>,
    /// `font_bchar` and `font_false_bchar` (§576).
    bchar: u16,
    false_bchar: u16,
}

/// One character's dimensions at the font's size (§554).
#[derive(Clone, Copy, Debug, Default)]
pub struct Metrics {
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    pub italic: Scaled,
}

impl Font {
    /// `\nullfont` (§552): no characters, seven zero parameters.
    pub fn null() -> Font {
        Font {
            name: "nullfont".into(),
            id_text: "nullfont".into(),
            size: 0,
            design_size: 0,
            checksum: 0,
            params: vec![0; 7],
            hyphen_char: '-' as i64,
            skew_char: -1,
            chars: Vec::new(),
            wide: rustc_hash::FxHashMap::default(),
            tfm: None,
            bchar: NON_CHAR,
            false_bchar: NON_CHAR,
        }
    }

    /// `read_font_info` (§560) for `tfm` loaded at `size` scaled points.
    pub fn load(tfm: std::sync::Arc<Tfm>, name: &str, id_text: &str, size: Scaled) -> Font {
        let scale = |v: f64| store_scaled(v, size);
        let chars = (0..=255u8)
            .map(|c| {
                tfm.char(c).map(|m| Metrics {
                    width: scale(m.width),
                    height: scale(m.height),
                    depth: scale(m.depth),
                    italic: scale(m.italic),
                })
            })
            .collect::<Vec<_>>();
        // §575: the slant is not scaled; every other parameter is, and a
        // font with fewer than seven gets zeros up to seven.
        let mut params: Vec<Scaled> = tfm
            .raw_params
            .iter()
            .enumerate()
            .map(|(i, &v)| match i {
                0 => (v * f64::from(1 << 16)).round() as Scaled,
                _ => scale(v),
            })
            .collect();
        if params.len() < 7 {
            params.resize(7, 0);
        }
        // §576: the false boundary character is the boundary character
        // unless the font has a character of that code.
        let bchar = tfm.boundary_char();
        let false_bchar = match bchar < NON_CHAR && chars[bchar as usize].is_some() {
            true => NON_CHAR,
            false => bchar,
        };
        Font {
            name: name.to_string(),
            id_text: id_text.to_string(),
            size,
            design_size: design_scaled(&tfm),
            checksum: tfm.checksum,
            params,
            hyphen_char: '-' as i64,
            skew_char: -1,
            chars,
            wide: rustc_hash::FxHashMap::default(),
            tfm: Some(tfm),
            bchar,
            false_bchar,
        }
    }

    /// A font made from metrics rather than read from a TFM: what LuaTeX's
    /// `define_font` callback answers with a font table. It has no
    /// ligature/kern program, no boundary character and no checksum.
    pub fn from_metrics(
        name: &str,
        id_text: &str,
        size: Scaled,
        design_size: Scaled,
        mut params: Vec<Scaled>,
        chars: impl IntoIterator<Item = (u32, Metrics)>,
    ) -> Font {
        if params.len() < 7 {
            params.resize(7, 0);
        }
        let mut font = Font {
            name: name.to_string(),
            id_text: id_text.to_string(),
            size,
            design_size,
            params,
            ..Font::null()
        };
        font.chars = vec![None; 256];
        for (code, m) in chars {
            match code < 256 {
                true => font.chars[code as usize] = Some(m),
                false => {
                    font.wide.insert(code, m);
                }
            }
        }
        font
    }

    /// The character's metrics, if the font has it (§554's `char_exists`).
    pub fn metrics(&self, c: char) -> Option<Metrics> {
        let code = u32::from(c);
        match code < 256 {
            true => self.chars.get(code as usize).copied().flatten(),
            false => self.wide.get(&code).copied(),
        }
    }

    /// `\fontdimen n` (§558), numbered from one.
    pub fn param(&self, n: usize) -> Scaled {
        n.checked_sub(1)
            .and_then(|i| self.params.get(i).copied())
            .unwrap_or(0)
    }

    /// A character node for `c`, measured.
    fn char_node(&self, f: usize, c: char, m: Metrics) -> CharNode {
        CharNode {
            font: f,
            character: c,
            width: m.width,
            height: m.height,
            depth: m.depth,
        }
    }

    /// `font_bchar` (§576): the right boundary character, 256 for none.
    pub(super) fn bchar_code(&self) -> u16 {
        self.bchar
    }

    /// §1042's `font_glue`: the interword glue of the font, from
    /// `\fontdimen2..4`.
    pub fn space_glue(&self) -> Glue {
        Glue {
            natural: self.param(2),
            stretch: self.param(3),
            shrink: self.param(4),
            ..Glue::default()
        }
    }

    pub(super) fn lig_word(&self, k: usize) -> Option<[u8; 4]> {
        self.tfm.as_ref()?.lig_kern_word(k)
    }

    /// `lig_kern_start` for `code` (256 is the left boundary, §1034's
    /// `bchar_label`), when its tag says it has a program.
    pub(super) fn lig_start(&self, code: u16) -> Option<usize> {
        self.tfm.as_ref()?.lig_kern_start(code)
    }

    pub(super) fn kern(&self, op: u8, rem: u8) -> Scaled {
        match &self.tfm {
            Some(t) => store_scaled(t.kern_value(op, rem), self.size),
            None => 0,
        }
    }
}

/// §568: the design size in scaled points, `z div 16` of the TFM's fix word.
fn design_scaled(tfm: &Tfm) -> Scaled {
    ((tfm.design_size * f64::from(1 << 20)).round() as Scaled) / 16
}

/// One item of §1032's `lig_stack`: a character read but not yet linked into
/// the list, or a pseudo-ligature (§1032's `new_lig_item`) holding the
/// character it took the place of.
#[derive(Clone, Copy, Debug)]
enum LigItem {
    Char(char),
    Pseudo { c: u16, lig_ptr: Option<char> },
}

impl LigItem {
    /// `character(lig_stack)`.
    fn character(self) -> u16 {
        match self {
            LigItem::Char(c) => code_of(c),
            LigItem::Pseudo { c, .. } => c,
        }
    }
}

/// A character's code in an 8-bit font; anything past 255 is a code no
/// font has.
fn code_of(c: char) -> u16 {
    match u32::from(c) < 256 {
        true => u32::from(c) as u16,
        false => NON_CHAR + 1,
    }
}

/// The labels of §1034-§1040, which the main loop jumps between.
#[derive(Clone, Copy)]
enum Label {
    /// `main_loop_wrapup`.
    Wrapup,
    /// `main_loop_move`.
    Move,
    /// `main_loop_move+1`.
    Move1,
    /// `main_loop_move+2`.
    Move2,
    /// `main_loop_lookahead`.
    Lookahead,
    /// `main_lig_loop`.
    LigLoop,
    /// `main_lig_loop+1`, at program word `main_k`.
    LigLoop1(usize),
    /// `main_loop_move_lig`.
    MoveLig,
}

/// The main loop's variables (§1032), held for one pass.
struct Loop {
    f: usize,
    cur_l: u16,
    cur_r: u16,
    /// `cur_q`: where the characters the ligature being built has consumed
    /// begin, as an index into the current list.
    cur_q: usize,
    /// Top of stack is the LAST element.
    lig_stack: Vec<LigItem>,
    bchar: u16,
    ligature_present: bool,
    lft_hit: bool,
    rt_hit: bool,
    ins_disc: bool,
}

impl Stomach {
    /// `adjust_space_factor` (§1034).
    fn adjust_space_factor(&mut self, c: char) {
        let main_s = self.sf_code(c);
        let cur = self.cur_mut();
        if main_s == 1000 {
            cur.space_factor = 1000;
        } else if main_s < 1000 {
            if main_s > 0 {
                cur.space_factor = main_s;
            }
        } else if cur.space_factor < 1000 {
            cur.space_factor = 1000;
        } else {
            cur.space_factor = main_s;
        }
    }

    /// §581's `char_warning`.
    fn char_warning(&mut self, f: usize, c: char) {
        if self.int_par("tracinglostchars") > 0 {
            let mut text = String::from("Missing character: There is no ");
            text.push(c);
            text.push_str(&format!(" in font {}!", self.font(f).name));
            self.diagnostic(&text);
        }
    }

    /// The main loop (§1034-§1040) over one run of characters in font `f`.
    ///
    /// `chars` is what §1038's lookahead would have found before the next
    /// token that is not a character: the lowerer gathers the run across
    /// macro expansions, as `x_token` there does. `cancel_left` is a
    /// `\noboundary` before the run (§1045's `cancel_boundary`), `no_right`
    /// one after it (§1038's `if cur_cmd=no_boundary then bchar:=non_char`).
    pub fn chars(&mut self, f: usize, chars: &[char], cancel_left: bool, no_right: bool) {
        let mut at = 0;
        let mut cancel = cancel_left;
        // A character the font lacks sends §1036 back to `big_switch`, and
        // the characters after it begin a main loop of their own -- with left
        // boundary processing again.
        while at < chars.len() {
            at = self.main_loop(f, chars, at, cancel, no_right);
            cancel = false;
        }
    }

    /// One pass of the main loop, starting at `chars[start]`; returns where
    /// the next pass starts -- past a character the font lacks -- or
    /// `chars.len()`.
    fn main_loop(
        &mut self,
        f: usize,
        chars: &[char],
        start: usize,
        cancel_left: bool,
        no_right: bool,
    ) -> usize {
        let font = self.font(f).clone();
        let false_bchar = font.false_bchar;
        // `j` is where `cur_chr` came from: the most recent character read.
        let mut j = start;
        // §1034: the first character.
        self.adjust_space_factor(chars[j]);
        let mut s = Loop {
            f,
            cur_l: code_of(chars[j]),
            cur_r: NON_CHAR,
            cur_q: self.cur().list.len(),
            lig_stack: vec![LigItem::Char(chars[j])],
            bchar: font.bchar,
            ligature_present: false,
            lft_hit: false,
            rt_hit: false,
            ins_disc: false,
        };
        let main_k = match cancel_left {
            true => None,
            false => font.lig_start(NON_CHAR),
        };
        let mut at = match main_k {
            None => Label::Move2,
            Some(k) => {
                s.cur_r = s.cur_l;
                s.cur_l = NON_CHAR;
                Label::LigLoop1(k)
            }
        };
        // §1040 calls `check_interrupt` "in case there's an infinite
        // ligature loop"; a run has no one to interrupt it, so it has a
        // budget no honest font can exhaust instead.
        let mut budget = 256 * (chars.len() + 4);
        loop {
            budget -= 1;
            if budget == 0 {
                return chars.len();
            }
            at = match at {
                Label::Wrapup => {
                    let right = s.rt_hit;
                    self.wrapup(&font, &mut s, right);
                    Label::Move
                }
                // §1036.
                Label::Move => {
                    let Some(&top) = s.lig_stack.last() else {
                        return chars.len();
                    };
                    s.cur_q = self.cur().list.len();
                    s.cur_l = top.character();
                    Label::Move1
                }
                Label::Move1 => match s.lig_stack.last() {
                    Some(LigItem::Pseudo { .. }) => Label::MoveLig,
                    _ => Label::Move2,
                },
                Label::Move2 => {
                    let c = chars[j];
                    // A code past 255 is in a font made from metrics, which
                    // has no ligature/kern program to have changed `cur_l`.
                    let exists = match u32::from(c) < 256 {
                        true => {
                            s.cur_l < 256
                                && font.metrics(char::from(s.cur_l as u8)).is_some()
                                && font.metrics(c).is_some()
                        }
                        false => font.metrics(c).is_some(),
                    };
                    if !exists {
                        self.char_warning(f, c);
                        return j + 1;
                    }
                    // `link(tail):=lig_stack`.
                    s.lig_stack.pop();
                    let m = font.metrics(c).unwrap_or_default();
                    self.tail_append(Node::Char(font.char_node(f, c, m)));
                    Label::Lookahead
                }
                // §1038.
                Label::Lookahead => {
                    if j + 1 < chars.len() {
                        j += 1;
                        let c = chars[j];
                        self.adjust_space_factor(c);
                        s.lig_stack.push(LigItem::Char(c));
                        s.cur_r = match code_of(c) == false_bchar {
                            true => NON_CHAR,
                            false => code_of(c),
                        };
                    } else {
                        if no_right {
                            s.bchar = NON_CHAR;
                        }
                        s.cur_r = s.bchar;
                        s.lig_stack.clear();
                    }
                    Label::LigLoop
                }
                // §1039.
                Label::LigLoop => match s.cur_r == NON_CHAR {
                    true => Label::Wrapup,
                    false => match font.lig_start(s.cur_l) {
                        None => Label::Wrapup,
                        Some(k) => Label::LigLoop1(k),
                    },
                },
                Label::LigLoop1(k) => self.lig_program(&font, &mut s, k),
                // §1037.
                Label::MoveLig => {
                    let main_p = match s.lig_stack.pop() {
                        Some(LigItem::Pseudo { lig_ptr, .. }) => lig_ptr,
                        _ => None,
                    };
                    if let Some(c) = main_p {
                        let m = font.metrics(c).unwrap_or_default();
                        self.tail_append(Node::Char(font.char_node(f, c, m)));
                    }
                    s.ligature_present = true;
                    match s.lig_stack.last() {
                        None if main_p.is_some() => Label::Lookahead,
                        None => {
                            s.cur_r = s.bchar;
                            Label::LigLoop
                        }
                        Some(top) => {
                            s.cur_r = top.character();
                            Label::LigLoop
                        }
                    }
                }
            };
        }
    }

    /// `main_lig_loop+1` and `+2` (§1039) and the command they find (§1040):
    /// walk the program of `cur_l` from word `k` for an instruction about
    /// `cur_r`, and carry it out.
    fn lig_program(&mut self, font: &Font, s: &mut Loop, mut k: usize) -> Label {
        loop {
            let Some([skip, next, op, rem]) = font.lig_word(k) else {
                return Label::Wrapup;
            };
            if u16::from(next) == s.cur_r && skip <= STOP_FLAG {
                return self.lig_command(font, s, op, rem);
            }
            if skip == 0 {
                k += 1;
            } else {
                if skip >= STOP_FLAG {
                    return Label::Wrapup;
                }
                k += usize::from(skip) + 1;
            }
        }
    }

    /// §1040: do the ligature or kern command.
    fn lig_command(&mut self, font: &Font, s: &mut Loop, op: u8, rem: u8) -> Label {
        if op >= KERN_FLAG {
            let right = s.rt_hit;
            self.wrapup(font, s, right);
            self.tail_append(Node::Kern {
                width: font.kern(op, rem),
                explicit: false,
            });
            return Label::Move;
        }
        if s.cur_l == NON_CHAR {
            s.lft_hit = true;
        } else if s.lig_stack.is_empty() {
            s.rt_hit = true;
        }
        let rem = u16::from(rem);
        match op {
            // =:| and =:|>
            1 | 5 => {
                s.cur_l = rem;
                s.ligature_present = true;
            }
            // |=: and |=:>
            2 | 6 => {
                s.cur_r = rem;
                match s.lig_stack.last_mut() {
                    // The right boundary character is being consumed.
                    None => {
                        s.lig_stack.push(LigItem::Pseudo {
                            c: rem,
                            lig_ptr: None,
                        });
                        s.bchar = NON_CHAR;
                    }
                    Some(top @ LigItem::Char(_)) => {
                        let LigItem::Char(was) = *top else {
                            unreachable!("just matched");
                        };
                        *top = LigItem::Pseudo {
                            c: rem,
                            lig_ptr: Some(was),
                        };
                    }
                    Some(LigItem::Pseudo { c, .. }) => *c = rem,
                }
            }
            // |=:|
            3 => {
                s.cur_r = rem;
                s.lig_stack.push(LigItem::Pseudo {
                    c: rem,
                    lig_ptr: None,
                });
            }
            // |=:|> and |=:|>>
            7 | 11 => {
                self.wrapup(font, s, false);
                s.cur_q = self.cur().list.len();
                s.cur_l = rem;
                s.ligature_present = true;
            }
            // =:
            _ => {
                s.cur_l = rem;
                s.ligature_present = true;
                return match s.lig_stack.is_empty() {
                    true => Label::Wrapup,
                    false => Label::Move1,
                };
            }
        }
        if op > 4 && op != 7 {
            return Label::Wrapup;
        }
        if s.cur_l < NON_CHAR {
            return Label::LigLoop;
        }
        match font.lig_start(NON_CHAR) {
            Some(k) => Label::LigLoop1(k),
            None => Label::Wrapup,
        }
    }

    /// §1035's `wrapup(#)`, with `pack_lig(#)` inside it.
    ///
    /// The nodes linked after `cur_q` are the characters the ligature has
    /// consumed; they leave the list and become its components (`lig_ptr`).
    fn wrapup(&mut self, font: &Font, s: &mut Loop, right: bool) {
        if s.cur_l >= NON_CHAR {
            return;
        }
        let outer = self.mode().outer();
        let list = &mut self.cur_mut().list;
        if list.len() > s.cur_q {
            if let Some(Node::Char(c)) = list.last() {
                if i64::from(u32::from(c.character)) == font.hyphen_char {
                    s.ins_disc = true;
                }
            }
        }
        if s.ligature_present {
            let components: Vec<char> = list
                .drain(s.cur_q..)
                .filter_map(|n| match n {
                    Node::Char(c) => Some(c.character),
                    _ => None,
                })
                .collect();
            let ch = char::from(s.cur_l as u8);
            let m = font.metrics(ch).unwrap_or_default();
            let mut lig = Ligature {
                components,
                left_boundary: false,
                right_boundary: false,
            };
            if s.lft_hit {
                lig.left_boundary = true;
                s.lft_hit = false;
            }
            if right && s.lig_stack.is_empty() {
                lig.right_boundary = true;
                s.rt_hit = false;
            }
            list.push(Node::Ligature(font.char_node(s.f, ch, m), lig));
            s.ligature_present = false;
        }
        if s.ins_disc {
            s.ins_disc = false;
            if outer {
                list.push(Node::Disc(DiscNode::default()));
            }
        }
    }

    /// §1041-§1044: an interword space in horizontal mode, from the space
    /// factor, `\spaceskip`, `\xspaceskip` and the current font.
    ///
    /// `normal` is §1045's `hmode+spacer` when `space_factor=1000` and
    /// §1041's `append_normal_space` otherwise: the font's own glue, or
    /// `\spaceskip` if it is set, with no space-factor arithmetic.
    pub fn app_space(&mut self, f: usize, space_skip: Glue, xspace_skip: Glue, normal: bool) {
        let font = self.font(f);
        let space_factor = self.cur().space_factor;
        let zero = Glue::default();
        let node = if normal || space_factor == 1000 {
            match space_skip == zero {
                true => GlueNode::new(font.space_glue()),
                false => GlueNode::param(space_skip, GlueSource::SpaceSkip),
            }
        } else if space_factor >= 2000 && xspace_skip != zero {
            GlueNode::param(xspace_skip, GlueSource::XSpaceSkip)
        } else {
            let mut g = match space_skip == zero {
                true => font.space_glue(),
                false => space_skip,
            };
            // §1044.
            if space_factor >= 2000 {
                g.natural += font.param(7);
            }
            g.stretch = crate::dimen::xn_over_d(g.stretch, space_factor, 1000);
            g.shrink = crate::dimen::xn_over_d(g.shrink, 1000, space_factor);
            GlueNode::new(g)
        };
        self.tail_append(Node::Glue(node));
    }

    /// `\/` (§1113): the italic correction of the character at the tail.
    pub fn italic_correction(&mut self) {
        let Some(last) = self.cur().list.last() else {
            return;
        };
        let (f, c) = match last {
            Node::Char(c) | Node::Ligature(c, _) => (c.font, c.character),
            _ => return,
        };
        let italic = self.font(f).metrics(c).map_or(0, |m| m.italic);
        self.tail_append(Node::Kern {
            width: italic,
            explicit: true,
        });
        let _ = Kind::H;
        let _ = Mode::Horizontal;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// cmr10 at its design size, or `None` where no TeX is installed.
    fn cmr10() -> Option<Stomach> {
        let path = crate::typeset::find_font("cmr10")?;
        let tfm = std::sync::Arc::new(Tfm::open(path).ok()?);
        let mut s = Stomach::new();
        s.fonts.push(Font::load(tfm, "cmr10", "tenrm", 10 * 65536));
        s.begin_box(
            super::super::BoxKind::H,
            crate::pack::NATURAL,
            super::super::BoxContext::Shift(0),
        );
        Some(s)
    }

    /// tex's own `\showbox` of `\hbox{abc}` under plain.tex: three characters
    /// and cmr10's implicit kern between b and c.
    #[test]
    fn abc_gets_cmr10s_kern_between_b_and_c() {
        let Some(mut s) = cmr10() else {
            return;
        };
        s.chars(1, &['a', 'b', 'c'], false, false);
        let shown = s.show_box_text(&s.cur().list.clone());
        assert_eq!(shown, "\n\\tenrm a\n\\tenrm b\n\\kern0.27779\n\\tenrm c\n");
    }

    /// §1040's `=:` twice over: `ffi` is one ligature, character 14, made of
    /// all three letters.
    #[test]
    fn ffi_is_one_ligature_of_three_letters() {
        let Some(mut s) = cmr10() else {
            return;
        };
        s.chars(1, &['f', 'f', 'i'], false, false);
        let list = &s.cur().list;
        assert_eq!(list.len(), 1, "{list:?}");
        match &list[0] {
            Node::Ligature(c, lig) => {
                assert_eq!(c.character, '\u{e}');
                assert_eq!(lig.components, vec!['f', 'f', 'i']);
            }
            other => panic!("expected a ligature, got {other:?}"),
        }
    }

    /// A character the font lacks is dropped, and the run carries on after it
    /// as a fresh main loop: `f` and `i` on either side of it do not fuse.
    #[test]
    fn a_missing_character_breaks_the_run() {
        let Some(mut s) = cmr10() else {
            return;
        };
        s.chars(1, &['f', '\u{263a}', 'i'], false, false);
        let chars: Vec<char> = s
            .cur()
            .list
            .iter()
            .filter_map(|n| match n {
                Node::Char(c) => Some(c.character),
                _ => None,
            })
            .collect();
        assert_eq!(chars, vec!['f', 'i']);
    }
}

/// Font memory as the lowerer keeps it: §549-§551's arrays for the fonts
/// `\font` has loaded so far, so that `\fontname`, `\fontdimen` and
/// `\meaning` answer while lowering and each font is defined in the program
/// once. The program's own copy is built from the `DefineFont` actions the
/// lowerer emits, in the same order, so the two agree on every number.
#[derive(Clone, Debug)]
pub struct Table {
    /// Index is the internal font number; 0 is `\nullfont` (§552).
    pub fonts: Vec<Font>,
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

/// What §1258 read after the file name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// `at <dimen>`: the size itself.
    At(Scaled),
    /// `scaled <n>`, or nothing (`scaled 1000`).
    Scaled(i64),
}

impl Table {
    pub fn new() -> Table {
        Table {
            fonts: vec![Font::null()],
        }
    }

    pub fn get(&self, f: u16) -> &Font {
        self.fonts.get(f as usize).unwrap_or(&self.fonts[0])
    }

    pub fn get_mut(&mut self, f: u16) -> Option<&mut Font> {
        self.fonts.get_mut(f as usize)
    }

    /// Put a font made from metrics into font memory, as LuaTeX's
    /// `font.define` does, and return its number. The program finds it under
    /// its name and size when it is told to define it ([`provided`]).
    pub fn add(&mut self, font: Font) -> u16 {
        PROVIDED.with(|p| {
            p.borrow_mut()
                .insert((font.name.clone(), font.size), font.clone())
        });
        self.fonts.push(font);
        (self.fonts.len() - 1) as u16
    }

    /// §1257-§1260's `new_font` from the file name on: reuse a font already
    /// loaded from the same file at the same size, or read the TFM. `None`
    /// when the file cannot be found or read (§561's `Font ... not
    /// loadable`), which leaves the identifier meaning `\nullfont`.
    ///
    /// The returned font is renamed to `id` either way: §1257's
    /// `font_id_text(f):=t` makes the newest name the one of record.
    pub fn load(&mut self, name: &str, size: Size, id: &str) -> Option<(u16, bool)> {
        // §1260.
        for (f, font) in self.fonts.iter_mut().enumerate().skip(1) {
            if font.name != name {
                continue;
            }
            let same = match size {
                Size::At(s) => s == font.size,
                Size::Scaled(m) => {
                    font.size == crate::dimen::xn_over_d(font.design_size, m, 1000)
                }
            };
            if same {
                font.id_text = id.to_string();
                return Some((f as u16, false));
            }
        }
        let path = crate::typeset::find_font(name)?;
        let tfm = std::sync::Arc::new(Tfm::open(path).ok()?);
        let design = design_scaled(&tfm);
        // §568: `scaled` multiplies the design size.
        let at = match size {
            Size::At(s) => s,
            Size::Scaled(m) => crate::dimen::xn_over_d(design, m, 1000),
        };
        self.fonts.push(Font::load(tfm, name, id, at));
        Some(((self.fonts.len() - 1) as u16, true))
    }

    /// §1261's `print_cmd_chr` for `set_font`, after `select font `: the
    /// name, and the size when it is not the design size.
    pub fn describe(&self, f: u16) -> String {
        let font = self.get(f);
        match font.size != font.design_size {
            true => format!(
                "{} at {}pt",
                font.name,
                crate::dimen::print_scaled(font.size)
            ),
            false => font.name.clone(),
        }
    }
}

thread_local! {
    /// The fonts [`Table::add`] put into the lowerer's font memory, by name
    /// and size, for the program's copy of font memory to take when it is
    /// told to define them: there is no TFM file it could read instead.
    static PROVIDED: std::cell::RefCell<rustc_hash::FxHashMap<(String, Scaled), Font>> =
        std::cell::RefCell::new(rustc_hash::FxHashMap::default());
}

/// A font [`Table::add`] was given, under the identifier `id_text`.
pub fn provided(name: &str, size: Scaled, id_text: &str) -> Option<Font> {
    PROVIDED.with(|p| {
        p.borrow().get(&(name.to_string(), size)).map(|f| Font {
            id_text: id_text.to_string(),
            ..f.clone()
        })
    })
}
