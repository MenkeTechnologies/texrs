//! Breaking a paragraph the way `tex.web` §813 does: by minimising the total
//! demerits of the WHOLE paragraph, with Liang hyphenation to widen the set of
//! places a line may end.
//!
//! First fit -- fill until the next word will not go on -- decides each line
//! without looking at the one after it, so one long word early in a paragraph
//! leaves a gap that every later line pays for. TeX considers every feasible
//! set of breakpoints at once, prices each line by how far its glue is from its
//! natural width (§108's `badness`), and picks the cheapest set (§859's
//! `demerits`). Hyphenation (§891) is the second pass: when no set of
//! breakpoints between words is good enough, words themselves may be split.
//!
//! WHY THIS CAN BE USED AT ALL. A breaker that prices a line over glue decides
//! that some lines should be SHRUNK, and an earlier attempt at this was
//! reverted because the renderer could only draw a run at its natural width, so
//! every shrunk line was drawn out past the measure. `pdf::Page::text_set` now
//! sets a run to a width with PDF's `Tw`, and `typeset::to_pdf` sets every full
//! line to the measure, so the answer this module returns is one the page can
//! honour. Nothing here is used on the DVI path, whose driver still cannot.
//!
//! WHAT IS NOT TeX. `\tolerance`, `\pretolerance` and the demerit weights are
//! read from nothing -- they are the constants below, which are plain TeX's and
//! LaTeX's defaults, because no document in the corpus sets them and the
//! engine does not yet resolve those registers. The interword glue stretches
//! and shrinks by cmr10's own fractions of the space rather than by each
//! embedded face's `\fontdimen3` and `\fontdimen4`, which a PDF font file does
//! not state. And TeX's final pass drops the demerits of a break it has no
//! alternative to (§855 `artificial_demerits`), which it can do because it
//! knows its active list is about to empty; a dynamic program has no such
//! moment, so an overfull line is priced above every paragraph of merely
//! dreadful ones instead (`OVERFULL_DEMERITS`). The outcome is TeX's: an
//! overfull line only where there is no other, and an underfull one preferred
//! to it, which is why tex reports `Underfull \hbox` far more often than
//! `Overfull`.

use std::collections::HashMap;
use std::sync::OnceLock;

/// What may happen after one piece of a paragraph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum After {
    /// An interword space of this width. Breaking here is free and the space
    /// is discarded.
    Glue(f64),
    /// A hyphenation point inside a word: breaking here sets a hyphen of this
    /// width at the end of the line and costs `HYPHEN_PENALTY`.
    Discretionary(f64),
    /// A break after a hyphen the AUTHOR wrote. The hyphen is already in the
    /// text, so nothing is added; it costs `EX_HYPHEN_PENALTY` (§869).
    Explicit,
    /// The pieces run together with no break between them and no space.
    Nothing,
}

/// One unbreakable chunk of a paragraph, already measured.
#[derive(Clone, Debug)]
pub struct Piece {
    /// The text as it will be set, markers and all.
    pub text: String,
    /// What that text measures in the face it is set in.
    pub width: f64,
    pub after: After,
}

/// `\linepenalty`: what every line costs before its badness (plain.tex).
const LINE_PENALTY: f64 = 10.0;
/// `\hyphenpenalty`, `\exhyphenpenalty`: what breaking a word costs.
const HYPHEN_PENALTY: f64 = 50.0;
const EX_HYPHEN_PENALTY: f64 = 50.0;
/// `\adjdemerits`: charged when two consecutive lines are more than one
/// fitness class apart -- a tight line under a very loose one.
const ADJ_DEMERITS: f64 = 10000.0;
/// `\doublehyphendemerits`, `\finalhyphendemerits`: charged for two hyphens in
/// a row, and for a hyphen on the second-to-last line.
const DOUBLE_HYPHEN_DEMERITS: f64 = 10000.0;
const FINAL_HYPHEN_DEMERITS: f64 = 5000.0;
/// `inf_bad` (§108): the badness of a line that cannot be set at all.
const INF_BAD: f64 = 10000.0;
/// What an OVERFULL line costs, which is more than any paragraph of merely bad
/// ones: §859 caps a line's demerits at 1e8 and no paragraph runs to ten
/// thousand lines, so this floor cannot be reached by adding ordinary lines up.
const OVERFULL_DEMERITS: f64 = 1e12;
/// `eject_penalty` (§157): the forced break at the end of the paragraph.
const EJECT_PENALTY: f64 = -10000.0;
/// `\pretolerance` and `\tolerance` at LaTeX's defaults: the first pass tries
/// to break with no hyphens at all, and only a paragraph it cannot is offered
/// them.
const PRETOLERANCE: f64 = 100.0;
const TOLERANCE: f64 = 200.0;
/// cmr10's interword glue is `3.33333pt plus 1.66666pt minus 1.11111pt`
/// (`\fontdimen2..4`), so it stretches by half the space and shrinks by a
/// third of it. A PDF font file states no such thing, so these fractions stand
/// in for every face.
const STRETCH_FRACTION: f64 = 0.5;
const SHRINK_FRACTION: f64 = 1.0 / 3.0;
/// Widths are in points and come out of a font's own table; a line that lands
/// on the measure to within this is on it.
const EPS: f64 = 1e-6;

/// TeX's four fitness classes (§817), in its own numbering.
const VERY_LOOSE: usize = 0;
const LOOSE: usize = 1;
const DECENT: usize = 2;
const TIGHT: usize = 3;

/// §108: how bad it is to stretch or shrink `t` points of glue that has `s`
/// points to give.
fn badness(t: f64, s: f64) -> f64 {
    if t <= 0.0 {
        return 0.0;
    }
    if s <= 0.0 {
        return INF_BAD;
    }
    (100.0 * (t / s).powi(3)).round().min(INF_BAD)
}

/// §817: which of the four classes a line of this badness falls in.
fn fitness(bad: f64, stretching: bool) -> usize {
    if bad <= 12.0 {
        return DECENT;
    }
    match stretching {
        false => TIGHT,
        true if bad > 99.0 => VERY_LOOSE,
        true => LOOSE,
    }
}

/// What one candidate line comes to: its badness, whether it is being
/// stretched, and whether it can be set at all.
///
/// `fil` is the last line of the paragraph, which TeX ends with
/// `\parfillskip = 0pt plus 1fil` -- infinitely stretchable, so a short last
/// line is never a bad one.
fn assess(natural: f64, stretch: f64, shrink: f64, measure: f64, fil: bool) -> (f64, bool, bool) {
    let short = measure - natural;
    if short > EPS {
        return match fil {
            true => (0.0, true, true),
            false => (badness(short, stretch), true, true),
        };
    }
    if short < -EPS {
        // Past the measure by more than the glue can give back is an overfull
        // line: TeX's adjustment ratio below -1, which is not a break at all.
        if -short > shrink + EPS {
            return (INF_BAD, false, false);
        }
        return (badness(-short, shrink), false, true);
    }
    (0.0, true, true)
}

/// Where to end each line of `pieces`, as the number of pieces consumed.
///
/// The answer always covers the whole paragraph: the last entry is
/// `pieces.len()`. An empty paragraph gets an empty answer.
///
/// TeX's three passes (§863): break with no hyphens at `\pretolerance`; if no
/// set of breakpoints is that good, offer the hyphens at `\tolerance`; and if
/// that fails too, take the least bad set there is, overfull lines included.
pub fn break_paragraph(pieces: &[Piece], measure: f64) -> Vec<usize> {
    if pieces.is_empty() {
        return Vec::new();
    }
    total_fit(pieces, measure, PRETOLERANCE, false, false)
        .or_else(|| total_fit(pieces, measure, TOLERANCE, true, false))
        .or_else(|| total_fit(pieces, measure, INF_BAD, true, true))
        .unwrap_or_else(|| vec![pieces.len()])
}

/// One pass of §813's algorithm.
///
/// `hyphens` says whether a discretionary counts as a breakpoint this pass;
/// `overfull` whether a line past the measure may be taken when nothing else
/// fits. `None` means no set of breakpoints met `threshold`.
fn total_fit(
    pieces: &[Piece],
    measure: f64,
    threshold: f64,
    hyphens: bool,
    overfull: bool,
) -> Option<Vec<usize>> {
    let n = pieces.len();
    // Prefix sums, so the width of a candidate line is a subtraction rather
    // than a walk: a paragraph of 200 words offers thousands of candidates and
    // each would otherwise re-add the words it holds.
    let mut wide = vec![0.0; n + 1];
    let mut gap = vec![0.0; n + 1];
    let mut give = vec![0.0; n + 1];
    let mut take = vec![0.0; n + 1];
    for (k, piece) in pieces.iter().enumerate() {
        wide[k + 1] = wide[k] + piece.width;
        let space = match piece.after {
            After::Glue(space) => space,
            _ => 0.0,
        };
        gap[k + 1] = gap[k] + space;
        give[k + 1] = give[k] + space * STRETCH_FRACTION;
        take[k + 1] = take[k] + space * SHRINK_FRACTION;
    }
    let breakable = |k: usize| match pieces[k].after {
        After::Glue(_) => true,
        After::Discretionary(_) => hyphens,
        After::Explicit => true,
        After::Nothing => false,
    };

    // best[b][f]: the least demerits of setting pieces 0..b as whole lines,
    // where the line ending at b is of fitness class f. Four classes because
    // the NEXT line's demerits depend on this one's class (§859's
    // `adj_demerits`), so they cannot be collapsed to one number per break.
    let mut best = vec![[f64::INFINITY; 4]; n + 1];
    let mut from = vec![[(0usize, 0usize); 4]; n + 1];
    // §864: the paragraph starts as though the line before it were decent.
    best[0][DECENT] = 0.0;

    for p in 0..n {
        if best[p].iter().all(|d| !d.is_finite()) {
            continue;
        }
        // A break at p is one the previous line ended on; whether it was a
        // hyphen decides `\doublehyphendemerits` below.
        let after_hyphen = p > 0
            && matches!(
                pieces[p - 1].after,
                After::Discretionary(_) | After::Explicit
            );
        for b in p + 1..=n {
            let dash = match pieces[b - 1].after {
                After::Discretionary(dash) => dash,
                _ => 0.0,
            };
            let natural = (wide[b] - wide[p]) + (gap[b - 1] - gap[p]) + dash;
            let stretch = give[b - 1] - give[p];
            let shrink = take[b - 1] - take[p];
            let last = b == n;
            let (bad, stretching, fits) = assess(natural, stretch, shrink, measure, last);
            // The end of the paragraph is a breakpoint whatever is written
            // there; anywhere else the piece has to offer one.
            if last || breakable(b - 1) {
                let take_it = match fits {
                    true => bad <= threshold,
                    // An overfull line is only ever taken in the final pass,
                    // and then at `OVERFULL_DEMERITS` below.
                    false => overfull,
                };
                if take_it {
                    let here = fitness(bad, stretching);
                    // §869: what the breakpoint itself costs.
                    let penalty = match last {
                        true => EJECT_PENALTY,
                        false => match pieces[b - 1].after {
                            After::Discretionary(_) => HYPHEN_PENALTY,
                            After::Explicit => EX_HYPHEN_PENALTY,
                            _ => 0.0,
                        },
                    };
                    // §859: the demerits of this line.
                    let mut cost = LINE_PENALTY + bad;
                    cost = match cost.abs() >= INF_BAD {
                        true => 1e8,
                        false => cost * cost,
                    };
                    // An overfull line is not a break TeX would make at all
                    // (§851 deactivates one), so it has to cost more than any
                    // number of merely dreadful lines -- 1e8 apiece and a
                    // paragraph is a few hundred of them at most. Charging it
                    // the same 1e8 they get is how one 1,100pt line came out
                    // preferred to two ordinary ones, and the four words above
                    // it were drawn a foot into the margin. Past that floor the
                    // overflow itself is priced, so the least bad overfull line
                    // is the one taken.
                    if !fits {
                        let excess = natural - shrink - measure;
                        cost = OVERFULL_DEMERITS + excess * excess;
                    }
                    if penalty > 0.0 {
                        cost += penalty * penalty;
                    } else if penalty > EJECT_PENALTY {
                        cost -= penalty * penalty;
                    }
                    // §873 asks for the final break as a HYPHENATED one, which
                    // is what makes `\finalhyphendemerits` reachable: a hyphen
                    // on the second-to-last line is the one TeX charges for.
                    let hyphen_here = last
                        || matches!(
                            pieces[b - 1].after,
                            After::Discretionary(_) | After::Explicit
                        );
                    if hyphen_here && after_hyphen {
                        cost += match last {
                            true => FINAL_HYPHEN_DEMERITS,
                            false => DOUBLE_HYPHEN_DEMERITS,
                        };
                    }
                    for before in 0..4 {
                        if !best[p][before].is_finite() {
                            continue;
                        }
                        let mut total = best[p][before] + cost;
                        if here.abs_diff(before) > 1 {
                            total += ADJ_DEMERITS;
                        }
                        if total < best[b][here] {
                            best[b][here] = total;
                            from[b][here] = (p, before);
                        }
                    }
                }
            }
            // Past this point every longer line is longer still, so there is
            // nothing left to consider from p.
            if !fits {
                break;
            }
        }
    }

    let end = (0..4).filter(|f| best[n][*f].is_finite()).min_by(|a, b| {
        best[n][*a]
            .partial_cmp(&best[n][*b])
            .unwrap_or(std::cmp::Ordering::Equal)
    })?;
    let mut breaks = Vec::new();
    let (mut at, mut class) = (n, end);
    while at > 0 {
        breaks.push(at);
        let (p, before) = from[at][class];
        at = p;
        class = before;
    }
    breaks.reverse();
    Some(breaks)
}

/// Liang's hyphenation patterns, as `hyphen.tex` states them.
///
/// The patterns are Knuth's own, read from the TeX installation rather than
/// copied: they are the ones the reference PDFs beside the corpus were set
/// with, and a copy here would be a second thing to keep in step. A machine
/// with no TeX installation gets an EMPTY set and no hyphenation, which is
/// what texrs did before this and is still a correct paragraph.
#[derive(Default)]
pub struct Hyphenator {
    /// A pattern's letters to the level between each of them: `.ach4` is
    /// `".ach"` and `[0, 0, 0, 0, 4]`.
    patterns: HashMap<Vec<u8>, Vec<u8>>,
    /// `\hyphenation{as-so-ciate}`: where a named word breaks, whatever the
    /// patterns say.
    exceptions: HashMap<Vec<u8>, Vec<usize>>,
    /// The longest pattern, so a lookup only walks as far as one can reach.
    longest: usize,
}

/// `\lefthyphenmin` and `\righthyphenmin`, at the values LaTeX sets for
/// English: no break with fewer than two letters before it or three after.
const LEFT_MIN: usize = 2;
const RIGHT_MIN: usize = 3;

impl Hyphenator {
    /// The patterns from a `hyphen.tex`, and nothing if it cannot be read.
    pub fn from_source(src: &str) -> Self {
        let mut me = Self::default();
        for pattern in block(src, "\\patterns{") {
            let (key, levels) = split_pattern(&pattern);
            if key.is_empty() {
                continue;
            }
            me.longest = me.longest.max(key.len());
            me.patterns.insert(key, levels);
        }
        for word in block(src, "\\hyphenation{") {
            let letters: Vec<u8> = word
                .bytes()
                .filter(|b| *b != b'-')
                .map(|b| b.to_ascii_lowercase())
                .collect();
            let mut at = Vec::new();
            let mut seen = 0usize;
            for byte in word.bytes() {
                match byte {
                    b'-' => at.push(seen),
                    _ => seen += 1,
                }
            }
            me.exceptions.insert(letters, at);
        }
        me
    }

    /// Whether anything was loaded at all.
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty() && self.exceptions.is_empty()
    }

    /// Where `word` may be broken, as the number of letters left on the line.
    ///
    /// Only a plain ASCII word is offered: a word carrying a typesetting
    /// marker, an accent or a digit is left whole rather than broken somewhere
    /// the patterns were never stated for.
    pub fn points(&self, word: &str) -> Vec<usize> {
        let len = word.len();
        if len < LEFT_MIN + RIGHT_MIN || !word.bytes().all(|b| b.is_ascii_alphabetic()) {
            return Vec::new();
        }
        let lower: Vec<u8> = word.bytes().map(|b| b.to_ascii_lowercase()).collect();
        let inside = |at: &usize| *at >= LEFT_MIN && len - *at >= RIGHT_MIN;
        if let Some(known) = self.exceptions.get(&lower) {
            return known.iter().copied().filter(inside).collect();
        }
        // Liang: the word between two full stops, every substring looked up,
        // and the highest level wins at each position. An odd level is a break.
        let mut edged = Vec::with_capacity(len + 2);
        edged.push(b'.');
        edged.extend_from_slice(&lower);
        edged.push(b'.');
        let mut level = vec![0u8; edged.len() + 1];
        for i in 0..edged.len() {
            let far = (i + self.longest).min(edged.len());
            for j in i + 1..=far {
                let Some(values) = self.patterns.get(&edged[i..j]) else {
                    continue;
                };
                for (k, value) in values.iter().enumerate() {
                    level[i + k] = level[i + k].max(*value);
                }
            }
        }
        // `level[i]` is the gap before `edged[i]`, and `edged` is the word with
        // a full stop welded to each end, so the gap after the word's `at`th
        // letter is `level[at + 1]`.
        (LEFT_MIN..=len.saturating_sub(RIGHT_MIN))
            .filter(|at| level[at + 1] % 2 == 1)
            .collect()
    }
}

/// The words inside one `\patterns{...}` or `\hyphenation{...}` group.
///
/// The group closes on a `}` alone on its line, which is how `hyphen.tex`
/// writes both of them, and `%` starts a comment as it does anywhere in TeX.
fn block(src: &str, opens: &str) -> Vec<String> {
    let Some(at) = src.find(opens) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in src[at + opens.len()..].lines() {
        let line = match line.find('%') {
            Some(comment) => &line[..comment],
            None => line,
        };
        if line.trim() == "}" {
            break;
        }
        out.extend(line.split_whitespace().map(str::to_string));
    }
    out
}

/// A pattern into the letters it matches and the level between each of them.
fn split_pattern(pattern: &str) -> (Vec<u8>, Vec<u8>) {
    let mut key = Vec::new();
    let mut levels = vec![0u8];
    for byte in pattern.bytes() {
        match byte.is_ascii_digit() {
            true => *levels.last_mut().unwrap_or(&mut 0) = byte - b'0',
            false => {
                key.push(byte);
                levels.push(0);
            }
        }
    }
    (key, levels)
}

/// The patterns this process breaks words with, read once.
pub fn hyphenator() -> &'static Hyphenator {
    static PATTERNS: OnceLock<Hyphenator> = OnceLock::new();
    PATTERNS.get_or_init(|| match patterns_source() {
        Some(src) => Hyphenator::from_source(&src),
        None => Hyphenator::default(),
    })
}

/// `hyphen.tex`, asked for the way every TeX program asks for a file.
fn patterns_source() -> Option<String> {
    let out = std::process::Command::new("kpsewhich")
        .arg("hyphen.tex")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if path.is_empty() {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every word breaks where TeX's own `\showhyphens` breaks it.
    ///
    /// The right-hand column is verbatim out of a `pdflatex` run of
    /// `\showhyphens{...}` over these words -- not out of Liang's paper and
    /// not out of this implementation -- so what the test says is "the same as
    /// TeX" rather than "the same as it was yesterday".
    #[test]
    fn liang_breaks_the_words_tex_breaks() {
        let words = hyphenator();
        if words.is_empty() {
            return; // no TeX installation: hyphenation degrades to none
        }
        let showhyphens = [
            "hy-phen-ation",
            "com-puter",
            "al-go-rithm",
            "as-so-ciate", // one of \hyphenation{}'s own exceptions
            "type-set-ting",
            "para-graph",
            "rep-re-sen-ta-tion",
            "im-ple-men-ta-tion",
            "un-der-stand-ing",
            "dif-fer-ence",
            "pro-fes-sional",
            "in-ter-na-tional",
            "de-vel-op-ment",
            "con-sid-er-a-tion",
        ];
        for dashed in showhyphens {
            let word: String = dashed.chars().filter(|c| *c != '-').collect();
            let mut wanted = Vec::new();
            let mut at = 0;
            for c in dashed.chars() {
                match c {
                    '-' => wanted.push(at),
                    _ => at += 1,
                }
            }
            assert_eq!(words.points(&word), wanted, "{dashed}");
        }
        // \lefthyphenmin and \righthyphenmin: neither of these breaks at all.
        assert!(words.points("into").is_empty(), "into");
    }

    /// A machine with no TeX installation gets no patterns, and no patterns
    /// means every word is one piece rather than a panic or a wrong break.
    #[test]
    fn no_patterns_means_no_hyphens() {
        let none = Hyphenator::default();
        assert!(none.is_empty());
        assert!(none.points("hyphenation").is_empty());
    }

    /// The breaker takes the set of breakpoints that costs the whole paragraph
    /// least, and that is not the set a left-to-right fill reaches.
    ///
    /// Seventeen boxes on a 400pt measure with 20pt glue, and every line of
    /// BOTH answers is inside `\tolerance`, so this is a difference of quality
    /// and not one answer failing:
    ///
    ///   first fit  391 / 369 / 385 / 86   badness 1 / 110 / 3 / 0
    ///   total fit  391 / 417 / 390 / 33   badness 1 /  26 / 1 / 0
    ///
    /// First fit stops line two one box early because the next box will not go
    /// on at its natural width, and pays 110 badness for the gap. Total fit
    /// takes that box and shrinks the line onto the measure instead -- 14,790
    /// demerits against 1,638 for the whole paragraph. Shrinking is exactly
    /// what the renderer could not do before `Page::text_set`.
    #[test]
    fn total_fit_is_not_first_fit() {
        let boxes = [
            63.0, 32.0, 52.0, 63.0, 101.0, 81.0, 44.0, 89.0, 95.0, 28.0, 30.0, 40.0, 56.0, 101.0,
            30.0, 33.0, 33.0,
        ];
        let pieces: Vec<Piece> = boxes
            .iter()
            .enumerate()
            .map(|(at, width)| Piece {
                text: format!("w{at}"),
                width: *width,
                after: match at + 1 == boxes.len() {
                    true => After::Nothing,
                    false => After::Glue(20.0),
                },
            })
            .collect();
        let measure = 400.0;

        // First fit, spelled out here so the comparison is not a claim about
        // some other function's behaviour.
        let mut greedy = Vec::new();
        let (mut width, mut count) = (0.0f64, 0usize);
        for (at, piece) in pieces.iter().enumerate() {
            let need = match count {
                0 => piece.width,
                _ => width + 20.0 + piece.width,
            };
            if count > 0 && need > measure {
                greedy.push(at);
                width = piece.width;
                count = 1;
                continue;
            }
            width = need;
            count += 1;
        }
        greedy.push(pieces.len());
        assert_eq!(greedy, vec![5, 9, 15, 17], "first fit stops at the measure");

        let total = break_paragraph(&pieces, measure);
        assert_eq!(total, vec![5, 10, 16, 17], "total fit takes one box more");
        assert_ne!(total, greedy, "the two answers differ, which is the point");
    }

    /// A paragraph always comes out whole, even when no line can be set: a
    /// word wider than the measure still reaches the page.
    #[test]
    fn an_unbreakable_paragraph_is_still_broken() {
        let pieces = vec![
            Piece {
                text: "enormous".into(),
                width: 900.0,
                after: After::Glue(4.0),
            },
            Piece {
                text: "word".into(),
                width: 900.0,
                after: After::Nothing,
            },
        ];
        assert_eq!(break_paragraph(&pieces, 100.0), vec![1, 2]);
    }
}
