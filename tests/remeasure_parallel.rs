//! Pictures and images are remeasured in parallel before the page is fitted;
//! the file must not depend on how many threads remeasured them.
//!
//! Every picture below is a different height and every image a different
//! width, so a remeasured line landing on the wrong index moves a page break
//! and changes the file. Images alternate between two files of different
//! proportions, so a height resolved against the wrong file does too.

use std::io::Write;
use texrs::typeset::{image_mark, image_options, picture_mark, to_pdf, Families, Layout};

/// A solid-colour truecolour PNG, written once per test run.
fn png(name: &str, w: u32, h: u32) -> std::path::PathBuf {
    let chunk = |kind: &[u8], data: &[u8]| -> Vec<u8> {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let mut crc = 0xffff_ffffu32;
        for byte in kind.iter().chain(data) {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = match crc & 1 {
                    1 => 0xedb8_8320 ^ (crc >> 1),
                    _ => crc >> 1,
                };
            }
        }
        out.extend_from_slice(&(crc ^ 0xffff_ffff).to_be_bytes());
        out
    };
    let mut head = w.to_be_bytes().to_vec();
    head.extend_from_slice(&h.to_be_bytes());
    head.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut raw = Vec::new();
    for _ in 0..h {
        raw.push(0u8);
        raw.extend(std::iter::repeat_n([0x20u8, 0x90, 0x60], w as usize).flatten());
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&raw).expect("deflate the rows");
    let mut file = b"\x89PNG\r\n\x1a\n".to_vec();
    file.extend(chunk(b"IHDR", &head));
    file.extend(chunk(b"IDAT", &z.finish().expect("deflate the rows")));
    file.extend(chunk(b"IEND", b""));
    let path = std::env::temp_dir().join(format!("texrs-remeasure-{name}-{w}x{h}.png"));
    std::fs::write(&path, file).expect("write the png");
    path
}

fn document() -> String {
    let wide = png("wide", 64, 16);
    let tall = png("tall", 16, 48);
    let prose = "the quick brown fox jumps over the lazy dog ".repeat(6);
    let mut text = String::new();
    for k in 0..30 {
        text.push_str(&prose);
        let reach = 1 + k % 5;
        text.push_str(&picture_mark(
            "",
            &format!("\\draw (0,0) -- ({reach},{reach}) node[right] {{P{k}}};"),
        ));
        let file = match k % 2 {
            0 => &wide,
            _ => &tall,
        };
        let (width, height) = image_options(&format!("width=0.{}\\textwidth", 2 + k % 7));
        text.push_str(&image_mark(&width, &height, &file.display().to_string()));
    }
    text
}

fn render(text: &str, threads: usize) -> Vec<u8> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .expect("pool");
    pool.install(|| to_pdf(text, &Families::default(), &Layout::default(), None, None).bytes)
}

#[test]
fn pictures_and_images_remeasured_on_many_threads_match_the_serial_run() {
    std::env::set_var("SOURCE_DATE_EPOCH", "1700000000");
    let text = document();
    let serial = render(&text, 1);
    // Both kinds of block reached the file: the images as XObjects, the
    // pictures as the paths their `\draw` makes.
    let raw = String::from_utf8_lossy(&serial);
    assert!(raw.contains("/Subtype /Image"), "the images must be drawn");
    assert!(
        texrs::pdf_page_count(&serial) > 1,
        "the blocks must fill more than one page"
    );
    for threads in [2, 8] {
        assert!(
            serial == render(&text, threads),
            "PDF differs at {threads} threads"
        );
    }
}
