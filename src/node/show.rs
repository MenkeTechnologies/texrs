//! Displaying a list: `tex.web` §173-§198.
//!
//! `\showbox`, `\showlists` and every `Overfull \hbox` message describe a list
//! in the same symbolic form, and a reader compares it against tex's log
//! character for character -- so this is the port of `show_node_list` and
//! `short_display`, with tex's spacing, its omissions and its truncation
//! rules, not a pretty-printer that happens to look similar.
//!
//! Two of tex's rules decide most of what a display looks like:
//!
//! - The nesting history is a STRING, one `.` per level (§180's
//!   `node_list_display` appends one, recurses, and flushes it), and each
//!   line starts with it. A discretionary's post-break list is introduced with
//!   `|` instead (§195), so `.|` is "inside the post-break text".
//! - `\showboxdepth` and `\showboxbreadth` (§181, §236) cut the display off:
//!   a list deeper than the depth prints ` []`, and the item past the breadth
//!   prints `etc.`. A non-positive breadth means 5 (§198).
//!
//! A glue's spec is printed with NO unit (§189's `print_spec(glue_ptr(p),0)`),
//! which is why a box display reads `\glue 3.33333 plus 1.66666` where `\the`
//! would write `3.33333pt plus 1.66666pt`.

use super::{BoxNode, GlueSign, GlueSource, LeaderKind, Node, Scaled};
use crate::dimen::{print_scaled, UNITY};

/// The names a display needs from the font table: §267's `font_id_text`,
/// which is the control sequence `\font` defined (`tenrm`), not the file name.
pub trait FontIds {
    /// The identifier of internal font `f`, without its escape character.
    fn font_id_text(&self, f: usize) -> String;
}

/// A font table that names every font by its number, for a caller with no
/// table of its own.
pub struct Numbered;

impl FontIds for Numbered {
    fn font_id_text(&self, f: usize) -> String {
        format!("FONT{f}")
    }
}

/// What a display prints with: §236's `\showboxdepth` and `\showboxbreadth`,
/// and the escape character §63's `print_esc` puts in front of every name.
pub struct Display<'a> {
    /// `depth_threshold` (§181).
    pub depth: i64,
    /// `breadth_max` (§181).
    pub breadth: i64,
    /// The `\escapechar` character, or empty when it is out of range.
    pub esc: &'a str,
    pub fonts: &'a dyn FontIds,
}

/// §68's `print_ASCII` of a character code, under the convention
/// `crate::ir::printed` uses for the rest of texrs's output: a control code
/// in §49's `^^` form, everything else as itself.
fn print_ascii(out: &mut String, c: char) {
    let code = u32::from(c);
    if code < 32 {
        out.push_str("^^");
        out.push(char::from(code as u8 + 64));
    } else if code == 127 {
        out.push_str("^^?");
    } else {
        out.push(c);
    }
}

/// §177's `print_glue`: one component, with its order's name, or `s` when it
/// is finite.
fn print_glue(out: &mut String, d: Scaled, order: crate::glue::Order, s: &str) {
    out.push_str(&print_scaled(d));
    match order {
        o if !(0..=3).contains(&o) => out.push_str("foul"),
        0 => out.push_str(s),
        o => {
            out.push_str("fil");
            for _ in 1..o {
                out.push('l');
            }
        }
    }
}

/// §178's `print_spec`.
pub fn print_spec(out: &mut String, g: &crate::glue::Glue, s: &str) {
    out.push_str(&print_scaled(g.natural));
    out.push_str(s);
    if g.stretch != 0 {
        out.push_str(" plus ");
        print_glue(out, g.stretch, g.stretch_order, s);
    }
    if g.shrink != 0 {
        out.push_str(" minus ");
        print_glue(out, g.shrink, g.shrink_order, s);
    }
}

/// §176's `print_rule_dimen`: a running dimension is `*`.
fn print_rule_dimen(out: &mut String, d: Scaled) {
    match d == super::NULL_FLAG {
        true => out.push('*'),
        false => out.push_str(&print_scaled(d)),
    }
}

/// §225's `print_skip_param`.
fn skip_param_name(code: usize) -> &'static str {
    crate::params::GLUE_NAMES
        .get(code)
        .copied()
        .unwrap_or("[unknown glue parameter!]")
}

impl Display<'_> {
    fn esc(&self, out: &mut String, name: &str) {
        out.push_str(self.esc);
        out.push_str(name);
    }

    /// §176's `print_font_and_char`.
    fn font_and_char(&self, out: &mut String, font: usize, c: char) {
        self.esc(out, &self.fonts.font_id_text(font));
        out.push(' ');
        print_ascii(out, c);
    }

    /// §198's `show_box`: the display of `list` that `\showbox` writes,
    /// starting on a fresh line as §182's first `print_ln` does, and ending
    /// with the `print_ln` §198 adds.
    pub fn show_box(&self, list: &[Node]) -> String {
        let breadth = match self.breadth <= 0 {
            true => 5,
            false => self.breadth,
        };
        let mut out = String::new();
        let mut history = String::new();
        self.show_node_list(&mut out, list, &mut history, breadth);
        out.push('\n');
        out
    }

    /// §182's `show_node_list`.
    fn show_node_list(&self, out: &mut String, list: &[Node], history: &mut String, breadth: i64) {
        if history.len() as i64 > self.depth {
            if !list.is_empty() {
                out.push_str(" []");
            }
            return;
        }
        let mut n = 0i64;
        let mut i = 0usize;
        while i < list.len() {
            out.push('\n');
            out.push_str(history);
            n += 1;
            if n > breadth {
                out.push_str("etc.");
                return;
            }
            self.display_node(out, &list[i], history, breadth);
            i += 1;
        }
    }

    /// §180's `node_list_display`: one level deeper, marked by `.`.
    fn nested(&self, out: &mut String, list: &[Node], history: &mut String, breadth: i64) {
        history.push('.');
        self.show_node_list(out, list, history, breadth);
        history.pop();
    }

    /// §183: display node `p`.
    fn display_node(&self, out: &mut String, p: &Node, history: &mut String, breadth: i64) {
        match p {
            Node::Char(c) => self.font_and_char(out, c.font, c.character),
            Node::Box(b) => {
                self.display_box(out, b);
                self.nested(out, &b.list, history, breadth);
            }
            // §187.
            Node::Rule(r) => {
                self.esc(out, "rule(");
                print_rule_dimen(out, r.height);
                out.push('+');
                print_rule_dimen(out, r.depth);
                out.push_str(")x");
                print_rule_dimen(out, r.width);
            }
            // §188.
            Node::Ins(ins) => {
                self.esc(out, "insert");
                out.push_str(&ins.number.to_string());
                out.push_str(", natural size ");
                out.push_str(&print_scaled(ins.height));
                out.push_str("; split(");
                print_spec(out, &ins.split_top_skip, "");
                out.push(',');
                out.push_str(&print_scaled(ins.depth));
                out.push_str("); float cost ");
                out.push_str(&ins.float_cost.to_string());
                self.nested(out, &ins.list, history, breadth);
            }
            // §1356's `\special` display, the one whatsit texrs carries.
            Node::Whatsit(text) => {
                self.esc(out, "special");
                out.push('{');
                out.push_str(text);
                out.push('}');
            }
            Node::Glue(g) if g.kind.is_leaders() => {
                // §190.
                self.esc(out, "");
                match g.kind {
                    LeaderKind::Centred => out.push('c'),
                    LeaderKind::Expanded => out.push('x'),
                    _ => {}
                }
                out.push_str("leaders ");
                print_spec(out, &g.spec, "");
                if let Some(leader) = &g.leader {
                    self.nested(out, std::slice::from_ref(leader), history, breadth);
                }
            }
            // §189.
            Node::Glue(g) => {
                self.esc(out, "glue");
                if g.source != GlueSource::Normal {
                    out.push('(');
                    match g.source {
                        GlueSource::CondMath => self.esc(out, "nonscript"),
                        GlueSource::MuGlue => self.esc(out, "mskip"),
                        other => {
                            self.esc(out, skip_param_name(other.param_code().unwrap_or(99)))
                        }
                    }
                    out.push(')');
                }
                if g.source != GlueSource::CondMath {
                    out.push(' ');
                    match g.source {
                        GlueSource::MuGlue => print_spec(out, &g.spec, "mu"),
                        _ => print_spec(out, &g.spec, ""),
                    }
                }
            }
            // §191: an explicit kern is shown by the space after `\kern`.
            Node::Kern { width, explicit } => {
                self.esc(out, "kern");
                if *explicit {
                    out.push(' ');
                }
                out.push_str(&print_scaled(*width));
            }
            // §192.
            Node::Math(width) | Node::MathOff(width) => {
                let side = match p {
                    Node::Math(_) => "mathon",
                    _ => "mathoff",
                };
                self.esc(out, side);
                if *width != 0 {
                    out.push_str(", surrounded ");
                    out.push_str(&print_scaled(*width));
                }
            }
            // §193.
            Node::Ligature(c, lig) => {
                self.font_and_char(out, c.font, c.character);
                out.push_str(" (ligature ");
                if lig.left_boundary {
                    out.push('|');
                }
                for &k in &lig.components {
                    print_ascii(out, k);
                }
                if lig.right_boundary {
                    out.push('|');
                }
                out.push(')');
            }
            // §194.
            Node::Penalty(v) => {
                self.esc(out, "penalty ");
                out.push_str(&v.to_string());
            }
            // §195.
            Node::Disc(d) => {
                self.esc(out, "discretionary");
                if d.replace_count > 0 {
                    out.push_str(" replacing ");
                    out.push_str(&d.replace_count.to_string());
                }
                self.nested(out, &d.pre_break, history, breadth);
                history.push('|');
                self.show_node_list(out, &d.post_break, history, breadth);
                history.pop();
            }
            // §196.
            Node::Mark(text) => {
                self.esc(out, "mark");
                out.push('{');
                out.push_str(text);
                out.push('}');
            }
            // §197.
            Node::Adjust(list) => {
                self.esc(out, "vadjust");
                self.nested(out, list, history, breadth);
            }
        }
    }

    /// §184 and §186: a box's head line.
    fn display_box(&self, out: &mut String, b: &BoxNode) {
        self.esc(out, if b.vertical { "v" } else { "h" });
        out.push_str("box(");
        out.push_str(&print_scaled(b.height));
        out.push('+');
        out.push_str(&print_scaled(b.depth));
        out.push_str(")x");
        out.push_str(&print_scaled(b.width));
        // §186. The `?.?` arm guards against a clobbered word in tex's
        // memory, which a Rust float cannot be.
        let g = b.glue_set;
        if g != 0.0 && b.glue_sign != GlueSign::Normal {
            out.push_str(", glue set ");
            if b.glue_sign == GlueSign::Shrinking {
                out.push_str("- ");
            }
            if g.abs() > 20000.0 {
                match g > 0.0 {
                    true => out.push('>'),
                    false => out.push_str("< -"),
                }
                print_glue(out, 20000 * UNITY, b.glue_order, "");
            } else {
                print_glue(out, (UNITY as f64 * g).round() as Scaled, b.glue_order, "");
            }
        }
        if b.shift_amount != 0 {
            out.push_str(", shifted ");
            out.push_str(&print_scaled(b.shift_amount));
        }
    }

    /// §174's `short_display`: the one-line summary an `Overfull \hbox`
    /// message gives of the list. `font` is `font_in_short_display`, which the
    /// caller starts at `null_font` (§663) and this updates.
    pub fn short_display(&self, out: &mut String, list: &[Node], font: &mut Option<usize>) {
        let mut i = 0usize;
        while i < list.len() {
            match &list[i] {
                Node::Char(c) => {
                    if *font != Some(c.font) {
                        self.esc(out, &self.fonts.font_id_text(c.font));
                        out.push(' ');
                        *font = Some(c.font);
                    }
                    print_ascii(out, c.character);
                }
                // §175.
                Node::Box(_) | Node::Ins(_) | Node::Whatsit(_) | Node::Mark(_) | Node::Adjust(_) => {
                    out.push_str("[]")
                }
                Node::Rule(_) => out.push('|'),
                Node::Glue(g) if g.spec != crate::glue::Glue::default() => out.push(' '),
                Node::Math(_) | Node::MathOff(_) => out.push('$'),
                // §175: a ligature is shown as what it was made of.
                Node::Ligature(c, lig) => {
                    if *font != Some(c.font) {
                        self.esc(out, &self.fonts.font_id_text(c.font));
                        out.push(' ');
                        *font = Some(c.font);
                    }
                    for &k in &lig.components {
                        print_ascii(out, k);
                    }
                }
                Node::Disc(d) => {
                    self.short_display(out, &d.pre_break, font);
                    self.short_display(out, &d.post_break, font);
                    i += d.replace_count;
                }
                _ => {}
            }
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glue::Glue;
    use crate::node::{CharNode, GlueNode, RuleNode};

    struct Tenrm;
    impl FontIds for Tenrm {
        fn font_id_text(&self, _f: usize) -> String {
            "tenrm".to_string()
        }
    }

    fn display() -> Display<'static> {
        Display {
            depth: 2147483647,
            breadth: 2147483647,
            esc: "\\",
            fonts: &Tenrm,
        }
    }

    fn ch(c: char, width: Scaled) -> Node {
        Node::Char(CharNode {
            font: 1,
            character: c,
            width,
            height: 0,
            depth: 0,
        })
    }

    /// tex's own log for `\setbox0\hbox{abc}\showbox0` under plain.tex,
    /// measured: the kern between b and c is cmr10's, and implicit, so it has
    /// no space after `\kern`.
    #[test]
    fn an_hbox_of_three_letters_reads_as_tex_logs_it() {
        let b = BoxNode {
            width: 1001247,
            height: 455111,
            list: vec![
                ch('a', 327681),
                ch('b', 364090),
                Node::Kern {
                    width: 18205,
                    explicit: false,
                },
                ch('c', 291268),
            ],
            ..BoxNode::null()
        };
        assert_eq!(
            display().show_box(&[Node::Box(b)]),
            "\n\\hbox(6.94444+0.0)x15.27782\n.\\tenrm a\n.\\tenrm b\n.\\kern0.27779\n.\\tenrm c\n"
        );
    }

    /// §189 prints a glue's spec without a unit and names the parameter it
    /// came from; §191 marks an explicit kern by a space; §187 prints a
    /// running dimension as `*`.
    #[test]
    fn glue_kerns_and_rules_follow_sections_187_to_191() {
        let list = vec![
            Node::Glue(GlueNode::param(
                Glue {
                    natural: 12 * UNITY,
                    stretch: UNITY,
                    stretch_order: 1,
                    ..Glue::default()
                },
                GlueSource::BaselineSkip,
            )),
            Node::Kern {
                width: 2 * UNITY,
                explicit: true,
            },
            Node::Rule(RuleNode {
                width: super::super::NULL_FLAG,
                height: 26214,
                depth: 0,
            }),
            Node::Penalty(-10000),
        ];
        assert_eq!(
            display().show_box(&list),
            "\n\\glue(\\baselineskip) 12.0 plus 1.0fil\n\\kern 2.0\n\\rule(0.4+0.0)x*\n\\penalty -10000\n"
        );
    }

    /// §182's two cut-offs: past `\showboxbreadth` items the display says
    /// `etc.`, and below `\showboxdepth` levels a non-empty list is ` []`.
    #[test]
    fn breadth_and_depth_cut_the_display_off() {
        let inner = BoxNode {
            list: vec![ch('x', 0)],
            ..BoxNode::null()
        };
        let d = Display {
            depth: 0,
            breadth: 1,
            ..display()
        };
        assert_eq!(
            d.show_box(&[Node::Box(inner.clone()), ch('y', 0)]),
            "\n\\hbox(0.0+0.0)x0.0 []\netc.\n"
        );
    }

    /// §186: a stretched box says how far, in the order it stretched by.
    #[test]
    fn glue_set_is_printed_with_its_order() {
        let b = BoxNode {
            glue_set: 0.5,
            glue_sign: GlueSign::Stretching,
            glue_order: 1,
            ..BoxNode::null()
        };
        assert_eq!(
            display().show_box(&[Node::Box(b)]),
            "\n\\hbox(0.0+0.0)x0.0, glue set 0.5fil\n"
        );
    }

    /// §193: a ligature is its glyph and, in parentheses, what it was made
    /// of -- cmr10's `fi` is character 12, so `^^L`.
    #[test]
    fn a_ligature_shows_its_components() {
        let fi = Node::Ligature(
            CharNode {
                font: 1,
                character: '\u{c}',
                width: 0,
                height: 0,
                depth: 0,
            },
            crate::node::Ligature {
                components: vec!['f', 'i'],
                ..Default::default()
            },
        );
        assert_eq!(display().show_box(&[fi]), "\n\\tenrm ^^L (ligature fi)\n");
    }

    /// §174: the font is named once, where it changes, and glue is a space.
    #[test]
    fn short_display_names_the_font_once() {
        let mut out = String::new();
        let mut font = None;
        display().short_display(
            &mut out,
            &[
                ch('a', 0),
                Node::Glue(GlueNode::new(Glue::fixed(UNITY))),
                ch('b', 0),
                Node::Rule(RuleNode::running()),
            ],
            &mut font,
        );
        assert_eq!(out, "\\tenrm a b|");
    }
}
