//! The smallest honest stomach: text and font metrics into DVI pages.
//!
//! texrs could read a document and say what its words were, and could read and
//! write the DVI format, and had nothing joining the two -- `dvi::Writer` was
//! never called outside its own tests. So a book "ran" and produced no page.
//! This is the join: measure each character in a real font, break the text into
//! lines at a measure, stack the lines down a page, and ship the result.
//!
//! What this is NOT is `tex.web`'s stomach. TeX breaks a paragraph by looking at
//! every feasible sequence of breakpoints and minimising total badness
//! (§813-§890); this takes the first break that fits, which is what every
//! word processor before TeX did and what TeX was written to improve on. There
//! is no hyphenation, no glue stretching or shrinking, no page-breaking by
//! penalties, no maths, no boxes a document can nest. A paragraph set here and
//! the same paragraph set by tex will not agree line for line.
//!
//! It is a page you can open, which is the difference between a document that
//! produces nothing and one that produces something imperfect.

use crate::dvi::Writer;
use crate::tfm::Tfm;

/// DVI's unit: 1 sp, of which there are 65536 to the printer's point.
const SP: f64 = 65536.0;

/// The page and paragraph shape, in points.
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

/// Set `text` in `font`, and return a DVI file.
///
/// The font is named in the DVI by `font_name` so a driver can find it; the
/// metrics come from the `.tfm` the caller opened, which is the same file the
/// driver will use to place the characters.
pub fn to_dvi(text: &str, font: &Tfm, font_name: &str, layout: &Layout) -> Vec<u8> {
    let mut w = Writer::new("texrs");
    // The design size and checksum have to match the .tfm or a driver refuses
    // the file: they are how it checks it has the font the document was set in.
    let at = (layout.size * SP) as i32;
    w.define_font(0, font_name, at, font.checksum, at);

    let lines = break_lines(text, font, layout);
    let per_page = ((layout.height / layout.leading).floor() as usize).max(1);

    for (page, chunk) in lines.chunks(per_page).enumerate() {
        let counts = [page as i32 + 1, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        w.begin_page(counts);
        w.font(0);
        let mut baseline = layout.margin + layout.leading;
        for line in chunk {
            // Each line starts at the left margin, on its own baseline. DVI is
            // relative, so the position is pushed and popped rather than
            // tracked: a driver reading it never has to know where the last
            // line ended.
            w.push();
            w.down((baseline * SP) as i32);
            w.right((layout.margin * SP) as i32);
            set_line(&mut w, line, font, layout);
            w.pop();
            baseline += layout.leading;
        }
        w.end_page();
    }
    w.finish()
}

/// Put one line's characters down, with the font's own kerns between them.
fn set_line(w: &mut Writer, line: &str, font: &Tfm, layout: &Layout) {
    let bytes: Vec<u8> = line.bytes().collect();
    for (i, b) in bytes.iter().enumerate() {
        let Some(m) = font.char(*b) else {
            // A character the font does not have cannot be set. Skipping it
            // silently would shorten the line without saying so; a space keeps
            // the measure honest and is visible as a gap.
            if let Some(sp) = font.char(b' ') {
                w.right((sp.width * layout.size * SP) as i32);
            }
            continue;
        };
        w.set_char(u32::from(*b), (m.width * layout.size * SP) as i32);
        // A kern between this character and the next is part of what the font
        // says the pair looks like; dropping it is why naive output looks
        // loose next to tex's.
        if let Some(next) = bytes.get(i + 1) {
            if let Some(crate::tfm::Step::Kern { by, .. }) = font.step(*b, *next) {
                w.right((by * layout.size * SP) as i32);
            }
        }
    }
}

/// Break `text` into lines that fit the measure.
///
/// First fit, not best fit: a word is added while it still fits and starts a new
/// line when it does not. `tex.web` §813 does far better -- it considers every
/// feasible set of breakpoints for the whole paragraph at once -- and the
/// difference is visible as a raggeder right edge here.
pub fn break_lines(text: &str, font: &Tfm, layout: &Layout) -> Vec<String> {
    use rayon::prelude::*;
    // Paragraph-parallel. A paragraph is broken independently of its
    // neighbours: the measure is the same for all of them, and no state crosses
    // the blank line between two, so this fans out with nothing to
    // synchronise. `map` keeps them in order, which matters -- a book whose
    // paragraphs arrived in completion order would be a different book.
    let paras: Vec<&str> = text.split("\n\n").collect();
    paras
        .par_iter()
        .map(|para| break_paragraph(para, font, layout))
        .reduce(Vec::new, |mut acc, mut more| {
            acc.append(&mut more);
            acc
        })
}

/// One paragraph, first-fit.
fn break_paragraph(para: &str, font: &Tfm, layout: &Layout) -> Vec<String> {
    let space = font.char(b' ').map(|m| m.width).unwrap_or(0.33) * layout.size;
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut width = 0.0f64;
    for word in para.split_whitespace() {
        let ww = font.width_of(word) * layout.size;
        let need = match line.is_empty() {
            true => ww,
            false => width + space + ww,
        };
        if !line.is_empty() && need > layout.measure {
            lines.push(std::mem::take(&mut line));
            width = ww;
            line.push_str(word);
            continue;
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
        width = need;
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Find a `.tfm` by name, asking the TeX installation first.
///
/// `kpsewhich` is how every TeX program answers this question, so asking it
/// gets the same answer the driver will get when it goes looking for the same
/// font. The fixed paths are the fallback for a machine without it.
pub fn find_font(name: &str) -> Option<std::path::PathBuf> {
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
