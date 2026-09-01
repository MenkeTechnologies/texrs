//! The paragraph breaker's answer, read line by line out of the PDF it set.
//!
//! Placed against UNMODIFIED code this fails at the first assertion: a first-fit
//! breaker reaches a different set of breakpoints for the same words.

/// Whether there is a TeX installation to measure in.
///
/// Every line pinned below is pinned to the exact width of the words in
/// cmr10, so the breaks are the breaks THAT FONT gives. Where there is no
/// installation -- CI -- `find_font` answers nothing and the widths fall back
/// to an estimate, which breaks the same prose somewhere else and makes an
/// exact line a false statement about the algorithm rather than a true one.
/// tests/typeset.rs guards its metric tests the same way.
fn metrics_available() -> bool {
    texrs::typeset::find_font("cmr10").is_some()
}

/// The text drawn on each baseline, topmost first.
fn lines(pdf: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(pdf);
    let mut out: Vec<String> = Vec::new();
    let mut last = String::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("BT ") || !line.ends_with(" ET") {
            continue;
        }
        let open = line.find('(').expect("a drawn run has a string");
        let close = line.rfind(") Tj").expect("a drawn run is shown with Tj");
        let head: Vec<&str> = line[..open].split_whitespace().collect();
        let tm = head
            .iter()
            .position(|t| *t == "Tm")
            .expect("a run is placed");
        let y = head[tm - 1].to_string();
        match y == last {
            true => out
                .last_mut()
                .expect("a line to continue")
                .push_str(&line[open + 1..close]),
            false => {
                last = y;
                out.push(line[open + 1..close].to_string());
            }
        }
    }
    out
}

fn set(body: &str) -> Vec<String> {
    let src =
        format!("\\documentclass{{article}}\n\\begin{{document}}\n{body}\n\\end{{document}}\n");
    lines(&texrs::run_pdf(&src).expect("typeset"))
}

/// The same paragraph, broken by total badness rather than by first fit.
///
/// Both answers are six lines and both are pinned here in full. What separates
/// them is line three: first fit stops it after "follow." because "The" will
/// not go on at its natural width, and every line after it is shifted by that
/// one decision. Total fit takes "The" and shrinks the line onto the measure --
/// which is a thing `Page::text_set` can now draw and could not before.
#[test]
fn a_paragraph_breaks_by_total_badness_and_not_by_first_fit() {
    if !metrics_available() {
        eprintln!("skipping: no TeX installation, so cmr10's widths are not there to break on");
        return;
    }
    let prose = "The typesetting of a paragraph is a global optimisation problem, not a \
                 local one. A greedy algorithm that fills each line until the following \
                 word will no longer fit is straightforward to implement and fast to run, \
                 but it commits to every decision it makes without ever considering the \
                 consequences for the lines that follow. The result is a paragraph whose \
                 right-hand edge is noticeably more ragged than one produced by an engine \
                 that considers every feasible sequence of breakpoints simultaneously and \
                 selects the sequence whose total badness is smallest.";

    let first_fit = [
        "The typesetting of a paragraph is a global optimisation problem, not a local one. A greedy algorithm that",
        "fills each line until the following word will no longer fit is straightforward to implement and fast to run, but",
        "it commits to every decision it makes without ever considering the consequences for the lines that follow.",
        "The result is a paragraph whose right-hand edge is noticeably more ragged than one produced by an engine",
        "that considers every feasible sequence of breakpoints simultaneously and selects the sequence whose total",
        "badness is smallest.",
    ];
    let total_fit = [
        "The typesetting of a paragraph is a global optimisation problem, not a local one. A greedy algorithm that",
        "fills each line until the following word will no longer fit is straightforward to implement and fast to run, but",
        "it commits to every decision it makes without ever considering the consequences for the lines that follow. The",
        "result is a paragraph whose right-hand edge is noticeably more ragged than one produced by an engine that",
        "considers every feasible sequence of breakpoints simultaneously and selects the sequence whose total badness",
        "is smallest.",
    ];

    let drawn = set(prose);
    assert_eq!(drawn, total_fit, "broken by total badness");
    assert_ne!(
        drawn, first_fit,
        "and NOT by first fit, which is the whole claim"
    );
}

/// A word the measure cannot hold whole is broken where Knuth's patterns say.
///
/// `incontrovertible` and `uncharacteristically` are hyphenated by `pdflatex`
/// as `in-con-tro-vert-ible` and `un-char-ac-ter-is-ti-cally`, so `-vert-` and
/// `-char-` are breaks TeX itself would take. A machine with no TeX
/// installation has no patterns to read, so the check is skipped rather than
/// failed there -- degrading to no hyphenation is the documented behaviour.
#[test]
fn a_word_is_broken_where_knuths_patterns_break_it() {
    if texrs::linebreak::hyphenator().is_empty() {
        return;
    }
    let prose = "Considering the extraordinarily counterintuitive ramifications, the \
                 interdisciplinary committee recommended a comprehensive reorganisation of \
                 the internationalisation infrastructure, notwithstanding the \
                 incontrovertible evidence that such transformations invariably \
                 precipitate organisational disintegration among the uncharacteristically \
                 overrepresented constituencies.";
    let drawn = set(prose);
    assert_eq!(
        drawn,
        [
            "Considering the extraordinarily counterintuitive ramifications, the interdisciplinary committee recommended",
            "a comprehensive reorganisation of the internationalisation infrastructure, notwithstanding the incontrovert-",
            "ible evidence that such transformations invariably precipitate organisational disintegration among the unchar-",
            "acteristically overrepresented constituencies.",
        ],
        "broken at the patterns' own points, with the hyphen the word did not carry"
    );
}
