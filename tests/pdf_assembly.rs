//! PDF assembly is deterministic: the same pages give the same bytes whatever
//! the thread count, and a resource is written once however the pages happen
//! to hold it.

use std::io::Write as _;
use std::sync::Arc;
use texrs::pdf::{document, Font, Page};

/// An RGBA PNG whose pixels vary, so the alpha split has real work to do.
fn rgba_png(width: u32, height: u32) -> Vec<u8> {
    let mut raw = Vec::new();
    for y in 0..height {
        raw.push(0);
        for x in 0..width {
            raw.extend([x as u8, y as u8, (x ^ y) as u8, (x + y) as u8]);
        }
    }
    let mut zlib = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    zlib.write_all(&raw).unwrap();
    let zlib = zlib.finish().unwrap();

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut chunk = |tag: &[u8], data: &[u8]| {
        out.extend((data.len() as u32).to_be_bytes());
        out.extend(tag);
        out.extend(data);
        out.extend([0u8; 4]);
    };
    let mut header = Vec::new();
    header.extend(width.to_be_bytes());
    header.extend(height.to_be_bytes());
    header.extend([8, 6, 0, 0, 0]);
    chunk(b"IHDR", &header);
    chunk(b"IDAT", &zlib);
    chunk(b"IEND", &[]);
    out
}

/// `count` pages, each drawing in every font of `fonts` and on every fifth page
/// a picture. `fonts` is what the pages hold: the same allocations on every
/// page, or a fresh equal one per page.
fn pages(count: usize, fonts: &dyn Fn() -> Vec<Arc<Font>>) -> Vec<Page> {
    let image = texrs::image::read(&rgba_png(64, 48)).unwrap();
    (0..count)
        .map(|n| {
            let mut page = Page::letter();
            for line in 0..40 {
                page.content.push_str(&format!(
                    "BT /F1 10 Tf 72 {} Td (line {line} of page {n}) Tj ET\n",
                    700 - line * 10
                ));
            }
            for (k, font) in fonts().into_iter().enumerate() {
                let name = format!("F{}", k + 1);
                page.used.insert(name.clone(), (32u8..127).collect());
                page.fonts.push((name, font));
            }
            if n % 5 == 0 {
                page.images.push(("Im1".to_string(), image.clone()));
            }
            page
        })
        .collect()
}

fn base14() -> Vec<Arc<Font>> {
    ["Helvetica", "Times-Roman", "Courier", "Symbol"]
        .iter()
        .map(|name| Arc::new(Font::Base14(name.to_string())))
        .collect()
}

fn in_pool(threads: usize, pages: &[Page]) -> Vec<u8> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap()
        .install(|| document(pages))
}

#[test]
fn bytes_do_not_depend_on_thread_count_or_run() {
    // The creation date is the one input that is not the pages.
    std::env::set_var("SOURCE_DATE_EPOCH", "1700000000");
    let shared = base14();
    let pages = pages(60, &|| shared.clone());

    let one = in_pool(1, &pages);
    assert_eq!(one, in_pool(4, &pages), "1 thread against 4");
    assert_eq!(one, in_pool(4, &pages), "two runs on the same pool size");
    assert_eq!(one, document(&pages), "the global pool");
}

#[test]
fn a_resource_is_written_once_however_the_pages_hold_it() {
    std::env::set_var("SOURCE_DATE_EPOCH", "1700000000");
    let shared = base14();
    let by_address = pages(30, &|| shared.clone());
    let by_value = pages(30, &base14);

    assert_eq!(document(&by_address), document(&by_value));
}

/// A grey picture of `len` bytes all `fill`: two of these with the same `len`
/// agree in everything but their pixels.
fn grey(len: usize, fill: u8) -> texrs::image::Image {
    texrs::image::Image {
        width: 4,
        height: 4,
        bits: 8,
        colours: texrs::image::Colours::Gray,
        palette: None,
        data: vec![fill; len],
        alpha: None,
        compression: texrs::image::Compression::Flate,
    }
}

/// How many pictures the file writes: each is a stream, and a stream is
/// never packed, so its dictionary is in the file as written.
fn pictures_in(file: &[u8]) -> usize {
    file.windows(b"/Subtype /Image".len())
        .filter(|w| w == b"/Subtype /Image")
        .count()
}

#[test]
fn pictures_alike_in_size_but_not_in_pixels_are_written_apart() {
    std::env::set_var("SOURCE_DATE_EPOCH", "1700000000");
    // Every page repeats one picture by value, and every third page adds a
    // second one of the same size and length whose pixels differ: matching
    // by size alone would write it as the first.
    let document_of = |second: u8| -> Vec<Page> {
        (0..40)
            .map(|n| {
                let mut page = Page::letter();
                page.content.push_str("q 10 0 0 10 72 72 cm /Im1 Do Q\n");
                page.images.push(("Im1".to_string(), grey(64, 1)));
                if n % 3 == 0 {
                    page.images.push(("Im2".to_string(), grey(64, second)));
                    page.images.push(("Im3".to_string(), grey(65, 1)));
                }
                page
            })
            .collect()
    };
    let apart = document_of(2);
    let one = in_pool(1, &apart);
    assert_eq!(pictures_in(&one), 3);
    assert_eq!(one, in_pool(8, &apart), "1 thread against 8");
    // The same second picture as the first is the first, written once.
    let alike = document_of(1);
    assert_eq!(pictures_in(&in_pool(8, &alike)), 2);
}
