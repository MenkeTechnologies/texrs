//! The LaTeX support scans and the PDF writer's moved page contents come out
//! the same whatever the thread count.
//!
//! Each register and counter command is searched for on its own, side by side
//! once the source is long enough, and the answers are joined in a fixed order;
//! a document's page contents are moved into the file rather than copied. None
//! of it needs a TeX installation: the kernel is the engine's own, and the
//! pages are drawn in the fourteen standard fonts.

use std::sync::Arc;
use texrs::pdf::{document, document_from, Font, Page};

fn in_pool<T: Send>(threads: usize, f: impl FnOnce() -> T + Send) -> T {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap()
        .install(f)
}

/// A source well past the size the scans fan out at, declaring registers and
/// counters of every kind in an interleaved order, with repeats, and naming
/// counters it never declares.
fn long_source() -> String {
    let mut src = String::new();
    for n in 0..1500 {
        src.push_str(&format!(
            "\\newcount\\c{n}\\newdimen\\d{n} \\newlength{{\\l{n}}}\\newtoks\\t{n}\n\
             \\newcounter{{k{n}}}\\newtheorem{{thm{n}}}{{Theorem}}\\stepcounter{{k{n}}}\n\
             \\setcounter{{u{}}}{{1}}\\arabic{{page}} \\newbox\\b{} words between the lines\n",
            n % 37,
            n % 11,
        ));
    }
    assert!(src.len() > 128 * 1024, "{}", src.len());
    src
}

#[test]
fn the_support_scans_do_not_depend_on_thread_count() {
    let src = long_source();
    let one = in_pool(1, || texrs::latex::support(&src));
    assert_eq!(
        one,
        in_pool(8, || texrs::latex::support(&src)),
        "1 thread against 8"
    );
    // The counters past the last register are reported, so the overflow path
    // is in the comparison too.
    assert!(one.contains("no register left"), "{one}");
    assert_eq!(
        texrs::latex::counters::mentioned(&src),
        in_pool(8, || texrs::latex::counters::mentioned(&src))
    );
}

fn pages(count: usize) -> Vec<Page> {
    let fonts: Vec<Arc<Font>> = ["Helvetica", "Times-Roman", "Courier"]
        .iter()
        .map(|name| Arc::new(Font::Base14(name.to_string())))
        .collect();
    (0..count)
        .map(|n| {
            let mut page = Page::letter();
            for line in 0..30 {
                page.content.push_str(&format!(
                    "BT /F{} 10 Tf 72 {} Td (line {line} of page {n}) Tj ET\n",
                    line % 3 + 1,
                    700 - line * 10
                ));
            }
            for (k, font) in fonts.iter().enumerate() {
                let name = format!("F{}", k + 1);
                page.used.insert(name.clone(), (32u8..127).collect());
                page.fonts.push((name, Arc::clone(font)));
            }
            page
        })
        .collect()
}

#[test]
fn moved_page_contents_write_the_file_copied_ones_do() {
    std::env::set_var("SOURCE_DATE_EPOCH", "1700000000");
    let copied = in_pool(1, || document(&pages(40)));
    assert_eq!(
        copied,
        in_pool(1, || document_from(pages(40))),
        "moved, 1 thread"
    );
    assert_eq!(
        copied,
        in_pool(8, || document_from(pages(40))),
        "moved, 8 threads"
    );
}
