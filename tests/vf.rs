//! The virtual-font reader against `vftovp`.
//!
//! `vftovp` is the program Knuth shipped for reading a `.vf`, and it prints
//! every character's program: what it sets, where it moves, which font it
//! selects. A virtual font is a program per character, so a reader that
//! mistook one operand would give a plausible answer for the wrong glyph --
//! which is what a whole-font comparison catches and a handful of spot checks
//! does not.

use std::collections::BTreeMap;
use std::process::Command;

use texrs::dvi::Op;
use texrs::vf::Vf;

fn installed(name: &str) -> Option<String> {
    let found = Command::new("kpsewhich").arg(name).output().ok()?;
    let path = String::from_utf8_lossy(&found.stdout).trim().to_string();
    (!path.is_empty()).then_some(path)
}

/// `vftovp`'s property list for a virtual font, which is the oracle. It needs
/// the `.tfm` beside the `.vf`, because the widths live there.
fn vpl(name: &str) -> Option<String> {
    let vf = installed(&format!("{name}.vf"))?;
    let tfm = installed(&format!("{name}.tfm"))?;
    let out = Command::new("vftovp").arg(&vf).arg(&tfm).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

/// A character code as a `.vpl` writes one: `C A`, `O 40`, `D 65`, `H 41`.
fn code_of(kind: &str, value: &str) -> Option<u32> {
    match kind {
        "C" => value.chars().next().map(|c| c as u32),
        "O" => u32::from_str_radix(value, 8).ok(),
        "H" => u32::from_str_radix(value, 16).ok(),
        "D" => value.parse().ok(),
        _ => None,
    }
}

/// One line of a `MAP` block, as text, so the two can be compared without
/// caring how either side stores it.
fn step_of(line: &str) -> Option<String> {
    let body = line.trim().trim_start_matches('(').trim_end_matches(')');
    let words: Vec<&str> = body.split_whitespace().collect();
    match words.as_slice() {
        ["SETCHAR", kind, value] => Some(format!("set {}", code_of(kind, value)?)),
        ["PUTCHAR", kind, value] => Some(format!("put {}", code_of(kind, value)?)),
        ["SELECTFONT", _, number] => Some(format!("font {number}")),
        ["MOVERIGHT", "R", amount] => Some(format!("right {}", round(amount)?)),
        ["MOVELEFT", "R", amount] => Some(format!("right {}", -round(amount)?)),
        ["MOVEDOWN", "R", amount] => Some(format!("down {}", round(amount)?)),
        ["MOVEUP", "R", amount] => Some(format!("down {}", -round(amount)?)),
        ["SETRULE", "R", height, "R", width] => {
            Some(format!("rule {} {}", round(height)?, round(width)?))
        }
        ["PUSH"] => Some("push".into()),
        ["POP"] => Some("pop".into()),
        // A SPECIAL's text runs to the end of the line.
        _ if body.starts_with("SPECIAL ") => Some(format!("special {}", &body[8..])),
        _ => None,
    }
}

/// A length to the six places a `.vpl` prints, as an integer, so the two sides
/// compare exactly rather than approximately.
fn round(text: &str) -> Option<i64> {
    let value: f64 = text.parse().ok()?;
    Some((value * 1_000_000.0).round() as i64)
}

fn ours(op: &Op) -> Option<String> {
    let fix = |raw: i32| (raw as f64 / (1 << 20) as f64 * 1_000_000.0).round() as i64;
    match op {
        Op::SetChar(code) => Some(format!("set {code}")),
        Op::PutChar(code) => Some(format!("put {code}")),
        Op::Font(number) => Some(format!("font {number}")),
        Op::Right(amount) => Some(format!("right {}", fix(*amount))),
        Op::Down(amount) => Some(format!("down {}", fix(*amount))),
        Op::Rule { height, width, .. } => Some(format!("rule {} {}", fix(*height), fix(*width))),
        Op::Push => Some("push".into()),
        Op::Pop => Some("pop".into()),
        Op::Special(text) => Some(format!("special {text}")),
        // A no-op is padding; vftovp does not print one.
        Op::Noop => None,
        _ => Some(format!("{op:?}")),
    }
}

/// Every character's width and program, as `vftovp` prints them.
fn characters(text: &str) -> BTreeMap<u32, (i64, Vec<String>)> {
    let mut out = BTreeMap::new();
    let mut current: Option<u32> = None;
    let mut width = 0i64;
    let mut steps: Vec<String> = Vec::new();
    let mut in_map = false;
    let mut in_comment = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("(CHARACTER ") {
            if let Some(code) = current.take() {
                out.insert(code, (width, std::mem::take(&mut steps)));
            }
            let words: Vec<&str> = trimmed.split_whitespace().collect();
            current = code_of(words[1], words[2].trim_end_matches(')'));
            width = 0;
            in_map = false;
            in_comment = false;
            continue;
        }
        if current.is_none() {
            continue;
        }
        // A COMMENT block repeats the ligature program; it is not the map.
        if trimmed.starts_with("(COMMENT") {
            in_comment = true;
            continue;
        }
        if in_comment {
            if trimmed == ")" {
                in_comment = false;
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("(CHARWD R ") {
            width = round(rest.trim_end_matches(')')).unwrap_or(0);
            continue;
        }
        if trimmed.starts_with("(MAP") {
            in_map = true;
            continue;
        }
        if in_map {
            if trimmed == ")" {
                in_map = false;
                continue;
            }
            if let Some(step) = step_of(trimmed) {
                steps.push(step);
            }
        }
    }
    if let Some(code) = current {
        out.insert(code, (width, steps));
    }
    out
}

/// Four virtual fonts from TeX Live, every character of each, compared with
/// what `vftovp` says about it.
#[test]
fn every_character_of_a_virtual_font_maps_where_vftovp_says() {
    // Times, Helvetica and Courier in TeX's text encoding, and Times in the
    // maths encoding, which is the one that moves things around rather than
    // just renaming them.
    let fonts = ["ptmr7t", "phvr7t", "pcrr7t", "ptmri7t"];
    let mut checked = 0usize;
    let mut programs = 0usize;
    for name in fonts {
        let Some(text) = vpl(name) else { continue };
        let Some(path) = installed(&format!("{name}.vf")) else {
            continue;
        };
        let vf = Vf::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let want = characters(&text);
        assert!(!want.is_empty(), "{name}: vftovp printed no characters");

        // The same characters, neither more nor fewer.
        assert_eq!(
            vf.codes(),
            want.keys().copied().collect::<Vec<u32>>(),
            "{name}: a different set of characters"
        );

        for (&code, (width, steps)) in &want {
            let got = vf
                .char(code)
                .unwrap_or_else(|| panic!("{name}: no 0o{code:o}"));
            assert_eq!(
                (got.width * 1_000_000.0).round() as i64,
                *width,
                "{name} 0o{code:o}: width"
            );
            let mine: Vec<String> = got.ops.iter().filter_map(ours).collect();
            assert_eq!(&mine, steps, "{name} 0o{code:o}: the program");
            checked += 1;
            if steps.len() > 1 {
                programs += 1;
            }
        }
    }
    if installed("ptmr7t.vf").is_some() {
        assert!(checked > 300, "only {checked} characters were compared");
        // The characters that are more than one glyph are the point of the
        // format; a comparison that only saw the simple ones would prove
        // little.
        assert!(programs > 20, "only {programs} characters were programs");
    }
}

/// What a virtual character really sets, which is what a driver needs from it.
#[test]
fn a_virtual_character_names_the_real_font_and_glyph() {
    let Some(path) = installed("ptmr7t.vf") else {
        return;
    };
    let vf = Vf::open(&path).expect("ptmr7t reads");

    // Every glyph a character sets is a character of a font the file defined.
    let mut glyphs = 0usize;
    let mut without = 0usize;
    for code in vf.codes() {
        let c = vf.char(code).expect("a character");
        let sets = c.glyphs();
        for (font, _glyph) in &sets {
            assert!(
                vf.font(*font).is_some(),
                "0o{code:o} sets a glyph in font {font}, which is not defined"
            );
            glyphs += 1;
        }
        if sets.is_empty() {
            // A character the real font has no glyph for is a rule and a
            // `\special` saying so, which is what a driver prints instead of
            // silently leaving a gap.
            without += 1;
            assert!(
                c.ops.iter().any(|op| matches!(op, Op::Special(_)))
                    && c.ops.iter().any(|op| matches!(op, Op::Rule { .. })),
                "0o{code:o} sets nothing and says nothing: {:?}",
                c.ops
            );
        }
    }
    assert!(glyphs > 100, "only {glyphs} glyphs were reached");
    assert!(
        without > 0 && without < vf.codes().len() / 4,
        "{without} of {} characters set no glyph",
        vf.codes().len()
    );
    assert_eq!(vf.font(0).map(|f| f.name.as_str()), Some("ptmr8r"));
}
