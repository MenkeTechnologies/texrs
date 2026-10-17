//! On a pool of more than one thread, a long document's finished paragraphs
//! are broken while the rest of it is still being lowered. A paragraph is
//! broken in the faces and on the measure the WHOLE document leaves in force,
//! so a document that switches either part way through must come out exactly
//! as it does on one thread, where nothing is broken until lowering is over:
//! paragraphs broken early, against the state as it stood then, would be set
//! to the wrong measure or in the wrong face.

use texrs::math::noad::TEXT_STYLE;
use texrs::math::set_mlist;

/// Paragraphs before the switch: well past what the lowerer hands on in one
/// go, so some are broken before the switch is read.
const BEFORE: usize = 400;
/// And after it.
const AFTER: usize = 40;

fn paragraph(k: usize, maths: bool) -> String {
    let formula = match maths {
        true => format!(" ${{x_{{{k}}}^2+\\frac{{a}}{{b+{k}}}}}$"),
        false => String::new(),
    };
    format!(
        "Paragraph {k} sets enough words to fill several lines of the measure, \
         so that where each line breaks depends on how wide that measure is and \
         on what the face makes of every word in it{formula}, and then it goes \
         on a little longer still so that the last line is not the only one.\n\n"
    )
}

/// A plain TeX document of `BEFORE` paragraphs, `switch`, and `AFTER` more.
fn document(switch: &str, maths: bool) -> String {
    let mut src = String::from(
        "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\$=3 \\catcode`\\^=7 \\catcode`\\_=8\n",
    );
    for k in 0..BEFORE {
        src.push_str(&paragraph(k, maths));
    }
    src.push_str(switch);
    src.push('\n');
    for k in BEFORE..BEFORE + AFTER {
        src.push_str(&paragraph(k, maths));
    }
    src.push_str("\\end\n");
    src
}

fn pdf(src: &str, threads: usize) -> Vec<u8> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .stack_size(16 << 20)
        .build()
        .expect("pool")
        .install(|| texrs::run_pdf(src).expect("run"))
}

#[test]
fn a_switch_after_paragraphs_were_broken_early_sets_as_one_thread_does() {
    // The file's date and id come from this, so two runs can be compared.
    std::env::set_var("SOURCE_DATE_EPOCH", "1");
    // Formulas exercise the slots the breaker fills before the lowerer does;
    // with no TeX installation there are no math fonts to set them in.
    let maths = set_mlist(&[], TEXT_STYLE, 10.0).is_some();
    for switch in [
        "\\PassOptionsToPackage{margin=2in}{geometry}",
        "\\setmainfont{Arimo}",
    ] {
        let src = document(switch, maths);
        // Long enough that the breaker runs beside the lowerer at all.
        assert!(src.len() > 128 * 1024, "{} bytes", src.len());
        let serial = pdf(&src, 1);
        for threads in [2, 8] {
            assert!(
                serial == pdf(&src, threads),
                "`{switch}`: the PDF differs at {threads} threads"
            );
        }
        // And the switch has to matter, or this tests nothing.
        assert!(
            serial != pdf(&document("", maths), 1),
            "`{switch}` changed nothing"
        );
    }
}
