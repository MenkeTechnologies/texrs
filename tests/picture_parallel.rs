//! A picture's span is measured off the lowering thread, through the same
//! slots and batches formulas are set in, and filled in document order. The
//! text must not depend on how many threads measured them, and each span must
//! be the one the picture makes when it is measured where it was read.
//!
//! Every picture below is a different height, so a measure landing in the
//! wrong slot changes a span. There are enough of them for batches to go to
//! the pool and a part-batch to be left for the join, and the prose between
//! them carries a `#` of its own, which no slot may take for one of its own.

use texrs::typeset::{picture_mark, PICTURE};

/// Past several of the batches that send measuring to the pool, and not a
/// multiple of one, so the last part-batch is measured at the join.
const PICTURES: usize = 101;

/// The `k`-th picture's option list and body.
fn picture(k: usize) -> (String, String) {
    let options = format!("scale={}", 1 + k % 3);
    let body = format!(
        "\n\\draw (0,0) -- ({},{}) node[above] {{P{k}}};\n\\fill (0,0) circle ({}pt);\n",
        1 + k % 7,
        1 + k % 11,
        1 + k % 4
    );
    (options, body)
}

fn document() -> String {
    let mut src = String::from("\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=12\n");
    for k in 0..PICTURES {
        let (options, body) = picture(k);
        src.push_str(&format!(
            "Figure #{k} follows.\n\n\\begin{{tikzpicture}}[{options}]{body}\\end{{tikzpicture}}\n\n"
        ));
    }
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

/// Every picture span in `text`, in document order, with its two markers.
fn spans(text: &str) -> Vec<String> {
    text.split(PICTURE)
        .skip(1)
        .step_by(2)
        .map(|inner| format!("{PICTURE}{inner}{PICTURE}"))
        .collect()
}

#[test]
fn pictures_measured_on_many_threads_match_one_and_each_picture_measured_alone() {
    let src = document();
    let serial = marked(&src, 1);
    for threads in [2, 8] {
        assert!(
            serial == marked(&src, threads),
            "text differs at {threads} threads"
        );
    }

    let got = spans(&serial);
    assert_eq!(got.len(), PICTURES, "one span per picture");
    for (k, have) in got.iter().enumerate() {
        let (options, body) = picture(k);
        let want = picture_mark(&options, &body);
        assert_eq!(
            have,
            want.trim(),
            "picture {k} has another picture's span, or an unfilled slot"
        );
    }
    assert!(
        serial.contains(&format!("#{}", PICTURES - 1)),
        "the prose's own # was taken for a slot"
    );
}
