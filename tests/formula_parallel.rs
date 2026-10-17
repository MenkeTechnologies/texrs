//! Formulas are read while the document is lowered and set afterwards, on the
//! pool when there are enough of them. Each setting depends on the state in
//! force where its formula was read -- the type size, the measure -- and that
//! state moves part way through the document below, so a setting made from
//! the state as lowering LEFT it, rather than as it stood at the formula,
//! comes out at the wrong size or with its number in the wrong place.

use texrs::math::noad::{DISPLAY_STYLE, TEXT_STYLE};
use texrs::math::set::{decode, Setting};
use texrs::math::{read_formula, set_display, set_mlist};
use texrs::typeset::Layout;

/// Formulas per state: past the count that sends setting to the pool.
const EACH: usize = 24;

fn inline(k: usize) -> String {
    format!("x_{{{k}}}^2+\\frac{{a}}{{b+{k}}}")
}

const NUMBERED: &str = "a+b\\eqno(1)";

fn document() -> String {
    let mut src = String::from(
        "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\$=3 \\catcode`\\^=7 \\catcode`\\_=8\n",
    );
    let part = |src: &mut String| {
        for k in 0..EACH {
            src.push_str(&format!("word ${}$ ", inline(k)));
        }
        src.push_str(&format!("\n\n$${NUMBERED}$$\n\n"));
    };
    part(&mut src);
    src.push_str("\\PassOptionsToClass{12pt}{article}\n");
    src.push_str("\\PassOptionsToPackage{margin=2in}{geometry}\n");
    part(&mut src);
    src.push_str("\\end\n");
    src
}

fn marked(src: &str, threads: usize) -> String {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .expect("pool")
        .install(|| texrs::run_text_marked(src).expect("run"))
}

/// Every formula marker's setting, in document order.
fn settings(text: &str) -> Vec<Setting> {
    text.split("\u{1}M")
        .skip(1)
        .map(|rest| {
            let spec = &rest[..rest.find('\u{2}').expect("marker closes")];
            decode(spec).unwrap_or_else(|| panic!("an unfilled or bad spec: {spec:?}"))
        })
        .collect()
}

#[test]
fn formulas_set_on_many_threads_match_one_and_the_state_each_was_read_in() {
    let before = Layout::default();
    let mut after = Layout::default();
    after.absorb_class_options("12pt");
    after.absorb_geometry_options("margin=2in");
    if set_mlist(&[], TEXT_STYLE, before.size).is_none() {
        eprintln!("skipping: no TeX installation, so there are no math fonts to set in");
        return;
    }
    // The document must move both, or it tests nothing.
    assert_ne!(before.size, after.size);
    assert_ne!(before.measure, after.measure);

    let src = document();
    let serial = marked(&src, 1);
    for threads in [2, 8] {
        assert!(
            serial == marked(&src, threads),
            "text differs at {threads} threads"
        );
    }

    let got = settings(&serial);
    assert_eq!(got.len(), 2 * (EACH + 1), "one setting per formula");
    let numbered = read_formula(NUMBERED).expect("parses");
    for (half, layout) in [before, after].iter().enumerate() {
        let base = half * (EACH + 1);
        for k in 0..EACH {
            let list = texrs::math::parse_formula(&inline(k)).expect("parses");
            let want = set_mlist(&list, TEXT_STYLE, layout.size).expect("fonts");
            let have = &got[base + k];
            assert_eq!(
                (have.width, &have.glyphs, &have.rules),
                (want.width, &want.glyphs, &want.rules),
                "formula {k} of part {half} was not set at {}pt",
                layout.size
            );
        }
        let want =
            set_display(&numbered, DISPLAY_STYLE, layout.size, "", layout.measure).expect("fonts");
        assert_eq!(
            got[base + EACH],
            want[0],
            "the numbered display of part {half} was not set on a {}pt measure",
            layout.measure
        );
    }
}
