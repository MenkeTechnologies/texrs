//! Shipping a box to a PDF page, as LuaTeX does it: `pdfshipout.c`'s
//! `ship_out` and `pdflistout.c`'s `hlist_out` and `vlist_out`, for boxes in
//! the default `TLT` direction.
//!
//! The walk is TeX's (§619-§640) with LuaTeX's bookkeeping: instead of a DVI
//! reader's position and the movements that get it there, it keeps where
//! each node is ON THE PAGE, in scaled points from the bottom left corner
//! (`posstruct->pos`), and hands that to the page writer
//! ([`crate::pdfpage::PageWriter`]) for every glyph and rule. The writer
//! decides what to write; this decides where things are, and the two
//! together are what makes a page's content stream the bytes luatex writes.
//!
//! The page is a `crate::node::BoxNode` as the stomach ships it, with
//! `\hoffset` and `\voffset`; what fonts its characters name is the caller's
//! to say ([`ShipFont`]).

use crate::node::{BoxNode, GlueSign, LeaderKind, Node, RuleNode, Scaled};
use crate::pdfpage::{GlyphFont, PageWriter};

/// A font the page's characters are set in, by the stomach's font number.
#[derive(Clone, Debug)]
pub struct ShipFont {
    /// The TFM name, which decides which fonts share a PDF font resource
    /// (`font_shareable`: the same name at another size is the same font).
    pub tfm_name: String,
    /// `font_size`, in scaled points.
    pub size: Scaled,
    /// Each code's width at that size.
    pub widths: Vec<Option<Scaled>>,
}

/// Where a page goes and how big it is: the reference point (`\pdfvariable
/// horigin` and `vorigin`, LuaTeX's `pdf_set_reference_point`) and the page
/// size (`\pagewidth` and `\pageheight`, or the box's extent with the page
/// offsets when they are zero).
#[derive(Clone, Copy, Debug)]
pub struct PageFrame {
    pub width: Scaled,
    pub height: Scaled,
    pub h_origin: Scaled,
    pub v_origin: Scaled,
}

/// One shipped page, drawn: the content stream, which codes each font
/// resource drew, and the page's size.
pub struct Drawn {
    pub content: Vec<u8>,
    /// By resource number (`/F<n>`), the codes it drew.
    pub used: std::collections::BTreeMap<u32, std::collections::BTreeSet<u8>>,
    pub width: Scaled,
    pub height: Scaled,
    /// What the page writer warned about.
    pub warnings: Vec<String>,
    /// The resources in the order the page first drew in them.
    pub font_order: Vec<u32>,
}

/// `billion` and `vet_glue` (`pdflistout.c`).
fn vet_glue(x: f64) -> f64 {
    x.clamp(-1_000_000_000.0, 1_000_000_000.0)
}

struct Shipper<'a> {
    w: PageWriter,
    fonts: &'a [ShipFont],
    /// `pdf->posstruct->pos`.
    pos: (Scaled, Scaled),
    doing_leaders: bool,
    /// Where the shipped box's own origin is, for `\gleaders`.
    shipbox: (Scaled, Scaled),
}

/// `ship_out(pdf, p, SHIPPING_PAGE)` for the page `page`: the page size and
/// the reference point, then `vlist_out` or `hlist_out` from the box's
/// origin, a box's height below the reference point.
pub fn ship_out(
    page: &BoxNode,
    h_offset: Scaled,
    v_offset: Scaled,
    frame: PageFrame,
    fonts: &[ShipFont],
    decimal_digits: u32,
) -> Drawn {
    let mut ship = Shipper {
        w: PageWriter::new(decimal_digits),
        fonts,
        pos: (0, 0),
        doing_leaders: false,
        shipbox: (0, 0),
    };
    // `pdf_set_reference_point`, moved by the offsets, then `cur.v =
    // height(p)` taken off it.
    let refpoint = (
        frame.h_origin + h_offset,
        frame.height - frame.v_origin - v_offset,
    );
    ship.pos = (refpoint.0, refpoint.1 - page.height);
    ship.shipbox = ship.pos;
    match page.vertical {
        true => ship.vlist_out(page),
        false => ship.hlist_out(page),
    }
    let warnings = ship.w.warnings.clone();
    let font_order = ship.w.font_order.clone();
    let (content, used) = ship.w.finish();
    Drawn {
        content,
        used,
        font_order,
        width: frame.width,
        height: frame.height,
        warnings,
    }
}

/// `synch_pos_with_cur` for `dir_TLT`.
fn at(refpos: (Scaled, Scaled), cur: (Scaled, Scaled)) -> (Scaled, Scaled) {
    (refpos.0 + cur.0, refpos.1 - cur.1)
}

/// The glue's set amount so far: `pdflistout.c`'s real multiplication,
/// `float_cast(glue_set)` being a C `float`.
fn glue_step(spec: &crate::glue::Glue, this_box: &BoxNode, cur_glue: &mut f64, cur_g: &mut Scaled) {
    let ratio = f64::from(this_box.glue_set as f32);
    match this_box.glue_sign {
        GlueSign::Stretching if spec.stretch_order == this_box.glue_order => {
            *cur_glue += spec.stretch as f64;
            *cur_g = vet_glue(ratio * *cur_glue).round() as Scaled;
        }
        GlueSign::Shrinking if spec.shrink_order == this_box.glue_order => {
            *cur_glue -= spec.shrink as f64;
            *cur_g = vet_glue(ratio * *cur_glue).round() as Scaled;
        }
        _ => {}
    }
}

impl Shipper<'_> {
    fn glyph(&mut self, font: usize, c: char) {
        let Some(f) = self.fonts.get(font) else {
            return;
        };
        // `pdf_set_font`: the resource is the first font of the same name.
        let resource = self
            .fonts
            .iter()
            .position(|g| g.tfm_name == f.tfm_name)
            .unwrap_or(font);
        let base = &self.fonts[resource];
        let widths = |code: u8| {
            base.widths
                .get(usize::from(code))
                .copied()
                .flatten()
                .unwrap_or(0)
        };
        let gf = GlyphFont {
            id: font as u32,
            resource: resource as u32,
            size: f.size,
            resource_size: base.size,
            resource_widths: &widths,
        };
        let code = u32::from(c).min(255) as u8;
        self.w.place_glyph(&gf, code, self.pos);
    }

    /// `hlist_out`.
    fn hlist_out(&mut self, this_box: &BoxNode) {
        let refpos = self.pos;
        let mut cur: (Scaled, Scaled) = (0, 0);
        let mut cur_glue = 0.0f64;
        let mut cur_g: Scaled = 0;
        let mut i = 0;
        while i < this_box.list.len() {
            let p = &this_box.list[i];
            i += 1;
            match p {
                Node::Char(c) | Node::Ligature(c, _) => {
                    self.glyph(c.font, c.character);
                    cur.0 += c.width;
                }
                Node::Box(b) => {
                    if !b.list.is_empty() {
                        self.pos = at(refpos, (cur.0, b.shift_amount));
                        match b.vertical {
                            true => self.vlist_out(b),
                            false => self.hlist_out(b),
                        }
                    }
                    cur.0 += b.width;
                }
                Node::Math(width) => cur.0 += width,
                Node::Kern { width, .. } => cur.0 += width,
                Node::Glue(g) => {
                    let mut rule_wd = g.spec.natural - cur_g;
                    glue_step(&g.spec, this_box, &mut cur_glue, &mut cur_g);
                    rule_wd += cur_g;
                    if g.kind.is_leaders() {
                        match g.leader.as_deref() {
                            Some(Node::Rule(r)) => {
                                let r = RuleNode {
                                    width: rule_wd,
                                    height: r.height,
                                    depth: r.depth,
                                };
                                self.hrule(&r, this_box, refpos, cur);
                            }
                            Some(Node::Box(leader)) => {
                                if let Some(h) = self.hleaders(g.kind, leader, rule_wd, refpos, cur)
                                {
                                    cur.0 = h;
                                    self.pos = at(refpos, cur);
                                    continue;
                                }
                            }
                            _ => {}
                        }
                    }
                    cur.0 += rule_wd;
                }
                Node::Rule(r) => {
                    self.hrule(r, this_box, refpos, cur);
                    cur.0 += r.width;
                }
                Node::Whatsit(text) => self.w.special(text.as_bytes(), self.pos),
                _ => {}
            }
            self.pos = at(refpos, cur);
        }
        self.pos = refpos;
    }

    /// `FIN_RULE` in an hlist: a running height or depth is the box's.
    fn hrule(
        &mut self,
        r: &RuleNode,
        this_box: &BoxNode,
        refpos: (Scaled, Scaled),
        cur: (Scaled, Scaled),
    ) {
        let ht = match RuleNode::is_running(r.height) {
            true => this_box.height,
            false => r.height,
        };
        let dp = match RuleNode::is_running(r.depth) {
            true => this_box.depth,
            false => r.depth,
        };
        if ht + dp > 0 && r.width > 0 {
            let pos = at(refpos, cur);
            self.w.place_rule((pos.0, pos.1 - dp), (r.width, ht + dp));
        }
    }

    /// Leaders of a box in an hlist; where `cur.h` ends, when they were set.
    fn hleaders(
        &mut self,
        kind: LeaderKind,
        leader: &BoxNode,
        rule_wd: Scaled,
        refpos: (Scaled, Scaled),
        cur: (Scaled, Scaled),
    ) -> Option<Scaled> {
        let leader_wd = leader.width;
        if leader_wd <= 0 || rule_wd <= 0 {
            return None;
        }
        let rule_wd = rule_wd + 10;
        let edge = cur.0 + rule_wd;
        let mut lx = 0;
        let mut h = cur.0;
        match kind {
            // `\gleaders` is luatex's: aligned to the shipped box.
            LeaderKind::Aligned => {
                let save = h;
                h = leader_wd * (h / leader_wd);
                if h < save {
                    h += leader_wd;
                }
            }
            LeaderKind::Centred | LeaderKind::Expanded => {
                let lq = rule_wd / leader_wd;
                let lr = rule_wd % leader_wd;
                match kind {
                    LeaderKind::Centred => h += lr / 2,
                    _ => {
                        lx = lr / (lq + 1);
                        h += (lr - (lq - 1) * lx) / 2;
                    }
                }
            }
            LeaderKind::Normal => {}
        }
        while h + leader_wd <= edge {
            self.pos = at(refpos, (h, leader.shift_amount));
            let outer = self.doing_leaders;
            self.doing_leaders = true;
            match leader.vertical {
                true => self.vlist_out(leader),
                false => self.hlist_out(leader),
            }
            self.doing_leaders = outer;
            h += leader_wd + lx;
        }
        Some(edge - 10)
    }

    /// `vlist_out`.
    fn vlist_out(&mut self, this_box: &BoxNode) {
        let refpos = self.pos;
        let mut cur: (Scaled, Scaled) = (0, -this_box.height);
        let top_edge = cur.1;
        let mut cur_glue = 0.0f64;
        let mut cur_g: Scaled = 0;
        self.pos = at(refpos, cur);
        for p in &this_box.list {
            match p {
                Node::Box(b) => {
                    if !b.list.is_empty() {
                        self.pos = at(refpos, (b.shift_amount, cur.1 + b.height));
                        match b.vertical {
                            true => self.vlist_out(b),
                            false => self.hlist_out(b),
                        }
                    }
                    cur.1 += b.height + b.depth;
                }
                Node::Rule(r) => {
                    let ht = r.height + r.depth;
                    self.vrule(r.width, ht, this_box, refpos, cur);
                    cur.1 += ht;
                }
                Node::Glue(g) => {
                    let mut rule_ht = g.spec.natural - cur_g;
                    glue_step(&g.spec, this_box, &mut cur_glue, &mut cur_g);
                    rule_ht += cur_g;
                    if g.kind.is_leaders() {
                        match g.leader.as_deref() {
                            Some(Node::Rule(r)) => {
                                self.vrule(r.width, rule_ht, this_box, refpos, cur);
                                cur.1 += rule_ht;
                                self.pos = at(refpos, cur);
                                continue;
                            }
                            Some(Node::Box(leader)) => {
                                let leader_ht = leader.height + leader.depth;
                                if leader_ht > 0 && rule_ht > 0 {
                                    let rule_ht = rule_ht + 10;
                                    let edge = cur.1 + rule_ht;
                                    let mut lx = 0;
                                    match g.kind {
                                        LeaderKind::Aligned => {
                                            let save = cur.1;
                                            cur.1 = top_edge
                                                + leader_ht * ((cur.1 - top_edge) / leader_ht);
                                            if cur.1 < save {
                                                cur.1 += leader_ht;
                                            }
                                        }
                                        kind => {
                                            let lq = rule_ht / leader_ht;
                                            let lr = rule_ht % leader_ht;
                                            match kind {
                                                LeaderKind::Centred => cur.1 += lr / 2,
                                                _ => {
                                                    lx = lr / (lq + 1);
                                                    cur.1 += (lr - (lq - 1) * lx) / 2;
                                                }
                                            }
                                        }
                                    }
                                    while cur.1 + leader_ht <= edge {
                                        self.pos = at(
                                            refpos,
                                            (leader.shift_amount, cur.1 + leader.height),
                                        );
                                        let outer = self.doing_leaders;
                                        self.doing_leaders = true;
                                        match leader.vertical {
                                            true => self.vlist_out(leader),
                                            false => self.hlist_out(leader),
                                        }
                                        self.doing_leaders = outer;
                                        cur.1 += leader_ht + lx;
                                    }
                                    cur.1 = edge - 10;
                                    self.pos = at(refpos, cur);
                                    continue;
                                }
                            }
                            _ => {}
                        }
                    }
                    cur.1 += rule_ht;
                }
                Node::Kern { width, .. } => cur.1 += width,
                Node::Whatsit(text) => self.w.special(text.as_bytes(), self.pos),
                _ => {}
            }
            self.pos = at(refpos, cur);
        }
        self.pos = refpos;
    }

    /// `FIN_RULE` in a vlist: a running width is the box's.
    fn vrule(
        &mut self,
        width: Scaled,
        ht: Scaled,
        this_box: &BoxNode,
        refpos: (Scaled, Scaled),
        cur: (Scaled, Scaled),
    ) {
        let wd = match RuleNode::is_running(width) {
            true => this_box.width,
            false => width,
        };
        if ht > 0 && wd > 0 {
            let pos = at(refpos, cur);
            self.w.place_rule((pos.0, pos.1 - ht), (wd, ht));
        }
    }
}

/// `pdftex.map`, read once: which Type 1 program a TFM name is drawn with.
fn font_map() -> Option<&'static crate::fontmap::FontMap> {
    static MAP: std::sync::OnceLock<Option<crate::fontmap::FontMap>> = std::sync::OnceLock::new();
    MAP.get_or_init(|| {
        let path = crate::kpse::locate("pdftex.map")?;
        crate::fontmap::FontMap::open(path).ok()
    })
    .as_ref()
}

/// A whole document of shipped pages, written as luatex writes one: each
/// page drawn by [`ship_out`], each font resource a [`crate::pdf::TexFont`]
/// out of its TFM and the program `pdftex.map` names for it, and the file
/// laid out by `crate::pdfdoc` with what the run gave the backend.
///
/// A font no map line names, or whose program is not installed, draws
/// nothing: luatex would make a Type 3 font of its PK bitmaps, which is not
/// ported.
pub fn document(
    pages: &[(BoxNode, Scaled, Scaled)],
    frame: PageFrame,
    fonts: &[ShipFont],
    prepared: crate::backend::Prepared,
) -> Vec<u8> {
    let dd = prepared.vars.decimal_digits;
    // One resource a name, made once for the document.
    let mut resources: std::collections::BTreeMap<u32, std::sync::Arc<crate::pdf::Font>> =
        Default::default();
    let mut out_pages = Vec::new();
    for (page, h, v) in pages {
        let drawn = ship_out(page, *h, *v, frame, fonts, dd);
        let mut p = crate::pdf::Page::letter();
        p.width = frame.width as f64 / crate::pdfpage::BY_ONE_BP;
        p.height = frame.height as f64 / crate::pdfpage::BY_ONE_BP;
        p.content = String::from_utf8_lossy(&drawn.content).into_owned();
        for &r in &drawn.font_order {
            let codes = &drawn.used[&r];
            let font = match resources.get(&r) {
                Some(f) => f.clone(),
                None => {
                    let Some(name) = fonts.get(r as usize).map(|f| f.tfm_name.clone()) else {
                        continue;
                    };
                    let program = font_map()
                        .and_then(|m| m.lookup(&name))
                        .and_then(|e| e.font_file.clone());
                    let Some(tex) =
                        program.and_then(|file| crate::pdf::TexFont::open(&name, &file))
                    else {
                        continue;
                    };
                    let f = std::sync::Arc::new(crate::pdf::Font::Tex(Box::new(tex)));
                    resources.insert(r, f.clone());
                    f
                }
            };
            let key = format!("F{r}");
            p.fonts.push((key.clone(), font));
            p.used.insert(key, codes.clone());
        }
        out_pages.push(p);
    }
    let contents = out_pages
        .iter_mut()
        .map(|p| std::mem::take(&mut p.content))
        .collect();
    crate::pdfdoc::write(&out_pages, contents, prepared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glue::Glue;
    use crate::node::{CharNode, GlueNode};

    fn cmr10_widths() -> Vec<Option<Scaled>> {
        let mut w = vec![None; 256];
        for (c, v) in [
            (b'H', 491521),
            (b'e', 291271),
            (b'l', 182045),
            (b'o', 327681),
            (b'1', 327681),
            (b'w', 473316),
            (b'r', 256683),
            (b'd', 364090),
            (b'.', 182045),
        ] {
            w[usize::from(c)] = Some(v);
        }
        w
    }

    fn char(c: char) -> Node {
        let w = cmr10_widths()[c as usize].unwrap_or(0);
        Node::Char(CharNode {
            font: 1,
            character: c,
            width: w,
            height: 0,
            depth: 0,
        })
    }

    fn hbox(width: Scaled, list: Vec<Node>) -> BoxNode {
        BoxNode {
            width,
            height: 455111,
            list,
            ..BoxNode::default()
        }
    }

    /// Plain luatex's page for `Hello world.`, built as its output routine
    /// builds it -- `\pagebody` holding the paragraph's line under `\topskip`,
    /// `\makefootline` holding the folio centred by two `\hfil`s. The widths,
    /// the `w`-`o` kern, the interword glue, `\hsize`, `\vsize`, the page size
    /// and the origin are luatex's own, measured.
    fn hello_world() -> (BoxNode, Vec<ShipFont>, PageFrame) {
        let fonts = vec![
            ShipFont {
                tfm_name: "nullfont".into(),
                size: 0,
                widths: vec![None; 256],
            },
            ShipFont {
                tfm_name: "cmr10".into(),
                size: 655360,
                widths: cmr10_widths(),
            },
        ];
        let hsize = 30785863;
        let mut line = vec![Node::Box(BoxNode {
            width: 1310720,
            ..BoxNode::default()
        })];
        for c in "Hello".chars() {
            line.push(char(c));
        }
        line.push(Node::Glue(GlueNode::new(Glue {
            natural: 218453,
            stretch: 109226,
            shrink: 72818,
            ..Glue::default()
        })));
        line.push(char('w'));
        line.push(Node::Kern {
            width: -18205,
            explicit: false,
        });
        for c in "orld.".chars() {
            line.push(char(c));
        }
        // `\parfillskip`, at its natural width: the line is not stretched.
        line.push(Node::Glue(GlueNode::new(Glue {
            natural: 0,
            stretch: 65536,
            stretch_order: 1,
            ..Glue::default()
        })));
        let mut first = hbox(hsize, line);
        first.glue_sign = GlueSign::Stretching;
        first.glue_order = 1;
        first.glue_set =
            (hsize - 1310720 - 491521 - 291271 * 2 - 182045 * 4 - 327681 * 2 - 218453 - 473316
                + 18205
                - 256683
                - 364090) as f64
                / 65536.0;
        let fil = || {
            Node::Glue(GlueNode::new(Glue {
                stretch: 65536,
                stretch_order: 1,
                ..Glue::default()
            }))
        };
        let mut footline = hbox(hsize, vec![fil(), char('1'), fil()]);
        footline.glue_sign = GlueSign::Stretching;
        footline.glue_order = 1;
        footline.glue_set = (hsize - 327681) as f64 / 131072.0;
        // `\vbox to\vsize`: `\topskip` puts the baseline 10pt down, the
        // rest is `\vfil`, and `\baselineskip 24pt` puts the footline 24pt
        // below the body.
        let vsize = 42152922;
        let body = BoxNode {
            width: hsize,
            height: vsize,
            vertical: true,
            list: vec![
                Node::Glue(GlueNode::new(Glue {
                    natural: 655360 - 455111,
                    ..Glue::default()
                })),
                Node::Box(first),
            ],
            ..BoxNode::default()
        };
        let page = BoxNode {
            width: hsize,
            height: vsize + 1572864,
            vertical: true,
            list: vec![
                Node::Box(body),
                Node::Glue(GlueNode::new(Glue {
                    natural: 1572864 - 455111,
                    ..Glue::default()
                })),
                Node::Box(footline),
            ],
            ..BoxNode::default()
        };
        let frame = PageFrame {
            width: 40258437,
            height: 52099153,
            h_origin: 4736286,
            v_origin: 4736286,
        };
        (page, fonts, frame)
    }

    /// That page, shipped: the content stream luatex 1.24.0 writes for it,
    /// byte for byte.
    #[test]
    fn plain_tex_s_hello_world_page_is_luatex_s_content_stream() {
        let (page, fonts, frame) = hello_world();
        let drawn = ship_out(&page, 0, 0, frame, &fonts, 3);
        assert_eq!(
            String::from_utf8(drawn.content).unwrap(),
            "BT\n/F1 9.96264 Tf\n1 0 0 1 91.925 710.037 Tm [(Hello)-333(w)27(orld.)]TJ\n\
             1 0 0 1 303.509 55.29 Tm [(1)]TJ\nET\n"
        );
    }

    /// The same page as a whole file, against what luatex writes for
    /// `tests/pdf_cases/two_words.tex` -- the same page, set by plain luatex
    /// -- object by object, where both are installed: every object agrees
    /// but the font program, which is the subsetter's, and the information
    /// dictionary, which names the engine.
    #[test]
    fn the_shipped_page_is_luatex_s_file_but_for_the_font_program() {
        let lua = std::process::Command::new("luatex")
            .arg("--version")
            .output();
        if !lua.is_ok_and(|o| o.status.success())
            || crate::pdf::TexFont::open("cmr10", "cmr10.pfb").is_none()
        {
            eprintln!("skipped: no luatex, or no cmr10");
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("case.tex"),
            "\\catcode`\\{=1 \\catcode`\\}=2\nHello world.\n\\end\n",
        )
        .expect("write");
        let ran = std::process::Command::new("luatex")
            .arg("-interaction=nonstopmode")
            .arg("case.tex")
            .env("SOURCE_DATE_EPOCH", "0")
            .current_dir(dir.path())
            .output();
        assert!(ran.is_ok());
        let theirs = std::fs::read(dir.path().join("case.pdf")).expect("luatex wrote it");
        let (page, fonts, frame) = hello_world();
        let ours = document(
            &[(page, 0, 0)],
            frame,
            &fonts,
            crate::backend::Prepared::default(),
        );
        let objects = |pdf: &[u8]| -> std::collections::BTreeMap<u32, String> {
            let text = String::from_utf8_lossy(&crate::pdf::unpacked(pdf)).into_owned();
            text.split("\nendobj")
                .filter_map(|chunk| {
                    let at = chunk.find(" 0 obj\n")?;
                    let n = chunk[..at].rsplit('\n').next()?.trim().parse().ok()?;
                    Some((n, chunk[at + 7..].to_string()))
                })
                .filter(|(_, b)| {
                    !b.contains("/Length1")
                        && !b.contains("/Producer")
                        && !b.contains("/Type /XRef")
                })
                .collect()
        };
        let (a, b) = (objects(&theirs), objects(&ours));
        assert_eq!(a.len(), 8, "{a:?}");
        assert_eq!(a, b);
        if let Ok(dump) = std::env::var("TEXRS_PDFSHIP_DUMP") {
            let _ = std::fs::write(dump, &ours);
        }
    }
}
