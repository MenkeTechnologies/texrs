//! The `.tfm` reader against the program Knuth wrote for reading `.tfm`s.
//!
//! `src/tfm.rs` has unit tests for the numbers a person can check by eye. This
//! is the other half: every character of a real font, compared with `tftopl`'s
//! own output for it. A `.tfm` is a file of indices into tables, so the way it
//! goes wrong is one character reading another's width — which a handful of
//! spot checks will not catch and 128 of them will.

use std::collections::HashMap;
use std::process::Command;

/// A font from TeX Live, or `None` when this machine has no TeX.
fn installed(name: &str) -> Option<String> {
    let found = Command::new("kpsewhich").arg(name).output().ok()?;
    let path = String::from_utf8_lossy(&found.stdout).trim().to_string();
    (!path.is_empty()).then_some(path)
}

/// `tftopl`'s property list for a font, which is the oracle.
fn pl(path: &str) -> Option<String> {
    let out = Command::new("tftopl").arg(path).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

/// The character code a `(CHARACTER …` line names: `C A` is a letter, `O 40`
/// is octal.
fn code_of(line: &str) -> Option<u8> {
    let mut words = line.split_whitespace().skip(1);
    match (words.next()?, words.next()?) {
        ("C", c) => c.chars().next().map(|c| c as u8),
        ("O", n) => u8::from_str_radix(n, 8).ok(),
        ("H", n) => u8::from_str_radix(n, 16).ok(),
        ("D", n) => n.parse().ok(),
        _ => None,
    }
}

/// Every character's four metrics, as tftopl printed them. A metric tftopl
/// leaves out is zero, which is how a `.pl` says "no depth".
fn metrics_from_pl(text: &str) -> HashMap<u8, [f64; 4]> {
    let mut out: HashMap<u8, [f64; 4]> = HashMap::new();
    let mut current: Option<u8> = None;
    // tftopl repeats a character's kerns inside a COMMENT; the metrics are the
    // lines before it, so the comment is skipped by depth rather than by name.
    let mut in_comment = 0usize;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("(CHARACTER") {
            current = code_of(trimmed);
            if let Some(code) = current {
                out.insert(code, [0.0; 4]);
            }
            in_comment = 0;
            continue;
        }
        if trimmed.starts_with("(COMMENT") {
            in_comment += 1;
            continue;
        }
        if in_comment > 0 {
            if trimmed == ")" {
                in_comment -= 1;
            }
            continue;
        }
        let Some(code) = current else { continue };
        for (name, at) in [("CHARWD", 0), ("CHARHT", 1), ("CHARDP", 2), ("CHARIC", 3)] {
            if let Some(rest) = trimmed.strip_prefix(&format!("({name} R ")) {
                if let Ok(value) = rest.trim_end_matches(')').parse::<f64>() {
                    out.get_mut(&code).expect("a character")[at] = value;
                }
            }
        }
    }
    out
}

/// Every character of every font a plain document loads, read twice.
#[test]
fn every_character_of_the_cm_fonts_measures_what_tftopl_says() {
    // The text font, the math italic, the symbols, the extensibles, and a
    // typewriter font whose characters are all one width — which is the font
    // that catches a reader that assumed distinct width indices.
    let fonts = [
        "cmr10.tfm",
        "cmmi10.tfm",
        "cmsy10.tfm",
        "cmex10.tfm",
        "cmtt10.tfm",
    ];
    let mut checked = 0usize;
    for name in fonts {
        let (Some(path),) = (installed(name),) else {
            continue;
        };
        let Some(text) = pl(&path) else { continue };
        let want = metrics_from_pl(&text);
        assert!(!want.is_empty(), "{name}: tftopl printed no characters");

        let tfm = texrs::tfm::Tfm::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        // The same set of characters, neither more nor fewer: a reader that
        // mistook the "does not exist" width index would invent characters.
        let mut got_codes: Vec<u8> = tfm.codes();
        let mut want_codes: Vec<u8> = want.keys().copied().collect();
        got_codes.sort();
        want_codes.sort();
        assert_eq!(
            got_codes, want_codes,
            "{name}: a different set of characters"
        );

        for (&code, &[w, h, d, i]) in &want {
            let m = tfm
                .char(code)
                .unwrap_or_else(|| panic!("{name}: no {code}"));
            for (got, want, what) in [
                (m.width, w, "width"),
                (m.height, h, "height"),
                (m.depth, d, "depth"),
                (m.italic, i, "italic"),
            ] {
                assert!(
                    (got - want).abs() < 5e-7,
                    "{name} 0o{code:o}: {what} {got}, tftopl says {want}"
                );
            }
            checked += 1;
        }
    }
    // If TeX is installed at all, this checked hundreds of numbers; if it is
    // not, it checked none and said so rather than passing quietly.
    if installed("cmr10.tfm").is_some() {
        assert!(checked > 400, "only {checked} characters were compared");
    }
}

/// Every ligature and kern of cmr10, compared with tftopl's LIGTABLE.
///
/// The lig/kern program is the part of a `.tfm` that is a program rather than
/// a table: steps skip, and a long one is reached through a jump. Reading it
/// wrong gives plausible numbers for the wrong pairs.
#[test]
fn the_ligature_program_of_cmr10_is_the_one_tftopl_prints() {
    let Some(path) = installed("cmr10.tfm") else {
        return;
    };
    let Some(text) = pl(&path) else { return };
    let tfm = texrs::tfm::Tfm::open(&path).expect("cmr10");

    // Walk the LIGTABLE. A LABEL says which character the steps that follow
    // belong to, and a later LABEL does not end the earlier one: an earlier
    // label falls through into it, which is how cmr10 gives k and v the same
    // kern and then lets k fall into w's list. So a label stays active until
    // STOP, and the FIRST step matching a right-hand character is the one that
    // applies -- k a is v's -0.055555, not w's -0.027779 further down.
    let mut active: Vec<(u8, std::collections::HashSet<u8>)> = Vec::new();
    let mut pairs = 0usize;
    let check = |right: u8,
                 want: Option<u8>,
                 kern: Option<f64>,
                 active: &mut Vec<(u8, std::collections::HashSet<u8>)>,
                 pairs: &mut usize| {
        for (left, seen) in active.iter_mut() {
            if !seen.insert(right) {
                continue; // an earlier step already claimed this pair
            }
            match (tfm.step(*left, right), want, kern) {
                (Some(texrs::tfm::Step::Ligature { with: got, .. }), Some(with), _) => {
                    assert_eq!(
                        got, with,
                        "0o{left:o} 0o{right:o} makes 0o{with:o}, not 0o{got:o}"
                    )
                }
                (Some(texrs::tfm::Step::Kern { by: got, .. }), _, Some(by)) => assert!(
                    (got - by).abs() < 5e-7,
                    "0o{left:o} 0o{right:o} kerns {got}, tftopl says {by}"
                ),
                (other, _, _) => {
                    panic!("0o{left:o} 0o{right:o}: {other:?} is not what tftopl printed")
                }
            }
            *pairs += 1;
        }
    };
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("(LABEL ") {
            if let Some(code) = code_of(t) {
                active.push((code, std::collections::HashSet::new()));
            }
        } else if t.starts_with("(STOP)") {
            active.clear();
        } else if let Some(rest) = t.strip_prefix("(LIG ") {
            let mut words = rest.trim_end_matches(')').split_whitespace();
            let right = code_of(&format!(
                "x {} {}",
                words.next().unwrap_or(""),
                words.next().unwrap_or("")
            ));
            let with = code_of(&format!(
                "x {} {}",
                words.next().unwrap_or(""),
                words.next().unwrap_or("")
            ));
            let (Some(right), Some(with)) = (right, with) else {
                continue;
            };
            check(right, Some(with), None, &mut active, &mut pairs);
        } else if let Some(rest) = t.strip_prefix("(KRN ") {
            let mut words = rest.trim_end_matches(')').split_whitespace();
            let right = code_of(&format!(
                "x {} {}",
                words.next().unwrap_or(""),
                words.next().unwrap_or("")
            ));
            let by: Option<f64> = match words.next() {
                Some("R") => words.next().and_then(|v| v.parse().ok()),
                _ => None,
            };
            let (Some(right), Some(by)) = (right, by) else {
                continue;
            };
            check(right, None, Some(by), &mut active, &mut pairs);
        }
    }
    assert!(
        pairs > 100,
        "cmr10 has a lig/kern program; only {pairs} pairs were checked"
    );
}

/// What a word actually SETS, not what it holds.
///
/// `Tfm::set_run` is `tex.web` §906-§911's `reconstitute` -- the routine TeX
/// uses to rebuild a word once it knows its characters, and the same algorithm
/// the main loop runs while reading one (§1034-§1040). It is what turns `f`
/// and `i` into cmr10's single character 0o14, and it has to CHAIN: `ffi` is
/// two ligature steps, `f`+`f` to 0o13 and then 0o13+`i` to 0o16, so a reader
/// that only looked at neighbouring pairs of the ORIGINAL text would get 0o13
/// followed by a bare `i`.
///
/// The expected codes are read off `tftopl`'s own ligature table for the font,
/// so this is checked against Knuth's program rather than against itself.
#[test]
fn a_word_is_set_as_the_characters_the_ligature_program_produces() {
    use texrs::tfm::{Set, Tfm};
    let Some(path) = installed("cmr10.tfm") else {
        eprintln!("skipping: no cmr10.tfm");
        return;
    };
    let tfm = Tfm::open(&path).expect("cmr10 reads");
    let table = pl(&path).expect("tftopl runs");

    // The pairs this asserts, confirmed present in the font's own table so a
    // font that stopped carrying them would fail here rather than pass wrongly.
    for wanted in [
        "(LABEL C f)",
        "(LIG C i O 14)",
        "(LIG C f O 13)",
        "(LABEL O 13)",
        "(LIG C i O 16)",
        "(LABEL O 140)",
        "(LIG O 140 O 134)",
        "(LABEL O 55)",
        "(LIG O 55 O 173)",
        "(LABEL O 173)",
        "(LIG O 55 O 174)",
    ] {
        assert!(
            table.contains(wanted),
            "cmr10's ligature table no longer has {wanted}"
        );
    }

    let chars = |text: &str| -> Vec<u8> {
        tfm.set_run(text.as_bytes())
            .into_iter()
            .filter_map(|s| match s {
                Set::Char(c) => Some(c),
                Set::Kern(_) => None,
            })
            .collect()
    };

    assert_eq!(chars("fi"), vec![0o14], "f i is one character");
    assert_eq!(chars("ff"), vec![0o13]);
    // Two steps: ff, then ffi. The `o` and the `ce` are untouched.
    assert_eq!(chars("office"), vec![b'o', 0o16, b'c', b'e']);
    assert_eq!(chars("fluffy"), vec![0o15, b'u', 0o13, b'y'], "fl, then ff");
    // Three characters into one, in two steps: f+f to 0o13, then 0o13+l to
    // 0o17. This is the case that says `set_run` re-reads the character it has
    // just made rather than walking the original text in pairs.
    assert_eq!(chars("shuffle"), vec![b's', b'h', b'u', 0o17, b'e']);
    // The quotes and the dashes are ligatures too, which is why a document
    // that writes ``like this'' gets real quotation marks.
    assert_eq!(chars("``"), vec![0o134]);
    assert_eq!(chars("''"), vec![0o42]);
    assert_eq!(chars("--"), vec![0o173], "an en dash");
    assert_eq!(chars("---"), vec![0o174], "an em dash, in two steps");

    // A kern is NOT a character: it is a movement between two that stay.
    let av = tfm.set_run(b"AV");
    assert_eq!(av.len(), 3, "A, a kern, V: {av:?}");
    match av[1] {
        Set::Kern(by) => assert!((by + 0.111112).abs() < 5e-7, "{by}"),
        ref other => panic!("the middle of AV is a kern, not {other:?}"),
    }

    // A word with neither is itself, and an empty run is empty.
    assert_eq!(chars("box"), vec![b'b', b'o', b'x']);
    assert!(tfm.set_run(b"").is_empty());
}
