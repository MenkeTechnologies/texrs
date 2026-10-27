//! The files a document's images name are found and read once each, on as
//! many threads as the pool has, before anything is measured; the file must
//! not depend on how many threads read them, and a file used many times must
//! be written once.
//!
//! Five files of different sizes, two of them with an alpha channel (the one
//! kind of picture taken apart on reading), are each used eight times in an
//! order that interleaves them, at widths that vary per use. A file that
//! does not exist and one that is not a picture stand among them: the first
//! reserves nothing, the second reserves the room its stated lengths ask for
//! and draws nothing.

use std::io::Write;
use texrs::typeset::{image_mark, image_options, to_pdf, Families, Layout};

/// A solid-colour PNG, RGB or RGBA, written once per test run.
fn png(name: &str, w: u32, h: u32, alpha: bool, shade: u8) -> std::path::PathBuf {
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
    let colour_type = match alpha {
        true => 6,
        false => 2,
    };
    let mut head = w.to_be_bytes().to_vec();
    head.extend_from_slice(&h.to_be_bytes());
    head.extend_from_slice(&[8, colour_type, 0, 0, 0]);
    let pixel: &[u8] = match alpha {
        true => &[shade, 0x90, 0x60, 0x80],
        false => &[shade, 0x90, 0x60],
    };
    // Each row is filter type 0 (None) followed by its pixels.
    let mut row = vec![0u8];
    row.extend(pixel.repeat(w as usize));
    let raw = row.repeat(h as usize);
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&raw).expect("deflate the rows");
    let mut file = b"\x89PNG\r\n\x1a\n".to_vec();
    file.extend(chunk(b"IHDR", &head));
    file.extend(chunk(b"IDAT", &z.finish().expect("deflate the rows")));
    file.extend(chunk(b"IEND", b""));
    let path = std::env::temp_dir().join(format!("texrs-images-read-{name}-{w}x{h}.png"));
    std::fs::write(&path, file).expect("write the png");
    path
}

/// The document, and how many image streams its readable files make: one
/// per file, and one more for each file's soft mask.
fn document() -> (String, usize) {
    let files = [
        (png("a", 64, 16, false, 0x10), false),
        (png("b", 16, 48, true, 0x20), true),
        (png("c", 40, 40, false, 0x30), false),
        (png("d", 24, 12, true, 0x40), true),
        (png("e", 12, 30, false, 0x50), false),
    ];
    let streams = files.iter().map(|(_, alpha)| 1 + usize::from(*alpha)).sum();
    let junk = std::env::temp_dir().join("texrs-images-read-junk.png");
    std::fs::write(&junk, b"not a picture").expect("write the junk");
    let missing = std::env::temp_dir().join("texrs-images-read-nowhere.png");
    let _ = std::fs::remove_file(&missing);

    let prose = "the quick brown fox jumps over the lazy dog ".repeat(4);
    let mut text = String::new();
    for k in 0..40 {
        text.push_str(&prose);
        let (file, _) = &files[(k * 3) % files.len()];
        let (width, height) = image_options(&format!("width=0.{}\\textwidth", 2 + k % 7));
        text.push_str(&image_mark(&width, &height, &file.display().to_string()));
        if k % 13 == 0 {
            let (width, height) = image_options("width=2cm,height=1cm");
            text.push_str(&image_mark(&width, &height, &junk.display().to_string()));
            let (width, height) = image_options("width=0.5\\textwidth");
            text.push_str(&image_mark(&width, &height, &missing.display().to_string()));
        }
    }
    (text, streams)
}

fn render(text: &str, threads: usize) -> Vec<u8> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .expect("pool");
    pool.install(|| to_pdf(text, &Families::default(), &Layout::default(), None, None).bytes)
}

/// How many image streams the file writes: a stream is never packed, so its
/// dictionary is in the file as written.
fn image_streams(file: &[u8]) -> usize {
    file.windows(b"/Subtype /Image".len())
        .filter(|w| w == b"/Subtype /Image")
        .count()
}

#[test]
fn images_read_on_many_threads_match_the_serial_run_and_are_written_once() {
    std::env::set_var("SOURCE_DATE_EPOCH", "1700000000");
    let (text, streams) = document();
    let serial = render(&text, 1);
    assert_eq!(
        image_streams(&serial),
        streams,
        "each file is one XObject, and each alpha channel one more"
    );
    assert!(
        texrs::pdf_page_count(&serial) > 1,
        "the images must fill more than one page"
    );
    for threads in [2, 8] {
        assert!(
            serial == render(&text, threads),
            "PDF differs at {threads} threads"
        );
    }
}
