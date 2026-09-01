//! Justification: a full line is SET to the measure, not drawn at whatever its
//! glyphs happen to come to.
//!
//! The renderer could only do the second, and that is what blocked the line
//! breaker: an algorithm that prices a line over glue -- every one of them does
//! -- decides that a line should be squeezed or stretched and then hands the
//! answer to a driver that draws every space at its natural width. A line it
//! chose to shrink went out past the measure into the right margin.
//!
//! So these read the operators back out of the PDF and ask what the drawn line
//! comes to. Nothing here trusts the typesetter's own arithmetic: the widths
//! come from the positions the file itself gives, the word spacing from its own
//! `Tw`, and the measure from the `Layout` the document was set on.

/// One `BT ... ET` in a content stream: a run of text, where it was put, and
/// what its spaces were widened by.
#[derive(Debug, Clone, PartialEq)]
struct Drawn {
    x: f64,
    y: f64,
    /// The `Tw` in force for this run, zero when the operator is absent.
    word_space: f64,
    /// Whether the run put `Tw` back to zero before its block closed. Word
    /// spacing is text state and outlives the block, so a run that leaves it
    /// set stretches every line drawn after it.
    reset: bool,
    text: String,
}

/// Every run drawn in a PDF, in the order the file draws them.
///
/// The content streams are the only ones the documents here produce: the fonts
/// are the standard fourteen, so nothing else is embedded.
fn runs(pdf: &[u8]) -> Vec<Drawn> {
    let text = String::from_utf8_lossy(pdf);
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("BT ") || !line.ends_with(" ET") {
            continue;
        }
        let open = line.find('(').expect("a drawn run has a string");
        let close = line.rfind(") Tj").expect("a drawn run is shown with Tj");
        let head: Vec<&str> = line[..open].split_whitespace().collect();
        // `BT /F1 10 Tf [ws Tw] 1 0 0 1 x y Tm (`
        let word_space = match head.iter().position(|t| *t == "Tw") {
            Some(at) => head[at - 1].parse().expect("a Tw operand is a number"),
            None => 0.0,
        };
        let tm = head
            .iter()
            .position(|t| *t == "Tm")
            .expect("a run is placed");
        out.push(Drawn {
            x: head[tm - 2].parse().expect("x"),
            y: head[tm - 1].parse().expect("y"),
            word_space,
            reset: line[close..].contains("0 Tw"),
            text: line[open + 1..close].to_string(),
        });
    }
    out
}

/// The runs of one line, which are the ones sharing a baseline.
fn line_at(runs: &[Drawn], y: f64) -> Vec<Drawn> {
    runs.iter().filter(|r| r.y == y).cloned().collect()
}

/// The baselines a PDF draws on, topmost first.
fn baselines(runs: &[Drawn]) -> Vec<f64> {
    let mut ys: Vec<f64> = Vec::new();
    for run in runs {
        if !ys.contains(&run.y) {
            ys.push(run.y);
        }
    }
    ys
}

fn pdf(src: &str) -> Vec<u8> {
    texrs::run_pdf(src).expect("typeset")
}

/// A full line is drawn at exactly the measure.
///
/// The natural width of the words is read out of the file rather than measured
/// here: whatever words the breaker put on the first line are set a SECOND
/// time, alone, with a `\textbf` welded to the end of them, which splits that
/// line into two runs, and the second run's own x says where the first one
/// ended. So the width being checked is the one the FILE says, and the check is
/// `natural + spaces * Tw == measure` -- which is the whole claim.
///
/// The document used to be four words and one 200-character word, on the
/// reasoning that the long word could not fit and so the four before it were a
/// full line. That reasoning was first fit's. `pdflatex` sets those same words
/// as ONE line and reports `Overfull \hbox (830.83832pt too wide)`, which is
/// what texrs now does too, so the document no longer holds a full line at all.
/// Every assertion below is the one that was there; only the paragraph the
/// claim is made about is a real one.
#[test]
fn a_full_line_is_drawn_at_the_measure() {
    let prose = "The typesetting of a paragraph is a global optimisation problem, \
                 not a local one. A greedy algorithm that fills each line until the \
                 following word will no longer fit is straightforward to implement \
                 and fast to run, but it commits to every decision it makes without \
                 ever considering the consequences for the lines that follow.";
    let src =
        format!("\\documentclass{{article}}\n\\begin{{document}}\n{prose}\n\\end{{document}}\n");
    let drawn_runs = runs(&pdf(&src));
    let measure = texrs::typeset::Layout::default().measure;

    // The first line drawn is a full one: the paragraph runs to several lines,
    // and only its last is ragged.
    let full = drawn_runs.first().expect("the paragraph is set").clone();
    assert!(
        full.word_space != 0.0,
        "the first line of a multi-line paragraph is set to the measure: {full:?}"
    );
    assert!(full.reset, "the word spacing has to be put back: {full:?}");
    assert!(
        !full.text.contains('\\'),
        "the line is plain text, so it can be set again as-is: {full:?}"
    );

    // Where the bold letter starts is where those same words ended, so this is
    // the natural width of exactly the text the full line holds.
    let natural = {
        let again = format!(
            "\\documentclass{{article}}\n\\begin{{document}}\n{}\\textbf{{Z}}\n\
             \\end{{document}}\n",
            full.text
        );
        let alone = runs(&pdf(&again));
        let (before, after) = (&alone[0], &alone[1]);
        assert_eq!(before.text, full.text, "the same words, alone: {before:?}");
        assert_eq!(before.word_space, 0.0, "and ragged, so not adjusted");
        assert_eq!(after.text, "Z", "the bold letter follows them: {after:?}");
        assert_eq!(after.y, before.y, "on the same line");
        after.x - before.x
    };

    let spaces = full.text.matches(' ').count() as f64;
    let drawn = natural + spaces * full.word_space;
    assert!(
        (drawn - measure).abs() < 1e-6,
        "a full line is set to the measure: natural {natural}, {spaces} spaces \
         widened by {}, drawn {drawn}, measure {measure}",
        full.word_space
    );
    // And it really was stretched: a test that passed with no adjustment at all
    // would be testing that the words happen to fill the line.
    assert!(
        natural < measure - 1.0,
        "the words are narrower than the measure to begin with: {natural} vs {measure}"
    );
}

/// The last line of a paragraph is left ragged, the way TeX leaves it.
#[test]
fn the_last_line_of_a_paragraph_is_not_stretched() {
    let long = "x".repeat(200);
    let src = format!(
        "\\documentclass{{article}}\n\\begin{{document}}\n\
         alpha beta gamma delta {long} epsilon zeta\n\
         \\end{{document}}\n"
    );
    let runs = runs(&pdf(&src));
    let ys = baselines(&runs);
    assert!(ys.len() >= 2, "more than one line: {runs:#?}");
    let last = line_at(&runs, *ys.last().expect("a last line"));
    for run in &last {
        assert_eq!(
            run.word_space, 0.0,
            "the last line of a paragraph is set at its natural width: {run:?}"
        );
    }
    // The line above it is full, so it is not: otherwise this test would pass
    // on a build that justifies nothing at all.
    let full = line_at(&runs, ys[0]);
    assert!(
        full.iter().any(|r| r.word_space != 0.0),
        "the line before it is full and is set to the measure: {full:#?}"
    );
}

/// A centred line is positioned by its own width, so nothing stretches it.
#[test]
fn a_centred_line_is_left_at_its_own_width() {
    let long = "x".repeat(200);
    let src = format!(
        "\\documentclass{{article}}\n\\begin{{document}}\n\
         \\begin{{center}}\n\
         alpha beta gamma delta {long} epsilon zeta eta theta\n\
         \\end{{center}}\n\
         \\end{{document}}\n"
    );
    let runs = runs(&pdf(&src));
    assert!(!runs.is_empty(), "the document sets something");
    let margin = texrs::typeset::Layout::default().margin;
    for run in &runs {
        assert_eq!(
            run.word_space, 0.0,
            "a centred line is not set to the measure: {run:?}"
        );
    }
    assert!(
        runs.iter().any(|r| r.x > margin + 1.0),
        "and it is still centred, off the margin: {runs:#?}"
    );
}

/// Word spacing is text state: a run that sets it and does not put it back
/// stretches every line drawn after it, on this page and the next.
#[test]
fn the_word_spacing_never_outlives_the_run_that_set_it() {
    let long = "x".repeat(200);
    let src = format!(
        "\\documentclass{{article}}\n\\begin{{document}}\n\
         alpha beta gamma delta {long} epsilon zeta\n\n\
         eta theta iota kappa {long} lambda mu\n\
         \\end{{document}}\n"
    );
    let runs = runs(&pdf(&src));
    let stretched: Vec<&Drawn> = runs.iter().filter(|r| r.word_space != 0.0).collect();
    assert!(!stretched.is_empty(), "something was set to the measure");
    for run in stretched {
        assert!(
            run.reset,
            "the run puts the word spacing back before its block closes: {run:?}"
        );
    }
}

/// The marker the breaker uses to say a line is full is not a character of the
/// document, and never reaches a page as one.
#[test]
fn the_justification_marker_is_never_drawn() {
    let long = "x".repeat(200);
    let src = format!(
        "\\documentclass{{article}}\n\\begin{{document}}\n\
         alpha beta gamma delta {long} epsilon zeta\n\
         \\end{{document}}\n"
    );
    let bytes = pdf(&src);
    for run in runs(&bytes) {
        assert!(
            !run.text.contains("\\023") && !run.text.contains('\u{13}'),
            "U+0013 was drawn as a glyph: {run:?}"
        );
    }
}
