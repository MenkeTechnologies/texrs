//! The smallest honest stomach: text and font metrics into DVI pages.
//!
//! texrs could read a document and say what its words were, and could read and
//! write the DVI format, and had nothing joining the two -- `dvi::Writer` was
//! never called outside its own tests. So a book "ran" and produced no page.
//! This is the join: measure each character in a real font, break the text into
//! lines at a measure, stack the lines down a page, and ship the result.
//!
//! Both paths break a paragraph the way TeX does: `linebreak::break_paragraph`
//! minimises total badness over every feasible sequence of breakpoints
//! (§813-§890) and hyphenates with Knuth's own patterns. The DVI path took the
//! first break that fits until it could SHIP a box tree, because a breaker
//! that prices glue asks for some lines to be shrunk and a writer that set
//! runs of characters at their natural widths had nowhere to put that answer.
//! It builds a `\vbox` of `\hbox`es now and hands it to `shipout::ship_out`
//! (§619-§640): `hpack` distributes the slack (§658) and `hlist_out` writes
//! each piece of glue at the width it was set to (§625), so a full line
//! reaches the measure in the file.
//!
//! What is still NOT `tex.web`'s stomach here: pages are stacked by line count
//! rather than broken by penalty on the DVI path, there is one type size, and
//! a document's own boxes are not nested. A paragraph set here and the same
//! paragraph set by tex will not agree line for line.
//!
//! It is a page you can open, which is the difference between a document that
//! produces nothing and one that produces something imperfect.

use crate::dvi::Writer;
use crate::node::{BoxNode, CharNode, GlueNode, Node, Scaled};
use crate::tfm::Tfm;

/// DVI's unit: 1 sp, of which there are 65536 to the printer's point.
const SP: f64 = 65536.0;

/// The page and paragraph shape, in points.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// Text width, TeX's `\hsize`.
    pub measure: f64,
    /// Text height, TeX's `\vsize`.
    pub height: f64,
    /// Distance between baselines, TeX's `\baselineskip`.
    pub leading: f64,
    /// Where the text starts, from the top-left of the page.
    pub margin: f64,
    /// The font's design size, at which its metrics are stated.
    pub size: f64,
}

impl Default for Layout {
    fn default() -> Self {
        // plain.tex's own page: 6.5in by 8.9in of text, 12pt leading, 1in
        // margins. A reader comparing against a tex run should not first have
        // to account for a different page.
        Self {
            measure: 469.75,
            height: 643.20,
            leading: 12.0,
            margin: 72.0,
            size: 10.0,
        }
    }
}

/// The paper the corpus is set on, and the only paper `pdf::Page` makes: 8.5
/// by 11 inches, in PDF's points.
const PAPER_WIDTH: f64 = 612.0;
const PAPER_HEIGHT: f64 = 792.0;

/// PDF's point is 1/72in; TeX's is 1/72.27in, and every dimension a LaTeX
/// preamble states is in TeX's. The page they land on is 612 by 792 of PDF's,
/// so the two cannot be used interchangeably. Measured, in the content stream
/// of the lualatex-built scifi2/docs/book.pdf: every body line begins at
/// x=68.4, which is 0.95in of PDF points and not of TeX's 68.66.
const BP_PER_PT: f64 = 72.0 / 72.27;

impl Layout {
    /// Take the type size from `\documentclass[11pt]{extreport}`, and with it
    /// the leading LaTeX pairs with that size.
    ///
    /// Every book in the corpus states a size and texrs set all of them at
    /// plain TeX's 10pt on 12pt leading regardless, so an 11pt book got 53
    /// lines on a page where lualatex gives it 48 -- and came out short by
    /// that ratio.
    pub fn absorb_class_options(&mut self, options: &str) {
        for option in options.split(',') {
            let Some(size) = option
                .trim()
                .strip_suffix("pt")
                .and_then(|n| n.parse::<f64>().ok())
                .filter(|size| *size > 0.0)
            else {
                continue;
            };
            self.size = size * BP_PER_PT;
            self.leading = normal_leading(size) * BP_PER_PT;
        }
    }

    /// Take the margins from `\usepackage[margin=0.95in]{geometry}`, and with
    /// them the measure and the text height they leave on the paper.
    ///
    /// Only `margin`, which sets all four sides at once: that is what pandoc
    /// writes and what every book in the corpus asks for. `left`, `top` and
    /// their siblings can each differ, and a `Layout` has ONE margin for all
    /// four sides, so honouring them here would put the text somewhere the
    /// document did not ask for rather than leave it where it was.
    pub fn absorb_geometry_options(&mut self, options: &str) {
        for option in options.split(',') {
            let Some((key, value)) = option.split_once('=') else {
                continue;
            };
            if key.trim() != "margin" {
                continue;
            }
            let Some(margin) = dimen_bp(value.trim()) else {
                continue;
            };
            self.margin = margin;
            self.measure = PAPER_WIDTH - 2.0 * margin;
            self.height = PAPER_HEIGHT - 2.0 * margin;
        }
    }
}

/// The leading LaTeX sets a type size on, taken from the class option files
/// that decide it: `size10.clo:48` sets 10pt on 12, `size11.clo:48` sets 11pt
/// on 13.6, `size12.clo:48` sets 12pt on 14.5, and extsizes' own
/// `size8/9/14/17/20.clo:12` carry the sizes `extreport` adds. A size no file
/// names is set on 1.2 of itself, which is what those files come to and what
/// TeX's `\baselineskip` assumes.
fn normal_leading(size: f64) -> f64 {
    let named = [
        (8.0, 9.5),
        (9.0, 11.0),
        (10.0, 12.0),
        (11.0, 13.6),
        (12.0, 14.5),
        (14.0, 17.0),
        (17.0, 22.0),
        (20.0, 25.0),
    ];
    match named.iter().find(|(pt, _)| *pt == size) {
        Some((_, on)) => *on,
        None => size * 1.2,
    }
}

/// A LaTeX dimension -- `0.95in`, `25mm`, `72bp` -- in PDF points.
///
/// Every TeX unit is two letters, so the suffix that matches is the only one
/// that can.
fn dimen_bp(text: &str) -> Option<f64> {
    let units = [
        ("in", 72.0),
        ("bp", 1.0),
        ("pt", BP_PER_PT),
        ("pc", 12.0 * BP_PER_PT),
        ("mm", 72.0 / 25.4),
        ("cm", 72.0 / 2.54),
    ];
    let (unit, per_unit) = units.iter().find(|(unit, _)| text.ends_with(unit))?;
    let number: f64 = text.strip_suffix(unit)?.trim().parse().ok()?;
    Some(number * per_unit)
}

/// Set `text` in `font`, and return a DVI file.
///
/// The font is named in the DVI by `font_name` so a driver can find it; the
/// metrics come from the `.tfm` the caller opened, which is the same file the
/// driver will use to place the characters.
pub fn to_dvi(text: &str, font: &Tfm, font_name: &str, layout: &Layout) -> Vec<u8> {
    // The single-font path, kept for callers that have one .tfm and want it
    // used literally. Everything else goes through the chain.
    let chain = FontChain {
        fonts: vec![Loaded {
            tfm: font.clone(),
            name: font_name.to_string(),
        }],
        map: Vec::new(),
    };
    to_dvi_chain(text, &chain, layout)
}

/// Set `text` through a font chain, falling back per glyph.
pub fn to_dvi_chain(text: &str, chain: &FontChain, layout: &Layout) -> Vec<u8> {
    let mut w = Writer::new("texrs");
    let at = (layout.size * SP) as i32;
    for (i, f) in chain.fonts.iter().enumerate() {
        // The design size and checksum have to match the .tfm or a driver
        // refuses the file: they are how it checks it has the font the document
        // was set in.
        w.define_font(i as u32, &f.name, at, f.tfm.checksum, at);
    }

    // A `\ref` asks for the number of the sectioning unit its label stands in,
    // which is a fact about the document's structure and needs no page broken:
    // this path can answer it, though it has neither a contents nor a page
    // numbering to answer a `\pageref` with. See `REF`.
    let text = refs_numbered(text);
    let lines = broken_lines(&text, chain, layout);
    let per_page = ((layout.height / layout.leading).floor() as usize).max(1);

    for (page, chunk) in lines.chunks(per_page).enumerate() {
        let counts = [page as i32 + 1, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        // The page is a BOX now, and `shipout` draws it: every position in the
        // file below comes out of `hpack`/`vpack`'s arithmetic rather than
        // being computed here. That is what lets a full line be set to the
        // measure -- its glue is written at the width `hpack` set it to
        // (`tex.web` §625) -- which is the reason this path could not use a
        // breaker that prices glue before.
        let page_box = page_box(chunk, page + 1, chain, layout);
        crate::shipout::ship_out(
            &mut w,
            &page_box,
            counts,
            sp(layout.margin),
            sp(layout.margin),
        );
    }
    w.finish()
}

/// Points to scaled points, which is the unit every node carries (§101).
fn sp(points: f64) -> Scaled {
    (points * SP) as Scaled
}

/// `\baselineskip` for `plain.tex`'s footline: 24pt below the page body.
const FOOTLINE_SKIP: f64 = 24.0;

/// The page number at the foot of the page, where `plain.tex`'s output routine
/// puts it.
///
/// A page is not the text alone. `\output` in `plain.tex` is `\plainoutput`,
/// which ships `\vbox{\makeheadline\pagebody\makefootline}`, and
/// `\makefootline` is `\baselineskip24pt \line{\the\footline}` with
/// `\footline={\hss\tenrm\folio\hss}` — an `\hbox to \hsize` holding the folio
/// between two `\hss`. So every page `tex` writes carries its number, and a
/// DVI without one differs from `tex`'s in the last character of every page.
///
/// The centring is not a division: it is `hpack` distributing the slack
/// between the two `\hss` glues (`tex.web` §658, §625), which is what puts the
/// odd scaled point on one side rather than losing it. The same code sets
/// every other box, so the folio is centred the way `\centerline` is.
fn footline_box(folio: usize, chain: &FontChain, layout: &Layout) -> BoxNode {
    let mut list = vec![Node::Glue(GlueNode::new(crate::box_::fill::ss()))];
    list.extend(line_hlist(&folio.to_string(), chain, layout));
    list.push(Node::Glue(GlueNode::new(crate::box_::fill::ss())));
    crate::pack::hpack(
        list,
        crate::pack::Spec::Exactly(sp(layout.measure)),
        crate::pack::Tolerances::plain(),
        None,
    )
    .node
}

/// One page as a `\vbox`: the lines, the glue between their baselines, and the
/// folio at the foot.
///
/// `plain.tex`'s output routine ships `\vbox{\makeheadline\pagebody
/// \makefootline}`, and this is that box with no headline. Everything about
/// where the ink lands is in the box's own dimensions from here on -- nothing
/// downstream computes a position.
fn page_box(lines: &[BrokenLine], folio: usize, chain: &FontChain, layout: &Layout) -> BoxNode {
    let mut list: Vec<Node> = Vec::with_capacity(lines.len() * 2 + 2);
    // §679: `prev_depth` is `ignore_depth` before the first box on a list.
    let mut prev_depth = crate::node::IGNORE_DEPTH;
    // Where `vlist_out` will be, measured from the top of the text.
    let mut at: Scaled = 0;
    for line in lines {
        let b = line_box(line, chain, layout);
        let skip = interline(prev_depth, b.height, layout);
        at += skip + b.height + b.depth;
        prev_depth = b.depth;
        list.push(Node::Kern {
            width: skip,
            explicit: false,
        });
        list.push(Node::Box(b));
    }
    let foot = footline_box(folio, chain, layout);
    // `\makefootline` is `\baselineskip24pt\line{\the\footline}`, so the folio's
    // baseline is 24pt below the bottom of the text.
    let drop = sp(layout.height + FOOTLINE_SKIP) - at - foot.height;
    list.push(Node::Kern {
        width: drop,
        explicit: false,
    });
    list.push(Node::Box(foot));
    crate::pack::vpack(list, crate::pack::NATURAL, crate::pack::Tolerances::plain()).node
}

/// §679: the glue that separates two baselines.
///
/// `\baselineskip` less the depth of what is above and the height of what is
/// below, so consecutive baselines are one `\baselineskip` apart however tall
/// the lines are -- and `\lineskip` instead when two lines would otherwise
/// touch.
fn interline(prev_depth: Scaled, height: Scaled, layout: &Layout) -> Scaled {
    // plain.tex: `\lineskiplimit=0pt`, `\lineskip=1pt`.
    const LINE_SKIP_LIMIT: Scaled = 0;
    // Before the first box, `prev_depth` is `ignore_depth` and TeX puts
    // `\topskip` in rather than interline glue. This path sets `\topskip` to
    // `\baselineskip`, which is what puts the first baseline one leading below
    // the top margin.
    let above = match prev_depth <= crate::node::IGNORE_DEPTH {
        true => 0,
        false => prev_depth,
    };
    let d = sp(layout.leading) - above - height;
    match d < LINE_SKIP_LIMIT {
        true => sp(1.0),
        false => d,
    }
}

/// One line as an `\hbox`.
///
/// A full line is set TO the measure, so `hpack` distributes the slack over its
/// interword glue and `hlist_out` writes each piece at the width it was set to
/// (§625). The line that ends a paragraph is left at its natural width, which
/// is what `\parfillskip` amounts to here (§816).
fn line_box(line: &BrokenLine, chain: &FontChain, layout: &Layout) -> BoxNode {
    let spec = match line.justify {
        true => crate::pack::Spec::Exactly(sp(layout.measure)),
        false => crate::pack::NATURAL,
    };
    crate::pack::hpack(
        line_hlist(&line.text, chain, layout),
        spec,
        crate::pack::Tolerances::plain(),
        None,
    )
    .node
}

/// One line's horizontal list: `tex.web` §1030-§1041's main loop, over text
/// that has already been broken.
///
/// Ordinary characters are gathered rather than turned into nodes one at a
/// time: a ligature is a fact about two NEIGHBOURING characters, so the run has
/// to be whole before any of it becomes a node. Everything below that is not an
/// ordinary character ends the run before it does its own work, which is also
/// what §545 requires -- a ligature never reaches across a `\special`, a font
/// change or a piece of glue.
fn line_hlist(line: &str, chain: &FontChain, layout: &Layout) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::new();
    let mut pending = String::new();
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        // Colour arrives in the text stream as U+0001 r,g,b U+0002 ... U+0003,
        // which the runtime writes where `\textcolor` was. DVI has no colour of
        // its own; a driver is told through a `\special`, and this pair is the
        // one dvipdfmx and dvips both read.
        if ch == '\u{1}' {
            append_carried(&mut out, &std::mem::take(&mut pending), chain, layout);
            let mut spec = String::new();
            for c in chars.by_ref() {
                if c == '\u{2}' {
                    break;
                }
                spec.push(c);
            }
            let parts: Vec<&str> = spec.split(',').collect();
            if parts.len() == 3 {
                out.push(Node::Whatsit(format!(
                    "color push rgb {} {} {}",
                    parts[0], parts[1], parts[2]
                )));
            }
            continue;
        }
        if ch == '\u{3}' {
            append_carried(&mut out, &std::mem::take(&mut pending), chain, layout);
            out.push(Node::Whatsit("color pop".to_string()));
            continue;
        }
        // A face marker names a font this path does not have: the chain is
        // Computer Modern and its fallbacks, chosen by which glyph is where,
        // not by what the document asked for. Skipping it is not just a
        // refusal to honour it -- the code character after U+000E is a LETTER,
        // so leaving the pair in the stream sets an `m` in the middle of every
        // \texttt in the book. It ends the run as well, because `tex` sets the
        // two sides of a face change in different fonts and §545's implicit
        // boundary stands between them.
        if ch == FACE_PUSH {
            append_carried(&mut out, &std::mem::take(&mut pending), chain, layout);
            let _ = chars.next();
            continue;
        }
        if ch == FACE_POP {
            append_carried(&mut out, &std::mem::take(&mut pending), chain, layout);
            continue;
        }
        // The list indent names a position this path does not honour -- it sets
        // every line at the margin -- and the depth digit after it is an
        // ordinary character, so leaving the pair in would put a `1` in front
        // of every list item.
        if ch == LIST_INDENT {
            append_carried(&mut out, &std::mem::take(&mut pending), chain, layout);
            let _ = chars.next();
            continue;
        }
        // A cross-reference span is the marker, the code saying which of the
        // three it is, the label key, and the marker again -- and the whole of
        // it is skipped. The key is ordinary letters, so leaving it in would
        // SET it: a pandoc label is a sentence of hyphenated words, and every
        // heading in every book carries one.
        if ch == REF {
            append_carried(&mut out, &std::mem::take(&mut pending), chain, layout);
            for c in chars.by_ref() {
                if c == REF {
                    break;
                }
            }
            continue;
        }
        pending.push(ch);
    }
    append_carried(&mut out, &pending, chain, layout);
    out
}

/// Turn a stretch of ordinary text into nodes: the characters its fonts
/// actually set, the kerns between them, and glue for each interword space.
///
/// The translation from what the document holds to what the page carries is
/// `FontChain::carried` -- §1034-§1040's main loop -- so this builds what
/// `FontChain::width_of` measured rather than a second reading of the string.
fn append_carried(out: &mut Vec<Node>, text: &str, chain: &FontChain, layout: &Layout) {
    for item in chain.carried(text.chars()) {
        match item {
            // An interword space is GLUE (§1041), so what reaches the file is a
            // movement rather than a glyph (§625). `tex` writes no character
            // for a space, and the glyph cmr10 has at code 32 is a Polish
            // suppressed-l, so a DVI that set one differed from tex's in the
            // very first word of every document.
            Carried::Space => out.push(Node::Glue(GlueNode::new(interword_glue(chain, layout)))),
            Carried::Set { font, sets } => {
                for set in sets {
                    match set {
                        crate::tfm::Set::Char(code) => {
                            let m = chain.fonts[font].tfm.char(code).unwrap_or_default();
                            out.push(Node::Char(CharNode {
                                font,
                                // The DVI file names a font SLOT, and this is
                                // the slot the chain resolved.
                                character: char::from(code),
                                width: sp(m.width * layout.size),
                                height: sp(m.height * layout.size),
                                depth: sp(m.depth * layout.size),
                            }));
                        }
                        // §625 again: a kern is a movement, not a glyph.
                        crate::tfm::Set::Kern(by) => out.push(Node::Kern {
                            width: sp(by * layout.size),
                            explicit: false,
                        }),
                    }
                }
            }
        }
    }
}

/// §1042: an interword space is the font's own `\fontdimen2`, stretching by
/// `\fontdimen3` and shrinking by `\fontdimen4`.
///
/// Each font's own, not cmr10's fractions applied to everything: those three
/// numbers are why a monospaced face does not stretch and a text face does.
fn interword_glue(chain: &FontChain, layout: &Layout) -> crate::glue::Glue {
    let p = chain.fonts[0].tfm.params;
    crate::glue::Glue {
        natural: sp(p.space * layout.size),
        stretch: sp(p.stretch * layout.size),
        stretch_order: 0,
        shrink: sp(p.shrink * layout.size),
        shrink_order: 0,
    }
}

/// One line of the DVI path, and what is to be done with it.
#[derive(Clone, Debug, PartialEq)]
pub struct BrokenLine {
    pub text: String,
    /// Whether the line is set TO the measure. A FULL line is: `hpack`
    /// distributes the slack over its interword glue and the shipper writes
    /// each piece at the width it was set to (`tex.web` §625). The line that
    /// ends a paragraph is not -- that is what `\parfillskip` amounts to
    /// (§816) -- and neither is a line the author broke.
    pub justify: bool,
}

/// Break `text` into lines that fit the measure, for the DVI path.
///
/// The same breaker the PDF path uses: `linebreak::break_paragraph` minimises
/// the total demerits of the WHOLE paragraph over every feasible set of
/// breakpoints (§813-§890) and hyphenates with Knuth's patterns (§891). This
/// path took the first break that fitted until there was a node-list shipper,
/// because a breaker that prices glue asks for some lines to be SHRUNK and
/// there was nowhere to put that answer: the writer set a run of characters at
/// their natural widths and a shrunk line drew out past the measure. `hpack`
/// sets the glue now and `hlist_out` writes it at the width it was set to, so
/// the answer has somewhere to go.
pub fn break_lines(text: &str, font: &Tfm, layout: &Layout) -> Vec<String> {
    let chain = FontChain {
        fonts: vec![Loaded {
            tfm: font.clone(),
            name: String::from("cmr10"),
        }],
        map: Vec::new(),
    };
    break_lines_chain(text, &chain, layout)
}

/// The same, measuring through a font chain.
pub fn break_lines_chain(text: &str, chain: &FontChain, layout: &Layout) -> Vec<String> {
    broken_lines(text, chain, layout)
        .into_iter()
        .map(|line| line.text)
        .collect()
}

/// The same again, keeping what the shipper needs to know about each line.
pub fn broken_lines(text: &str, chain: &FontChain, layout: &Layout) -> Vec<BrokenLine> {
    use rayon::prelude::*;
    // Paragraph-parallel. A paragraph is broken independently of its
    // neighbours: the measure is the same for all of them, and no state crosses
    // the blank line between two, so this fans out with nothing to
    // synchronise. `map` keeps them in order, which matters -- a book whose
    // paragraphs arrived in completion order would be a different book.
    let paras: Vec<&str> = text.split("\n\n").collect();
    paras
        .par_iter()
        .map(|para| break_paragraph(para, chain, layout))
        .reduce(Vec::new, |mut acc, mut more| {
            acc.append(&mut more);
            acc
        })
}

/// One paragraph, by total demerits (§813).
fn break_paragraph(para: &str, chain: &FontChain, layout: &Layout) -> Vec<BrokenLine> {
    // A line the author broke is set as it stands: a code listing is what the
    // program is, and a table's rows are rows. Neither is stretched to the
    // measure, and neither is offered to the breaker.
    let fixed = |lines: Vec<String>| -> Vec<BrokenLine> {
        lines
            .into_iter()
            .map(|text| BrokenLine {
                text,
                justify: false,
            })
            .collect()
    };
    // A picture sets nothing on this path, and says so by setting nothing.
    //
    // DVI has no path model: a drawing reaches a driver as a `\special`, which
    // every driver reads differently and which dvipdfmx answers by running
    // PostScript. Emitting the picture's source as characters would set several
    // hundred glyphs of TikZ where the document asked for a diagram, and
    // emitting a rule in its place would draw something the document does not
    // contain. So `--dvi` leaves the room empty and draws nothing: what a
    // picture comes to there is the PDF path, and this refuses rather than
    // guesses. See `PICTURE`.
    if is_picture_para(para) {
        return Vec::new();
    }
    // A code listing is already broken, by the author. Its lines are what the
    // program is; the measure has no say in where they end.
    if let Some(code) = listing_lines(para) {
        return fixed(code.map(str::to_string).collect());
    }
    // A table's rows are lines too. This path has no column measure of its own
    // and no way to draw a rule, so the cells are spaced and the rules left
    // out -- but the rows stop running into one another, and no table mark
    // reaches the shipper to be drawn as whatever glyph the font has in that
    // slot.
    if para.contains(TABLE_ROW) {
        return fixed(
            table_entries(para)
                .into_iter()
                .filter_map(|entry| match entry {
                    Entry::Row(cells) => Some(cells.join("  ")),
                    Entry::Rule(_) => None,
                })
                .collect(),
        );
    }

    // Everything else goes through §813's breaker. The pieces are the same
    // ones the PDF path measures -- a word, or the fragments a hyphenation
    // point cuts it into -- measured through this chain rather than through a
    // face, because this path has one face.
    let width_of = |text: &str, _face: Face, size: f64| chain.width_of(text, size);
    let mut colours: Vec<Spec> = Vec::new();
    let mut faces: Vec<Face> = Vec::new();
    let mut sizes: Vec<TypeSize> = vec![TypeSize {
        size: layout.size,
        leading: layout.leading,
    }];
    let mut pieces: Vec<crate::linebreak::Piece> = Vec::new();
    for word in words_carrying_refs(para) {
        if let Some(previous) = pieces.last_mut() {
            previous.after = crate::linebreak::After::Glue(chain.width(' ', layout.size));
        }
        measure_word(
            &word,
            &mut colours,
            &mut faces,
            &mut sizes,
            &width_of,
            &mut pieces,
        );
    }
    let breaks = crate::linebreak::break_paragraph(&pieces, layout.measure);

    let mut lines = Vec::with_capacity(breaks.len());
    let mut from = 0usize;
    for (number, end) in breaks.iter().enumerate() {
        let mut text = String::new();
        for (offset, piece) in pieces[from..*end].iter().enumerate() {
            // The pieces of one word run together; two words are joined by the
            // space that stood between them.
            if offset > 0
                && matches!(
                    pieces[from + offset - 1].after,
                    crate::linebreak::After::Glue(_)
                )
            {
                text.push(' ');
            }
            text.push_str(&piece.text);
        }
        // A line ending inside a word carries the hyphen the word did not
        // write. One ending after a hyphen the AUTHOR wrote already has it.
        if matches!(
            pieces[*end - 1].after,
            crate::linebreak::After::Discretionary(_)
        ) {
            text.push('-');
        }
        from = *end;
        lines.push(BrokenLine {
            justify: number + 1 != breaks.len(),
            text,
        });
    }
    lines
}

/// Find a `.tfm` by name, asking the TeX installation first.
///
/// `kpsewhich` is how every TeX program answers this question, so asking it
/// gets the same answer the driver will get when it goes looking for the same
/// font. The fixed paths are the fallback for a machine without it.
pub fn find_font(name: &str) -> Option<std::path::PathBuf> {
    // Answered once per name per run. Finding a .tfm means spawning kpsewhich,
    // which reads TeX Live's ls-R databases and costs the better part of a
    // second; the typesetting path asks for the same handful of fonts over and
    // over, and that -- not the typesetting -- was why setting a three-line
    // document took twenty-four seconds.
    static SEEN: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, Option<std::path::PathBuf>>>,
    > = std::sync::OnceLock::new();
    let seen = SEEN.get_or_init(Default::default);
    if let Some(hit) = seen.lock().ok().and_then(|m| m.get(name).cloned()) {
        return hit;
    }
    let found = find_font_uncached(name);
    if let Ok(mut m) = seen.lock() {
        m.insert(name.to_string(), found.clone());
    }
    found
}

/// The lookup itself, which is what costs.
fn find_font_uncached(name: &str) -> Option<std::path::PathBuf> {
    let file = format!("{name}.tfm");
    if let Ok(out) = std::process::Command::new("kpsewhich").arg(&file).output() {
        if out.status.success() {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !p.is_empty() && std::path::Path::new(&p).exists() {
                return Some(std::path::PathBuf::from(p));
            }
        }
    }
    for root in [
        "/usr/local/texlive",
        "/Library/TeX/texbin/../../texmf-dist",
        "/usr/share/texmf-dist",
    ] {
        let hit = std::process::Command::new("find")
            .args([root, "-name", &file, "-maxdepth", "8"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string()
            })
            .filter(|p| !p.is_empty());
        if let Some(p) = hit {
            return Some(std::path::PathBuf::from(p));
        }
    }
    None
}

/// A font and the name a DVI file calls it by.
pub struct Loaded {
    pub tfm: Tfm,
    pub name: String,
}

/// A chain of fonts, tried in order for each character.
///
/// This is `luaotfload.add_fallback` in a TFM world, and it is the reason the
/// publication scripts required LuaTeX: a per-glyph fallback fixes missing
/// arrows, box drawing and symbols in EVERY context, verbatim code included,
/// and no other engine offered one. A `.tfm` addresses 256 slots, so a chain
/// here is not "another Unicode font" but another 256-slot font plus the map
/// saying which character lives in which of its slots.
///
/// Resolution order per character: the primary font's own map, then each
/// fallback in turn, then an ASCII stand-in for the shapes Computer Modern
/// simply does not have. What it never does is drop the character silently --
/// a glyph that vanishes takes the meaning of a line with it, and a line that
/// reads `a -- b` where the source said `a → b` is worse than one that says so.
pub struct FontChain {
    pub fonts: Vec<Loaded>,
    /// Where a character lives: which font of the chain, and which slot.
    map: Vec<(char, usize, u8)>,
}

/// Computer Modern's own positions for the characters these documents use.
///
/// `cmr10` is the text font; `cmsy10` is the symbol font and carries the
/// arrows, the set operators AND the section and paragraph marks -- those last
/// two are not in the text font, which is the mistake this table was written
/// with the first time: `§` was pointed at cmr10 slot 120 and printed an `x`.
///
/// The slots are read off what `tex` itself emits for `\S`, `\P`,
/// `\rightarrow`, `\leftarrow`, `\cup` and `\times` -- `dvitype` on its
/// output names the character each one set. A wrong slot here prints a wrong
/// glyph in silence, so guessing at them is not good enough.
const CM_SYMBOLS: &[(char, &str, u8)] = &[
    ('→', "cmsy10", 33),
    ('←', "cmsy10", 32),
    ('↔', "cmsy10", 36),
    ('−', "cmsy10", 0),
    ('×', "cmsy10", 2),
    ('÷', "cmsy10", 4),
    ('±', "cmsy10", 6),
    ('∪', "cmsy10", 91),
    ('∩', "cmsy10", 92),
    ('≤', "cmsy10", 20),
    ('≥', "cmsy10", 21),
    ('≠', "cmsy10", 54),
    ('·', "cmsy10", 1),
    ('∞', "cmsy10", 49),
    ('§', "cmsy10", 120),
    ('¶', "cmsy10", 123),
    ('†', "cmsy10", 121),
    ('‡', "cmsy10", 122),
];

/// Shapes Computer Modern has no glyph for, and the nearest honest stand-in.
///
/// Box drawing is the case that matters here: the documents draw trees with it,
/// and cm has no box-drawing at all. A rule could be drawn for each, but a
/// character grid built out of rules is a different picture; the ASCII shapes
/// keep the tree readable and keep the columns lined up, which is what the
/// drawing is for.
const APPROXIMATIONS: &[(char, &str)] = &[
    // Computer Modern has no copyright sign; `(c)` is what a reader of the text
    // would have written anyway.
    ('©', "(c)"),
    ('®', "(R)"),
    ('™', "(TM)"),
    ('—', "---"),
    ('–', "--"),
    ('…', "..."),
    ('─', "-"),
    ('│', "|"),
    ('├', "|-"),
    ('└', "`-"),
    ('┌', ",-"),
    ('┐', "-."),
    ('┘', "-'"),
    ('┬', "-,-"),
    ('┴', "-'-"),
    ('┼', "-+-"),
    ('•', "*"),
    ('“', "``"),
    ('”', "''"),
    ('‘', "`"),
    ('’', "'"),
    // The double rules, the blocks and the marks the corpus draws with, none of
    // which any of the three sources above carries: not Computer Modern, not
    // WinAnsi, not the Symbol font. Counted over the 41 books, U+2500 appears
    // 9,134 times, U+2550 700 and U+2588 473, so without a stand-in the tables
    // and the bars in them are holes on the page.
    ('═', "="),
    ('║', "|"),
    ('╔', ",="),
    ('╗', "=."),
    ('╚', "`="),
    ('╝', "='"),
    ('╠', "|="),
    ('╣', "=|"),
    ('╦', "=,="),
    ('╩', "='="),
    ('╬', "=+="),
    ('█', "#"),
    ('▉', "#"),
    ('▊', "#"),
    ('▋', "#"),
    ('▌', "|"),
    ('▍', "|"),
    ('▎', "|"),
    ('▏', "|"),
    ('▄', "_"),
    ('▅', "_"),
    ('▂', "_"),
    ('●', "*"),
    ('○', "o"),
    ('◆', "*"),
    ('◇', "o"),
    ('★', "*"),
    ('☆', "*"),
    ('▲', "^"),
    ('▼', "v"),
    ('►', ">"),
    ('▶', ">"),
    ('◀', "<"),
    ('❯', ">"),
    // A tick and a cross, as a terminal writes them when it has no glyph
    // either. A checklist whose ticks vanished reads as a list of things that
    // were NOT done, which is the opposite of what it says.
    ('✓', "+"),
    ('✔', "+"),
    ('✗', "x"),
    ('✘', "x"),
    // The arrows no font on either path carries. The length is part of what a
    // long arrow says, so a long one is not set as a short one.
    ('↕', "|"),
    ('⟶', "-->"),
    ('⟵', "<--"),
    ('⟷', "<->"),
    ('⟹', "==>"),
    ('⟸', "<=="),
    ('↦', "|->"),
    ('↪', "->"),
    ('↩', "<-"),
    ('↗', "/"),
    ('↘', "\\"),
    ('↙', "/"),
    ('↖', "\\"),
    ('∓', "-+"),
    ('∘', "o"),
    ('↯', "!"),
    ('⚠', "!"),
    ('ℹ', "i"),
    ('┤', "-|"),
    ('╭', ",-"),
    ('╮', "-."),
    ('╰', "`-"),
    ('╯', "-'"),
    ('▁', "_"),
    ('▃', "_"),
    ('▆', "#"),
    ('▇', "#"),
    ('░', "."),
    ('▒', ":"),
    ('▓', "#"),
    ('▸', ">"),
    ('▹', ">"),
    ('◂', "<"),
    // The superscript and subscript digits. WinAnsi has ¹ ² ³ and stops, so
    // the rest of a footnote's numbering came out as nothing at all.
    ('⁰', "0"),
    ('⁴', "4"),
    ('⁵', "5"),
    ('⁶', "6"),
    ('⁷', "7"),
    ('⁸', "8"),
    ('⁹', "9"),
    ('ⁿ', "n"),
    ('₀', "0"),
    ('₁', "1"),
    ('₂', "2"),
    ('₃', "3"),
    ('₄', "4"),
    // The Mac modifier keys, which these books name in every keybinding table.
    // Spelling them is what a reader without the glyph needs; dropping them
    // leaves a shortcut with no key in it.
    ('⌘', "Cmd"),
    ('⌥', "Opt"),
    ('⌃', "Ctrl"),
    ('⇧', "Shift"),
];

/// The characters a document spells as a control sequence.
///
/// `\rightarrow` is not decoration: a document that writes one and meets an
/// engine that has never heard of it stops dead -- `! Undefined control
/// sequence \rightarrow.` -- and produces no page at all. Each name yields the
/// CHARACTER it means and nothing else, so every table that decides how a
/// character is DRAWN -- `CM_SYMBOLS` for the DVI path, `SYMBOL_FONT` and
/// `APPROXIMATIONS` for the PDF one -- stays the only place that decides it. A
/// name mapped straight to a font slot would be a second such table, and it
/// would drift from the first the moment either was touched.
///
/// The characters are the ones LaTeX's own `fontmath.ltx` and `latex.ltx` give
/// these names, read as Unicode: `\to` and `\rightarrow` are one macro in TeX
/// and are one entry here.
pub const SYMBOL_MACROS: &[(&str, char)] = &[
    // Arrows. `\to` and `\gets` are plain TeX's names for the first two.
    ("rightarrow", '→'),
    ("to", '→'),
    ("leftarrow", '←'),
    ("gets", '←'),
    ("uparrow", '↑'),
    ("downarrow", '↓'),
    ("leftrightarrow", '↔'),
    ("updownarrow", '↕'),
    ("Rightarrow", '⇒'),
    ("Leftarrow", '⇐'),
    ("Leftrightarrow", '⇔'),
    ("Uparrow", '⇑'),
    ("Downarrow", '⇓'),
    ("longrightarrow", '⟶'),
    ("longleftarrow", '⟵'),
    ("longleftrightarrow", '⟷'),
    ("Longrightarrow", '⟹'),
    ("Longleftarrow", '⟸'),
    ("mapsto", '↦'),
    ("hookrightarrow", '↪'),
    ("hookleftarrow", '↩'),
    ("nearrow", '↗'),
    ("searrow", '↘'),
    ("swarrow", '↙'),
    ("nwarrow", '↖'),
    // Greek, lower case. TeX has no `\omicron`: it is the letter `o`, and a
    // document writes it as one.
    ("alpha", 'α'),
    ("beta", 'β'),
    ("gamma", 'γ'),
    ("delta", 'δ'),
    // `\epsilon` is the lunate U+03F5 in TeX and `\varepsilon` the open one;
    // neither the Symbol font nor a text face carries the lunate, so both are
    // read as U+03B5, which is the letter either way.
    ("epsilon", 'ε'),
    ("varepsilon", 'ε'),
    ("zeta", 'ζ'),
    ("eta", 'η'),
    ("theta", 'θ'),
    ("vartheta", 'ϑ'),
    ("iota", 'ι'),
    ("kappa", 'κ'),
    ("lambda", 'λ'),
    ("mu", 'μ'),
    ("nu", 'ν'),
    ("xi", 'ξ'),
    ("pi", 'π'),
    ("varpi", 'ϖ'),
    ("rho", 'ρ'),
    ("sigma", 'σ'),
    ("varsigma", 'ς'),
    ("tau", 'τ'),
    ("upsilon", 'υ'),
    ("phi", 'φ'),
    ("varphi", 'ϕ'),
    ("chi", 'χ'),
    ("psi", 'ψ'),
    ("omega", 'ω'),
    // Greek, upper case. Only the letters that differ from a Latin capital
    // have a name in TeX, which is why there is no `\Alpha`.
    ("Gamma", 'Γ'),
    ("Delta", 'Δ'),
    ("Theta", 'Θ'),
    ("Lambda", 'Λ'),
    ("Xi", 'Ξ'),
    ("Pi", 'Π'),
    ("Sigma", 'Σ'),
    ("Upsilon", 'Υ'),
    ("Phi", 'Φ'),
    ("Psi", 'Ψ'),
    ("Omega", 'Ω'),
    // Relations.
    ("le", '≤'),
    ("leq", '≤'),
    ("ge", '≥'),
    ("geq", '≥'),
    ("ne", '≠'),
    ("neq", '≠'),
    ("approx", '≈'),
    ("equiv", '≡'),
    ("sim", '∼'),
    ("cong", '≅'),
    ("propto", '∝'),
    ("subset", '⊂'),
    ("supset", '⊃'),
    ("subseteq", '⊆'),
    ("supseteq", '⊇'),
    ("in", '∈'),
    ("notin", '∉'),
    ("ni", '∋'),
    ("perp", '⊥'),
    // Operators and the rest of the mathematics these books write.
    ("times", '×'),
    ("div", '÷'),
    ("pm", '±'),
    ("mp", '∓'),
    ("cdot", '⋅'),
    ("bullet", '•'),
    ("ast", '∗'),
    ("circ", '∘'),
    ("oplus", '⊕'),
    ("otimes", '⊗'),
    ("cup", '∪'),
    ("cap", '∩'),
    ("emptyset", '∅'),
    ("varnothing", '∅'),
    ("forall", '∀'),
    ("exists", '∃'),
    ("neg", '¬'),
    ("lnot", '¬'),
    ("land", '∧'),
    ("wedge", '∧'),
    ("lor", '∨'),
    ("vee", '∨'),
    ("sum", '∑'),
    ("prod", '∏'),
    ("int", '∫'),
    ("partial", '∂'),
    ("nabla", '∇'),
    ("infty", '∞'),
    ("surd", '√'),
    ("angle", '∠'),
    ("therefore", '∴'),
    ("aleph", 'ℵ'),
    ("prime", '′'),
    ("diamond", '◆'),
    ("star", '★'),
    ("dagger", '†'),
    ("ddagger", '‡'),
    // The text symbols that are spelled as a control sequence and are not
    // characters a keyboard has. `\S` and `\P` are Computer Modern's section
    // and paragraph marks, which `CM_SYMBOLS` already places.
    ("S", '§'),
    ("P", '¶'),
    ("dag", '†'),
    ("ddag", '‡'),
    ("copyright", '©'),
    ("textcopyright", '©'),
    ("textregistered", '®'),
    ("texttrademark", '™'),
    ("pounds", '£'),
    ("textsterling", '£'),
    ("texteuro", '€'),
    ("euro", '€'),
    ("textdegree", '°'),
    ("textbullet", '•'),
    ("textperiodcentered", '·'),
    ("textemdash", '—'),
    ("textendash", '–'),
    ("textellipsis", '…'),
    ("textmu", 'µ'),
    ("textpm", '±'),
    ("texttimes", '×'),
    ("textdiv", '÷'),
];

/// The character `\name` stands for, if it is one of the symbols above.
///
/// Asked only where a control sequence would otherwise be undefined, so a
/// document that defines its own `\star` keeps it.
pub fn symbol_char(name: &str) -> Option<char> {
    SYMBOL_MACROS
        .iter()
        .find(|(macro_name, _)| *macro_name == name)
        .map(|(_, ch)| *ch)
}

/// The characters of `text` that will actually be SET, with the colour and face
/// markers dropped.
///
/// A marker's SPEC -- the `r,g,b` between U+0001 and U+0002 -- has to be
/// skipped whole. Charging the three control characters nothing is not enough:
/// the digits and commas between them are ordinary characters the font does
/// have, so a coloured word measured that way comes out wider than it sets and
/// every line after it breaks short. The DVI side learned that first and the
/// PDF side kept charging for them, breaking its lines at a quarter of the
/// measure -- so the skip lives here, where both measuring paths reach it,
/// rather than being written out twice and drifting apart again.
///
/// A face marker is the same shape and skipped the same way: U+000E and the
/// ONE character naming the face, U+000F on its own.
pub fn printing_chars(text: &str) -> impl Iterator<Item = char> + '_ {
    let mut in_spec = false;
    let mut face_code = false;
    let mut in_ref = false;
    let mut in_picture = false;
    let mut in_size_spec = false;
    let mut in_image = false;
    text.chars().filter(move |&ch| match ch {
        // A cross-reference span -- the marker, its code, the label key, and
        // the marker again -- is a question the typesetter answers by
        // REPLACING it, so nothing inside one is ever drawn. A label survives
        // to the page because that is how `label_pages` finds which page it
        // fell on, and it must measure nothing where it stands.
        REF => {
            in_ref = !in_ref;
            false
        }
        _ if in_ref => false,
        // A picture span is the same shape and is measured the same way: it is
        // a block DRAWN on the page, not a run of characters set into a line,
        // so nothing in it -- neither the marker nor the base64 between the two
        // -- has a width. Charging the font for it would push a line off the
        // measure by several hundred characters of picture source.
        PICTURE => {
            in_picture = !in_picture;
            false
        }
        _ if in_picture => false,
        // An image is a block DRAWN on the page, not a run of characters:
        // neither the marker nor the base64 path between the two has a width.
        IMAGE => {
            in_image = !in_image;
            false
        }
        _ if in_image => false,
        '\u{1}' => {
            in_spec = true;
            false
        }
        '\u{2}' => {
            in_spec = false;
            false
        }
        '\u{3}' => false,
        // The centring and vertical-space markers are instructions to the
        // page, not glyphs: a centred line carries one at its head, and
        // measuring it would push the line off centre by whatever the font
        // happens to have in that slot.
        CENTRE | CENTRE_END | VERTICAL_SPACE | JUSTIFY => false,
        // The table marks are instructions too: a cell boundary and a row end
        // are where the columns are, and a rule is drawn rather than set.
        TABLE_CELL | TABLE_ROW => false,
        // The list indent is a position, not a character: it says how far in
        // this line starts. Its depth digit is skipped the way a face code is,
        // below.
        LIST_INDENT => {
            face_code = true;
            false
        }
        FACE_PUSH => {
            face_code = true;
            false
        }
        FACE_POP => false,
        // A size marker's spec is digits, a semicolon and a dot, every one of
        // which the font has a real width for. Measuring it would charge a
        // heading for the number it was set at -- which is the fault the
        // colour spec already had, and it cost whole pages.
        SIZE_PUSH => {
            in_size_spec = !in_size_spec;
            false
        }
        _ if in_size_spec => false,
        SIZE_POP => false,
        // Its code character goes the same way the face's does, and so does
        // the code saying which part of a longtable a line is -- and the code
        // on a contents mark, which says whether the mark is a request, a
        // heading the contents lists, or where the page numbering starts.
        TABLE_MARK | LONGTABLE | TOC => {
            face_code = true;
            false
        }
        _ if face_code => {
            face_code = false;
            false
        }
        _ => !in_spec,
    })
}

impl FontChain {
    /// Load `primary` and each fallback by name, skipping any that is missing.
    ///
    /// A missing fallback is not an error: the chain degrades to what is
    /// installed, and the approximations catch what no loaded font carries.
    pub fn load(primary: &str, fallbacks: &[&str]) -> Result<FontChain, String> {
        let mut fonts = Vec::new();
        let path = find_font(primary).ok_or_else(|| format!("{primary}.tfm not found"))?;
        fonts.push(Loaded {
            tfm: Tfm::open(&path)?,
            name: primary.to_string(),
        });
        for f in fallbacks {
            if let Some(p) = find_font(f) {
                if let Ok(tfm) = Tfm::open(&p) {
                    fonts.push(Loaded {
                        tfm,
                        name: (*f).to_string(),
                    });
                }
            }
        }
        let mut map = Vec::new();
        for (ch, font_name, slot) in CM_SYMBOLS {
            if let Some(i) = fonts.iter().position(|f| f.name == *font_name) {
                // Only claim the slot if the font really defines it.
                if fonts[i].tfm.char(*slot).is_some() {
                    map.push((*ch, i, *slot));
                }
            }
        }
        Ok(FontChain { fonts, map })
    }

    /// Which font and slot carries `ch`, if any font in the chain does.
    pub fn resolve(&self, ch: char) -> Option<(usize, u8)> {
        // ASCII is the primary font's own, which is the common case and worth
        // answering before any search.
        if ch.is_ascii() && self.fonts[0].tfm.char(ch as u8).is_some() {
            return Some((0, ch as u8));
        }
        self.map
            .iter()
            .find(|(c, _, _)| *c == ch)
            .map(|(_, f, s)| (*f, *s))
    }

    /// The ASCII stand-in for a character no font in the chain carries.
    pub fn approximate(ch: char) -> Option<&'static str> {
        APPROXIMATIONS
            .iter()
            .find(|(c, _)| *c == ch)
            .map(|(_, s)| *s)
    }

    /// The width of `ch` in points at `size`, however it is being rendered.
    pub fn width(&self, ch: char, size: f64) -> f64 {
        // The colour markers are instructions to the driver, not glyphs: they
        // occupy no space on the page, and measuring them would push words onto
        // the next line for text that is not there.
        if matches!(ch, '\u{1}' | '\u{2}' | '\u{3}') || ch.is_control() {
            return 0.0;
        }
        // An interword space is GLUE, not a character (`tex.web` §1041): its
        // width is the font's `\fontdimen2`, and no glyph is set for it.
        //
        // Resolving it as a character finds one -- cmr10 defines code 32, and
        // in OT1 that slot holds the Polish suppressed-l, 0.27778em wide
        // against the space's 0.33333em. So every space in a DVI was drawn as
        // a wrong glyph at a wrong width, and every line measured 0.056em per
        // space too narrow.
        if ch == ' ' {
            return self.fonts[0].tfm.params.space * size;
        }
        if let Some((f, slot)) = self.resolve(ch) {
            return self.fonts[f].tfm.char(slot).map(|m| m.width).unwrap_or(0.0) * size;
        }
        if let Some(text) = Self::approximate(ch) {
            return self.fonts[0].tfm.width_of(text) * size;
        }
        0.0
    }

    /// The width of a whole string, resolving each character through the chain.
    ///
    /// The colour markers and their spec are not measured -- see
    /// `printing_chars`, which the PDF path measures through as well.
    ///
    /// This measures what will be DRAWN, not what the string holds: `carried`
    /// runs each font's ligature and kern program first, so `fi` is measured
    /// as the one character cmr10 sets it as and `AV` carries its kern. A
    /// measure that skipped that would break lines at widths the shipper then
    /// contradicts.
    pub fn width_of(&self, text: &str, size: f64) -> f64 {
        self.carried(printing_chars(text))
            .iter()
            .map(|item| match item {
                Carried::Space => self.fonts[0].tfm.params.space * size,
                Carried::Set { font, sets } => self.run_width(*font, sets) * size,
            })
            .sum()
    }

    /// The design-size width of one resolved run.
    fn run_width(&self, font: usize, sets: &[crate::tfm::Set]) -> f64 {
        sets.iter()
            .map(|set| match set {
                crate::tfm::Set::Char(c) => self.fonts[font]
                    .tfm
                    .char(*c)
                    .map(|m| m.width)
                    .unwrap_or(0.0),
                crate::tfm::Set::Kern(by) => *by,
            })
            .sum()
    }

    /// Resolve `chars` into the runs a page is actually made of.
    ///
    /// `tex.web` §1034-§1040's main loop builds a list of characters ONE FONT
    /// at a time and lets that font's ligature and kern program rewrite it
    /// before anything is drawn. §545 says how far a run reaches: "TeX puts
    /// implicit boundary characters before and after each consecutive string
    /// of characters from the same font", so a space -- which is glue, not a
    /// character (§1041) -- and a change of font each end one, and no ligature
    /// or kern is looked for across either.
    pub fn carried(&self, chars: impl Iterator<Item = char>) -> Vec<Carried> {
        let mut out = Vec::new();
        let mut run: Vec<u8> = Vec::new();
        let mut run_font = 0usize;
        for ch in chars {
            if ch == ' ' {
                self.push_run(&mut out, &mut run, run_font);
                out.push(Carried::Space);
                continue;
            }
            // Every marker this path understands has been taken out by the
            // caller; a control character that is left is not a glyph.
            if ch.is_control() {
                continue;
            }
            match self.resolve(ch) {
                Some((f, slot)) => {
                    if f != run_font {
                        self.push_run(&mut out, &mut run, run_font);
                        run_font = f;
                    }
                    run.push(slot);
                }
                // No font in the chain has it. The stand-in is set in the
                // primary font rather than dropped: a glyph that vanishes
                // takes the meaning of the line with it.
                None => {
                    let Some(text) = Self::approximate(ch) else {
                        continue;
                    };
                    if run_font != 0 {
                        self.push_run(&mut out, &mut run, run_font);
                        run_font = 0;
                    }
                    run.extend(text.bytes());
                }
            }
        }
        self.push_run(&mut out, &mut run, run_font);
        out
    }

    /// Close the run being gathered, putting it through its font's own
    /// ligature and kern program on the way out.
    fn push_run(&self, out: &mut Vec<Carried>, run: &mut Vec<u8>, font: usize) {
        if run.is_empty() {
            return;
        }
        let sets = self.fonts[font].tfm.set_run(run);
        run.clear();
        out.push(Carried::Set { font, sets });
    }
}

/// What a page actually carries for a stretch of text.
///
/// The characters a document holds are not the characters TeX draws: cmr10
/// sets `f` then `i` as the single character 0o14, and puts a negative
/// movement between `A` and `V`. `FontChain::carried` is where that
/// translation happens, and both the measure and the shipper go through it so
/// they cannot disagree.
#[derive(Debug, Clone, PartialEq)]
pub enum Carried {
    /// Characters of one font of the chain, and the kerns between them.
    Set {
        font: usize,
        sets: Vec<crate::tfm::Set>,
    },
    /// An interword space. Glue (§1041), so the file carries a movement
    /// rather than a glyph (§625).
    Space,
}

/// The name every PDF reader knows the fallback font by.
///
/// Symbol is one of the fourteen a reader is required to have, so naming it
/// costs nothing in the file and works everywhere -- which is the only reason a
/// per-glyph fallback can be had here at all: an arbitrary system font would
/// have to be embedded AND addressed, and a simple font's encoding has 256
/// slots to address it with.
pub const SYMBOL_FONT_NAME: &str = "Symbol";

/// The Symbol font's own encoding: the code that draws a character, and what
/// that code advances, in 1/1000 em.
///
/// This IS the per-glyph fallback. `luaotfload.add_fallback` is what the
/// publication scripts needed LuaTeX for -- a character the chosen face has no
/// glyph for is fetched from another font, in every context including verbatim
/// -- and this is the same thing with the other font fixed at the one every
/// reader already has. It carries the arrows, the Greek and the relations,
/// which is most of what a text face is missing.
///
/// Read off `psyr.afm`, Adobe's own metrics for the font, joined to Unicode
/// through `glyphlist.txt`: `C 174 ; WX 987 ; N arrowright` and
/// `arrowright;2192`. Both files ship with every TeX installation. A wrong code
/// here draws a wrong glyph in silence, so they are not guessed.
const SYMBOL_FONT: &[(char, u8, i64)] = &[
    ('\u{AC}', 216, 713),    // logicalnot
    ('\u{B0}', 176, 400),    // degree
    ('\u{B1}', 177, 549),    // plusminus
    ('\u{B5}', 109, 576),    // mu
    ('\u{D7}', 180, 549),    // multiply
    ('\u{F7}', 184, 549),    // divide
    ('\u{192}', 166, 500),   // florin
    ('\u{391}', 65, 722),    // Alpha
    ('\u{392}', 66, 667),    // Beta
    ('\u{393}', 71, 603),    // Gamma
    ('\u{395}', 69, 611),    // Epsilon
    ('\u{396}', 90, 611),    // Zeta
    ('\u{397}', 72, 722),    // Eta
    ('\u{398}', 81, 741),    // Theta
    ('\u{399}', 73, 333),    // Iota
    ('\u{39A}', 75, 722),    // Kappa
    ('\u{39B}', 76, 686),    // Lambda
    ('\u{39C}', 77, 889),    // Mu
    ('\u{39D}', 78, 722),    // Nu
    ('\u{39E}', 88, 645),    // Xi
    ('\u{39F}', 79, 722),    // Omicron
    ('\u{3A0}', 80, 768),    // Pi
    ('\u{3A1}', 82, 556),    // Rho
    ('\u{3A3}', 83, 592),    // Sigma
    ('\u{3A4}', 84, 611),    // Tau
    ('\u{3A5}', 85, 690),    // Upsilon
    ('\u{3A6}', 70, 763),    // Phi
    ('\u{3A7}', 67, 722),    // Chi
    ('\u{3A8}', 89, 795),    // Psi
    ('\u{3B1}', 97, 631),    // alpha
    ('\u{3B2}', 98, 549),    // beta
    ('\u{3B3}', 103, 411),   // gamma
    ('\u{3B4}', 100, 494),   // delta
    ('\u{3B5}', 101, 439),   // epsilon
    ('\u{3B6}', 122, 494),   // zeta
    ('\u{3B7}', 104, 603),   // eta
    ('\u{3B8}', 113, 521),   // theta
    ('\u{3B9}', 105, 329),   // iota
    ('\u{3BA}', 107, 549),   // kappa
    ('\u{3BB}', 108, 549),   // lambda
    ('\u{3BD}', 110, 521),   // nu
    ('\u{3BE}', 120, 493),   // xi
    ('\u{3BF}', 111, 549),   // omicron
    ('\u{3C0}', 112, 549),   // pi
    ('\u{3C1}', 114, 549),   // rho
    ('\u{3C2}', 86, 439),    // sigma1
    ('\u{3C3}', 115, 603),   // sigma
    ('\u{3C4}', 116, 439),   // tau
    ('\u{3C5}', 117, 576),   // upsilon
    ('\u{3C6}', 102, 521),   // phi
    ('\u{3C7}', 99, 549),    // chi
    ('\u{3C8}', 121, 686),   // psi
    ('\u{3C9}', 119, 686),   // omega
    ('\u{3D1}', 74, 631),    // theta1
    ('\u{3D2}', 161, 620),   // Upsilon1
    ('\u{3D5}', 106, 603),   // phi1
    ('\u{3D6}', 118, 713),   // omega1
    ('\u{2022}', 183, 460),  // bullet
    ('\u{2026}', 188, 1000), // ellipsis
    ('\u{2032}', 162, 247),  // minute
    ('\u{2033}', 178, 411),  // second
    ('\u{2044}', 164, 167),  // fraction
    ('\u{2111}', 193, 686),  // Ifraktur
    ('\u{2118}', 195, 987),  // weierstrass
    ('\u{211C}', 194, 795),  // Rfraktur
    ('\u{2126}', 87, 768),   // Omega
    ('\u{2135}', 192, 823),  // aleph
    ('\u{2190}', 172, 987),  // arrowleft
    ('\u{2191}', 173, 603),  // arrowup
    ('\u{2192}', 174, 987),  // arrowright
    ('\u{2193}', 175, 603),  // arrowdown
    ('\u{2194}', 171, 1042), // arrowboth
    ('\u{21B5}', 191, 658),  // carriagereturn
    ('\u{21D0}', 220, 987),  // arrowdblleft
    ('\u{21D1}', 221, 603),  // arrowdblup
    ('\u{21D2}', 222, 987),  // arrowdblright
    ('\u{21D3}', 223, 603),  // arrowdbldown
    ('\u{21D4}', 219, 1042), // arrowdblboth
    ('\u{2200}', 34, 713),   // universal
    ('\u{2202}', 182, 494),  // partialdiff
    ('\u{2203}', 36, 549),   // existential
    ('\u{2205}', 198, 823),  // emptyset
    ('\u{2206}', 68, 612),   // Delta
    ('\u{2207}', 209, 713),  // gradient
    ('\u{2208}', 206, 713),  // element
    ('\u{2209}', 207, 713),  // notelement
    ('\u{220B}', 39, 439),   // suchthat
    ('\u{220F}', 213, 823),  // product
    ('\u{2211}', 229, 713),  // summation
    ('\u{2212}', 45, 549),   // minus
    ('\u{2217}', 42, 500),   // asteriskmath
    ('\u{221A}', 214, 549),  // radical
    ('\u{221D}', 181, 713),  // proportional
    ('\u{221E}', 165, 713),  // infinity
    ('\u{2220}', 208, 768),  // angle
    ('\u{2227}', 217, 603),  // logicaland
    ('\u{2228}', 218, 603),  // logicalor
    ('\u{2229}', 199, 768),  // intersection
    ('\u{222A}', 200, 768),  // union
    ('\u{222B}', 242, 274),  // integral
    ('\u{2234}', 92, 863),   // therefore
    ('\u{223C}', 126, 549),  // similar
    ('\u{2245}', 64, 549),   // congruent
    ('\u{2248}', 187, 549),  // approxequal
    ('\u{2260}', 185, 549),  // notequal
    ('\u{2261}', 186, 549),  // equivalence
    ('\u{2264}', 163, 549),  // lessequal
    ('\u{2265}', 179, 549),  // greaterequal
    ('\u{2282}', 204, 713),  // propersubset
    ('\u{2283}', 201, 713),  // propersuperset
    ('\u{2284}', 203, 713),  // notsubset
    ('\u{2286}', 205, 713),  // reflexsubset
    ('\u{2287}', 202, 713),  // reflexsuperset
    ('\u{2295}', 197, 768),  // circleplus
    ('\u{2297}', 196, 768),  // circlemultiply
    ('\u{22A5}', 94, 658),   // perpendicular
    ('\u{22C5}', 215, 250),  // dotmath
    ('\u{2320}', 243, 686),  // integraltp
    ('\u{2321}', 245, 686),  // integralbt
    ('\u{2329}', 225, 329),  // angleleft
    ('\u{232A}', 241, 329),  // angleright
    ('\u{25CA}', 224, 494),  // lozenge
    ('\u{2660}', 170, 753),  // spade
    ('\u{2663}', 167, 753),  // club
    ('\u{2665}', 169, 753),  // heart
    ('\u{2666}', 168, 753),  // diamond
    // Three glyphs the Adobe list names for their MATHEMATICAL character when
    // the Greek letter is the same shape and the same slot: `mu` is U+00B5 the
    // micro sign, `Delta` U+2206 the increment, `Omega` U+2126 the ohm. A
    // document writing Greek writes U+03BC, U+0394 and U+03A9, and with only
    // the names above those three fell past every table and were dropped --
    // measured on zshrs/docs/book.tex, whose two U+03BC reached no page.
    ('\u{3BC}', 109, 576), // mu, as the letter
    ('\u{394}', 68, 612),  // Delta, as the letter
    ('\u{3A9}', 87, 768),  // Omega, as the letter
];

/// WinAnsi's own codes, for the characters that are not where Unicode puts
/// them.
///
/// A PDF font here is written with `/Encoding /WinAnsiEncoding`, so a code IS a
/// WinAnsi code. From U+00A0 up WinAnsi is Latin-1 and the code is the
/// codepoint; between 0x80 and 0x9F it is Windows' own set of punctuation, and
/// that is where the em dash, the ellipsis and the curly quotes live -- 2,063
/// em dashes and 3,486 ellipses in the corpus, every one of them written into
/// the file as its UTF-8 bytes and drawn as two or three wrong letters.
const WINANSI_PUNCTUATION: &[(char, u8)] = &[
    ('€', 0x80),
    ('‚', 0x82),
    ('ƒ', 0x83),
    ('„', 0x84),
    ('…', 0x85),
    ('†', 0x86),
    ('‡', 0x87),
    ('ˆ', 0x88),
    ('‰', 0x89),
    ('Š', 0x8A),
    ('‹', 0x8B),
    ('Œ', 0x8C),
    ('Ž', 0x8E),
    ('‘', 0x91),
    ('’', 0x92),
    ('“', 0x93),
    ('”', 0x94),
    ('•', 0x95),
    ('–', 0x96),
    ('—', 0x97),
    ('˜', 0x98),
    ('™', 0x99),
    ('š', 0x9A),
    ('›', 0x9B),
    ('œ', 0x9C),
    ('ž', 0x9E),
    ('Ÿ', 0x9F),
];

/// The WinAnsi code for a character, if WinAnsi has one.
pub fn winansi_code(ch: char) -> Option<u8> {
    match ch {
        ' '..='~' => Some(ch as u8),
        '\u{A0}'..='\u{FF}' => Some(ch as u8),
        _ => WINANSI_PUNCTUATION
            .iter()
            .find(|(c, _)| *c == ch)
            .map(|(_, code)| *code),
    }
}

/// What a WinAnsi code means, which is what a font's `cmap` has to be asked
/// for.
///
/// A reader resolves a code in a non-symbolic TrueType font by its WinAnsi
/// GLYPH NAME and that name's Unicode value, so the width written beside the
/// code has to be looked up the same way. Reading code 0x97 as U+0097 finds
/// nothing and writes `.notdef`'s width where the em dash's belongs.
pub fn winansi_unicode(code: u8) -> Option<char> {
    match code {
        0x20..=0x7E => Some(code as char),
        0xA0..=0xFF => Some(code as char),
        _ => WINANSI_PUNCTUATION
            .iter()
            .find(|(_, c)| *c == code)
            .map(|(ch, _)| *ch),
    }
}

/// Where one character comes from when it is drawn.
///
/// The order the answer is looked for in is the chain: the face the document
/// asked for, then the fallback font, then a stand-in built out of characters
/// every face has. What it never does is drop the character -- a glyph that
/// vanishes takes the meaning of the line with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Glyph {
    /// A code in the face in force, which HAS the character.
    Own(u8),
    /// A code in the Symbol font, which has it where the face does not.
    Fallback(u8),
    /// A glyph in one of the faces the DOCUMENT named as its fallback chain:
    /// which face of the chain, and which glyph of that face.
    Outside(usize, u16),
    /// ASCII the face draws itself, standing in for a shape nothing has.
    StandIn(&'static str),
}

/// How `ch` reaches the page, given what the face in force can draw.
///
/// `covers` answers from the font file's own `cmap`, which `embed_file` already
/// reads: whether a face HAS a codepoint is a question the file answers, and
/// asking it is the difference between a glyph and a blank.
///
/// `outside` is the document's own fallback chain, asked after the Symbol font
/// and before the stand-ins. After Symbol, because Symbol is one of the
/// fourteen and costs the file nothing where it has the character; before the
/// stand-ins, because `─` set as `-` is a picture redrawn and `─` set from a
/// face that has it is the picture.
pub fn glyph_for(
    ch: char,
    covers: &dyn Fn(char) -> bool,
    outside: &dyn Fn(char) -> Option<(usize, u16)>,
) -> Option<Glyph> {
    if let Some(code) = winansi_code(ch) {
        if ch.is_ascii() || covers(ch) {
            return Some(Glyph::Own(code));
        }
    }
    if let Some((_, code, _)) = SYMBOL_FONT.iter().find(|(c, _, _)| *c == ch) {
        return Some(Glyph::Fallback(*code));
    }
    if let Some((face, glyph)) = outside(ch) {
        return Some(Glyph::Outside(face, glyph));
    }
    FontChain::approximate(ch).map(Glyph::StandIn)
}

/// The faces a document named to fetch a missing glyph from, loaded.
///
/// This is `luaotfload.add_fallback` itself rather than the fixed stand-in for
/// it: the chain is the document's, the answer per character comes out of each
/// face's own `cmap`, and what is drawn is that face's glyph. A face that
/// cannot be found, or that carries PostScript outlines a `/FontFile2` may not
/// hold, is skipped -- the chain then degrades to the next one, and to the
/// stand-ins after that, exactly as it did before there was a chain at all.
#[derive(Default)]
pub struct Fallbacks {
    faces: Vec<Outside>,
    /// Each face as the file will carry it, built once `reserve` has been told
    /// every character the document draws. Building it per piece instead would
    /// subset a 23 MB face thirty thousand times over.
    carried: Vec<Option<crate::pdf::Font>>,
}

/// One face of the chain: enough of its file to answer for a character, and the
/// glyphs the document has asked it for so far.
struct Outside {
    name: String,
    bytes: Vec<u8>,
    cmap: std::collections::BTreeMap<u32, u16>,
    advances: Vec<u16>,
    upem: f64,
    bbox: [i64; 4],
    ascent: i64,
    descent: i64,
    /// Glyph id to the character it was fetched for. A `BTreeMap` so the `/W`
    /// array and the `/ToUnicode` map come out in the same order every run,
    /// which is what makes two runs of the same book the same file.
    used: std::collections::BTreeMap<u16, char>,
}

impl Fallbacks {
    /// Load each family of the chain, skipping any that cannot be embedded.
    pub fn load(families: &[String]) -> Fallbacks {
        let mut faces = Vec::new();
        for family in families {
            let Some(path) = find_fallback_family(family) else {
                continue;
            };
            let Some(face) = Outside::open(&path) else {
                continue;
            };
            faces.push(face);
        }
        Fallbacks {
            faces,
            carried: Vec::new(),
        }
    }

    /// Which face of the chain has `ch`, and which of its glyphs it is.
    ///
    /// Glyph 0 is `.notdef` -- the empty box -- so a `cmap` that answers with
    /// it has not answered, and the search goes on to the next face.
    pub fn glyph(&self, ch: char) -> Option<(usize, u16)> {
        self.faces.iter().enumerate().find_map(|(at, face)| {
            match face.cmap.get(&(ch as u32)).copied() {
                Some(gid) if gid != 0 => Some((at, gid)),
                _ => None,
            }
        })
    }

    /// Note that the document draws `ch`, so the file carries a width and a
    /// meaning for the glyph it comes out as.
    pub fn reserve(&mut self, ch: char) {
        if let Some((at, gid)) = self.glyph(ch) {
            self.faces[at].used.insert(gid, ch);
        }
    }

    /// What one glyph of one face advances, in points at `size`.
    pub fn width(&self, face: usize, glyph: u16, size: f64) -> f64 {
        self.faces
            .get(face)
            .map(|f| f.width(glyph) as f64 / 1000.0 * size)
            .unwrap_or(0.0)
    }

    /// Build each face into the font the file will carry, now that every
    /// character the document draws has been reserved.
    pub fn settle(&mut self) {
        self.carried = self.faces.iter().map(Outside::font).collect();
    }

    /// One face of the chain as a font the file can carry.
    pub fn font(&self, face: usize) -> Option<&crate::pdf::Font> {
        self.carried.get(face)?.as_ref()
    }

    /// Whether the chain resolved to anything at all.
    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }
}

impl Outside {
    fn open(path: &std::path::Path) -> Option<Outside> {
        let bytes = std::fs::read(path).ok()?;
        let sfnt = crate::sfnt::Sfnt::parse(bytes.clone()).ok()?;
        // A CFF-flavoured OpenType carries PostScript outlines, which a
        // `/FontFile2` must not: that entry means a TrueType font program.
        if sfnt.is_cff() {
            return None;
        }
        let head = sfnt.head().ok()?;
        let hhea = sfnt.hhea().ok()?;
        let upem = f64::from(head.units_per_em.max(1));
        let scale = |v: f64| (v * 1000.0 / upem).round() as i64;
        Some(Outside {
            name: path.file_stem()?.to_string_lossy().replace(' ', ""),
            cmap: sfnt.cmap().ok()?,
            advances: sfnt.advance_widths().ok()?,
            upem,
            bbox: [
                scale(f64::from(head.x_min)),
                scale(f64::from(head.y_min)),
                scale(f64::from(head.x_max)),
                scale(f64::from(head.y_max)),
            ],
            ascent: scale(f64::from(hhea.ascender)),
            descent: scale(f64::from(hhea.descender)),
            bytes,
            used: std::collections::BTreeMap::new(),
        })
    }

    /// A glyph's advance in 1/1000 em, which is what a PDF width is.
    ///
    /// `hmtx` runs short of `maxp` by design: every glyph past the last entry
    /// advances what that entry does, which is how a monospace face stores one
    /// width for thousands of glyphs.
    fn width(&self, glyph: u16) -> i64 {
        let adv = self
            .advances
            .get(glyph as usize)
            .or_else(|| self.advances.last())
            .copied()
            .unwrap_or(0);
        (f64::from(adv) * 1000.0 / self.upem).round() as i64
    }

    /// The face as the file will carry it: the glyphs this document borrowed,
    /// and nothing else of a face that may hold fifty thousand.
    ///
    /// `None` when nothing was borrowed from it, and when the subset cannot be
    /// built -- a face whose `glyf` or `loca` cannot be read is one nothing
    /// here can embed, and the chain has already fallen through to it.
    fn font(&self) -> Option<crate::pdf::Font> {
        if self.used.is_empty() {
            return None;
        }
        let sfnt = crate::sfnt::Sfnt::parse(self.bytes.clone()).ok()?;
        let bytes = sfnt.subset(&self.used.keys().copied().collect()).ok()?;
        Some(crate::pdf::Font::Glyphs {
            name: self.name.clone(),
            bytes,
            glyphs: self
                .used
                .iter()
                .map(|(gid, ch)| (*gid, *ch, self.width(*gid)))
                .collect(),
            bbox: self.bbox,
            ascent: self.ascent,
            descent: self.descent,
        })
    }
}

/// The file for a family named in a fallback chain.
///
/// Looser than `find_family` in one way, and deliberately: a chain names
/// families as a person writes them -- `Arial Unicode MS` -- and the file is
/// `Arial Unicode.ttf`, so a rule that only accepts a stem STARTING with the
/// name asked for rejects the very font the chain was written for. Here either
/// may be the longer, which resolves that pair and still refuses `Arial` for
/// `Helvetica`. The strict rule is left alone where `\setmainfont` uses it: a
/// body face resolved loosely would set a whole book in the wrong one, while a
/// fallback resolved loosely draws one glyph from a near neighbour.
pub fn find_fallback_family(family: &str) -> Option<std::path::PathBuf> {
    let wanted = normalise(family);
    // Too short a name matches half the system: `MS` would take `MSGothic`.
    if wanted.len() < 4 {
        return None;
    }
    let close = |path: &std::path::Path| {
        let stem = path
            .file_stem()
            .map(|s| normalise(&s.to_string_lossy()))
            .unwrap_or_default();
        stem.len() >= 4 && (stem.starts_with(&wanted) || wanted.starts_with(&stem))
    };
    if let Ok(out) = std::process::Command::new("fc-match")
        .args(["-f", "%{file}", family])
        .output()
    {
        if out.status.success() {
            let answer = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let path = std::path::Path::new(&answer);
            // fc-match ALWAYS answers, with a default when it has no match, so
            // the answer is only useful if it looks like the family asked for.
            if path.exists() && close(path) {
                return Some(path.to_path_buf());
            }
        }
    }
    for dir in [
        "/System/Library/Fonts",
        "/System/Library/Fonts/Supplemental",
        "/Library/Fonts",
        "/usr/share/fonts",
        "/usr/local/share/fonts",
    ] {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext != "ttf" && ext != "otf" {
                continue;
            }
            if close(&path) {
                return Some(path);
            }
        }
    }
    None
}

/// What a code in the fallback font MEANS, as against which glyph it draws.
///
/// A PDF says only the second, so a page that draws an arrow from the fallback
/// is a page nobody can search for one: `pdftotext` on the first run of these
/// came back with the arrow simply missing from the text. The driver writes
/// this out as the font's `/ToUnicode`.
pub fn symbol_unicode(code: u8) -> Option<char> {
    SYMBOL_FONT
        .iter()
        .find(|(_, c, _)| *c == code)
        .map(|(ch, _, _)| *ch)
}

/// What a Symbol code advances, in 1/1000 em.
fn symbol_width(code: u8) -> i64 {
    SYMBOL_FONT
        .iter()
        .find(|(_, c, _)| *c == code)
        .map(|(_, _, w)| *w)
        .unwrap_or(500)
}

/// The typefaces a document asked for.
///
/// `\setmainfont{Arimo}` is a statement about the book, and setting it in
/// Computer Modern anyway is the thing this exists to stop. What texrs can
/// honour depends on the output: a DVI names `.tfm` fonts and cannot carry an
/// OpenType one at all, while a PDF can name the fourteen fonts every reader
/// has. Arimo is metric-compatible with Arial, which is metric-compatible with
/// Helvetica, so that substitution is a real one rather than a shrug.
/// Where a document keeps the font it supplied with itself.
///
/// fontspec's `\setmainfont{Arimo}[Path=..., Extension=.ttf,
/// UprightFont=Arimo-VF]` does not name an INSTALLED family: it names a file
/// that ships beside the document. Looking that name up among the installed
/// families finds nothing, and `fc-match` answers anyway, with its default --
/// so the book was set in whatever that default happened to be, which is the
/// "everything comes out in the wrong face" complaint exactly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontFile {
    /// `Path=` -- a directory, written with a trailing separator.
    pub path: Option<String>,
    /// `UprightFont=` -- the file's name, without its extension.
    pub upright: Option<String>,
    /// `BoldFont=` -- the file `\textbf` is set from, when there is one.
    pub bold: Option<String>,
    /// `ItalicFont=` -- the file `\emph` and `\textit` are set from.
    pub italic: Option<String>,
    /// `Extension=` -- `.ttf`, including the dot.
    pub extension: Option<String>,
}

impl FontFile {
    /// Read the options fontspec was given: `Key=Value` separated by commas,
    /// with `{...}` groups around values that contain commas of their own.
    pub fn parse(options: &str) -> FontFile {
        let mut out = FontFile::default();
        for (key, value) in top_level_pairs(options) {
            let value = value.trim().trim_matches(|c| c == '{' || c == '}');
            match key.trim() {
                "Path" => out.path = Some(value.to_string()),
                "UprightFont" => out.upright = Some(value.to_string()),
                // The faces `\textbf` and `\emph` ask for. A book names them in
                // the same option list as the upright file and every one of the
                // corpus books ships the files, so the only thing that ever
                // stopped `\emph` from being italic was that these two keys
                // were read past.
                "BoldFont" => out.bold = Some(value.to_string()),
                "ItalicFont" => out.italic = Some(value.to_string()),
                "Extension" => out.extension = Some(value.to_string()),
                _ => {}
            }
        }
        out
    }

    /// Let the mandatory `{...}` argument name the FILE, when it names one.
    ///
    /// fontspec has two spellings for the same thing, and lualatex honours
    /// both: `\setmainfont{Arimo}[Path=..., Extension=.ttf,
    /// UprightFont=Arimo-VF]` names the family and describes the file, while
    /// `\setmainfont{Arimo-VF.ttf}[Path=...]` names the file outright. The
    /// second spelling put the filename nowhere near the file resolution --
    /// `upright` stayed empty, no path was ever built, and the document was
    /// set in one of the fourteen without a word about it.
    ///
    /// The option list wins wherever it said anything: it is the more specific
    /// statement, and a document that writes both meant the keys.
    pub fn absorb_filename(&mut self, name: &str) {
        let Some((stem, extension)) = split_font_filename(name.trim()) else {
            return;
        };
        if self.upright.is_none() {
            self.upright = Some(stem.to_string());
        }
        if self.extension.is_none() {
            self.extension = Some(extension.to_string());
        }
    }

    /// The file this names, looked for where it might actually be.
    ///
    /// `Path=` is written by whatever produced the document and is regularly an
    /// absolute path into a directory that no longer exists -- a build
    /// scratchpad, another machine. The fonts themselves ship beside the
    /// document, so a path that has gone stale is retried by its last
    /// component against the document's own directory.
    pub fn resolve(&self, near: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
        self.resolve_face(Face::Main, near)
    }

    /// The file for one face, looked for in the same two places.
    ///
    /// `BoldFont=` and `ItalicFont=` name a file the way `UprightFont=` does
    /// and live in the same directory, so they are resolved by the same rules
    /// rather than by a second copy of them. A face the document named no file
    /// for resolves to nothing, and the caller falls back to the main face.
    pub fn resolve_face(
        &self,
        face: Face,
        near: Option<&std::path::Path>,
    ) -> Option<std::path::PathBuf> {
        let named = match face {
            Face::Bold => self.bold.as_deref(),
            Face::Italic => self.italic.as_deref(),
            _ => self.upright.as_deref(),
        }?;
        let extension = self.extension.as_deref().unwrap_or(".ttf");
        let file = format!("{named}{extension}");
        if let Some(path) = &self.path {
            let full = std::path::Path::new(path).join(&file);
            if full.is_file() {
                return Some(full);
            }
            // The stale-path case: keep the directory the fonts are IN and look
            // for it beside the document instead.
            if let (Some(dir), Some(near)) = (last_component(path), near) {
                let beside = near.join(dir).join(&file);
                if beside.is_file() {
                    return Some(beside);
                }
            }
        }
        let beside = near?.join(&file);
        beside.is_file().then_some(beside)
    }
}

/// The extensions that make a fontspec argument a FILENAME rather than a
/// family name, without their dots and in lower case.
///
/// `.ttc` and `.otc` are collections; naming one resolves to a real file and
/// falls back on its own if the collection cannot be read as a single font.
const FONT_FILE_EXTENSIONS: [&str; 4] = ["ttf", "otf", "ttc", "otc"];

/// `Arimo-VF.ttf` as its stem and its extension, or `None` for a family name.
///
/// The test is the extension, which is how fontspec itself tells the two
/// spellings apart. It is case-insensitive because the file on disk is
/// regularly `.TTF` and a document names it as it is written there; the
/// extension is returned as WRITTEN, since that is the name to be opened on a
/// case-sensitive filesystem.
pub fn split_font_filename(name: &str) -> Option<(&str, &str)> {
    let dot = name.rfind('.')?;
    let (stem, extension) = name.split_at(dot);
    if stem.is_empty() {
        return None;
    }
    FONT_FILE_EXTENSIONS
        .iter()
        .any(|known| extension[1..].eq_ignore_ascii_case(known))
        .then_some((stem, extension))
}

/// The family a mandatory fontspec argument stands for.
///
/// A filename stands for the family its stem names: the family lookup strips
/// every non-alphanumeric character before comparing, so `Arimo-VF.ttf` asked
/// it for `arimovfttf` and matched nothing installed either. A name that is
/// not a filename is its own family and is returned untouched.
pub fn font_family_name(name: &str) -> &str {
    split_font_filename(name).map_or(name, |(stem, _)| stem)
}

/// The last directory of a path written with a trailing separator.
fn last_component(path: &str) -> Option<&str> {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|c| !c.is_empty())
}

/// `Key=Value` pairs, splitting only on the commas that are not inside braces.
///
/// fontspec values nest: `UprightFeatures={RawFeature={axis={wght=400}}}` holds
/// commas in some documents, and splitting on every comma tears it apart and
/// makes the keys after it unreadable.
fn top_level_pairs(options: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let bytes = options.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                if let Some(pair) = split_pair(&options[start..i]) {
                    out.push(pair);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    if let Some(pair) = split_pair(&options[start..]) {
        out.push(pair);
    }
    out
}

fn split_pair(text: &str) -> Option<(&str, &str)> {
    let at = text.find('=')?;
    Some((&text[..at], &text[at + 1..]))
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Families {
    pub main: Option<String>,
    /// The file `\setmainfont`s options named, when the document ships its
    /// own font rather than naming an installed one.
    pub main_file: FontFile,
    pub sans: Option<String>,
    pub mono: Option<String>,
    /// The same for `\setmonofont`: every corpus book ships its monospace face
    /// beside itself, so the family name alone resolves to nothing and
    /// `\texttt` would be set in the body font after all.
    pub mono_file: FontFile,
    /// The families a character none of the above can draw is fetched from,
    /// in the order the document listed them.
    ///
    /// A book SHIPS its faces -- `\setmainfont{Arimo}` with `Path=` and
    /// `UprightFont=`, embedded whole -- and a character Arimo has no glyph for
    /// has nowhere to go: the Symbol font covers the arrows and the Greek and
    /// nothing else, so U+2500 fell to an ASCII stand-in and the box drawing
    /// came out as hyphens. This is the chain the document itself names, and
    /// naming it is the reason its build required LuaTeX.
    pub fallbacks: Vec<String>,
}

/// The families named by `luaotfload.add_fallback`, in the order given.
///
/// Every book in the corpus opens with one statement of where a missing glyph
/// comes from:
///
/// ```text
/// \directlua{luaotfload.add_fallback("symfb", {"Arial Unicode MS:mode=base;",
///   "Arial:mode=base;", "STIX Two Math:mode=base;", "Noto Emoji:mode=base;"})}
/// ```
///
/// This READS the chunk rather than running it -- `crate::lua` runs it, and the
/// two are asked for different things. What is wanted here is the list of family
/// names, and a name is everything before the `:` that
/// introduces luaotfload's own options. Anything else inside `\directlua`
/// yields nothing, which leaves the document with no chain and the stand-ins it
/// had before.
pub fn fallback_chain(chunk: &str) -> Vec<String> {
    let Some(rest) = chunk.split_once("add_fallback").map(|(_, r)| r) else {
        return Vec::new();
    };
    // The braced table is the second argument; the first is the chain's own
    // name, which nothing here refers to.
    let Some(table) = rest.split_once('{').and_then(|(_, r)| r.split_once('}')) else {
        return Vec::new();
    };
    table
        .0
        .split(',')
        .filter_map(|item| {
            let quoted = item.trim().trim_matches('"');
            let name = quoted.split(':').next().unwrap_or("").trim();
            match name.is_empty() {
                true => None,
                false => Some(name.to_string()),
            }
        })
        .collect()
}

/// The face a stretch of text is set in.
///
/// `\texttt` appears 683,577 times in the corpus, `\emph` 35,369 and `\textbf`
/// 34,159, and all three were set in the body face: nothing between the mouth
/// and the page carried WHICH face was asked for, so a PDF came out with one
/// font resource and one `Tf` operator, and every code identifier in every book
/// was set in the prose font.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Face {
    /// What `\setmainfont` named: the document's prose.
    #[default]
    Main,
    /// `\ttfamily`, which is what `\texttt` is.
    Mono,
    /// `\bfseries`, which is what `\textbf` is.
    Bold,
    /// `\itshape`, which is what `\emph` and `\textit` are.
    Italic,
}

impl Face {
    /// Every face a page can be set in, which is what a question asked of all
    /// of them iterates: whether a character needs fetching from outside is one
    /// such question, and it is asked once for the document rather than once a
    /// line.
    pub const ALL: [Face; 4] = [Face::Main, Face::Mono, Face::Bold, Face::Italic];

    /// The one character that names this face inside a marker.
    pub fn code(self) -> char {
        match self {
            Face::Main => 'r',
            Face::Mono => 'm',
            Face::Bold => 'b',
            Face::Italic => 'i',
        }
    }

    /// The face a marker names. Anything else is the main face: a marker that
    /// arrives damaged must not take the rest of the document with it.
    pub fn from_code(code: char) -> Face {
        match code {
            'm' => Face::Mono,
            'b' => Face::Bold,
            'i' => Face::Italic,
            _ => Face::Main,
        }
    }

    /// Where this face's font sits in the four the page is set from.
    fn index(self) -> usize {
        match self {
            Face::Main => 0,
            Face::Mono => 1,
            Face::Bold => 2,
            Face::Italic => 3,
        }
    }
}

/// A face marker opens: U+000E and one [`Face::code`] character.
///
/// Colour travels the text as a marker because it wraps a run of characters
/// rather than being one; a face is the same shape of thing and travels the
/// same way. Two characters rather than colour's variable-length spec, because
/// there are four faces and no spec to carry.
pub const FACE_PUSH: char = '\u{11}';

/// A face marker closes. The stack under it is what `\ttfamily` inside a
/// `\textbf` needs: the outer face comes back when the inner one ends.
pub const FACE_POP: char = '\u{12}';

/// The size a run is set at, and the leading its line is set on.
///
/// The two travel together because `\@setfontsize` receives both and because
/// setting one without the other is what makes a heading's own lines collide.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypeSize {
    /// The type size in PDF points, which is what a `Tf` operator states.
    pub size: f64,
    /// `\baselineskip` in PDF points, which is what the line under this one
    /// drops by.
    pub leading: f64,
}

impl Default for TypeSize {
    /// plain.tex's, matching [`Layout::default`]: a size marker that arrives
    /// damaged must leave the document readable rather than set at nothing.
    fn default() -> Self {
        TypeSize {
            size: 10.0,
            leading: 12.0 * BP_PER_PT,
        }
    }
}

/// The size in force, which is the top of the stack the caller carries.
pub fn current_size(sizes: &[TypeSize]) -> TypeSize {
    sizes.last().copied().unwrap_or_default()
}

/// What each of LaTeX's ten size commands sets, in TeX points.
///
/// Read off `size10.clo`, `size11.clo` and `size12.clo` -- the class option
/// files themselves -- joined to the point values `latex.ltx` gives their
/// roman-numeral names (`\@xivpt` is 14.4, `\@xxvpt` is 24.88). The three
/// tables are NOT one table scaled: `\footnotesize` is 8pt in a 10pt document
/// and 10pt in a 12pt one, and `\Large` is 14.4pt in both a 10pt and an 11pt
/// document. Scaling one of them by the base size would be a guess that looks
/// right for `\large` and is wrong for half the rest.
///
/// Each entry is `(name, size, baselineskip)`.
const SIZE_STEPS_10: &[(&str, f64, f64)] = &[
    ("tiny", 5.0, 6.0),
    ("scriptsize", 7.0, 8.0),
    ("footnotesize", 8.0, 9.5),
    ("small", 9.0, 11.0),
    ("normalsize", 10.0, 12.0),
    ("large", 12.0, 14.0),
    ("Large", 14.4, 18.0),
    ("LARGE", 17.28, 22.0),
    ("huge", 20.74, 25.0),
    ("Huge", 24.88, 30.0),
];

/// The 11pt class's table. See [`SIZE_STEPS_10`].
const SIZE_STEPS_11: &[(&str, f64, f64)] = &[
    ("tiny", 6.0, 7.0),
    ("scriptsize", 8.0, 9.5),
    ("footnotesize", 9.0, 11.0),
    ("small", 10.0, 12.0),
    ("normalsize", 10.95, 13.6),
    ("large", 12.0, 14.0),
    ("Large", 14.4, 18.0),
    ("LARGE", 17.28, 22.0),
    ("huge", 20.74, 25.0),
    ("Huge", 24.88, 30.0),
];

/// The 12pt class's table, where the top two steps meet: `\huge` and `\Huge`
/// are both 24.88pt, as `size12.clo` sets them. See [`SIZE_STEPS_10`].
const SIZE_STEPS_12: &[(&str, f64, f64)] = &[
    ("tiny", 6.0, 7.0),
    ("scriptsize", 8.0, 9.5),
    ("footnotesize", 10.0, 12.0),
    ("small", 10.95, 13.6),
    ("normalsize", 12.0, 14.5),
    ("large", 14.4, 18.0),
    ("Large", 17.28, 22.0),
    ("LARGE", 20.74, 25.0),
    ("huge", 24.88, 30.0),
    ("Huge", 24.88, 30.0),
];

/// What a size command sets, in PDF points, for a document whose body size is
/// `base` PDF points.
///
/// `None` for a command that is not one of the ten, which is how the lowerer
/// tells a size declaration from every other control sequence.
pub fn size_step(name: &str, base: f64) -> Option<TypeSize> {
    // The class the document was loaded with, back from the body size it left
    // behind. A base that is none of the three takes the 10pt table, which is
    // what `\documentclass` itself falls back to.
    let table = match (base / BP_PER_PT).round() as i64 {
        11 => SIZE_STEPS_11,
        12 => SIZE_STEPS_12,
        _ => SIZE_STEPS_10,
    };
    table
        .iter()
        .find(|(step, _, _)| *step == name)
        .map(|(_, size, leading)| TypeSize {
            size: size * BP_PER_PT,
            leading: leading * BP_PER_PT,
        })
}

/// A size marker opens, and closes its own spec: U+0019, the size and the
/// baselineskip it was set with, and U+0019 again.
///
/// A face needs one character because there are four faces; a size cannot,
/// because `\fontsize{14.4}{18}` names a number rather than one of a fixed
/// set. So this carries a spec the way colour does and delimits it the way
/// `PICTURE` does -- the same marker at both ends -- rather than spending two
/// more control characters on it.
///
/// The pair is `size;baselineskip`, both in PDF points, because the two travel
/// together: `\@setfontsize` receives both, and a heading that is set larger
/// and led the same is a heading whose lines collide.
pub const SIZE_PUSH: char = '\u{19}';

/// A size marker closes, restoring the size under it. `\large` inside a
/// `\Large` heading gets the `\Large` back, which is the same stack the face
/// and the colour markers keep and for the same reason.
pub const SIZE_POP: char = '\u{1a}';

/// The size and baselineskip a size marker carries, in PDF points.
///
/// `None` for a marker that arrives damaged: a document must not lose its
/// remaining pages to one unreadable spec, so the reader falls back to the
/// size already in force rather than to zero.
pub fn size_spec(spec: &str) -> Option<(f64, f64)> {
    let (size, leading) = spec.split_once(';')?;
    Some((size.trim().parse().ok()?, leading.trim().parse().ok()?))
}

/// A size marker wrapping `body`, stated at `size` with `leading`.
pub fn size_span(size: f64, leading: f64, body: &str) -> String {
    format!("{SIZE_PUSH}{size};{leading}{SIZE_PUSH}{body}{SIZE_POP}")
}

/// What a requested family maps to among the fourteen fonts a PDF reader has.
///
/// The mapping is by what the face IS, not by its name: Arimo is Arial's
/// metrics, Liberation Sans is Arial's metrics, and Arial is Helvetica's, so
/// all three set at the same widths as Helvetica. A monospace request goes to
/// Courier whatever it was called. A name nothing is known about falls to
/// Helvetica rather than to Computer Modern, because a document that asked for
/// a font at all was asking not to be set in a book face.
pub fn base14_for(family: &str) -> &'static str {
    let f = family.to_ascii_lowercase();
    let mono = [
        "mono",
        "courier",
        "consolas",
        "menlo",
        "inconsolata",
        "sharetechmono",
        "jetbrains",
        "fira code",
        "source code",
    ];
    if mono.iter().any(|m| f.contains(m)) {
        return "Courier";
    }
    let serif = [
        "times",
        "serif",
        "georgia",
        "garamond",
        "minion",
        "palatino",
        "book",
        "charter",
        "libertine",
        "stix",
    ];
    if serif.iter().any(|m| f.contains(m)) {
        return "Times-Roman";
    }
    "Helvetica"
}

/// The member of the fourteen that carries one face of `base`.
///
/// The fourteen are four faces of three families plus two symbol fonts, so a
/// document that named a family but shipped no file for its bold or italic
/// still gets a face that is bold or italic -- which is the whole request. The
/// names are not built the same way for all three: Helvetica and Courier take
/// `-Bold` and `-Oblique`, and Times-Roman's siblings drop the `Roman` for
/// `Times-Bold` and `Times-Italic`. Getting that wrong names a font no reader
/// has and the substitution is silent.
pub fn base14_face(base: &str, face: Face) -> String {
    match face {
        Face::Main => base.to_string(),
        Face::Mono => "Courier".to_string(),
        Face::Bold if base == "Times-Roman" => "Times-Bold".to_string(),
        Face::Italic if base == "Times-Roman" => "Times-Italic".to_string(),
        Face::Bold => format!("{base}-Bold"),
        Face::Italic => format!("{base}-Oblique"),
    }
}

/// Set a document straight to PDF, honouring the font it asked for.
///
/// The DVI path names `.tfm` fonts and so can only ever set in Computer Modern;
/// a PDF can name the fourteen faces every reader has, which is how
/// `\setmainfont{Arimo}` becomes Helvetica's metrics rather than a book face.
/// Colour travels the same markers the DVI path reads and comes out as PDF's
/// own `rg` operator.
/// `page` is what `\pagecolor` asked for, painted under everything else.
/// `near` is the directory the document was read from, which is where a font
/// it ships with itself is looked for.
pub fn to_pdf(
    text: &str,
    families: &Families,
    layout: &Layout,
    page_colour: Option<crate::colour::Rgb>,
    near: Option<&std::path::Path>,
) -> Vec<u8> {
    use crate::pdf::{document, Font, Page, Set};

    // The font the document asked for, EMBEDDED if it can be found: that is
    // the difference between a page set in Arimo's metrics and one set in
    // Arimo. Failing that, one of the fourteen, which gets the widths right
    // and the shapes wrong -- better than refusing to typeset.
    let requested = families.main.as_deref();
    // A font the document SHIPS is tried first: it is the one the document
    // actually meant, and its family name is regularly not installed at all.
    let embedded = families
        .main_file
        .resolve(near)
        .and_then(|file| embed_file(&file))
        .or_else(|| requested.and_then(embed_family));
    let main = embedded.clone().unwrap_or_else(|| match requested {
        // A document that asked for a family and did not get it falls to the
        // member of the fourteen with that family's metrics.
        Some(family) => Font::Base14(base14_for(family).to_string()),
        // A document that asked for NO family is a TeX document, and TeX's text
        // font is cmr10 -- which is the face luatex embeds. Naming Helvetica
        // here set every plain document in a typeface no TeX engine uses.
        None => crate::pdf::Font::computer_modern()
            .unwrap_or_else(|| Font::Base14("Helvetica".to_string())),
    });

    // The other three faces, in the order `Face::index` puts them: main, mono,
    // bold, italic. Each is the file the document named for it, then the family
    // it named, then the member of the fourteen that carries that face -- and
    // the main font when the document asked for none of those, so a `\texttt`
    // in a document with no monospace family still sets rather than vanishing.
    let base = match &main {
        Font::Base14(name) => name.clone(),
        _ => requested.map(base14_for).unwrap_or("Helvetica").to_string(),
    };
    let mono = families
        .mono_file
        .resolve(near)
        .and_then(|file| embed_file(&file))
        .or_else(|| families.mono.as_deref().and_then(embed_family))
        .or_else(|| {
            families
                .mono
                .as_deref()
                .map(|f| Font::Base14(base14_for(f).to_string()))
        })
        .unwrap_or_else(|| main.clone());
    // A bold or italic FILE is what the corpus books ship: `BoldFont=Arimo-VF`
    // beside `UprightFont=Arimo-VF`. Where the two name the same file the font
    // is the same font, `Page::text_in` recognises it as one and the page is
    // set in the main face -- which is the honest outcome, since a variable
    // font's weight axis is not something this can instantiate.
    let face_file = |face: Face| {
        families
            .main_file
            .resolve_face(face, near)
            .and_then(|file| embed_file(&file))
    };
    let bold =
        face_file(Face::Bold).unwrap_or_else(|| Font::Base14(base14_face(&base, Face::Bold)));
    let italic =
        face_file(Face::Italic).unwrap_or_else(|| Font::Base14(base14_face(&base, Face::Italic)));
    let fonts = [main.clone(), mono, bold, italic];

    // Measure in the face that will be printed. An embedded font carries its
    // own widths; without one, cmr10's are the closest thing installed, and a
    // line may then run a little long or short of the measure.
    let embedded_widths: Vec<Option<Vec<i64>>> = fonts
        .iter()
        .map(|f| match f {
            Font::TrueType { widths, .. } => Some(widths.clone()),
            _ => None,
        })
        .collect();
    // Which characters each face can actually draw, from the font file's own
    // cmap. This is the question `luaotfload.add_fallback` asks per glyph, and
    // it is the reason the publication scripts required LuaTeX at all: a
    // character the face has no glyph for has to come from somewhere else, and
    // it cannot be fetched without first knowing that it is missing.
    let coverage: Vec<Option<std::collections::BTreeSet<u32>>> =
        fonts.iter().map(face_coverage).collect();
    // A face with no file to read is one of the fourteen, and carrying all of
    // WinAnsi is what being one of them means.
    let covers = |ch: char, face: Face| match &coverage[face.index()] {
        Some(has) => has.contains(&(ch as u32)),
        None => true,
    };
    // The faces the DOCUMENT named to fetch a missing glyph from, and the
    // glyphs it is about to ask them for. Reserving them first is what keeps
    // the file's `/W` array to the nine glyphs a book borrowed rather than to
    // all fifty thousand a broad face carries; the scan is over the DISTINCT
    // characters, so it costs one pass whatever the book's length.
    let mut fallbacks = Fallbacks::load(&families.fallbacks);
    if !fallbacks.is_empty() {
        let distinct: std::collections::BTreeSet<char> =
            printing_chars(text).filter(|c| !c.is_ascii()).collect();
        for ch in distinct {
            // Only what a face cannot draw and the Symbol font has not got:
            // asking for the rest would carry glyphs nothing ever draws.
            let drawn_already = SYMBOL_FONT.iter().any(|(c, _, _)| *c == ch)
                || Face::ALL
                    .iter()
                    .all(|face| winansi_code(ch).is_some() && covers(ch, *face));
            if !drawn_already {
                fallbacks.reserve(ch);
            }
        }
        fallbacks.settle();
    }
    let outside = |ch: char| fallbacks.glyph(ch);
    let metrics = find_font("cmr10").and_then(|p| Tfm::open(&p).ok());
    // What a stretch the face draws ITSELF costs. The codes are what the file
    // will hold and the characters are what the document wrote; the two differ
    // wherever WinAnsi puts a character somewhere other than its codepoint, and
    // each table is indexed by the one it is stated in.
    let own_width = |codes: &str, source: &str, face: Face, size: f64| -> f64 {
        if let Some(Some(w)) = embedded_widths.get(face.index()) {
            // PDF widths are 1/1000 em, and codes below 32 are not in the table.
            return codes
                .chars()
                .map(|c| {
                    let at = (c as usize).saturating_sub(32);
                    let mille = w.get(at).copied().unwrap_or(500);
                    mille as f64 / 1000.0 * size
                })
                .sum();
        }
        match &metrics {
            // The .tfm reader kerns and ligatures across neighbouring
            // characters, so it needs the string whole rather than an iterator.
            Some(f) => f.width_of(source) * size,
            None => source.chars().count() as f64 * size * 0.5,
        }
    };
    let piece_width = |piece: &Piece, face: Face, size: f64| -> f64 {
        match piece.from {
            // The fallback font's metrics are its own, and they are nothing
            // like the face's: `arrowright` is 987/1000 em where a letter is
            // about 500. Charging the face's widths for it would push every
            // line holding an arrow off the measure.
            Source::Symbol => piece
                .codes
                .chars()
                .map(|c| symbol_width(c as u8) as f64 / 1000.0 * size)
                .sum(),
            // A borrowed face's metrics are its own for the same reason, and
            // they are read from its `hmtx` by the glyph the piece names --
            // two bytes to the glyph, so the codes are taken in pairs.
            Source::Outside(at) => glyph_ids(&piece.codes)
                .map(|glyph| fallbacks.width(at, glyph, size))
                .sum(),
            Source::Face => own_width(&piece.codes, &piece.source, face, size),
        }
    };
    // Every branch measures through `printing_chars`. A marker's spec is digits
    // and commas, which every one of these tables has a real width for, so a
    // word inside one \textcolor was charged for its five letters plus fourteen
    // spec characters plus three markers -- and a line of them broke after four
    // words where the same line uncoloured holds seventeen. It cost whole
    // pages: rubyrs/docs/book.tex set in 340 and sets in 186 with them skipped.
    let width_of = |word: &str, face: Face, size: f64| -> f64 {
        // A formula measures what `mlist_to_hlist` made it, not what its
        // characters come to in the text face: the width of `$\frac{1}{2}$` is
        // the fraction's, and nothing about it can be read off the glyphs.
        if let Some(w) = crate::math::run_width(word) {
            return w;
        }
        // Plain ASCII is the case that runs a million times a book: every face
        // draws it, and the code and the character are the same number, so
        // there is nothing to decide per character. Answering it here keeps an
        // ordinary word measured exactly as it was before any of this.
        if word.is_ascii() {
            let plain: String = printing_chars(word).collect();
            return own_width(&plain, &plain, face, size);
        }
        drawn(word, &|c| covers(c, face), &outside)
            .iter()
            .map(|piece| piece_width(piece, face, size))
            .sum()
    };

    // The contents, if the document asked for one: set here rather than where
    // `\tableofcontents` stood, because an entry names the page its chapter
    // starts on and no such page exists until the whole document has been
    // broken and paginated. See `TOC`.
    // A `\ref` is the number of the sectioning unit its label stands in, which
    // no page has to be broken to know -- so it is resolved BEFORE the
    // contents, and its digits are on the lines the contents then counts. See
    // `REF`.
    let numbered = refs_numbered(text);
    let contented = contents_set(&numbered, layout, &width_of);
    // A `\pageref` is the page its label fell on, which the contents moves:
    // resolved after it, off the pages the contents itself is part of.
    let text = refs_paged(&contented, layout, &width_of);
    // A picture's node text is sized by the face the page is set in, through
    // the same widths every word on it is measured by -- `tikz::Estimate`,
    // which is all the lowerer had, charges half an em a character and sizes
    // every node box wrong.
    let picture_metrics = FaceMetrics {
        measure: &width_of,
        size: layout.size,
    };
    // Each picture's reserved height is restated against those metrics before
    // anything is fitted, so the room the page keeps for a drawing and the room
    // the drawing is given below are the same number. See `PICTURE`.
    let lines: Vec<String> = break_lines_measured(&text, layout, &width_of)
        .into_iter()
        .map(|line| picture_remeasured(line, &picture_metrics))
        // An image's lengths are resolved here for the same reason a picture's
        // height is: `width=0.8\textwidth` is a fraction of a measure, and the
        // natural size is a fact about a file, and the lowerer had neither.
        // Resolved BEFORE the page is fitted, so the room the fitter keeps for
        // a figure and the room the figure is given below are one number.
        .map(|line| image_remeasured(line, layout, near))
        .collect();

    // The colour stack, seeded with what the document is set in where it has
    // not said otherwise. Seeding it means a run with nothing pushed still
    // carries a colour, so every run is emitted with an explicit `rg` and none
    // inherits whatever the run before it left behind.
    //
    // It lives out here, across lines and across pages, because a `\color` is
    // in force until its group closes and a book says `\color{textPrim}` ONCE,
    // above everything. Restarting it per line put that colour back to black
    // on the second line of the document.
    let mut colours: Vec<Spec> = vec![(
        DEFAULT_COLOUR.0.to_string(),
        DEFAULT_COLOUR.1.to_string(),
        DEFAULT_COLOUR.2.to_string(),
    )];
    // The face stack, for the same reason: `\ttfamily` holds until its group
    // closes, and a group can hold a paragraph.
    let mut faces: Vec<Face> = vec![Face::Main];
    // Seeded with the document's own size, which is the entry never popped:
    // an unbalanced size marker leaves the body size in force.
    let mut sizes: Vec<TypeSize> = vec![TypeSize {
        size: layout.size,
        leading: layout.leading,
    }];

    // The printed page number. It counts from one and starts again after a
    // title page, which is what `\end{titlepage}` does to LaTeX's counter.
    let mut folio: usize = 1;
    let mut pages = Vec::new();
    for chunk in paginate(&lines, layout) {
        // Asked before the lines are drawn, because drawing consumes them.
        let restarts = restarts_here(&chunk);
        let mut page = Page::letter();
        // The page is painted first, and only first: a fill drawn after the
        // text covers it, and a document that sets a dark page sets light
        // text to go on it -- so getting the order wrong loses the words.
        if let Some((r, g, b)) = page_colour {
            page.content.push_str(&format!(
                "{r} {g} {b} rg\n0 0 {} {} re\nf\n0 0 0 rg\n",
                page.width, page.height
            ));
        }
        let mut y = layout.height + layout.margin - layout.leading;
        for line in chunk {
            // Which part of a table a line is was a question for `paginate`,
            // which has answered it: the page draws the line.
            let line = without_longtable(line);
            // A line of vertical space is space: the baseline moves down by
            // what that space measures -- see `line_height` -- and nothing is
            // drawn on it. Falling through to the run loop below would draw the
            // marker itself as a character.
            if is_space_line(line) {
                y -= line_height(line, leading_on(line, &mut sizes.clone()));
                continue;
            }
            // A picture is DRAWN where the line it stands on is, in the
            // operators `tikz::draw_on` writes: paths, arrow tips, node borders
            // and node text, with the `/ExtGState` and `/Shading` entries any
            // of them name registered on this page.
            //
            // It hangs BELOW the baseline the line above it left -- its box top
            // is at `y` and its bottom a bounding box lower -- because a
            // picture is a block and not a word, and drawing it up from `y`
            // would put it through the descenders of the line before. The
            // origin is chosen so the box's bottom left corner lands at the
            // margin: a picture's coordinates are its own and regularly
            // negative, and `bounds` is where they actually reach.
            // An image is drawn where the fitter reserved room for it. Both
            // lengths are already points -- `image_remeasured` resolved them
            // against the layout and the file before anything was fitted.
            if let Some((width, height, path)) = image_parts(line) {
                let (Ok(w), Ok(h)) = (width.parse::<f64>(), height.parse::<f64>()) else {
                    continue;
                };
                let left = match is_centred(line) {
                    true => layout.margin + (layout.measure - w).max(0.0) / 2.0,
                    false => layout.margin,
                };
                // A file readable when the room was reserved and unreadable
                // now leaves the room reserved and nothing in it, which is
                // what LaTeX does with a figure it cannot find.
                if let Some(file) = image_file(&path, near) {
                    if let Ok(image) = crate::image::open(&file) {
                        page.image(image, left, y - h, w, h);
                    }
                }
                // Exactly what `line_height` reserved, or the line after the
                // figure is set somewhere the fitter did not put it.
                y -= h + leading_on(line, &mut sizes.clone());
                continue;
            }
            if let Some((height, options, body)) = picture_parts(line) {
                let picture = crate::tikz::parse_document(
                    &options,
                    &body,
                    &crate::colour::Colours::new(),
                    &picture_metrics,
                );
                let (min_x, min_y, max_x, _) = picture.bounds();
                // A centred picture is placed by its own width, exactly as a
                // centred line of text is -- `\begin{center}` around a
                // `tikzpicture` is how most documents put a drawing on a page.
                let left = match is_centred(line) {
                    true => layout.margin + (layout.measure - (max_x - min_x)).max(0.0) / 2.0,
                    false => layout.margin,
                };
                let bottom = y - height;
                crate::tikz::draw_on(
                    &picture,
                    &mut page,
                    left - min_x,
                    bottom - min_y,
                    main.clone(),
                );
                // Exactly what `line_height` reserved for it, or the line after
                // the picture is set somewhere the fitter did not put it.
                y -= height + layout.leading;
                continue;
            }
            // A booktabs rule is DRAWN, not set: the breaker's line carries the
            // mark, the code saying which rule it is, and the spaces that
            // measure how far the table runs. See `rule_line`.
            if let Some(rest) = line.strip_prefix(TABLE_MARK) {
                let mut rest = rest.chars();
                let kind = rest.next().unwrap_or(RULE_MID);
                let span = width_of(
                    rest.as_str(),
                    current_face(&faces),
                    current_size(&sizes).size,
                );
                // booktabs' own weights, relative to the type size:
                // `\heavyrulewidth` above and below the table, `\lightrulewidth`
                // between its head and its body (booktabs.sty).
                let em = match kind == RULE_MID {
                    true => 0.05,
                    false => 0.08,
                };
                let weight = em * layout.size;
                // In the colour the text around it is in, so a rule in a book
                // with a dark page is not drawn in the black it inherited.
                let (r, g, b) = colours.last().cloned().unwrap_or_else(|| {
                    (
                        DEFAULT_COLOUR.0.to_string(),
                        DEFAULT_COLOUR.1.to_string(),
                        DEFAULT_COLOUR.2.to_string(),
                    )
                });
                page.content.push_str(&format!("{r} {g} {b} rg\n"));
                page.rule(layout.margin, y + layout.size * 0.25, span, weight);
                y -= layout.leading;
                continue;
            }
            // A centred line is positioned by its measured width rather than
            // at the margin -- that is the whole of what centring is here.
            // The marker is a PREFIX the breaker put on, so it comes off
            // before the line is measured or drawn.
            let (centred, line) = match line.strip_prefix(CENTRE) {
                Some(rest) => (true, rest),
                None => (false, line),
            };
            // A full line is set TO the measure; a ragged one is set at
            // whatever it comes to. The breaker decides which -- only a line
            // the next word would not fit on is full -- and says so with the
            // same kind of prefix. A centred line never carries it.
            let (justified, line) = match line.strip_prefix(JUSTIFY) {
                Some(rest) => (true, rest),
                None => (false, line),
            };
            // A list item's lines start in from the margin, and every line of
            // the item -- the one carrying the bullet and the three it wrapped
            // onto -- carries the same depth, so they stack at one left edge.
            let (depth, line) = strip_indent(line);
            let indent = list_indent(depth, layout);
            // A line is a sequence of RUNS, each with its own colour: the
            // markers turn colour on and off part way along it. Collapsing the
            // line to one colour state was the first attempt and drew none at
            // all, because the closing marker put the state back before
            // anything was emitted.
            // A line is a sequence of RUNS, each with its own colour and face:
            // the markers turn both on and off part way along it. Collapsing the
            // line to one colour state was the first attempt and drew none at
            // all, because the closing marker put the state back before
            // anything was emitted.
            //
            // A centred line is measured the way it will be DRAWN, each run in
            // its own face, because a line of code centred on the prose font's
            // widths sits off centre by the difference. The splitter is asked
            // on copies of the stacks so the drawing pass below still walks
            // them for real.
            let width: f64 = match centred {
                true => {
                    let (mut c, mut f, mut s) = (colours.clone(), faces.clone(), sizes.clone());
                    styled_runs(line, &mut c, &mut f, &mut s)
                        .iter()
                        .map(|(plain, _, face, ts)| width_of(plain, *face, ts.size))
                        .sum()
                }
                false => 0.0,
            };
            // What each space on a justified line is widened by: the room left
            // over at the measure, shared out between them. Measured the same
            // way a centred line is, on copies of the stacks, because it is the
            // same question -- what does this line come to as it will be drawn.
            // A line with no space in it has nothing to share the room between
            // and is set where it stands.
            let extra: f64 = match justified {
                true => {
                    let (mut c, mut f, mut s) = (colours.clone(), faces.clone(), sizes.clone());
                    let runs = styled_runs(line, &mut c, &mut f, &mut s);
                    let natural: f64 = runs
                        .iter()
                        .map(|(plain, _, face, ts)| width_of(plain, *face, ts.size))
                        .sum();
                    let spaces: usize = runs
                        .iter()
                        .map(|(plain, _, _, _)| plain.chars().filter(|c| *c == ' ').count())
                        .sum();
                    match spaces {
                        0 => 0.0,
                        // Against the NARROWED measure: a justified line inside
                        // a list is set to the list's right edge, not out to
                        // the page's, or the indent would be paid for twice.
                        n => (layout.measure - indent - natural) / n as f64,
                    }
                }
                false => 0.0,
            };
            // Asked on a COPY: `styled_runs` below walks the real stack for
            // this same line, and advancing it twice would lose a size.
            let led = leading_on(line, &mut sizes.clone());
            let mut x = match centred {
                true => layout.margin + (layout.measure - width).max(0.0) / 2.0,
                false => layout.margin + indent,
            };
            for (plain, (r, g, b), face, run) in
                styled_runs(line, &mut colours, &mut faces, &mut sizes)
            {
                if plain.is_empty() {
                    continue;
                }
                // Every run says its colour, so the run after a `\textcolor`
                // gets the colour it was NESTED IN back. Resetting to `0 g`
                // instead is what drew the rest of a dark-paged book black.
                page.content.push_str(&format!("{r} {g} {b} rg\n"));
                // A formula is DRAWN where `mlist_to_hlist` put every glyph of
                // it -- each at its own position, in its own size, with the
                // fraction bars and radical rules between them -- rather than
                // set as a run of characters. Each glyph still goes through the
                // font chain, because which face can draw a sigma is a question
                // about the document's fonts and not about the formula.
                if let Some(set) = crate::math::run_setting(&plain) {
                    for (glyph, gx, gy, size) in crate::math::glyphs(&set) {
                        for piece in drawn(&glyph, &|c| covers(c, face), &outside) {
                            let font = match piece.from {
                                Source::Symbol => Font::Base14(SYMBOL_FONT_NAME.to_string()),
                                Source::Outside(at) => match fallbacks.font(at) {
                                    Some(font) => font.clone(),
                                    None => continue,
                                },
                                Source::Face => fonts[face.index()].clone(),
                            };
                            let at = Set {
                                natural: 0.0,
                                width: 0.0,
                            };
                            page.text_set(font, size, x + gx, y + gy, &piece.codes, at);
                        }
                    }
                    for (rx, ry, rw, rh) in crate::math::rules(&set) {
                        page.rule(x + rx, y + ry, rw, rh);
                    }
                    x += crate::math::pt(set.width);
                    continue;
                }
                // A run is not necessarily one font either: a character the
                // face cannot draw is drawn from the fallback, which is a
                // different font resource and so a `Tf` of its own. The pieces
                // are positioned exactly as the runs are, each one advancing x
                // by what it just drew.
                for piece in drawn(&plain, &|c| covers(c, face), &outside) {
                    let font = match piece.from {
                        Source::Symbol => Font::Base14(SYMBOL_FONT_NAME.to_string()),
                        // The face the chain answered with, carried in the file
                        // as the glyphs this document borrowed from it.
                        Source::Outside(at) => match fallbacks.font(at) {
                            Some(font) => font.clone(),
                            None => continue,
                        },
                        Source::Face => fonts[face.index()].clone(),
                    };
                    // The piece takes its share of the room: what it measures,
                    // plus the widening for each space that falls inside it.
                    // x advances by what was actually SET and not by what the
                    // glyphs come to, or the piece after it would be drawn back
                    // over the space just widened.
                    let natural = piece_width(&piece, face, run.size);
                    // A borrowed face's codes are glyph ids, and a byte 0x20
                    // inside one is half an id rather than a space: there is
                    // nothing on such a piece for the room to be shared over.
                    let spaces = match piece.from {
                        Source::Outside(_) => 0.0,
                        _ => piece.codes.chars().filter(|c| *c == ' ').count() as f64,
                    };
                    let set = Set {
                        natural,
                        width: natural + extra * spaces,
                    };
                    // At the run's own size, not the document's: this is the
                    // `Tf` operator, and a heading set at the body size is
                    // exactly what made every book short. The width above was
                    // measured at the same number, so the two agree.
                    page.text_set(font, run.size, x, y, &piece.codes, set);
                    x += set.width;
                }
            }
            // The same step the fitter charged for this line: `leading_on` is
            // asked once here and once there, never spelled twice.
            y -= led;
        }
        // The page number, as LaTeX's `plain` style sets it: centred at the
        // foot. texrs drew none at all, and it is the ONE thing that held every
        // case of the parity ladder at PAGESIZE -- lualatex's text for
        // `Hello world.` is "Hello world. 1" against texrs's "Hello world.",
        // one word apart, and that word is the folio on every document.
        //
        // Numbered the way the contents numbers it (`heading_pages`), so the
        // page a contents entry names is the number printed on that page: a
        // title page resets the count, since `\end{titlepage}` does.
        if restarts {
            folio = 1;
        }
        let shown = folio.to_string();
        let width = width_of(&shown, Face::Main, layout.size);
        page.text_in(
            main.clone(),
            layout.size,
            (page.width - width) / 2.0,
            page.height - layout.margin - layout.height - FOOTSKIP,
            &shown,
        );
        folio += 1;
        pages.push(page);
    }
    document(&pages)
}

/// The document's own face, as the metrics a picture's nodes are sized by.
///
/// A node's border is drawn around its text, so its size is not in the source:
/// it has to be measured, and measuring it is a question about the fonts this
/// page is being set in. `to_pdf` answers exactly that question for every word
/// it sets, so a node's text is measured the same way rather than by a second
/// idea of what the document's font is.
struct FaceMetrics<'a> {
    /// `to_pdf`'s own `width_of`, which measures at `size`.
    measure: &'a dyn Fn(&str, Face, f64) -> f64,
    /// The size those widths are stated at: the document's type size.
    size: f64,
}

impl crate::tikz::Metrics for FaceMetrics<'_> {
    /// A node is set in the body face -- `\node[font=\ttfamily]` is a font
    /// option this does not read -- and a font's widths scale with its size.
    fn width_of(&self, text: &str, size: f64) -> f64 {
        (self.measure)(text, Face::Main, self.size) * size / self.size
    }
}

/// One stretch of a run that a single font draws.
struct Piece {
    /// What goes in the content stream: one byte a glyph, in that font's own
    /// encoding, held as `char`s that are all under 256.
    codes: String,
    /// The document's own characters, for a face whose widths are not in the
    /// file and are measured out of `cmr10` instead.
    source: String,
    /// Which font of the chain draws this stretch.
    from: Source,
}

/// The glyph ids a borrowed face's piece names, out of its two-byte codes.
///
/// The codes are held as `char`s under 256 and not as bytes, because that is
/// what the content stream escapes one at a time; reading them back as UTF-8
/// bytes would split every code above 127 into two.
fn glyph_ids(codes: &str) -> impl Iterator<Item = u16> + '_ {
    let mut chars = codes.chars();
    std::iter::from_fn(move || {
        let hi = chars.next()?;
        let lo = chars.next()?;
        Some((hi as u16) << 8 | lo as u16)
    })
}

/// Which font of the chain a piece is drawn from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    /// The face in force, in its own WinAnsi codes.
    Face,
    /// The Symbol font, in its own codes.
    Symbol,
    /// One face of the document's own fallback chain, in glyph ids: two bytes
    /// a glyph, because that is what `/Identity-H` addresses.
    Outside(usize),
}

/// Resolve a run through the chain, into the stretches each font draws.
///
/// Neighbouring characters that land in the same font are one piece, so a line
/// of prose is still one `Tj` and only the arrow in the middle of it is its
/// own. A character that no face, no fallback and no stand-in has is left out,
/// which is what the DVI path does with the same character.
fn drawn(
    text: &str,
    covers: &dyn Fn(char) -> bool,
    outside: &dyn Fn(char) -> Option<(usize, u16)>,
) -> Vec<Piece> {
    let mut pieces: Vec<Piece> = Vec::new();
    for ch in printing_chars(text) {
        let Some(glyph) = glyph_for(ch, covers, outside) else {
            continue;
        };
        let (from, codes, source) = match glyph {
            Glyph::Own(code) => (Source::Face, (code as char).to_string(), ch.to_string()),
            Glyph::Fallback(code) => (Source::Symbol, (code as char).to_string(), ch.to_string()),
            // A glyph id is a two-byte code, written high byte first the way
            // `/Identity-H` reads it, and held as two `char`s under 256 so the
            // content stream escapes each of them as the one byte it is.
            Glyph::Outside(face, glyph) => (
                Source::Outside(face),
                [(glyph >> 8) as u8 as char, (glyph & 0xFF) as u8 as char]
                    .iter()
                    .collect(),
                ch.to_string(),
            ),
            // A stand-in is measured as what it SETS and not as what it stands
            // for: `Cmd` takes three letters' room wherever it lands.
            Glyph::StandIn(text) => (Source::Face, text.to_string(), text.to_string()),
        };
        match pieces.last_mut() {
            Some(last) if last.from == from => {
                last.codes.push_str(&codes);
                last.source.push_str(&source);
            }
            _ => pieces.push(Piece {
                codes,
                source,
                from,
            }),
        }
    }
    pieces
}

/// Which characters a face can draw, out of the font file's own `cmap`.
///
/// `None` for one of the fourteen: it has no file to read, and carrying all of
/// WinAnsi is what being one of them means.
fn face_coverage(font: &crate::pdf::Font) -> Option<std::collections::BTreeSet<u32>> {
    let crate::pdf::Font::TrueType { bytes, .. } = font else {
        return None;
    };
    let sfnt = crate::sfnt::Sfnt::parse(bytes.clone()).ok()?;
    Some(sfnt.cmap().ok()?.into_keys().collect())
}

/// Break lines with a caller-supplied measurer.
///
/// The measurer is asked for a width IN A FACE, and the face is state the text
/// carries: a `\ttfamily` holds until its group closes, so the stack has to
/// live across words, lines and paragraphs here exactly as it does where the
/// page is drawn. Measuring a monospace word in the prose font is how a line
/// of code came out narrower than it sets.
fn break_lines_measured(
    text: &str,
    layout: &Layout,
    width_of: &dyn Fn(&str, Face, f64) -> f64,
) -> Vec<String> {
    // The breaker cares about the face and not about the colour, but the two
    // markers are interleaved in one stream, so the same splitter reads both
    // and the colour half goes on a stack nothing here looks at.
    let mut colours: Vec<Spec> = Vec::new();
    let mut faces: Vec<Face> = vec![Face::Main];
    // Seeded with the document's own size, which is the entry never popped:
    // an unbalanced size marker leaves the body size in force.
    let mut sizes: Vec<TypeSize> = vec![TypeSize {
        size: layout.size,
        leading: layout.leading,
    }];
    let mut lines = Vec::new();
    // Centring is a REGION and outlives the paragraph its marker landed in: a
    // title page is one `\begin{center}` holding half a dozen `\par`-separated
    // pieces, so the flag is carried across the loops rather than reset in
    // them.
    let mut centred = false;
    // And so is the list depth: a `\begin{itemize}` opens a region that runs
    // to its `\end`, and an item's own body is regularly several paragraphs.
    // 0 is "not in a list".
    let mut depth = 0usize;
    for para in text.split("\n\n") {
        // `\parskip`: the space LaTeX leaves BETWEEN two paragraphs, which
        // texrs left out entirely -- and which is the largest reason it set a
        // book short. See `PARAGRAPH_SPACE`. It goes in only between two
        // paragraphs that both set text, so a heading's own space is not added
        // to, and nothing is left at the head of the document.
        let after_text = lines
            .last()
            .is_some_and(|line: &String| !is_space_line(line) && !is_break_line(line));
        if after_text && sets_text(para) {
            lines.push(VERTICAL_SPACE.to_string());
        }
        // A picture is one line of its own, whatever it measures: it is drawn
        // rather than set, so there is nothing in it to fill to a measure and
        // nothing a break may fall inside. `line_height` reads the room it
        // takes off the marker, and the drawing loop below draws it there.
        if is_block_para(para) {
            // `\begin{center}\begin{tikzpicture}` is how most documents put a
            // drawing on a page, and the centring is a REGION that outlives the
            // paragraph its marker landed in -- so the flag `fill` is carrying
            // is what says whether this picture is centred, and it goes on the
            // line the way it goes on a line of text.
            lines.push(match centred {
                true => format!("{CENTRE}{}", para.trim()),
                false => para.trim().to_string(),
            });
            continue;
        }
        // A code listing is already broken, by the author -- see
        // `listing_lines`, which the DVI breaker reads the same way.
        if let Some(code) = listing_lines(para) {
            lines.extend(code.map(str::to_string));
            continue;
        }
        // A table is set as a table -- rows on their own lines, columns as
        // wide as their content -- rather than filled into prose. A row end is
        // what says a paragraph is one, the way a listing break says a
        // paragraph is a listing.
        if para.contains(TABLE_ROW) {
            table_lines(para, layout, &colours, &faces, &sizes, width_of, &mut lines);
            continue;
        }
        // A forced break is its own line, so the paginator can see one. It has
        // to come out here because `split_whitespace` below counts a form feed
        // as whitespace and would silently drop it.
        for (part_number, part) in para.split(PAGE_BREAK).enumerate() {
            if part_number > 0 {
                lines.push(PAGE_BREAK.to_string());
            }
            // Vertical space is its own line for the same reason: a vertical
            // tab is whitespace to Rust, so the space a heading asked for
            // would vanish into the gap between two words.
            for (space_number, part) in part.split(VERTICAL_SPACE).enumerate() {
                if space_number > 0 {
                    lines.push(VERTICAL_SPACE.to_string());
                }
                fill(
                    part,
                    &mut centred,
                    &mut depth,
                    layout,
                    &mut colours,
                    &mut faces,
                    &mut sizes,
                    width_of,
                    &mut lines,
                );
            }
        }
    }
    lines
}

/// Fill one stretch of text into lines at the measure, first-fit, marking each
/// line the centring markers put it inside.
///
/// The markers are cut out BEFORE words are counted because a region opens
/// against the text it centres -- `\centering` is followed straight by the
/// title -- so a word carrying one would be measured with it and set with it.
/// Cutting there also ends the line in hand, which is what a change of
/// alignment means: LaTeX's own `\centering` applies to whole paragraphs.
#[allow(clippy::too_many_arguments)]
fn fill(
    text: &str,
    centred: &mut bool,
    depth: &mut usize,
    layout: &Layout,
    colours: &mut Vec<Spec>,
    faces: &mut Vec<Face>,
    sizes: &mut Vec<TypeSize>,
    width_of: &dyn Fn(&str, Face, f64) -> f64,
    lines: &mut Vec<String>,
) {
    use crate::linebreak::After;
    let mut rest = text;
    loop {
        let (stretch, marker) = match rest.find([CENTRE, CENTRE_END, LIST_INDENT]) {
            Some(at) => (&rest[..at], rest[at..].chars().next()),
            None => (rest, None),
        };
        // A list narrows the measure by exactly what it moves the text in, so
        // a long item wraps INSIDE the list rather than running back out to
        // the right margin. A centred line is not moved in -- see `start` --
        // so it is not narrowed either.
        let indent = match *centred {
            true => 0.0,
            false => list_indent(*depth, layout),
        };
        let measure = layout.measure - indent;
        // A centred line says so in its first character, so the page can
        // position it by what it measures. The alternative -- a parallel list
        // of which lines are centred -- would have to survive pagination, and
        // the form feed a forced break travels as is the pattern already here.
        //
        // Centring is asked FIRST and the indent second, because the two are
        // not independent: a `\begin{center}` inside a list is still centred,
        // over the whole measure. An earlier revision of this put the indent
        // arm ahead of the centring one, so a centred line at any list depth
        // lost its centring marker and set flush at the list indent instead of
        // by its own width. A line can carry one prefix, and centring is the
        // one that wins;
        // `centring_inside_a_list_still_centres_over_the_whole_measure` in
        // tests/typeset.rs is what holds the order.
        let start = |centred: bool, depth: usize| match (centred, depth) {
            (true, _) => CENTRE.to_string(),
            (_, 0) => String::new(),
            (_, deep) => indent_mark(deep),
        };
        // Every word is measured ONCE, here, and the sequence is handed to the
        // total-fit breaker. Measuring here rather than inside the breaker is
        // what keeps the colour and face stacks walked exactly once and in
        // document order, which is the only order they mean anything in.
        let mut pieces: Vec<crate::linebreak::Piece> = Vec::new();
        for word in words_carrying_refs(stretch) {
            // The space between two words is set in the face in force where it
            // falls, which is the one the word BEFORE it left; and the word
            // itself costs what it costs in the faces its own markers select.
            // Measuring a monospace word in the prose font is how a line of
            // code came out narrower than it sets.
            if let Some(previous) = pieces.last_mut() {
                previous.after = crate::linebreak::After::Glue(width_of(
                    " ",
                    current_face(faces),
                    current_size(sizes).size,
                ));
            }
            measure_word(&word, colours, faces, sizes, width_of, &mut pieces);
        }
        // tex.web §813: the set of breakpoints that costs the WHOLE paragraph
        // least, rather than the ones a left-to-right fill happens to reach.
        let breaks = crate::linebreak::break_paragraph(&pieces, measure);
        let mut from = 0usize;
        for (number, end) in breaks.iter().enumerate() {
            let mut line = start(*centred, *depth);
            for (offset, piece) in pieces[from..*end].iter().enumerate() {
                // The pieces of one word run together; two words are joined by
                // the space that stood between them.
                if offset > 0 && matches!(pieces[from + offset - 1].after, After::Glue(_)) {
                    line.push(' ');
                }
                line.push_str(&piece.text);
            }
            // A line ending inside a word carries the hyphen the word did not
            // write. One ending after a hyphen the AUTHOR wrote already has it.
            if matches!(pieces[*end - 1].after, After::Discretionary(_)) {
                line.push('-');
            }
            from = *end;
            // Every line but the paragraph's last is a FULL line and is the one
            // set to the measure. The last stays ragged, which is what TeX's
            // `\parfillskip` makes of it, and a centred line is positioned by
            // its own width so it is left alone.
            let last = number + 1 == breaks.len();
            lines.push(match *centred || last {
                true => line,
                false => format!("{JUSTIFY}{line}"),
            });
        }
        match marker {
            // The depth marker carries the new depth as one digit, so a nested
            // list opening and the outer list resuming after it are the same
            // instruction with different arguments -- and nothing has to track
            // a stack on this side.
            Some(LIST_INDENT) => {
                let (deeper, after) = strip_indent(&rest[stretch.len()..]);
                *depth = deeper;
                rest = after;
            }
            Some(m) => {
                *centred = m == CENTRE;
                rest = &rest[stretch.len() + m.len_utf8()..];
            }
            None => return,
        }
    }
}

/// Measure one word into the pieces a line may end between.
///
/// A word carrying a marker is ONE piece: `word_width` walks the colour and
/// face stacks to measure it, and a fragment of it would be measured in a face
/// its own markers had not selected yet. Only a plain word is offered to the
/// hyphenator, which is also the only kind Liang's patterns were stated for.
fn measure_word(
    word: &str,
    colours: &mut Vec<Spec>,
    faces: &mut Vec<Face>,
    sizes: &mut Vec<TypeSize>,
    width_of: &dyn Fn(&str, Face, f64) -> f64,
    out: &mut Vec<crate::linebreak::Piece>,
) {
    use crate::linebreak::{After, Piece};
    let face = current_face(faces);
    let size = current_size(sizes).size;
    let whole = |text: &str, width: f64| Piece {
        text: text.to_string(),
        width,
        after: After::Nothing,
    };
    // A cross-reference span is one piece for a stronger reason than the face
    // and colour markers are: it is DELIMITED by its markers, so a word broken
    // between them -- and a pandoc label key is full of the hyphens the
    // breakpoint scan below cuts at -- would leave a fragment with no opening
    // marker on it, and the label key would be measured and drawn as text.
    if word.contains(['\u{1}', '\u{3}', FACE_PUSH, FACE_POP, REF]) {
        let width = word_width(word, colours, faces, sizes, width_of);
        out.push(whole(word, width));
        return;
    }
    // A hyphen the author wrote is a breakpoint of its own (tex.web §869), and
    // TeX does not go looking for more inside a word that already has one.
    if word.contains('-') {
        let parts: Vec<&str> = word.split_inclusive('-').collect();
        for (number, part) in parts.iter().enumerate() {
            out.push(Piece {
                text: part.to_string(),
                width: width_of(part, face, size),
                after: match number + 1 < parts.len() {
                    true => After::Explicit,
                    false => After::Nothing,
                },
            });
        }
        return;
    }
    // A word is regularly wrapped in punctuation -- an opening quote, a full
    // stop -- and the patterns are stated for LETTERS. The letters in the
    // middle are what is offered; the punctuation stays welded to the fragment
    // it sits against.
    let letter = |c: char| c.is_ascii_alphabetic();
    let head = word.len() - word.trim_start_matches(|c| !letter(c)).len();
    let tail = word.len() - word.trim_end_matches(|c| !letter(c)).len();
    let points: Vec<usize> = match head + tail <= word.len() {
        true => crate::linebreak::hyphenator()
            .points(&word[head..word.len() - tail])
            .iter()
            .map(|at| at + head)
            .collect(),
        false => Vec::new(),
    };
    if points.is_empty() {
        out.push(whole(word, width_of(word, face, size)));
        return;
    }
    let dash = width_of("-", face, size);
    let mut from = 0usize;
    for at in points.iter().copied().chain(std::iter::once(word.len())) {
        out.push(Piece {
            text: word[from..at].to_string(),
            width: width_of(&word[from..at], face, size),
            after: match at == word.len() {
                true => After::Nothing,
                false => After::Discretionary(dash),
            },
        });
        from = at;
    }
}

/// How far in from the margin a list at `depth` sets, in points.
///
/// LaTeX's `\leftmargini` is 2.5em at the type size (`article.cls`), and each
/// level in moves by the same again, so a nested item is visibly inside the one
/// holding it rather than a hair off it.
fn list_indent(depth: usize, layout: &Layout) -> f64 {
    depth as f64 * 2.5 * layout.size
}

/// The marker that puts what follows it at `depth`, which is what the lowerer
/// writes at every list boundary and what the breaker reads back.
///
/// The depth is ONE character, because that is what the marker registry
/// declares and what every reader of these markers skips. Nine levels is far
/// past anything a document nests -- LaTeX itself refuses past four -- and
/// deeper lists set at the ninth level rather than being read as a depth of
/// nothing. Depth 0 is the marker that ENDS a list: back at the margin.
pub fn indent_mark(depth: usize) -> String {
    let digit = char::from_digit(depth.min(9) as u32, 10).unwrap_or('0');
    format!("{LIST_INDENT}{digit}")
}

/// The depth a marked line carries, and the line without the marker.
///
/// Both the breaker and the page ask this, so the pair is split in one place
/// rather than in two that could disagree about how wide one level is.
fn strip_indent(line: &str) -> (usize, &str) {
    let Some(rest) = line.strip_prefix(LIST_INDENT) else {
        return (0, line);
    };
    let mut chars = rest.chars();
    let depth = chars.next().and_then(|c| c.to_digit(10)).unwrap_or(0) as usize;
    (depth, chars.as_str())
}

/// How far below the text block the page number sits, LaTeX's `\footskip`.
///
/// 30pt in every class the corpus uses. Measured against lualatex's own output
/// for a default article: it draws the folio at y=89.365 on a 792pt page, which
/// is the text block's bottom edge less this.
const FOOTSKIP: f64 = 30.0;

/// `\parskip`, as a fraction of the leading.
///
/// Every book in the corpus loads pandoc's preamble, which loads `parskip.sty`
/// and so sets `\parskip` to half a line. Measured, in the lualatex-built
/// scifi2/docs/book.pdf: consecutive baselines inside a paragraph are 13.549bp
/// apart and consecutive baselines ACROSS a paragraph boundary are 20.324bp
/// apart, and 20.324 - 13.549 = 6.775 = 13.549/2, on all 2,613 of that book's
/// paragraph boundaries.
const PARAGRAPH_SPACE: f64 = 0.5;

/// Whether the page holding these lines is the one after which numbering
/// starts again.
///
/// `\end{titlepage}` resets LaTeX's page counter (extreport.cls:514-518), so a
/// cover sheet is not page one and everything after it is a page lower than the
/// sheet it sits on. `heading_pages` reads the same mark for the contents; both
/// have to agree or a contents entry names a number no page carries.
fn restarts_here(lines: &[&str]) -> bool {
    lines.iter().any(|line| {
        let mut chars = line.chars();
        while let Some(ch) = chars.next() {
            if ch == TOC && chars.next() == Some(TOC_PAGE_ONE) {
                return true;
            }
        }
        false
    })
}

/// Whether a broken line is vertical space rather than text.
fn is_space_line(line: &str) -> bool {
    !line.is_empty() && line.chars().all(|c| c == VERTICAL_SPACE)
}

/// Whether a broken line is a forced page break rather than text.
fn is_break_line(line: &str) -> bool {
    !line.is_empty() && line.chars().all(|c| c == PAGE_BREAK)
}

/// What one broken line takes down the page.
///
/// A line of text takes a leading. A line of vertical space takes ONE unit of
/// `\parskip`, which is half of one: that is the smallest space the page
/// spends, so it is the unit the rest are counted in -- a heading asks for its
/// space as that many of them (`lower::push_heading`).
/// What each broken line is led on, in order, once for the whole document.
///
/// A line does not carry its own size the way a picture carries its own
/// height: a heading long enough to wrap sets its marker on the FIRST line
/// only, and the lines after it are inside a size that opened before them. So
/// the stack has to be walked in order across every line, which is what this
/// does once, rather than asked of a line on its own -- the fitter considers a
/// line many times and would get a different answer each way round.
///
/// A line is led on the LARGEST size standing anywhere on it, which is what
/// keeps a body line holding one `\large` word from being set into the line
/// above it.
fn line_leadings(lines: &[String], layout: &Layout) -> Vec<f64> {
    let mut sizes: Vec<TypeSize> = vec![TypeSize {
        size: layout.size,
        leading: layout.leading,
    }];
    lines
        .iter()
        .map(|line| leading_on(line, &mut sizes))
        .collect()
}

/// The leading one line is set on, advancing `sizes` by the markers it holds.
///
/// The fitter and the drawing pass MUST take their vertical step from this
/// same function: the fitter charges what it returns and the page advances by
/// it, and two spellings of the same rule are two chances for the page to
/// drift from what was fitted for it. The drawing pass asks on a copy of the
/// stack, because `styled_runs` walks the real one for the same line.
fn leading_on(line: &str, sizes: &mut Vec<TypeSize>) -> f64 {
    // What is in force as the line OPENS counts: a wrapped heading's second
    // line carries no marker of its own.
    let mut most = current_size(sizes).leading;
    let mut chars = line.chars();
    while let Some(ch) = chars.next() {
        match ch {
            SIZE_PUSH => {
                let mut spec = String::new();
                for c in chars.by_ref() {
                    if c == SIZE_PUSH {
                        break;
                    }
                    spec.push(c);
                }
                let pushed = match size_spec(&spec) {
                    Some((size, leading)) => TypeSize { size, leading },
                    None => current_size(sizes),
                };
                sizes.push(pushed);
                most = most.max(pushed.leading);
            }
            // The bottom entry is the document's own size and is never popped,
            // so an unbalanced close leaves the body size in force.
            SIZE_POP if sizes.len() > 1 => {
                sizes.pop();
            }
            _ => {}
        }
    }
    most
}

fn line_height(line: &str, leading: f64) -> f64 {
    // A picture takes its bounding box, and a leading under it before the next
    // line's baseline -- so its slot is the height of the drawing plus one
    // ordinary line. The number is read off the marker rather than measured
    // here, because measuring means parsing the picture and this is asked once
    // per line per page the fitter considers. See `PICTURE`.
    // An image takes the room it was remeasured to, and a leading under it
    // before the next baseline -- the same slot a picture gets.
    if let Some((_, height, _)) = image_parts(line) {
        if let Ok(height) = height.parse::<f64>() {
            return height + leading;
        }
    }
    if let Some((height, _, _)) = picture_parts(line) {
        return height + leading;
    }
    match is_space_line(line) {
        true => leading * PARAGRAPH_SPACE,
        false => leading,
    }
}

/// Whether a paragraph of the text stream will set any text at all.
///
/// A heading is written as its own paragraphs with paragraphs of vertical
/// space either side of it, and the space between two paragraphs goes in only
/// where there are two paragraphs of TEXT: adding it around a heading would
/// pay for the heading's own space twice. Every marker and every kind of
/// whitespace -- the vertical tab a listing breaks its lines on, the form feed
/// a forced break travels as -- answers false here.
fn sets_text(para: &str) -> bool {
    // A picture sets no CHARACTERS and is still something on the page, so the
    // space between two paragraphs goes in either side of it: without this a
    // drawing was welded to the prose above it, since every character of a
    // picture span is an instruction and none of them prints.
    is_block_para(para) || printing_chars(para).any(|c| !c.is_whitespace())
}

/// Split broken lines into pages: at a forced break, and otherwise wherever
/// LaTeX's penalties say a page costs least to end, carrying a longtable's
/// head and foot onto every page it runs onto.
///
/// Two questions, and they are answered in two places. `page_items` says what
/// the page HOLDS -- the lines that go together and the ones held back to be
/// repeated -- and `plan_pages` says where each page ENDS. Splitting them is
/// what lets the second be answered by cost over the whole document rather
/// than by the first break that does not fit; the third piece, `page_stops`,
/// is the fitting, and it is what the two share.
///
/// `chunks` alone cannot do this: it fills every page to the brim, so a
/// `\newpage` has nowhere to say anything and a chapter starts wherever the
/// previous one happened to end. Nor can it now be a count of lines at all,
/// since a line of vertical space is half the height of a line of text.
///
/// Nor can a table be filled that way. A table is not a run of lines a break
/// may fall anywhere in:
///
///   * a row whose cell wrapped is several lines and one thing, and a break
///     between them tears the row in half;
///   * the head belongs above the rows, on EVERY page they run onto, and a
///     continuation page whose first row has no head over it is a page of
///     unlabelled numbers;
///   * the foot belongs under the rows on every page the table runs PAST, and
///     under the end of the table on the last -- so room for it has to be kept
///     back before the page is full, not found after it is.
///
/// So the lines say what they are (`LONGTABLE`), and this reads them: the
/// repeating head and foot are lines of the stream that are held back rather
/// than set where they stand, and a break inside a table sets the foot, ends
/// the page and opens the next one with the head. The head and foot are lines
/// of `lines` like any other, so repeating one costs nothing -- the page holds
/// the same `&str` again.
fn paginate<'a>(lines: &'a [String], layout: &Layout) -> Vec<Vec<&'a str>> {
    let leadings = line_leadings(lines, layout);
    let items = page_items(lines, layout, &leadings);
    let mut pages: Vec<Vec<&str>> = Vec::new();
    for Planned { from, upto, foot } in plan_pages(&items, lines, layout, &leadings) {
        let mut page: Vec<&str> = Vec::new();
        // The head of a table this page opens in the middle of.
        if items[from].repeat_head {
            let (at, end) = items[from].head;
            page.extend(lines[at..end].iter().map(String::as_str));
        }
        for item in &items[from..upto] {
            if !matches!(item.kind, Kind::Set(_)) {
                continue;
            }
            page.extend(
                lines[item.at..item.at + item.len]
                    .iter()
                    .map(String::as_str),
            );
        }
        // And the foot of a table that runs past it.
        if let Some((at, end)) = foot {
            page.extend(lines[at..end].iter().map(String::as_str));
        }
        if !page.is_empty() {
            pages.push(page);
        }
    }
    pages
}

/// What one item of the page stream is.
enum Kind {
    /// A forced break: `\newpage`, which ends the page wherever it falls.
    Break,
    /// Lines the page sets nothing of, because they are a longtable's
    /// repeating head or its foot: held back to be put where they belong.
    Held,
    /// Lines to set, of this height, that no page break may fall inside --
    /// one ordinary line, or the several a table row wrapped onto.
    Set(f64),
}

/// One thing the paginator moves: the unit a page holds a whole number of.
///
/// `paginate` used to walk the lines and decide as it went, which is why the
/// two questions it answers -- WHAT a page holds and WHERE it ends -- were one
/// loop and only the second of them could be reconsidered. Splitting the first
/// out here is what lets the second be answered by cost over the whole
/// document rather than by the first break that fits.
///
/// The item stream does not depend on where the pages end, and that is what
/// makes this sound: the head a page repeats, the foot it holds back, and the
/// number of lines a too-tall group is cut into -- `room` below, which is
/// measured against the head and not against what the page has already used --
/// are all fixed by the lines alone.
struct Item {
    /// Where in `lines` this item starts, and how many lines it is.
    at: usize,
    len: usize,
    kind: Kind,
    /// The head a page beginning with this item repeats above it.
    head: (usize, usize),
    repeat_head: bool,
    /// The foot a page ending BEFORE this item carries below it, because the
    /// table runs past that page.
    foot: (usize, usize),
}

/// The height a run of lines takes.
fn run_height(lines: &[String], run: (usize, usize), leadings: &[f64]) -> f64 {
    lines[run.0..run.1]
        .iter()
        .enumerate()
        .map(|(i, l)| line_height(l, leadings[run.0 + i]))
        .sum()
}

/// Read the broken lines as the stream of items a page is filled with.
///
/// This is the walk `paginate` did, with the fitting taken out of it: every
/// decision left here is about what the lines ARE.
fn page_items(lines: &[String], layout: &Layout, leadings: &[f64]) -> Vec<Item> {
    const EMPTY: (usize, usize) = (0, 0);
    // A run of lines held back, extended one line at a time. They are written
    // consecutively (`table_lines`), so the run is a range; a line that does
    // not continue the run starts a new one rather than swallowing the gap.
    let extend = |run: (usize, usize), at: usize| match run.0 != run.1 && run.1 == at {
        true => (run.0, at + 1),
        false => (at, at + 1),
    };
    let mut head = EMPTY;
    let mut foot = EMPTY;
    let mut in_table = false;
    let mut items = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].as_str();
        if is_break_line(line) {
            // A forced break cannot fall inside a table: a table is one
            // paragraph and is recognised as one before the break character is
            // split out of it, so the break stays inside a cell. It can stand
            // BETWEEN two of them, and the second must not inherit the first
            // one's head.
            in_table = false;
            head = EMPTY;
            foot = EMPTY;
            items.push(Item {
                at: i,
                len: 1,
                kind: Kind::Break,
                head,
                repeat_head: false,
                foot,
            });
            i += 1;
            continue;
        }
        let Some(code) = longtable_code(line) else {
            // Out of the table again: the next one brings its own head, and
            // this one's must not be carried into the prose after it.
            in_table = false;
            head = EMPTY;
            foot = EMPTY;
            items.push(Item {
                at: i,
                len: 1,
                kind: Kind::Set(line_height(line, leadings[i])),
                head,
                repeat_head: false,
                foot,
            });
            i += 1;
            continue;
        };
        if !in_table {
            in_table = true;
            head = EMPTY;
            foot = EMPTY;
        }
        // What is only repeated is not set here.
        if code == LT_REPEAT || code == LT_FOOT {
            match code == LT_REPEAT {
                true => head = extend(head, i),
                false => foot = extend(foot, i),
            }
            items.push(Item {
                at: i,
                len: 1,
                kind: Kind::Held,
                head,
                repeat_head: false,
                foot,
            });
            i += 1;
            continue;
        }
        // What must not be broken into: the rest of this head, or the lines
        // this row wrapped onto.
        let continues = match code {
            LT_HEAD => LT_HEAD,
            _ => LT_CONT,
        };
        let mut group = 1;
        while lines
            .get(i + group)
            .is_some_and(|next| longtable_code(next) == Some(continues))
        {
            group += 1;
        }
        if code == LT_HEAD {
            head = (i, i + group);
        }
        // A group taller than the page it would be repeated on cannot be kept
        // whole; it is set where it falls rather than pushing every page after
        // it out of shape.
        // A table's lines are all lines of text, so the room a page under the
        // head has for them is what is left of it divided by the leading.
        let room = ((layout.height - run_height(lines, head, leadings)) / layout.leading)
            .floor()
            .max(1.0) as usize;
        let step = group.min(room);
        items.push(Item {
            at: i,
            len: step,
            kind: Kind::Set(step as f64 * layout.leading),
            head,
            // A page opening with the head is where the table STARTS on it,
            // and the head is what is about to be set anyway.
            repeat_head: code != LT_HEAD,
            foot,
        });
        i += step;
    }
    items
}

/// One page of the plan: the items it holds, and the foot that goes under it
/// because a table runs past it.
struct Planned {
    from: usize,
    upto: usize,
    foot: Option<(usize, usize)>,
}

/// A place a page may end: the item the NEXT page starts at, the height this
/// page comes to when it ends here, and the foot that goes under it.
#[derive(Clone, Copy)]
struct Stop {
    next: usize,
    used: f64,
    foot: Option<(usize, usize)>,
    /// The document, or a `\newpage`, ended the page rather than the fitting:
    /// the room left over is not this break's fault and is not charged for.
    forced: bool,
}

/// Every place a page filled from `from` may end, in order.
///
/// The last of them is where the old paginator broke -- the first break that
/// did not fit -- and the ones before it are what it never offered.
fn page_stops(
    items: &[Item],
    lines: &[String],
    layout: &Layout,
    leadings: &[f64],
    from: usize,
) -> Vec<Stop> {
    // Float slack, so a page that comes to exactly its height is not one line
    // short of it.
    const SLACK: f64 = 1e-6;
    let mut stops = Vec::new();
    let mut used = match items[from].repeat_head {
        true => run_height(lines, items[from].head, leadings),
        false => 0.0,
    };
    let mut set = 0usize;
    let mut j = from;
    while j < items.len() {
        let item = &items[j];
        match item.kind {
            // Consecutive breaks do not make blank pages: \clearpage after
            // \newpage is one break, which is what both mean together.
            Kind::Break if set == 0 => j += 1,
            Kind::Break => {
                stops.push(Stop {
                    next: j + 1,
                    used,
                    foot: None,
                    forced: true,
                });
                return stops;
            }
            Kind::Held => j += 1,
            Kind::Set(tall) => {
                if set > 0 {
                    // Room for the group AND for the foot under it: longtable
                    // keeps the foot's height back on every page for the same
                    // reason.
                    let foot = (item.foot.0 != item.foot.1).then_some(item.foot);
                    let below = foot.map_or(0.0, |f| run_height(lines, f, leadings));
                    stops.push(Stop {
                        next: j,
                        used: used + below,
                        foot,
                        forced: false,
                    });
                    // Nothing further down the page can fit either.
                    if used + below > layout.height + SLACK {
                        return stops;
                    }
                }
                used += tall;
                set += 1;
                j += 1;
            }
        }
    }
    stops.push(Stop {
        next: items.len(),
        used,
        foot: None,
        forced: true,
    });
    stops
}

/// `\clubpenalty`: what leaving a paragraph's first line alone at the foot of
/// a page costs. latex.ltx:500 of the 2026 release sets it to 150.
const CLUB_PENALTY: f64 = 150.0;

/// `\widowpenalty`: what leaving its last line alone at the top of the next
/// page costs. latex.ltx:501 sets it to 150 as well.
const WIDOW_PENALTY: f64 = 150.0;

/// `\brokenpenalty` (latex.ltx:503), charged for ending a page on a line that
/// broke inside a word.
const BROKEN_PENALTY: f64 = 100.0;

/// `\@secpenalty` (latex.ltx:17229), which `\@startsection` adds BEFORE the
/// space above a heading (latex.ltx:17242): a page would rather end there than
/// anywhere else nearby, which is what keeps a heading off the foot of a page.
const SEC_PENALTY: f64 = -300.0;

/// `\@M`. tex.web §157: a penalty this large or larger forbids the break.
const FORBID_PENALTY: f64 = 10000.0;

/// What a page pays for each leading of room it leaves empty.
///
/// The reference is set `\raggedbottom` -- report.cls:731-733 turns it on for
/// every one-sided document and no book in the corpus is two-sided -- so the
/// badness lualatex adds to a page's penalty is zero however short the page
/// is, and its output routine takes the cheapest penalty on the page whatever
/// that costs in white space. Priced that way, the least-cost plan for a WHOLE
/// document is degenerate: every page could end after one line for nothing. So
/// the room a page leaves is charged for, at a rate that says what the two are
/// worth against each other -- a widow is worth a line and a half of white
/// space, and a page is worth about thirty of them, so a page is added only
/// where the penalties really have piled up.
const SLACK_COST: f64 = 100.0;

/// What ending a page immediately before each line costs.
///
/// The penalties are the ones LaTeX states and TeX adds up between the lines
/// of a paragraph (tex.web §890): `\clubpenalty` after a paragraph's first
/// line, `\widowpenalty` before its last, `\brokenpenalty` after a line that
/// broke inside a word, and `\@M` -- no break at all -- inside a heading,
/// between a heading and the text it introduces, and after the first line of
/// that text. Two of them can fall in the same place and TeX charges both: the
/// one break in a two-line paragraph is a club AND a widow.
///
/// A paragraph is a run of consecutive lines of text here, which is what one
/// is on this side: the breaker puts a line of vertical space between two of
/// them, and a table's lines say they are a table's. A code listing is such a
/// run too and is treated as a paragraph -- leaving one line of a program
/// alone at the top of a page is the same fault as leaving one line of prose
/// there.
fn break_penalties(lines: &[String]) -> Vec<f64> {
    let n = lines.len();
    let mut cost = vec![0.0f64; n + 1];
    let space: Vec<bool> = lines.iter().map(|l| is_space_line(l)).collect();
    let text: Vec<bool> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| !space[i] && !is_break_line(l) && longtable_code(l).is_none())
        .collect();
    let mut i = 0;
    while i < n {
        if !text[i] {
            i += 1;
            continue;
        }
        let from = i;
        let mut upto = i;
        while upto < n && text[upto] {
            upto += 1;
        }
        i = upto;
        if (from..upto).any(|k| heading_line(&lines[k])) {
            // `\@xsect`: `\par \nobreak` after the title (latex.ltx:17282) and
            // `\clubpenalty\@M` on the paragraph under it (latex.ltx:17322), so
            // the heading, the space below it and the first two lines of that
            // paragraph are one block. `\interlinepenalty\@M` (latex.ltx:17259)
            // is what forbids a break inside a title that took two lines.
            let mut below = upto;
            while below < n && space[below] {
                below += 1;
            }
            let held = match below < n && text[below] {
                true => below + 1,
                false => below,
            };
            cost[from + 1..=held.min(n)].fill(FORBID_PENALTY);
            // `\addpenalty\@secpenalty\addvspace` (latex.ltx:17242): the
            // penalty goes before the space above the heading, and that space
            // is ONE `\vskip` -- so it is one breakpoint, at its head, and not
            // one per line of it.
            let mut above = from;
            while above > 0 && space[above - 1] {
                above -= 1;
            }
            // `.min(from)` because a heading with NO space above it -- one at
            // the very head of the stream -- has no `\vskip` to forbid a break
            // inside, and an empty range is what that has to come to. No
            // document in the corpus reaches it, because `push_heading` always
            // writes the space; the guard is here so that one which did would
            // not index backwards.
            cost[(above + 1).min(from)..from].fill(FORBID_PENALTY);
            // Two headings running together: the space between them is the
            // space below the first, which no break may fall in.
            if cost[above] != FORBID_PENALTY {
                cost[above] = SEC_PENALTY;
            }
            continue;
        }
        for e in from + 1..upto {
            if e == from + 1 {
                cost[e] += CLUB_PENALTY;
            }
            if e == upto - 1 {
                cost[e] += WIDOW_PENALTY;
            }
            if printing_chars(&lines[e - 1]).last() == Some('-') {
                cost[e] += BROKEN_PENALTY;
            }
        }
    }
    cost
}

/// Whether this line is a heading the contents lists.
///
/// The lowerer writes the mark and the code naming the level
/// (`toc_entry_mark`) at the head of the title, and it survives the breaker:
/// `heading_pages` reads the same mark off the pages to number the contents.
fn heading_line(line: &str) -> bool {
    let mut chars = line.chars();
    while let Some(ch) = chars.next() {
        if ch == TOC && chars.next().is_some_and(|code| toc_level(code).is_some()) {
            return true;
        }
    }
    false
}

/// Where the pages end: the plan that costs the document least.
///
/// The first break that fits is what `paginate` used to take, and a page
/// filled that way has no way of knowing that the line it is about to keep is
/// the last of its paragraph, or that the heading it ends on introduces text
/// that is now overleaf. So the breaks are chosen the way
/// `linebreak::break_paragraph` chooses a paragraph's: every place a page MAY
/// end is offered, each costs what it leaves empty plus what LaTeX's penalties
/// say about the break itself, and the plan taken is the one whose total over
/// the whole document is least. `best[k]` is what setting `items[k..]` costs
/// with a page starting at `k`, so the answer is `best[0]` and it is built
/// backwards from the end of the document.
///
/// Returns each page as the items it holds and the foot that goes under it.
fn plan_pages(items: &[Item], lines: &[String], layout: &Layout, leadings: &[f64]) -> Vec<Planned> {
    // Float slack, so a page that comes to exactly its height is not one line
    // short of it.
    const SLACK: f64 = 1e-6;
    let penalty = break_penalties(lines);
    let count = items.len();
    let mut best = vec![f64::INFINITY; count + 1];
    let mut chosen: Vec<Option<Stop>> = vec![None; count + 1];
    best[count] = 0.0;
    for from in (0..count).rev() {
        // A page has to hold something and the document has to have a plan, so
        // where every break is forbidden or none of them fits, the fullest is
        // taken anyway -- which is what the old paginator did with all of them.
        let mut fallback: Option<(Stop, f64)> = None;
        for stop in page_stops(items, lines, layout, leadings, from) {
            let room = layout.height - stop.used;
            let fits = room >= -SLACK;
            let empty = match stop.forced {
                true => 0.0,
                false => SLACK_COST * room.max(0.0) / layout.leading,
            };
            let rest = best[stop.next] + empty;
            if fits || fallback.is_none() {
                fallback = Some((stop, rest));
            }
            let broken = match stop.forced {
                true => 0.0,
                false => penalty[items[stop.next].at],
            };
            if !fits || broken >= FORBID_PENALTY {
                continue;
            }
            if rest + broken < best[from] {
                best[from] = rest + broken;
                chosen[from] = Some(stop);
            }
        }
        if chosen[from].is_none() {
            if let Some((stop, cost)) = fallback {
                best[from] = cost;
                chosen[from] = Some(stop);
            }
        }
    }
    let mut plan = Vec::new();
    let mut at = 0usize;
    while at < count {
        let Some(stop) = chosen[at] else {
            break;
        };
        plan.push(Planned {
            from: at,
            upto: stop.next.min(count),
            foot: stop.foot,
        });
        at = stop.next;
    }
    plan
}

/// A table of contents: where one belongs, which headings feed it, and where
/// the document's own page numbering starts.
///
/// `\tableofcontents` is 123 occurrences across the corpus -- every book opens
/// with a contents page -- and the prelude answered it with nothing, so none
/// of them was set: a missing feature, and pages the reference has that this
/// does not.
///
/// A contents cannot be built in one pass. An entry says which page its
/// chapter starts on, that page is not known until the document has been
/// broken and paginated, and the contents itself then moves every page after
/// it -- it is pages of its own. TeX answers this with the `.aux` file and a
/// second run. There is no `.aux` here and no multi-pass driver, so the two
/// passes are run INSIDE the typesetter, in `contents_set`: the text is broken
/// and paginated with the contents in place, the page each heading landed on
/// is read back off the pages, and the contents is rebuilt with those numbers
/// until the numbers stop moving. That is the fixed point `latexmk` reaches by
/// running latex twice, and it is reached in two passes here for the same
/// reason -- an entry is one line whatever its page number reads, so only the
/// digits change the second time.
///
/// Recording the page as the page is BUILT was the alternative, and it cannot
/// answer this: the contents is set before the chapters it lists, so at the
/// moment its own page is built not one of the numbers on it exists yet.
///
/// The one character after the marker says what it is:
///
///   * a DIGIT -- the contents belongs here, listing headings down to that
///     level. It is `tocdepth`, which every book in the corpus sets to 0:
///     chapters only;
///   * `TOC_CHAPTER`, `TOC_SECTION`, `TOC_SUBSECTION` -- the heading this
///     marks is an entry of that level;
///   * `TOC_PAGE_ONE` -- the page after the one this falls on is page 1.
///
/// U+0015 is the next free control character after U+0014 (LIST_INDENT); the
/// `MARKERS` registry says what the rest are spent on.
pub const TOC: char = '\u{15}';

/// A `\chapter` heading, which is level 0 -- `tocdepth` 0 lists these alone.
pub const TOC_CHAPTER: char = 'c';

/// A `\section` heading: level 1.
pub const TOC_SECTION: char = 's';

/// A `\subsection` heading: level 2.
pub const TOC_SUBSECTION: char = 'u';

/// The page after the one this marker is set on is page 1.
///
/// The `titlepage` environment closes with `\newpage` and then, unless the
/// class is two-sided, `\setcounter{page}\@ne` -- extreport.cls:514-518, which
/// is the class every book in the corpus loads and no document in it says
/// `twoside`. So the cover sheet is not one of the document's numbered pages,
/// and every folio a contents entry prints is one less than the sheet it
/// stands on: lualatex's own contents for `rubyrs/docs/book.tex` reads 1 for a
/// chapter on sheet 2 and 6 for one on sheet 7, and this reads the same.
pub const TOC_PAGE_ONE: char = 'p';

/// What a contents page is headed: `\contentsname` in book.cls and report.cls.
const CONTENTS_NAME: &str = "Contents";

/// How many times the contents is rebuilt before its numbers are taken as
/// settled. Two passes is what a document needs; the second is the one that
/// finds them unchanged, and the rest are only ever reached by a document
/// whose entries wrap differently each time.
const CONTENTS_PASSES: usize = 4;

/// The heading level a TOC code names, or `None` when the code is not an entry.
fn toc_level(code: char) -> Option<usize> {
    match code {
        TOC_CHAPTER => Some(0),
        TOC_SECTION => Some(1),
        TOC_SUBSECTION => Some(2),
        _ => None,
    }
}

/// The mark naming a heading of this level, for the lowerer to write.
///
/// Empty past a subsection: `\subsubsection` is a heading the contents does
/// not list here, and there is no code for it to carry.
pub fn toc_entry_mark(level: usize) -> String {
    match level {
        0 => format!("{TOC}{TOC_CHAPTER}"),
        1 => format!("{TOC}{TOC_SECTION}"),
        2 => format!("{TOC}{TOC_SUBSECTION}"),
        _ => String::new(),
    }
}

/// The mark asking for a contents listing headings down to `depth`.
pub fn toc_request_mark(depth: usize) -> String {
    let digit = char::from_digit(depth.min(9) as u32, 10).unwrap_or('0');
    format!("{TOC}{digit}")
}

/// The mark saying the page after this one is the document's page 1.
pub fn toc_page_one_mark() -> String {
    format!("{TOC}{TOC_PAGE_ONE}")
}

/// How deep a contents this text asks for, or `None` when it asks for none.
///
/// The digit is what tells a request from an entry: an entry's code is a
/// letter, so the scan walks past every heading in the document without
/// mistaking one for the `\tableofcontents` that lists it.
fn contents_depth(text: &str) -> Option<usize> {
    let mut rest = text;
    while let Some(at) = rest.find(TOC) {
        let after = &rest[at + TOC.len_utf8()..];
        let code = after.chars().next()?;
        if let Some(depth) = code.to_digit(10) {
            return Some(depth as usize);
        }
        rest = &after[code.len_utf8()..];
    }
    None
}

/// Every heading the contents lists, in the order the document sets them.
///
/// The title is taken through `printing_chars`, so a heading's own colour and
/// face markers do not travel into the contents line: they are balanced where
/// the heading stands, but the colour and face stacks carry from line to line
/// across the whole document, and a marker copied into a second place is a
/// marker pushed twice.
fn contents_entries(text: &str, depth: usize) -> Vec<(usize, String)> {
    let mut entries = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(TOC) {
        let after = &rest[at + TOC.len_utf8()..];
        let Some(code) = after.chars().next() else {
            break;
        };
        rest = &after[code.len_utf8()..];
        let Some(level) = toc_level(code) else {
            continue;
        };
        if level > depth {
            continue;
        }
        // The heading owns its paragraph: the lowerer writes the mark, the
        // title, and the paragraph break that ends it. A title broken across
        // source lines is one line here, so the words are rejoined.
        let title = rest.split("\n\n").next().unwrap_or("");
        let plain: String = printing_chars(title).collect();
        entries.push((
            level,
            plain.split_whitespace().collect::<Vec<_>>().join(" "),
        ));
    }
    entries
}

/// Which page of the document each heading landed on, in the document's own
/// page numbering.
///
/// Read off the PAGES rather than off the lines, because that is the question
/// an entry asks -- and only the paginator knows where a page ends.
fn heading_pages(lines: &[String], layout: &Layout, depth: usize) -> Vec<usize> {
    let mut found = Vec::new();
    for (folio, page) in folio_pages(lines, layout) {
        for line in page {
            let mut chars = line.chars();
            while let Some(ch) = chars.next() {
                if ch != TOC {
                    continue;
                }
                let Some(code) = chars.next() else {
                    break;
                };
                if matches!(toc_level(code), Some(level) if level <= depth) {
                    found.push(folio);
                }
            }
        }
    }
    found
}

/// Whether this line carries the mark saying the page after it is page 1.
///
/// The code character belongs to the mark whatever it is, so the scan steps
/// over it: a contents mark reading `TOC` `TOC_CHAPTER` followed by a `p` the
/// document wrote is not this.
fn carries_page_one(line: &str) -> bool {
    let mut chars = line.chars();
    while let Some(ch) = chars.next() {
        if ch == TOC && chars.next() == Some(TOC_PAGE_ONE) {
            return true;
        }
    }
    false
}

/// Every page of the document paired with the folio it prints, in the
/// document's own page numbering.
///
/// The folio restarts at 1 on the page AFTER the one carrying `TOC_PAGE_ONE`,
/// because that mark stands on a cover sheet the class does not number -- see
/// `toc_page_one_mark`.
///
/// The contents and the cross-references ask the same question of the pages --
/// which page did this mark land on -- so they ask it in one place rather than
/// keeping two copies of the folio arithmetic that could drift apart.
fn folio_pages<'a>(lines: &'a [String], layout: &Layout) -> Vec<(usize, Vec<&'a str>)> {
    let mut folio = 1usize;
    let mut pages = Vec::new();
    for page in paginate(lines, layout) {
        // Read off the page the mark stands on and applied to the NEXT one:
        // the mark says the page after it is page 1.
        let restart = page.iter().any(|line| carries_page_one(line));
        pages.push((folio, page));
        folio = match restart {
            true => 1,
            false => folio + 1,
        };
    }
    pages
}

/// One entry of the contents: its title, the leaders, and the page it starts
/// on set against the right margin.
///
/// A title too long for one line is filled to what the number and a space
/// either side of the leaders leave of the measure, and the number goes on the
/// last of its lines -- which is where LaTeX's own `\@dottedtocline` puts it.
fn entry_lines(
    level: usize,
    title: &str,
    page: usize,
    layout: &Layout,
    width_of: &dyn Fn(&str, Face, f64) -> f64,
) -> Vec<String> {
    let face = Face::Main;
    let number = page.to_string();
    let indent = list_indent(level, layout);
    let space = width_of(" ", face, layout.size);
    // A face whose full stop measures nothing would divide by zero; one dot is
    // then the whole leader, which is honest about knowing no better.
    let dot = width_of(".", face, layout.size).max(f64::EPSILON);
    let room = layout.measure - indent - width_of(&number, face, layout.size) - 2.0 * space;
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in title.split_whitespace() {
        let wider = match current.is_empty() {
            true => word.to_string(),
            false => format!("{current} {word}"),
        };
        match !current.is_empty() && width_of(&wider, face, layout.size) > room {
            true => lines.push(std::mem::replace(&mut current, word.to_string())),
            false => current = wider,
        }
    }
    let dots = ((room - width_of(&current, face, layout.size)) / dot)
        .floor()
        .max(0.0) as usize;
    lines.push(format!("{current} {} {number}", ".".repeat(dots)));
    // Level 0 sets at the margin and carries no mark at all; a deeper entry
    // carries the indent marker a list item does, so the page positions it the
    // one way it knows.
    let mark = match level {
        0 => String::new(),
        deep => indent_mark(deep),
    };
    lines.into_iter().map(|l| format!("{mark}{l}")).collect()
}

/// The contents itself: the heading, and the entries under it.
///
/// The entry lines are handed to the breaker the way a code listing's are --
/// each already broken, terminated by `LISTING_BREAK` -- because a contents
/// line is set as it is written. Filled into a paragraph instead, the leaders
/// of one entry would run the next entry onto the end of it.
fn contents_block(
    entries: &[(usize, String)],
    pages: &[usize],
    layout: &Layout,
    width_of: &dyn Fn(&str, Face, f64) -> f64,
) -> String {
    let mut listing = String::new();
    for (at, (level, title)) in entries.iter().enumerate() {
        // A heading whose page has not been read yet -- the first pass, before
        // anything has been paginated -- still takes the one line it will take
        // when it has one, so the pass measures the contents at its real
        // height.
        let page = pages.get(at).copied().unwrap_or(0);
        // A chapter entry is set clear of the one above it: report.cls spends
        // `\vskip 1.0em` above every `\l@chapter` and nothing above an
        // `\l@section`. A line is the unit this page has, and the difference
        // it makes is whole pages -- lualatex sets rubyrs's ninety-six chapter
        // entries in four pages where this set them in two.
        if *level == 0 {
            listing.push(VERTICAL_SPACE);
            listing.push(LISTING_BREAK);
        }
        for line in entry_lines(*level, title, page, layout, width_of) {
            listing.push_str(&line);
            listing.push(LISTING_BREAK);
        }
    }
    // A contents starts a page, as `\chapter*{\contentsname}` does in
    // report.cls, and the chapter after it starts one of its own.
    format!(
        "\n\n{PAGE_BREAK}\n\n{VERTICAL_SPACE}\n\n{CONTENTS_NAME}\n\n{VERTICAL_SPACE}\n\n{listing}\n\n"
    )
}

/// The text with every contents request replaced by the contents itself.
fn with_contents(
    text: &str,
    entries: &[(usize, String)],
    pages: &[usize],
    layout: &Layout,
    width_of: &dyn Fn(&str, Face, f64) -> f64,
) -> String {
    let block = contents_block(entries, pages, layout, width_of);
    let mut out = String::with_capacity(text.len() + block.len());
    let mut rest = text;
    while let Some(at) = rest.find(TOC) {
        let after = &rest[at + TOC.len_utf8()..];
        let Some(code) = after.chars().next() else {
            break;
        };
        out.push_str(&rest[..at]);
        match code.is_ascii_digit() {
            true => out.push_str(&block),
            // An entry mark stays where it is: the pass that follows reads the
            // page it lands on back out of it.
            false => {
                out.push(TOC);
                out.push(code);
            }
        }
        rest = &after[code.len_utf8()..];
    }
    out.push_str(rest);
    out
}

/// The document's text with its contents set into it, page numbers and all.
///
/// Returns the text untouched -- and does no extra work at all -- for a
/// document that asked for no contents, which is every document that is not
/// one of the books.
fn contents_set<'a>(
    text: &'a str,
    layout: &Layout,
    width_of: &dyn Fn(&str, Face, f64) -> f64,
) -> std::borrow::Cow<'a, str> {
    let Some(depth) = contents_depth(text) else {
        return std::borrow::Cow::Borrowed(text);
    };
    let entries = contents_entries(text, depth);
    let mut pages: Vec<usize> = vec![0; entries.len()];
    for _ in 0..CONTENTS_PASSES {
        let staged = with_contents(text, &entries, &pages, layout, width_of);
        let lines = break_lines_measured(&staged, layout, width_of);
        let found = heading_pages(&lines, layout, depth);
        // The numbers stopped moving, and the text they were read out of is
        // the text that carries them: that is what a second run leaves behind.
        if found == pages {
            return std::borrow::Cow::Owned(staged);
        }
        pages = found;
    }
    std::borrow::Cow::Owned(with_contents(text, &entries, &pages, layout, width_of))
}

/// A cross-reference: `\ref` and `\pageref`, and the `\label` they name.
///
/// `\label` is 88,341 occurrences across the corpus and the prelude answered
/// all three of these with nothing, so `See chapter \ref{ch:one} on page
/// \pageref{ch:one}.` set as `See chapter  on page .` -- a book full of "see
/// chapter" with no number after it, which reads as broken prose rather than
/// as a missing feature.
///
/// A reference cannot be answered where it is written, for the same reason a
/// contents entry cannot: the number belongs to a unit that may not have been
/// read yet, and the page belongs to a pagination that has not happened. So
/// what goes into the text is the QUESTION -- this marker, one character
/// saying which of the three it is, the label key, and the marker again to
/// close it -- and the typesetter answers it, in the two places either side of
/// where the contents is built:
///
///   * `refs_numbered` gives a `\ref` the number of the sectioning unit its
///     label stands in. That needs no page broken, so it runs BEFORE the
///     contents is built and its digits are on the lines the contents counts;
///   * `refs_paged` gives a `\pageref` the page its label fell on, which is
///     only known AFTER the contents is in place -- the contents is pages of
///     its own and moves every page after it.
///
/// U+0017 is the next free control character after U+0016; the `MARKERS`
/// registry says what the rest are spent on.
pub const REF: char = '\u{17}';

/// A `\label`: the key it declares, standing where the document put it.
pub const REF_LABEL: char = 'l';

/// A `\ref`: the number of the unit holding this key.
pub const REF_NUMBER: char = 'n';

/// A `\pageref`: the page this key fell on.
pub const REF_PAGE: char = 'p';

/// What LaTeX sets for a reference whose label it has not got: `??`, from
/// latex.ltx's `\@setref`, which sets that and warns.
///
/// Setting nothing was what this did, and a gap in a sentence is a fault the
/// author cannot see. Two question marks are one they can.
const UNRESOLVED: &str = "??";

/// How many times the page references are resolved before their numbers are
/// taken as settled, for the reason `CONTENTS_PASSES` gives: a `\pageref` that
/// resolves from nothing to `12` is two characters wider than it was, and a
/// line that then does not fit moves the page the next label falls on.
const REF_PASSES: usize = 4;

/// The mark naming one cross-reference, for the lowerer to write.
///
/// The key is stripped of whitespace and of control characters: the span is
/// delimited by the marker, and a line is split into words on spaces, so a key
/// holding either could not be read back whole. A LaTeX label key holds
/// neither.
pub fn ref_mark(code: char, key: &str) -> String {
    let key: String = key
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect();
    format!("{REF}{code}{key}{REF}")
}

/// One cross-reference span in the text.
struct RefSpan<'a> {
    /// Where the opening marker is.
    at: usize,
    /// One past the closing marker.
    end: usize,
    /// Which of the three this is: a label, a `\ref` or a `\pageref`.
    code: char,
    /// The label key between them.
    key: &'a str,
}

/// The next cross-reference span at or after `from`, or `None`.
///
/// A marker with no closing marker after it is not a span. The lowerer writes
/// both or neither, so this is only reached by a document that wrote the
/// character itself -- and half a span read as a whole one would swallow the
/// rest of the book.
fn next_ref(text: &str, from: usize) -> Option<RefSpan<'_>> {
    let at = from + text[from..].find(REF)?;
    let after = at + REF.len_utf8();
    let code = text[after..].chars().next()?;
    let key_at = after + code.len_utf8();
    let close = key_at + text[key_at..].find(REF)?;
    Some(RefSpan {
        at,
        end: close + REF.len_utf8(),
        code,
        key: &text[key_at..close],
    })
}

/// Whether this word is nothing but cross-reference spans.
///
/// Pandoc writes `\label` straight after the heading it names, so the label
/// opens the paragraph under it and stands between the paragraph break and the
/// first word -- a word of its own, once the text is split on whitespace. It
/// prints nothing, so measured as a word it would pay for the space beside it:
/// a space the document did not write, at 88,341 places in the corpus, at the
/// head of the paragraph under every heading in every book.
/// `words_carrying_refs` puts it onto the word that follows instead, where it
/// measures nothing and marks the same place.
fn is_ref_only(word: &str) -> bool {
    let mut at = 0;
    while let Some(span) = next_ref(word, at) {
        if span.at != at {
            return false;
        }
        at = span.end;
    }
    at > 0 && at == word.len()
}

/// The words of `text`, with every span that is nothing but a cross-reference
/// carried onto the word after it.
///
/// Both breakers split a stretch into words here rather than each calling
/// `split_whitespace` and deciding for itself what to do with a label, because
/// a label measured as a word on one path and not on the other is two
/// different books.
fn words_carrying_refs(text: &str) -> Vec<std::borrow::Cow<'_, str>> {
    let mut words: Vec<std::borrow::Cow<str>> = Vec::new();
    let mut carried = String::new();
    for word in text.split_whitespace() {
        if is_ref_only(word) {
            carried.push_str(word);
            continue;
        }
        words.push(match carried.is_empty() {
            true => std::borrow::Cow::Borrowed(word),
            false => std::borrow::Cow::Owned(format!("{}{word}", std::mem::take(&mut carried))),
        });
    }
    // A span the text ENDED on has no word to be carried onto and goes onto
    // the word before it, so that the page it fell on can still be read back
    // off the line. Text that is nothing but spans has no word either way; the
    // label is then dropped, and a `\pageref` to it sets `??` -- which is the
    // honest answer, since there is no line for it to have landed on.
    if !carried.is_empty() {
        if let Some(last) = words.last_mut() {
            last.to_mut().push_str(&carried);
        }
    }
    words
}

/// Whether the text holds a cross-reference of this kind.
///
/// Both resolving passes ask this first and hand the text straight back when
/// the answer is no, so a document that writes no reference -- which is every
/// document in the corpus, and every test that is not about this -- is not
/// walked twice or rewritten at all.
fn has_ref(text: &str, code: char) -> bool {
    let mut at = 0;
    while let Some(span) = next_ref(text, at) {
        if span.code == code {
            return true;
        }
        at = span.end;
    }
    false
}

/// The number the class prints for each label: `1` for a chapter, `2.1` for
/// the first section of the second chapter, and so on down.
///
/// Counted off the contents marks, because those are the only record in the
/// text of where a sectioning unit begins and how deep it is -- and the
/// lowerer writes one for every heading whether or not a contents was asked
/// for. `tocdepth` is not consulted: what the contents LISTS is a different
/// question from what a unit is numbered.
///
/// Two limits, both of which the corpus reaches with nothing:
///
///   * a `\chapter*` is counted, because `toc_entry_mark` does not record the
///     star. No document in the corpus writes a starred heading;
///   * a `\subsubsection` carries no mark at all -- `toc_entry_mark` is empty
///     past a subsection -- so a label under one takes its subsection's
///     number. The corpus holds one `\subsubsection` in 169 files.
///
/// A label standing before any heading has no unit to be numbered by and is
/// left out, so a `\ref` to it sets `??`, which is what LaTeX sets for a
/// reference it cannot resolve.
fn unit_numbers(text: &str) -> std::collections::HashMap<String, String> {
    // Chapter, section, subsection: the three levels `toc_level` names.
    let mut counters = [0usize; 3];
    let mut current = String::new();
    let mut numbers = std::collections::HashMap::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        match ch {
            TOC => {
                let Some(code) = chars.next() else { break };
                let Some(level) = toc_level(code).filter(|l| *l < counters.len()) else {
                    continue;
                };
                counters[level] += 1;
                // A new chapter puts its sections back to zero: the second
                // chapter's first section is 2.1 and not 2.4.
                for deeper in counters[level + 1..].iter_mut() {
                    *deeper = 0;
                }
                current = counters[..=level]
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<String>>()
                    .join(".");
            }
            REF => {
                let Some(code) = chars.next() else { break };
                let key: String = chars.by_ref().take_while(|c| *c != REF).collect();
                // The last definition of a key wins, which is what a rerun
                // over LaTeX's own `.aux` does with a duplicated label.
                if code == REF_LABEL && !current.is_empty() {
                    numbers.insert(key, current.clone());
                }
            }
            _ => {}
        }
    }
    numbers
}

/// The text with every `\ref` replaced by the number of the unit its label
/// stands in, and `??` where there is no such label.
///
/// Public because `--text` resolves them too: a document read as text has no
/// pages, but it has its own structure, and "see chapter 2" is what the
/// sentence says.
pub fn refs_numbered(text: &str) -> std::borrow::Cow<'_, str> {
    if !has_ref(text, REF_NUMBER) {
        return std::borrow::Cow::Borrowed(text);
    }
    let numbers = unit_numbers(text);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(span) = next_ref(text, at) {
        out.push_str(&text[at..span.at]);
        match span.code == REF_NUMBER {
            true => out.push_str(numbers.get(span.key).map_or(UNRESOLVED, String::as_str)),
            // A label stays where it is: the pass that follows reads the page
            // it lands on back out of it, exactly as a contents entry mark is
            // left for `heading_pages`.
            false => out.push_str(&text[span.at..span.end]),
        }
        at = span.end;
    }
    out.push_str(&text[at..]);
    std::borrow::Cow::Owned(out)
}

/// Which page of the document each label fell on, in the document's own page
/// numbering.
///
/// The question `heading_pages` asks, asked at the labels instead and answered
/// off the same pages.
fn label_pages(lines: &[String], layout: &Layout) -> std::collections::HashMap<String, usize> {
    let mut found = std::collections::HashMap::new();
    for (folio, page) in folio_pages(lines, layout) {
        for line in page {
            let mut at = 0;
            while let Some(span) = next_ref(line, at) {
                if span.code == REF_LABEL {
                    found.insert(span.key.to_string(), folio);
                }
                at = span.end;
            }
        }
    }
    found
}

/// The text with every `\pageref` replaced by the page its label fell on, and
/// `??` where no page carries that label.
fn with_pagerefs(text: &str, pages: &std::collections::HashMap<String, usize>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(span) = next_ref(text, at) {
        out.push_str(&text[at..span.at]);
        match span.code == REF_PAGE {
            true => match pages.get(span.key) {
                Some(page) => out.push_str(&page.to_string()),
                None => out.push_str(UNRESOLVED),
            },
            false => out.push_str(&text[span.at..span.end]),
        }
        at = span.end;
    }
    out.push_str(&text[at..]);
    out
}

/// The document's text with its page references resolved.
///
/// Run AFTER the contents is set, because the contents is pages of its own and
/// moves every page after it: a number read before it was built would name the
/// sheet the label stood on without it.
///
/// The fixed point is `contents_set`'s, for the same reason and to the same
/// depth -- the number a reference sets is text on a line, so resolving one
/// can move the page the next one names. Returns the text untouched, and does
/// no work at all, for a document that wrote no `\pageref`.
fn refs_paged<'a>(
    text: &'a str,
    layout: &Layout,
    width_of: &dyn Fn(&str, Face, f64) -> f64,
) -> std::borrow::Cow<'a, str> {
    if !has_ref(text, REF_PAGE) {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut pages = std::collections::HashMap::new();
    for _ in 0..REF_PASSES {
        let staged = with_pagerefs(text, &pages);
        let lines = break_lines_measured(&staged, layout, width_of);
        let found = label_pages(&lines, layout);
        // The numbers stopped moving, and the text they were read out of is
        // the text that carries them.
        if found == pages {
            return std::borrow::Cow::Owned(staged);
        }
        pages = found;
    }
    std::borrow::Cow::Owned(with_pagerefs(text, &pages))
}

/// The left edge of a list item's lines, carried through the text the way
/// centring is, with the nesting depth as the one character after it.
///
/// `\item` is 8,683 occurrences across the corpus and set inline: an itemize
/// with two items read back as "first item second item" on one line, with no
/// bullet and no break, because the prelude answered `\begin{itemize}` with
/// nothing and `\item` with its optional argument. What a list needs from the
/// page is a left edge and a narrower measure, and both follow from one number
/// -- so the marker carries the depth and the page turns it into a position
/// (`list_indent`).
///
/// U+0014 is the next free control character after U+0013 (JUSTIFY); the
/// `MARKERS` registry below says what the rest are spent on.
pub const LIST_INDENT: char = '\u{14}';

/// A forced page break, carried through the text the way colour is.
///
/// `\newpage` and `\clearpage` were defined by the prelude to expand to
/// nothing, so a book's title page, copyright page and first chapter ran
/// together into one stream of prose and the page count came out at half what
/// the document asks for. A form feed is what the character means, it survives
/// the run because it is not a word, and it is split out of the text BEFORE
/// words are, since Rust counts it as whitespace and would otherwise drop it.
pub const PAGE_BREAK: char = '\u{c}';

/// A `tikzpicture`, carried whole through the text the way a page break is.
///
/// A picture is not text and cannot be expanded into any: its body is TikZ,
/// full of backslashes that are not control sequences and semicolons that end
/// commands, and the prelude used to answer `\draw#1;` with nothing -- so
/// every picture in every document was read, discarded, and drawn nowhere.
/// The body reaches the page instead, as ONE marker span holding the whole
/// picture, because a line is all that survives breaking and pagination.
///
/// The span is `PICTURE`, the height the picture reserves, `;`, the option
/// list and the body base64-encoded with a `:` between them, and `PICTURE`
/// again. Encoded because a picture body holds newlines, spaces and every
/// punctuation mark there is, and a paragraph splitter would tear it apart;
/// base64 is one word of `A-Za-z0-9+/=`, which nothing in the pipeline splits.
///
/// The height is carried rather than recomputed because `line_height` -- which
/// every page-fitting decision goes through -- has no fonts and no palette to
/// parse a picture with, and because parsing one per fitting decision would be
/// paid for over and over. `to_pdf` restates it once, against the document's
/// own metrics, so the room reserved is the room the picture is drawn in.
///
/// U+0018 is the next free control character after U+0017 (REF); the `MARKERS`
/// registry below says what the rest are spent on.
pub const PICTURE: char = '\u{18}';

/// An `\includegraphics` marker, bracketing its own spec at both ends.
///
/// U+0018 is `PICTURE` and U+0019/U+001A are the size pair; this is the next
/// free control character. See `MARKERS`.
pub const IMAGE: char = '\u{1b}';

/// One image span: the width and height asked for, and the file.
///
/// The two lengths are carried as the document WROTE them and resolved later:
/// `width=0.8\textwidth` is a fraction of a measure the lowerer does not know,
/// and the natural size is a fact about a file it has not opened. `f0.8` is
/// that fraction, a bare number is absolute PDF points, and an empty field is
/// "not asked for". `image_remeasured` turns all three into points once the
/// layout and the file are both in hand -- the same two-step `PICTURE` uses,
/// and for the same reason.
pub fn image_span(width: &str, height: &str, path: &str) -> String {
    format!("{IMAGE}{width};{height};{}{IMAGE}", base64(path.as_bytes()))
}

/// One `\includegraphics` length, as the spec [`image_span`] carries.
///
/// `\textwidth` and its two synonyms are the measure, which is a number this
/// side of the engine does not have -- so a length stated in them is carried
/// as the fraction it is and resolved where the layout is known. Every other
/// length is a dimension and becomes points here.
///
/// A bare `\textwidth` is the whole measure: the factor LaTeX reads in front
/// of a length defaults to one.
pub fn image_length(value: &str) -> String {
    let value = value.trim();
    let relative = ["textwidth", "linewidth", "columnwidth", "textheight"]
        .iter()
        .find_map(|name| value.split_once(&format!("\\{name}")));
    if let Some((factor, _)) = relative {
        let factor = factor.trim();
        let factor: f64 = match factor.is_empty() {
            true => 1.0,
            false => match factor.parse() {
                Ok(f) => f,
                Err(_) => return String::new(),
            },
        };
        return format!("f{factor}");
    }
    match dimen_bp(value) {
        Some(points) => points.to_string(),
        None => String::new(),
    }
}

/// The `width=` and `height=` an `\includegraphics` was given, as specs.
///
/// Every other key -- `scale`, `angle`, `keepaspectratio`, `trim`, `clip` --
/// is dropped rather than guessed at. A figure at the wrong size is a figure
/// a reader can still see; there is nothing to be gained by inventing what
/// `angle=90` would have done to the page.
pub fn image_options(options: &str) -> (String, String) {
    let mut width = String::new();
    let mut height = String::new();
    for option in options.split(',') {
        let Some((key, value)) = option.split_once('=') else {
            continue;
        };
        match key.trim() {
            "width" => width = image_length(value),
            "height" => height = image_length(value),
            _ => {}
        }
    }
    (width, height)
}

/// An image as its own paragraph, the way a picture is one.
pub fn image_mark(width: &str, height: &str, path: &str) -> String {
    format!("\n\n{}\n\n", image_span(width, height, path))
}

/// What an image line says: its width spec, its height spec, and its file.
///
/// `None` for every line that is not one, which is nearly all of them.
pub fn image_parts(line: &str) -> Option<(String, String, String)> {
    let inner = line.trim();
    // Centring is a prefix on the line rather than part of the span, so that
    // everything reading one reads it the same way centred or not -- as for a
    // picture, and `\begin{center}` around a figure is how most documents put
    // an image on a page.
    let inner = inner.strip_prefix(CENTRE).unwrap_or(inner);
    let inner = inner.strip_prefix(IMAGE)?.strip_suffix(IMAGE)?;
    let (width, rest) = inner.split_once(';')?;
    let (height, path) = rest.split_once(';')?;
    Some((
        width.to_string(),
        height.to_string(),
        String::from_utf8(unbase64(path)?).ok()?,
    ))
}

/// The room an image line takes, in PDF points, with its spec resolved.
///
/// A length is `f0.8` for a fraction of the measure, a bare number for points,
/// or empty for "take it from the file". A file that cannot be read reserves
/// nothing and draws nothing: a missing figure must cost a document its
/// picture, never its remaining pages.
///
/// With one length given the other follows the file's own proportions, which
/// is what `\includegraphics[width=\textwidth]` means and what every book in
/// the corpus writes.
fn image_size(
    width: &str,
    height: &str,
    file: &std::path::Path,
    layout: &Layout,
) -> Option<(f64, f64)> {
    let resolve = |spec: &str| -> Option<f64> {
        match spec.strip_prefix('f') {
            Some(fraction) => fraction.parse::<f64>().ok().map(|f| f * layout.measure),
            None => spec.parse().ok(),
        }
    };
    match (resolve(width), resolve(height)) {
        (Some(w), Some(h)) => Some((w, h)),
        (w, h) => {
            // A pixel is a big point unless the document says otherwise, which
            // is graphicx's rule for a file stating no resolution -- and
            // neither PNG's `pHYs` nor JPEG's density is read here.
            let image = crate::image::open(file).ok()?;
            let (nw, nh) = (f64::from(image.width), f64::from(image.height));
            if nw <= 0.0 || nh <= 0.0 {
                return None;
            }
            match (w, h) {
                (Some(w), None) => Some((w, w * nh / nw)),
                (None, Some(h)) => Some((h * nw / nh, h)),
                _ => Some((nw, nh)),
            }
        }
    }
}

/// The file an `\includegraphics` names, beside the document that named it.
///
/// A book writes `\includegraphics{figures/plot.png}` and is built from
/// another directory; the path is the document's, not the caller's. An
/// absolute path is taken as it stands.
fn image_file(path: &str, near: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    let named = std::path::Path::new(path);
    if named.is_absolute() && named.is_file() {
        return Some(named.to_path_buf());
    }
    if let Some(dir) = near {
        let beside = dir.join(named);
        if beside.is_file() {
            return Some(beside);
        }
    }
    named.is_file().then(|| named.to_path_buf())
}

/// An image line with its lengths resolved to points against the layout and
/// the file, so the fitter and the drawing pass read one number rather than
/// each working it out. A file that cannot be read leaves the line empty: it
/// reserves nothing and draws nothing.
fn image_remeasured(line: String, layout: &Layout, near: Option<&std::path::Path>) -> String {
    let Some((width, height, path)) = image_parts(&line) else {
        return line;
    };
    let centred = is_centred(&line);
    let Some(file) = image_file(&path, near) else {
        return String::new();
    };
    let Some((w, h)) = image_size(&width, &height, &file, layout) else {
        return String::new();
    };
    let span = image_span(&w.to_string(), &h.to_string(), &path);
    match centred {
        true => format!("{CENTRE}{span}"),
        false => span,
    }
}

/// The marker span for a picture, as its own paragraph.
///
/// The paragraph breaks are part of it: a picture is a block on the page and
/// not a word in a line, so it owns its own lines rather than running into the
/// prose either side of it -- exactly as a heading does (`lower::push_heading`).
///
/// The height is measured here with [`crate::tikz::parse_document`] against no
/// font at all, because the lowerer has none to size node text by. `to_pdf`
/// measures it again through the document's own face and rewrites this
/// number; nothing between the two reads it.
pub fn picture_mark(options: &str, body: &str) -> String {
    let picture = crate::tikz::parse_document(
        options,
        body,
        &crate::colour::Colours::new(),
        &crate::tikz::Estimate,
    );
    let (_, height) = picture.extent();
    format!("\n\n{}\n\n", picture_span(height, options, body))
}

/// One picture span: the height it reserves and the source it was read from.
fn picture_span(height: f64, options: &str, body: &str) -> String {
    format!(
        "{PICTURE}{height};{}:{}{PICTURE}",
        base64(options.as_bytes()),
        base64(body.as_bytes())
    )
}

/// What a picture line says: the height it reserves, its options and its body.
///
/// `None` for every line that is not one, which is all but a handful of them.
fn picture_parts(line: &str) -> Option<(f64, String, String)> {
    let inner = line.trim();
    // A centred picture carries the same prefix a centred line of text does,
    // and it is a prefix rather than part of the span so that everything which
    // reads one -- `line_height`, the page fitter, the drawing loop -- reads it
    // the same way whether the picture is centred or not.
    let inner = inner.strip_prefix(CENTRE).unwrap_or(inner);
    let inner = inner.strip_prefix(PICTURE)?.strip_suffix(PICTURE)?;
    let (height, source) = inner.split_once(';')?;
    let (options, body) = source.split_once(':')?;
    Some((
        height.parse().ok()?,
        String::from_utf8(unbase64(options)?).ok()?,
        String::from_utf8(unbase64(body)?).ok()?,
    ))
}

/// Whether a paragraph of the text stream is a picture and nothing else.
fn is_image_para(para: &str) -> bool {
    image_parts(para).is_some()
}

/// Whether a paragraph is a picture, or an image, which the breaker and the
/// fitter treat alike: one line, drawn rather than set.
fn is_block_para(para: &str) -> bool {
    is_picture_para(para) || is_image_para(para)
}

fn is_picture_para(para: &str) -> bool {
    picture_parts(para).is_some()
}

/// Whether a line -- of text or of picture -- is set by its own width.
fn is_centred(line: &str) -> bool {
    line.trim_start().starts_with(CENTRE)
}

/// The picture line with the height it really reserves, measured through the
/// document's own font.
///
/// The number `picture_mark` wrote was measured with no font at all, because
/// the lowerer has none: a node's text was charged half an em a character.
/// This is the same picture read again through the face the page is set in,
/// and it is what every fitting decision below then uses -- so the room a page
/// keeps for a picture is the room the picture is drawn in.
fn picture_remeasured(line: String, metrics: &dyn crate::tikz::Metrics) -> String {
    let Some((_, options, body)) = picture_parts(&line) else {
        return line;
    };
    let picture =
        crate::tikz::parse_document(&options, &body, &crate::colour::Colours::new(), metrics);
    let (_, height) = picture.extent();
    let span = picture_span(height, &options, &body);
    match is_centred(&line) {
        true => format!("{CENTRE}{span}"),
        false => span,
    }
}

/// The base64 alphabet, RFC 4648 §4.
const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// `bytes` as base64: one word, with nothing in it a text pipeline splits on.
fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let packed = group
            .iter()
            .enumerate()
            .fold(0u32, |acc, (at, b)| acc | (*b as u32) << (16 - 8 * at));
        for at in 0..4 {
            out.push(match at <= group.len() {
                true => BASE64[(packed >> (18 - 6 * at)) as usize & 0x3f] as char,
                false => '=',
            });
        }
    }
    out
}

/// The bytes back, or `None` for anything that is not base64.
fn unbase64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut packed = 0u32;
    let mut held = 0u32;
    for ch in text.chars() {
        if ch == '=' {
            break;
        }
        let six = BASE64.iter().position(|b| *b as char == ch)? as u32;
        packed = packed << 6 | six;
        held += 6;
        if held >= 8 {
            held -= 8;
            out.push((packed >> held) as u8);
        }
    }
    Some(out)
}

/// Every character the typesetting path reads as an INSTRUCTION rather than as
/// text, and whether one character of argument follows it.
///
/// A marker says where a line sits, what face it is in, what colour it takes --
/// none of which the document wrote. So none may ever reach a reader, and
/// `without_marks` in lib.rs is the one place that decides what a reader gets.
///
/// The list exists because forgetting that place is the mistake this port keeps
/// making. Three separate parallel implementations added a marker, taught
/// `to_pdf` and the line breaker about it, and left `without_marks` alone: one
/// of them put 122 raw control characters into a book's `--text` output, and
/// the file's own comment already recorded the same fault as fixed once before.
/// `every_marker_is_stripped_from_the_text_a_reader_gets` walks this list, so a
/// constant added without an entry here fails a test rather than shipping.
pub const MARKERS: &[(char, bool)] = &[
    // The colour trio: a spec opens, runs to its close, and a pop ends the run.
    ('\u{1}', false),
    ('\u{2}', false),
    ('\u{3}', false),
    (LISTING_BREAK, false),
    (PAGE_BREAK, false),
    (CENTRE, false),
    (CENTRE_END, false),
    (VERTICAL_SPACE, false),
    (JUSTIFY, false),
    // The one that carries an argument: the character naming the face.
    (FACE_PUSH, true),
    (FACE_POP, false),
    // The size pair. `SIZE_PUSH` brackets its own spec -- it appears twice,
    // once at each end -- so it swallows no single character the way a face
    // code does; the walk toggles on it instead, as it does for `PICTURE`.
    (SIZE_PUSH, false),
    (SIZE_POP, false),
    // The image span, bracketed by its own marker the way a picture is.
    (IMAGE, false),
    (TABLE_CELL, false),
    (TABLE_ROW, false),
    // The list indent carries an argument too: the digit saying how deep the
    // list holding this line is nested.
    (LIST_INDENT, true),
    // The other one that carries an argument: the character saying which rule,
    // or which of longtable's section boundaries, this mark is.
    (TABLE_MARK, true),
    // And the one saying which part of a longtable a line is, whose argument
    // is the code naming that part.
    (LONGTABLE, true),
    // The contents marker, whose argument says which of the three things it
    // is: a request, a heading it lists, or where page 1 begins.
    (TOC, true),
    // The cross-reference marker. Its argument says which of the three it is
    // -- a label, a `\ref` or a `\pageref` -- and the label key follows, up to
    // a second marker that closes the span.
    (REF, true),
    // The picture marker, span-shaped like the cross-reference one: everything
    // from it to the second one is the encoded picture, and a reader asking for
    // the document's TEXT gets none of it -- a picture has no words.
    (PICTURE, true),
];

/// The end of a line INSIDE a code listing, carried through the text the way a
/// page break is.
///
/// Pandoc wraps every code block in `Highlighting`, and that body is real TeX --
/// `\NormalTok{…}` and its siblings have to expand -- so it cannot be read as
/// verbatim and its newlines reached the breaker as ordinary spaces.
/// `split_whitespace` then reflowed the program into the prose around it.
/// Measured: rubyrs/docs/book.tex set a nine-line `dup_value` as three, one of
/// them `... => v.clone(), Some(obj) => { let copy = obj.clone();
/// self.alloc(copy) } None =>`, and the book came out in 208 pages where
/// lualatex sets it in 332. A vertical tab is a line separator by definition,
/// it survives the run because it is not a word, and like the page break it is
/// split out BEFORE words are.
pub const LISTING_BREAK: char = '\u{b}';

/// The lines of a code listing, or `None` when the paragraph is prose.
///
/// A listing arrives as `LISTING_BREAK`-TERMINATED lines, so an empty segment
/// is an empty code line and is kept: a blank line in a program is part of the
/// program, and a paragraph break -- which is what the lexer would have made of
/// it -- is not. Prose holds no break and is broken to the measure as before.
///
/// Both breakers ask this, so a listing is recognised the same way on the DVI
/// and PDF paths rather than in two places that could drift apart.
fn listing_lines(para: &str) -> Option<impl Iterator<Item = &str>> {
    para.contains(LISTING_BREAK)
        .then(|| para.split_terminator(LISTING_BREAK))
}

/// A cell boundary inside a table row: the `&` the document wrote, where a
/// table is being SET rather than flattened.
///
/// ASCII's own unit separator, which is what a field boundary means.
pub const TABLE_CELL: char = '\u{1f}';

/// The end of a table row: the `\\` the document wrote inside a table. ASCII's
/// record separator, for the same reason.
///
/// A paragraph holding one of these is a table, the way a paragraph holding a
/// `LISTING_BREAK` is a listing -- so the region needs no marker of its own.
pub const TABLE_ROW: char = '\u{1e}';

/// A table mark that is not text: one of booktabs' three rules, or the end of
/// one of longtable's sections. The character AFTER it says which.
pub const TABLE_MARK: char = '\u{1d}';

/// The part of a table a line belongs to: the character AFTER it is one of the
/// `LT_` codes below.
///
/// A longtable repeats its head at the top of every page it runs onto and sets
/// its foot at the bottom of every page it runs past. The only thing that
/// knows where a page ends is `paginate`, and all it sees is lines -- so the
/// line has to say what part of which table it is. That is what this carries.
///
/// It also says where a row STARTS, because a row is not a line: a cell that
/// wrapped makes a row several lines tall, and a page break inside those lines
/// tears the row in half.
///
/// U+0016 is the next free control character after U+0015; the `MARKERS`
/// registry above says what the rest are spent on.
pub const LONGTABLE: char = '\u{16}';

/// The codes `LONGTABLE` carries.
///
/// `LT_HEAD` is a head that is set where it stands AND repeated above every
/// continuation page. `LT_REPEAT` is the repeating head of a table that gave a
/// DIFFERENT first head, so it is set only above the pages after the first.
/// `LT_FOOT` is never set where it stands: it belongs at the bottom of a page
/// the table runs past, and `paginate` puts it there. `LT_ROW` opens a line
/// that is set where it stands, and `LT_CONT` continues it -- the two together
/// are what says a row and the lines its cells wrapped onto are one thing.
const LT_HEAD: char = 'h';
const LT_REPEAT: char = 'H';
const LT_FOOT: char = 'f';
const LT_ROW: char = 'r';
const LT_CONT: char = 'c';

/// Which part of a table this line is, or `None` when it is not a table line.
fn longtable_code(line: &str) -> Option<char> {
    line.strip_prefix(LONGTABLE)?.chars().next()
}

/// The line without the part it says it is: what the page draws.
fn without_longtable(line: &str) -> &str {
    let Some(rest) = line.strip_prefix(LONGTABLE) else {
        return line;
    };
    let mut chars = rest.chars();
    chars.next();
    chars.as_str()
}

/// The codes `TABLE_MARK` carries. The three rules, then the boundaries that
/// say where longtable's head and foot end.
///
/// Public because `lower.rs` writes them and this reads them, and one spelling
/// in one place is what keeps the two agreeing.
pub const RULE_TOP: char = 't';
pub const RULE_MID: char = 'm';
pub const RULE_BOTTOM: char = 'b';
pub const HEAD_END: char = 'h';
pub const FIRST_HEAD_END: char = 'H';
pub const FOOT_END: char = 'f';
/// `\endlastfoot`, which is not `\endfoot`: one is set at the bottom of every
/// page the table runs past and the other once, under the end of the table.
/// They shared `FOOT_END` and a table writing both put its last foot into its
/// body.
pub const LAST_FOOT_END: char = 'F';

/// One entry of a table: a rule across it, or a row of cells.
///
/// Cloneable because a section can be set twice: a table that gave no
/// `\endlastfoot` sets its repeating foot under the end of the table as well
/// as at the bottom of every page before it.
#[derive(Clone)]
enum Entry {
    Rule(char),
    Row(Vec<String>),
}

/// A longtable's sections: what each of its four boundaries closes, and the
/// body that follows the last of them.
///
/// longtable states its head and its foot BEFORE its body -- `\endhead` closes
/// the head, `\endlastfoot` the foot -- so nothing can be set in the order a
/// reader gets it until the sections are told apart. Keeping them apart is
/// also what lets the head be set AGAIN at the top of the next page, which is
/// the whole of what a longtable is for.
#[derive(Default)]
struct Sections {
    /// What `\endfirsthead` closes: set at the top of the FIRST page only.
    first_head: Vec<Entry>,
    /// What `\endhead` closes: set again at the top of every page the table
    /// runs onto.
    head: Vec<Entry>,
    /// What `\endfoot` closes: set at the bottom of every page the table runs
    /// PAST, and so on no page at all when it fits on one.
    foot: Vec<Entry>,
    /// What `\endlastfoot` closes: set once, under the end of the table.
    last_foot: Vec<Entry>,
    /// Everything after the last boundary: the rows. A `tabular` names no
    /// boundary and is all body, which leaves its entries in the order it
    /// wrote them.
    body: Vec<Entry>,
}

impl Sections {
    /// The head the FIRST page gets: the first head where one is given, and
    /// the repeating head otherwise -- longtable.sty defaults `\endfirsthead`
    /// to `\endhead`.
    fn opening_head(&self) -> &[Entry] {
        match self.first_head.is_empty() {
            true => &self.head,
            false => &self.first_head,
        }
    }

    /// The foot under the END of the table: the last foot where one is given,
    /// and the repeating foot otherwise -- `\endlastfoot` defaults to
    /// `\endfoot` the same way.
    fn closing_foot(&self) -> &[Entry] {
        match self.last_foot.is_empty() {
            true => &self.foot,
            false => &self.last_foot,
        }
    }
}

/// Split a table paragraph into its sections, each holding its rules and rows
/// in the order they were written.
///
/// A boundary closes the material written since the last one: whatever stands
/// before `\endhead` IS the head. Reading it that way, rather than switching a
/// destination on each boundary, is what tells `\endfoot` from `\endlastfoot`;
/// before this the two shared one code and a table writing both put its last
/// foot into its body.
///
/// A row whose cells are all blank is dropped: the newline between the last
/// `\\` and `\end{longtable}` is one, and a blank line in the middle of a
/// table is not something the document asked for.
fn table_sections(para: &str) -> Sections {
    let mut sections = Sections::default();
    // The entries written since the last boundary. Which section they belong
    // to is decided by the boundary that closes them, which has not been read
    // yet -- so they wait here until one is.
    let mut pending: Vec<Entry> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    // A cell is measured and wrapped as one line, so the line ends the source
    // wrote inside it are spaces like any other.
    let finish = |cell: &mut String| -> String {
        let taken = std::mem::take(cell);
        taken.split_whitespace().collect::<Vec<&str>>().join(" ")
    };
    let mut chars = para.chars();
    while let Some(ch) = chars.next() {
        match ch {
            TABLE_MARK => {
                let code = chars.next().unwrap_or(RULE_MID);
                let section = match code {
                    FIRST_HEAD_END => &mut sections.first_head,
                    HEAD_END => &mut sections.head,
                    FOOT_END => &mut sections.foot,
                    LAST_FOOT_END => &mut sections.last_foot,
                    rule => {
                        pending.push(Entry::Rule(rule));
                        continue;
                    }
                };
                // A boundary written twice keeps what the first one closed:
                // taking `pending` again would empty the section rather than
                // add to it.
                if section.is_empty() {
                    *section = std::mem::take(&mut pending);
                }
            }
            TABLE_CELL => row.push(finish(&mut cell)),
            TABLE_ROW => {
                row.push(finish(&mut cell));
                if row.iter().any(|c| !c.is_empty()) {
                    pending.push(Entry::Row(std::mem::take(&mut row)));
                }
                row.clear();
            }
            c => cell.push(c),
        }
    }
    sections.body = pending;
    sections
}

/// The entries of a table in the order they are SET, for a path with no page
/// to repeat anything on.
///
/// That is not the order longtable WRITES them: the bottom rule is written
/// before the first row of data, and setting the stream in arrival order would
/// draw it under the head. Every markdown table pandoc emits has this shape.
fn table_entries(para: &str) -> Vec<Entry> {
    let sections = table_sections(para);
    sections
        .opening_head()
        .iter()
        .chain(&sections.body)
        .chain(sections.closing_foot())
        .cloned()
        .collect()
}

/// Set a table: each row on its own line, each column as wide as its content,
/// and each booktabs rule as a line the page draws a rule for.
///
/// Before this a table was one paragraph of prose -- "Name Value alpha 1 beta 2",
/// running into the sentence after it -- because `&` was lowered to a space and
/// `\\` to a newline. The corpus leans on tables heavily: pandoc emits a
/// longtable for every markdown table, 132 of them in groovyrs/docs/book.tex.
///
/// The ambient colour and face stacks are READ and not advanced. Every line
/// this produces balances its own markers (see `wrap_cell`), so the stacks
/// `to_pdf` walks down the page are exactly where the table found them.
fn table_lines(
    para: &str,
    layout: &Layout,
    colours: &[Spec],
    faces: &[Face],
    sizes: &[TypeSize],
    width_of: &dyn Fn(&str, Face, f64) -> f64,
    out: &mut Vec<String>,
) {
    let sections = table_sections(para);
    // Every section is measured against every other: the head repeated at the
    // top of page four stands over the rows set under it, so how wide its
    // cells are is part of how wide the columns are.
    let entries: Vec<&Entry> = sections
        .first_head
        .iter()
        .chain(&sections.head)
        .chain(&sections.foot)
        .chain(&sections.last_foot)
        .chain(&sections.body)
        .collect();
    let cols = entries
        .iter()
        .filter_map(|e| match e {
            Entry::Row(cells) => Some(cells.len()),
            Entry::Rule(_) => None,
        })
        .max()
        .unwrap_or(0);
    if cols == 0 {
        return;
    }
    // Padding is written in spaces, so the space is the unit every width here
    // is counted in. booktabs leaves a `\tabcolsep` either side of a column;
    // two spaces is that gap in this unit.
    let space = width_of(" ", current_face(faces), current_size(sizes).size).max(f64::MIN_POSITIVE);
    let gutter = 2.0 * space;
    // What a cell costs, in the faces its own markers select, measured on
    // copies of the stacks so measuring never moves them.
    let measure = |text: &str| -> f64 {
        let (mut c, mut f, mut s) = (colours.to_vec(), faces.to_vec(), sizes.to_vec());
        styled_runs(text, &mut c, &mut f, &mut s)
            .iter()
            .map(|(plain, _, face, ts)| width_of(plain, *face, ts.size))
            .sum()
    };

    // Each column asks for what its widest cell needs.
    let mut natural = vec![0.0f64; cols];
    for entry in &entries {
        if let Entry::Row(cells) = *entry {
            for (j, cell) in cells.iter().enumerate() {
                natural[j] = natural[j].max(measure(cell));
            }
        }
    }
    // If they do not all fit, the columns that are under their fair share keep
    // what they asked for and the ones over it share what is left, in
    // proportion to what they asked for. That is what makes a table of one
    // long prose column and two short ones readable rather than three equal
    // columns of wrapped fragments.
    let room = (layout.measure - gutter * (cols - 1) as f64).max(space);
    let asked: f64 = natural.iter().sum();
    let fair = room / cols as f64;
    let over: f64 = natural.iter().filter(|w| **w > fair).sum();
    let under: f64 = natural.iter().filter(|w| **w <= fair).sum();
    let widths: Vec<f64> = match asked <= room || over <= 0.0 {
        true => natural.clone(),
        false => natural
            .iter()
            .map(|w| match *w > fair {
                // Never below four spaces: a column narrower than a short word
                // wraps every cell to one letter a line.
                true => ((room - under) * w / over).max(4.0 * space),
                false => *w,
            })
            .collect(),
    };
    // Where each column starts, and how far the rules run.
    let mut starts = vec![0.0f64; cols];
    for j in 1..cols {
        starts[j] = starts[j - 1] + widths[j - 1] + gutter;
    }
    let span = starts[cols - 1] + widths[cols - 1];

    // One entry, set: a rule is the line the page draws a rule from, a row is
    // its cells wrapped to their columns -- which is one line, or several when
    // a cell wrapped.
    let set = |entry: &Entry| -> Vec<String> {
        let cells = match entry {
            Entry::Rule(kind) => return vec![rule_line(*kind, span, space)],
            Entry::Row(cells) => cells,
        };
        let wrapped: Vec<Vec<String>> = cells
            .iter()
            .enumerate()
            .map(|(j, cell)| wrap_cell(cell, widths[j], colours, faces, sizes, width_of))
            .collect();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(0);
        let mut lines = Vec::with_capacity(height);
        for k in 0..height {
            let mut line = String::new();
            let mut at = 0.0f64;
            for (j, fragments) in wrapped.iter().enumerate() {
                let Some(fragment) = fragments.get(k).filter(|f| !f.is_empty()) else {
                    continue;
                };
                // Pad to the column, in the unit the padding is written in.
                // The nearest whole space rather than the last one that fits,
                // so a column stands within HALF a space of where it was
                // measured to be and not within a whole one; and never fewer
                // than one space, so two cells cannot touch even where the
                // first overran the width it was given.
                let want = ((starts[j] - at) / space).round() as i64;
                for _ in 0..want.max(i64::from(j > 0)) {
                    line.push(' ');
                    at += space;
                }
                line.push_str(fragment);
                at += measure(fragment);
            }
            lines.push(line);
        }
        lines
    };

    // A section, with every line saying which part of the table it is. That is
    // what `paginate` reads to repeat the head and to hold the foot back.
    //
    // `rows` says whether the section's entries are separate things: a body
    // row is one, so the lines a cell wrapped onto are marked as continuing it
    // and no page break can fall between them. A head or a foot is set whole
    // or not at all, so every one of its lines carries the section's own code
    // and the whole block is one thing.
    let mut emit = |entries: &[Entry], code: char, rows: bool| {
        for entry in entries {
            for (n, line) in set(entry).into_iter().enumerate() {
                let mut marked = String::from(LONGTABLE);
                marked.push(match rows && n > 0 {
                    true => LT_CONT,
                    false => code,
                });
                marked.push_str(&line);
                out.push(marked);
            }
        }
    };
    // The repeating head of a table that gave a different first head, and the
    // repeating foot: neither is set where it stands. They are written into
    // the stream all the same, because the lines `paginate` repeats have to be
    // lines it can point at.
    if !sections.first_head.is_empty() {
        emit(&sections.head, LT_REPEAT, false);
    }
    emit(&sections.foot, LT_FOOT, false);
    // The head of the first page, which is also the head that repeats unless a
    // first head was given -- and says which of the two it is.
    let opening = match sections.first_head.is_empty() {
        true => LT_HEAD,
        false => LT_ROW,
    };
    emit(sections.opening_head(), opening, opening == LT_ROW);
    emit(&sections.body, LT_ROW, true);
    // And the foot under the end of the table. A table that gave no
    // `\endlastfoot` sets its repeating foot here as well, which is what
    // longtable does with it.
    emit(sections.closing_foot(), LT_ROW, true);
}

/// The line a rule is set from: the mark, the code saying which rule, and the
/// spaces that MEASURE how far it runs.
///
/// The span has to reach `to_pdf` somehow, and a line is all that survives
/// pagination. Spaces carry it because the page already measures text and so
/// needs nothing new to read this. They are added by the breaker and never
/// reach `--text`, which sees the mark and its code in the text stream and
/// strips both.
fn rule_line(kind: char, span: f64, space: f64) -> String {
    let mut line = String::from(TABLE_MARK);
    line.push(kind);
    let mut at = 0.0;
    while at + space <= span {
        line.push(' ');
        at += space;
    }
    line
}

/// Break one cell into fragments no wider than its column, each of which
/// balances its own markers.
///
/// This is the defect a previous attempt at tables shipped. A row taller than
/// one line is set by putting the nth fragment of every column on the nth
/// line, while `to_pdf` walks ONE colour stack and ONE face stack down the
/// page. A fragment that opened `\texttt` and left it open therefore handed
/// that face to the cell BESIDE it, because the next thing drawn is the next
/// column's fragment and not the rest of this cell. So a fragment closes what
/// it opened, and the next one writes those markers again at its head; the
/// markers are not measured, so re-opening them costs the column nothing.
fn wrap_cell(
    cell: &str,
    width: f64,
    colours: &[Spec],
    faces: &[Face],
    sizes: &[TypeSize],
    width_of: &dyn Fn(&str, Face, f64) -> f64,
) -> Vec<String> {
    let (mut c, mut f, mut s) = (colours.to_vec(), faces.to_vec(), sizes.to_vec());
    let space = width_of(" ", current_face(&f), current_size(&s).size);
    // What this cell has opened and not closed, as the text that opens it and
    // the character that closes it.
    let mut open: Vec<(String, char)> = Vec::new();
    let mut fragments = Vec::new();
    let mut line = String::new();
    let mut at = 0.0f64;
    for word in cell.split(' ').filter(|w| !w.is_empty()) {
        // Measuring advances the stacks, which is what keeps the word after a
        // `\texttt` measured in the face that `\texttt` left in force.
        let cost: f64 = styled_runs(word, &mut c, &mut f, &mut s)
            .iter()
            .map(|(plain, _, face, ts)| width_of(plain, *face, ts.size))
            .sum();
        let need = match line.is_empty() {
            true => cost,
            false => at + space + cost,
        };
        if !line.is_empty() && need > width {
            for (_, close) in open.iter().rev() {
                line.push(*close);
            }
            fragments.push(std::mem::take(&mut line));
            for (opener, _) in &open {
                line.push_str(opener);
            }
            at = cost;
        } else {
            if !line.is_empty() {
                line.push(' ');
            }
            at = need;
        }
        line.push_str(word);
        absorb(word, &mut open);
    }
    if !line.is_empty() {
        for (_, close) in open.iter().rev() {
            line.push(*close);
        }
        fragments.push(line);
    }
    fragments
}

/// Record which markers a word leaves OPEN, as the text that opens each and the
/// character that closes it.
///
/// Colour and face are separate stacks -- they nest inside each other, and a
/// `\texttt` closing does not close the `\color` under it -- so a close pops
/// the topmost entry of its OWN kind rather than whatever is on top.
fn absorb(word: &str, open: &mut Vec<(String, char)>) {
    fn close(open: &mut Vec<(String, char)>, kind: char) {
        if let Some(at) = open.iter().rposition(|(_, c)| *c == kind) {
            open.remove(at);
        }
    }
    let mut chars = word.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\u{1}' => {
                let mut spec = String::from('\u{1}');
                for c in chars.by_ref() {
                    spec.push(c);
                    if c == '\u{2}' {
                        break;
                    }
                }
                open.push((spec, '\u{3}'));
            }
            '\u{3}' => close(open, '\u{3}'),
            FACE_PUSH => {
                let code = chars.next().unwrap_or_else(|| Face::Main.code());
                open.push((format!("{FACE_PUSH}{code}"), FACE_POP));
            }
            FACE_POP => close(open, FACE_POP),
            _ => {}
        }
    }
}

/// The start of a centred region, and, on a broken line, that the line is
/// centred.
///
/// `\begin{center}` and `\centering` were defined by the prelude to expand to
/// nothing, so "centred line" and "left line" came out as one flowing line and
/// a title page -- which is built entirely out of centred pieces -- collapsed
/// into the prose after it. Centring is carried the way a forced break is,
/// in the text itself: a region marker the breaker consumes, and then one
/// character at the head of each line it produced, which is what survives
/// pagination without a second structure beside the lines saying which of them
/// were centred.
pub const CENTRE: char = '\u{e}';

/// The end of a centred region, back to the margin.
pub const CENTRE_END: char = '\u{f}';

/// That a broken line is FULL, and so is set to the measure rather than at the
/// width its glyphs come to.
///
/// Justification is a property of the line and not of the text, so it cannot be
/// worked out again where the page is drawn: by then the paragraphs have been
/// flattened into one list of lines and nothing says which of them the author
/// ended and which the breaker did. The breaker knows -- it pushed the line
/// because the next word did not fit -- and says so at the head of the line,
/// the way a centred line is marked, which is what survives pagination without
/// a second structure beside the lines.
///
/// A device control character rather than a letter, for the same reason the
/// others are: it is not a character any document writes.
pub const JUSTIFY: char = '\u{13}';

/// A line's worth of vertical space with nothing drawn on it.
///
/// `\chapter` and `\section` set their heading hard against the paragraph
/// after it, so a heading was indistinguishable from the text it introduces
/// and the page held lines that lualatex spends on white space. A vertical tab
/// is what the character means; like the form feed it is split out of the text
/// before words are, because Rust counts it as whitespace too.
pub const VERTICAL_SPACE: char = '\u{10}';

/// The r, g and b a marker carried, kept verbatim as written rather than
/// parsed, because they are handed straight back to PDF's `rg` operator.
type Spec = (String, String, String);

/// A stretch of a line, the colour it is drawn in and the face it is set in.
///
/// Always a colour, never "none": the stack it comes off is seeded with the
/// document's default, so every run says what it is drawn in. The face is the
/// same -- the bottom of its stack is the main face.
type StyledRun = (String, Spec, Face, TypeSize);

/// The colour every LaTeX document starts in, as PDF wants it written.
///
/// A document that wants another says `\color`, which arrives here as a marker
/// and pushes on top of this one.
const DEFAULT_COLOUR: (&str, &str, &str) = ("0", "0", "0");

/// The markers the runtime writes turn colour on and off part way along a line,
/// so a line is not one string in one colour: `let x` may be black and `= 1`
/// blue. Each run is emitted with its own colour and its own position, which is
/// why the caller advances x by the width of what it just drew.
///
/// Colour is a STACK, not one current colour, because the markers nest:
/// `\color{textPrim}` at the top of a book is still in force inside the
/// `\texttt` that pushes neonCyan for one word and pops again. Popping to "no
/// colour" -- what this did -- meant that after the first closing marker
/// everything was drawn in black, on a page `\pagecolor` had painted #05050A:
/// 573,723 of the 715,546 drawn characters of `rubyrs/docs/book.tex`, a whole
/// book black on black. The DVI path has had this right all along, as `color
/// push rgb` / `color pop` at typeset.rs:117-137; this is the same thing.
///
/// The stack belongs to the CALLER and carries from line to line, because a
/// `\color` set once above the whole document is in force on every line under
/// it, not just the one the marker landed on. Its bottom entry is the
/// document's default and is never popped, so an unbalanced closing marker
/// leaves that in force rather than nothing at all.
///
/// The face markers are read in the SAME pass and split the line the same way,
/// because the two nest inside each other: a book's `\texttt` is a mono face
/// wrapped around a `\color`, and two passes would have to agree about where
/// the other one's markers were.
fn styled_runs(
    line: &str,
    stack: &mut Vec<Spec>,
    faces: &mut Vec<Face>,
    sizes: &mut Vec<TypeSize>,
) -> Vec<StyledRun> {
    let mut runs = Vec::new();
    let mut text = String::new();
    let mut chars = line.chars().peekable();
    let default: Spec = (
        DEFAULT_COLOUR.0.to_string(),
        DEFAULT_COLOUR.1.to_string(),
        DEFAULT_COLOUR.2.to_string(),
    );
    let top = |stack: &[Spec]| stack.last().cloned().unwrap_or_else(|| default.clone());
    while let Some(ch) = chars.next() {
        match ch {
            FACE_PUSH => {
                if !text.is_empty() {
                    runs.push((
                        std::mem::take(&mut text),
                        top(stack),
                        current_face(faces),
                        current_size(sizes),
                    ));
                }
                // The code character belongs to the marker whatever it is: a
                // marker read as one character would set the other as a glyph.
                let code = chars.next().unwrap_or_else(|| Face::Main.code());
                faces.push(Face::from_code(code));
            }
            FACE_POP => {
                if !text.is_empty() {
                    runs.push((
                        std::mem::take(&mut text),
                        top(stack),
                        current_face(faces),
                        current_size(sizes),
                    ));
                }
                // The bottom entry is the main face and is never popped, so an
                // unbalanced close leaves the document in its own face.
                if faces.len() > 1 {
                    faces.pop();
                }
            }
            SIZE_PUSH => {
                if !text.is_empty() {
                    runs.push((
                        std::mem::take(&mut text),
                        top(stack),
                        current_face(faces),
                        current_size(sizes),
                    ));
                }
                // The spec runs to the marker that closes it, which is the
                // same character: read to it whatever is between, so a
                // damaged spec costs its own heading and not the rest of the
                // document.
                let mut spec = String::new();
                for c in chars.by_ref() {
                    if c == SIZE_PUSH {
                        break;
                    }
                    spec.push(c);
                }
                // A spec that cannot be read still pushes, because the
                // `SIZE_POP` that closes it is coming either way and has to
                // pop what this pushed rather than the entry underneath.
                let pushed = match size_spec(&spec) {
                    Some((size, leading)) => TypeSize { size, leading },
                    None => current_size(sizes),
                };
                sizes.push(pushed);
            }
            SIZE_POP => {
                if !text.is_empty() {
                    runs.push((
                        std::mem::take(&mut text),
                        top(stack),
                        current_face(faces),
                        current_size(sizes),
                    ));
                }
                // The bottom entry is the document's own size and is never
                // popped: an unbalanced close leaves the body size in force
                // rather than nothing at all.
                if sizes.len() > 1 {
                    sizes.pop();
                }
            }
            '\u{1}' => {
                if !text.is_empty() {
                    runs.push((
                        std::mem::take(&mut text),
                        top(stack),
                        current_face(faces),
                        current_size(sizes),
                    ));
                }
                let mut spec = String::new();
                for c in chars.by_ref() {
                    if c == '\u{2}' {
                        break;
                    }
                    spec.push(c);
                }
                // A formula's marker has the same shape and carries its
                // SETTING rather than a colour: where every glyph of it goes,
                // decided by `mlist_to_hlist`. The text between the marker and
                // its close is the formula spelled out for a reader, which the
                // page does not draw -- it draws the setting -- so it is
                // skipped here along with the `\u{3}` that ends it.
                if crate::math::is_setting(&spec) {
                    for c in chars.by_ref() {
                        if c == '\u{3}' {
                            break;
                        }
                    }
                    runs.push((
                        crate::math::run(&spec),
                        top(stack),
                        current_face(faces),
                        current_size(sizes),
                    ));
                    continue;
                }
                let p: Vec<&str> = spec.split(',').collect();
                // A spec that is not three components still pushes -- the
                // colour it inherits -- because the `\u{3}` that closes it is
                // coming either way and has to pop what this pushed, not the
                // entry underneath.
                let pushed = match p.len() == 3 {
                    true => (p[0].to_string(), p[1].to_string(), p[2].to_string()),
                    false => top(stack),
                };
                stack.push(pushed);
            }
            '\u{3}' => {
                if !text.is_empty() {
                    runs.push((
                        std::mem::take(&mut text),
                        top(stack),
                        current_face(faces),
                        current_size(sizes),
                    ));
                }
                if stack.len() > 1 {
                    stack.pop();
                }
            }
            c => text.push(c),
        }
    }
    if !text.is_empty() {
        runs.push((text, top(stack), current_face(faces), current_size(sizes)));
    }
    runs
}

/// The face in force, which is the main one when nothing has pushed.
fn current_face(faces: &[Face]) -> Face {
    faces.last().copied().unwrap_or_default()
}

/// What one word costs, in the faces it is set in.
///
/// A word can carry markers -- `\texttt{x}` is a marker, a letter and a marker
/// -- so it is not necessarily one width in one face. The plain case is the
/// one that runs a million times a book, so it is answered without splitting
/// anything.
fn word_width(
    word: &str,
    colours: &mut Vec<Spec>,
    faces: &mut Vec<Face>,
    sizes: &mut Vec<TypeSize>,
    width_of: &dyn Fn(&str, Face, f64) -> f64,
) -> f64 {
    if !word.contains(['\u{1}', '\u{3}', FACE_PUSH, FACE_POP]) {
        return width_of(word, current_face(faces), current_size(sizes).size);
    }
    styled_runs(word, colours, faces, sizes)
        .iter()
        .map(|(text, _, face, ts)| width_of(text, *face, ts.size))
        .sum()
}

/// Find the file for a font family the document named.
///
/// `\setmainfont{Arimo}` names a FAMILY, not a path, and resolving one is what
/// fontconfig exists for. `fc-match` is asked first because it answers the way
/// every other program on the machine would; the directory walk is the fallback
/// for a machine without it. A family nothing matches returns `None` and the
/// caller falls back to naming one of the fourteen, which gets the widths right
/// and the shapes wrong -- better than refusing to typeset.
pub fn find_family(family: &str) -> Option<std::path::PathBuf> {
    // Memoised for the same reason `find_font` is: this spawns fc-match and,
    // when that answers with something else, reads every font file in the
    // system directories to check its name. Neither answer changes during a
    // run.
    static SEEN: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, Option<std::path::PathBuf>>>,
    > = std::sync::OnceLock::new();
    let seen = SEEN.get_or_init(Default::default);
    if let Some(hit) = seen.lock().ok().and_then(|m| m.get(family).cloned()) {
        return hit;
    }
    let found = find_family_uncached(family);
    if let Ok(mut m) = seen.lock() {
        m.insert(family.to_string(), found.clone());
    }
    found
}

fn find_family_uncached(family: &str) -> Option<std::path::PathBuf> {
    if let Ok(out) = std::process::Command::new("fc-match")
        .args(["-f", "%{file}", family])
        .output()
    {
        if out.status.success() {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let path = std::path::Path::new(&p);
            // fc-match ALWAYS answers, with a default when it has no match, so
            // the answer is only useful if it looks like the family asked for.
            if path.exists() && file_matches(path, family) {
                return Some(path.to_path_buf());
            }
        }
    }
    let wanted = normalise(family);
    for dir in [
        "/System/Library/Fonts",
        "/System/Library/Fonts/Supplemental",
        "/Library/Fonts",
        "/usr/share/fonts",
        "/usr/local/share/fonts",
    ] {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext != "ttf" && ext != "otf" {
                continue;
            }
            if normalise(&path.file_stem()?.to_string_lossy()).starts_with(&wanted) {
                return Some(path);
            }
        }
    }
    None
}

/// A font file's stem, compared the way a family name should be: case and
/// spacing are not part of the identity.
fn file_matches(path: &std::path::Path, family: &str) -> bool {
    path.file_stem()
        .map(|s| normalise(&s.to_string_lossy()).starts_with(&normalise(family)))
        .unwrap_or(false)
}

fn normalise(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Load a family as an embeddable PDF font: its outlines, and its own widths.
///
/// This is the difference between a page set in Arimo's METRICS and one set in
/// Arimo. The widths come from the font's `hmtx` through its `cmap`, so a line
/// is measured in the face it will be printed in rather than in Computer
/// Modern's.
pub fn embed_family(family: &str) -> Option<crate::pdf::Font> {
    embed_file(&find_family(family)?)
}

/// Embed the font FILE a document supplied itself.
///
/// fontspec `Path=` names a directory of `.ttf` files that ship WITH the
/// document, which is how a book uses a face nobody has installed. Looking
/// such a name up among the installed families finds nothing, and fc-match
/// answers with a default -- so the book was set in whatever that default was.
pub fn embed_file(path: &std::path::Path) -> Option<crate::pdf::Font> {
    let bytes = std::fs::read(path).ok()?;
    let sfnt = crate::sfnt::Sfnt::parse(bytes.clone()).ok()?;
    // A CFF-flavoured OpenType carries PostScript outlines, which a
    // /FontFile2 must not: that entry means a TrueType font program.
    if sfnt.is_cff() {
        return None;
    }
    let head = sfnt.head().ok()?;
    let hhea = sfnt.hhea().ok()?;
    let cmap = sfnt.cmap().ok()?;
    let advances = sfnt.advance_widths().ok()?;
    let upem = f64::from(head.units_per_em.max(1));
    let scale = |v: f64| (v * 1000.0 / upem).round() as i64;

    // PDF wants a width for every code in the range, in 1/1000 em, so a code
    // the font has no glyph for still needs an entry.
    //
    // A code is a WinAnsi code -- that is the encoding the font is written with
    // -- and a `cmap` is indexed by Unicode, so the two are joined by what the
    // code MEANS. They agree from 0xA0 up, where WinAnsi is Latin-1, and part
    // company between 0x80 and 0x9F: reading code 0x97 as U+0097 finds nothing
    // and writes `.notdef`'s width where the em dash's belongs.
    let mut widths = Vec::with_capacity(224);
    for code in 32u32..=255 {
        let character = u8::try_from(code).ok().and_then(winansi_unicode);
        let gid = character
            .and_then(|c| cmap.get(&(c as u32)))
            .copied()
            .unwrap_or(0) as usize;
        let adv = advances
            .get(gid)
            .or_else(|| advances.last())
            .copied()
            .unwrap_or(0);
        widths.push(scale(f64::from(adv)));
    }
    Some(crate::pdf::Font::TrueType {
        name: path.file_stem()?.to_string_lossy().replace(' ', ""),
        bytes,
        widths,
        bbox: [
            scale(f64::from(head.x_min)),
            scale(f64::from(head.y_min)),
            scale(f64::from(head.x_max)),
            scale(f64::from(head.y_max)),
        ],
        ascent: scale(f64::from(hhea.ascender)),
        descent: scale(f64::from(hhea.descender)),
    })
}

#[cfg(test)]
mod font_file_tests {
    use super::FontFile;

    /// The options a real book writes, verbatim.
    const REAL: &str = "\n    Path=/private/tmp/build/scifi2/docs/.fonts/,\n    Extension=.ttf,\n    RawFeature={fallback=symfb},\n    UprightFont=Arimo-VF,\n    UprightFeatures={RawFeature={axis={wght=400}}},\n    BoldFont=Arimo-VF,\n    BoldFeatures={RawFeature={axis={wght=700}}},\n";

    #[test]
    fn the_options_a_document_ships_its_font_with() {
        let spec = FontFile::parse(REAL);
        assert_eq!(spec.upright.as_deref(), Some("Arimo-VF"));
        assert_eq!(spec.extension.as_deref(), Some(".ttf"));
        assert_eq!(
            spec.path.as_deref(),
            Some("/private/tmp/build/scifi2/docs/.fonts/")
        );
    }

    #[test]
    fn the_face_files_are_read_from_the_same_options() {
        // `BoldFont=` and `ItalicFont=` sit in the option list `UprightFont=`
        // does, and reading past them is why `\textbf` and `\emph` were set in
        // the upright face. The corpus's bold IS the upright file at another
        // weight, which is honest to report as such: what is asserted is that
        // the key was READ, not that the file differs.
        let spec = FontFile::parse(REAL);
        assert_eq!(spec.bold.as_deref(), Some("Arimo-VF"));
        assert_eq!(spec.italic.as_deref(), None, "this book names no italic");
        let both = FontFile::parse("Extension=.ttf,UprightFont=A,ItalicFont=A-Italic");
        assert_eq!(both.italic.as_deref(), Some("A-Italic"));
        // A face the document named no file for resolves to nothing, whatever
        // is beside the document, so the caller falls back to the main face
        // rather than setting the italic in some file that happens to be there.
        assert_eq!(both.resolve_face(super::Face::Bold, None), None);
    }

    #[test]
    fn a_value_holding_commas_does_not_tear_the_keys_after_it() {
        // `UprightFeatures={RawFeature={axis={wght=400}}}` sits BEFORE the keys
        // that matter in some documents. Splitting on every comma leaves
        // `wght=400}}}` as a key and loses everything past it.
        let spec = FontFile::parse("Extension=.ttf,Numbers={Proportional,Lining},UprightFont=X-VF");
        assert_eq!(spec.upright.as_deref(), Some("X-VF"));
        assert_eq!(spec.extension.as_deref(), Some(".ttf"));
    }

    #[test]
    fn a_stale_path_is_retried_beside_the_document() {
        // `Path=` is written when the document is BUILT and regularly points
        // into a scratch directory that is gone by the time it is read. The
        // fonts ship with the document, so the last component is tried there.
        let dir = std::env::temp_dir().join(format!("texrs_ff_{}", std::process::id()));
        let fonts = dir.join(".fonts");
        std::fs::create_dir_all(&fonts).expect("mkdir");
        let file = fonts.join("Arimo-VF.ttf");
        std::fs::write(&file, b"not really a font").expect("write");

        let spec =
            FontFile::parse("Path=/gone/for/good/.fonts/,Extension=.ttf,UprightFont=Arimo-VF");
        assert_eq!(spec.resolve(Some(&dir)), Some(file.clone()));
        // With nowhere to look, it resolves to nothing rather than to a guess.
        assert_eq!(spec.resolve(None), None);

        // A path that IS there wins outright.
        let direct = format!(
            "Path={}/,Extension=.ttf,UprightFont=Arimo-VF",
            fonts.display()
        );
        assert_eq!(FontFile::parse(&direct).resolve(None), Some(file));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_mandatory_argument_ending_in_a_font_extension_names_the_file_itself() {
        // `\setmainfont{Arimo-VF.ttf}[Path=...]` is the spelling lualatex
        // honours and this read as a family name, which named nothing
        // installed: `upright` stayed empty, `resolve_face` returned at its
        // first `?`, and no path was ever built.
        let mut spec = FontFile::parse("Path=/somewhere/.fonts/");
        spec.absorb_filename("Arimo-VF.ttf");
        assert_eq!(spec.upright.as_deref(), Some("Arimo-VF"));
        assert_eq!(spec.extension.as_deref(), Some(".ttf"));
        assert_eq!(spec.path.as_deref(), Some("/somewhere/.fonts/"));
        // The extension is kept as the document wrote it, since that is the
        // name to open on a filesystem that tells the cases apart.
        let mut upper = FontFile::default();
        upper.absorb_filename("Arimo-VF.TTF");
        assert_eq!(upper.upright.as_deref(), Some("Arimo-VF"));
        assert_eq!(upper.extension.as_deref(), Some(".TTF"));
        for spelt in ["A.otf", "A.OTF", "A.ttc", "A.otc"] {
            let mut one = FontFile::default();
            one.absorb_filename(spelt);
            assert_eq!(one.upright.as_deref(), Some("A"), "{spelt} is a filename");
        }
    }

    #[test]
    fn a_family_name_is_not_read_as_a_file_and_explicit_options_win() {
        // The other spelling must be untouched: `\setmainfont{Arimo}` names a
        // family, and a family with a dot in it -- `Dr. Sugiyama` is one --
        // is still not a filename.
        for family in ["Arimo", "NoSuchFontExistsAnywhere", "Dr. Sugiyama", ".ttf"] {
            assert_eq!(super::split_font_filename(family), None, "{family}");
            assert_eq!(super::font_family_name(family), family);
            let mut spec = FontFile::default();
            spec.absorb_filename(family);
            assert_eq!(spec.upright, None, "{family} named no file");
            assert_eq!(spec.extension, None, "{family} named no extension");
        }
        // A document that writes both said the keys last and meant them: the
        // filename fills what the options left empty and overrides nothing.
        let mut both = FontFile::parse("Path=/f/,Extension=.otf,UprightFont=Arimo-Regular");
        both.absorb_filename("Arimo-VF.ttf");
        assert_eq!(both.upright.as_deref(), Some("Arimo-Regular"));
        assert_eq!(both.extension.as_deref(), Some(".otf"));
        assert_eq!(super::font_family_name("Arimo-VF.ttf"), "Arimo-VF");
    }

    #[test]
    fn the_filename_spelling_resolves_to_the_file_beside_the_document() {
        // The whole point: a path is built, and it is the file the document
        // ships. A file that is NOT there resolves to nothing so the caller
        // still falls back rather than failing the run.
        let dir = std::env::temp_dir().join(format!("texrs_ffn_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let file = dir.join("Arimo-VF.ttf");
        std::fs::write(&file, b"not really a font").expect("write");

        let mut spec = FontFile::parse(&format!("Path={}/", dir.display()));
        spec.absorb_filename("Arimo-VF.ttf");
        assert_eq!(spec.resolve(None), Some(file));

        let mut missing = FontFile::parse(&format!("Path={}/", dir.display()));
        missing.absorb_filename("NotThere-VF.ttf");
        assert_eq!(missing.resolve(None), None, "a missing file falls back");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn options_that_name_no_file_resolve_to_nothing() {
        // `\setmainfont{Georgia}` with no options at all must fall through to
        // the installed families rather than resolving to some file nearby.
        assert_eq!(
            FontFile::parse("").resolve(Some(std::path::Path::new("/tmp"))),
            None
        );
    }
}
