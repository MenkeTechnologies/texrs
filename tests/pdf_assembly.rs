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
