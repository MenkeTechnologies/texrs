//! Reading an image's header, ported from `pngimage.c` and `jpegimage.c` in
//! tectonic's `xdvipdfmx`.
//!
//! A document says `\includegraphics{figure.png}` and a driver has to put the
//! picture in the PDF. The good news is that it almost never has to decode
//! one: PDF's own image compressions are PNG's and JPEG's. A JPEG is a
//! `/DCTDecode` stream, and a PNG's pixel data is a `/FlateDecode` stream with
//! the same predictor PNG filters its rows with. So embedding a picture is
//! reading its header, writing what the header said as a dictionary, and
//! copying the bytes across untouched -- which is why a PDF full of
//! photographs is the size of the photographs.
//!
//! What is read here is what a driver must know before it can write that
//! dictionary: how big the picture is, how many components a pixel has, how
//! many bits each takes, and where the pixels are.
//!
//! The one picture that cannot be copied is a PNG with an alpha channel. PNG
//! interleaves the alpha with the colour and PDF wants it as a separate soft
//! mask, so those pixels are inflated, un-filtered, split in two and deflated
//! again -- the only case where a picture is taken apart, and the reason this
//! module knows what a PNG row filter is.

/// What a pixel is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colours {
    Gray,
    Rgb,
    /// Indexed into a palette, which travels with the image.
    Indexed,
    Cmyk,
}

impl Colours {
    /// How many numbers a pixel takes.
    pub fn components(&self) -> usize {
        match self {
            Colours::Gray | Colours::Indexed => 1,
            Colours::Rgb => 3,
            Colours::Cmyk => 4,
        }
    }
}

/// A picture, ready to be written into a PDF.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// Bits per component: 1, 2, 4, 8 or 16.
    pub bits: u8,
    pub colours: Colours,
    /// The palette, for an indexed image: three bytes a colour.
    pub palette: Option<Vec<u8>>,
    /// The pixels: exactly as they were in the file, or -- for a picture whose
    /// alpha had to be taken out of them -- the colour on its own.
    pub data: Vec<u8>,
    /// The alpha channel, when the picture had one, as a stream of its own:
    /// one component a pixel, which is what PDF calls a soft mask.
    pub alpha: Option<Vec<u8>>,
    /// Which of PDF's compressions the data is already in.
    pub compression: Compression,
}

/// How the pixels are compressed, in PDF's names for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// A PNG's `IDAT`, which is a zlib stream with PNG's row filters inside
    /// it -- the same thing PDF calls a predictor of 15.
    Flate,
    /// A JPEG, whole.
    Dct,
}

/// Read a picture's header and take its data.
pub fn read(bytes: &[u8]) -> Result<Image, String> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        return png(bytes);
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        return jpeg(bytes);
    }
    Err("not a PNG or a JPEG".into())
}

/// Read a file.
pub fn open(path: impl AsRef<std::path::Path>) -> Result<Image, String> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    read(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn number(bytes: &[u8], at: usize, width: usize) -> Result<u32, String> {
    bytes
        .get(at..at + width)
        .map(|region| {
            region
                .iter()
                .fold(0u32, |value, &b| (value << 8) | b as u32)
        })
        .ok_or_else(|| format!("byte {at} is past the end of the file"))
}

/// A PNG: chunks, each with a length, a tag, its data and a checksum.
fn png(bytes: &[u8]) -> Result<Image, String> {
    let mut at = 8usize;
    let (mut width, mut height, mut bits) = (0u32, 0u32, 0u8);
    let mut colour_type = 0u8;
    let mut palette: Option<Vec<u8>> = None;
    let mut data: Vec<u8> = Vec::new();
    let mut seen_header = false;

    while at + 8 <= bytes.len() {
        let length = number(bytes, at, 4)? as usize;
        let tag: String = bytes
            .get(at + 4..at + 8)
            .ok_or("a chunk header past the end")?
            .iter()
            .map(|&b| b as char)
            .collect();
        let from = at + 8;
        let to = from
            .checked_add(length)
            .filter(|&to| to <= bytes.len())
            .ok_or_else(|| format!("the {tag} chunk runs past the end of the file"))?;
        match tag.as_str() {
            "IHDR" => {
                width = number(bytes, from, 4)?;
                height = number(bytes, from + 4, 4)?;
                bits = *bytes.get(from + 8).ok_or("a short IHDR")?;
                colour_type = *bytes.get(from + 9).ok_or("a short IHDR")?;
                // §  : an interlaced PNG's rows are not in order, so its data
                // cannot be handed to a reader as it is.
                if bytes.get(from + 12) != Some(&0) {
                    return Err("interlaced, so its rows are not in order".into());
                }
                seen_header = true;
            }
            "PLTE" => palette = Some(bytes[from..to].to_vec()),
            // The data may arrive in several chunks and is one stream.
            "IDAT" => data.extend_from_slice(&bytes[from..to]),
            "IEND" => break,
            _ => {}
        }
        // The length, the tag, the data, and the four bytes of checksum.
        at = to + 4;
    }
    if !seen_header {
        return Err("no IHDR, so it is not a PNG".into());
    }

    let (colours, has_alpha) = match colour_type {
        0 => (Colours::Gray, false),
        2 => (Colours::Rgb, false),
        3 => (Colours::Indexed, false),
        4 => (Colours::Gray, true),
        6 => (Colours::Rgb, true),
        other => return Err(format!("colour type {other} is not one a PNG has")),
    };
    if !matches!(bits, 1 | 2 | 4 | 8 | 16) {
        return Err(format!("{bits} is not a number of bits a PNG uses"));
    }
    if colours == Colours::Indexed && palette.is_none() {
        return Err("indexed, and carries no palette".into());
    }
    if data.is_empty() {
        return Err("carries no pixels".into());
    }

    // A picture with no alpha goes across as it is; one with alpha is the only
    // case where the pixels are touched at all.
    let (data, alpha) = match has_alpha {
        false => (data, None),
        true => {
            let (colour, alpha) = split_alpha(&data, width, height, bits, colours)?;
            (colour, Some(alpha))
        }
    };

    Ok(Image {
        width,
        height,
        bits,
        colours,
        palette,
        data,
        alpha,
        compression: Compression::Flate,
    })
}

/// A JPEG: markers, each `0xff` and a byte, most with a length.
fn jpeg(bytes: &[u8]) -> Result<Image, String> {
    let mut at = 2usize;
    while at + 4 <= bytes.len() {
        if bytes[at] != 0xff {
            return Err(format!("byte {at} is not the start of a marker"));
        }
        let marker = bytes[at + 1];
        // These stand alone, without a length.
        if (0xd0..=0xd9).contains(&marker) || marker == 0x01 {
            at += 2;
            continue;
        }
        let length = number(bytes, at + 2, 2)? as usize;
        match marker {
            // Every kind of frame header says the same first five numbers, and
            // a driver needs no more than those: a progressive JPEG is as
            // embeddable as a baseline one, because a reader decodes it.
            0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf => {
                let bits = *bytes.get(at + 4).ok_or("a short frame header")?;
                let height = number(bytes, at + 5, 2)?;
                let width = number(bytes, at + 7, 2)?;
                let components = *bytes.get(at + 9).ok_or("a short frame header")?;
                let colours = match components {
                    1 => Colours::Gray,
                    3 => Colours::Rgb,
                    4 => Colours::Cmyk,
                    other => return Err(format!("{other} components is not a picture")),
                };
                if width == 0 || height == 0 {
                    return Err("a picture of no size".into());
                }
                return Ok(Image {
                    width,
                    height,
                    bits,
                    colours,
                    palette: None,
                    alpha: None,
                    // A JPEG goes into a PDF whole: the marker structure is
                    // what a reader's decoder expects to see.
                    data: bytes.to_vec(),
                    compression: Compression::Dct,
                });
            }
            // The scan is the last thing before the pixels, and a file with no
            // frame header before it has none.
            0xda => break,
            _ => {}
        }
        at += 2 + length;
    }
    Err("no frame header, so its size is not stated".into())
}

/// Take the alpha out of a PNG's pixels.
///
/// The data is a zlib stream of rows, each beginning with the number of the
/// filter it was written with, and each filter is defined against the bytes to
/// the left and in the row above -- so undoing them means walking the picture
/// once, in order, keeping the row before. What comes back is two streams, the
/// colour and the alpha, each written as rows filtered with nothing, so the
/// predictor a reader is told about has nothing left to undo.
fn split_alpha(
    data: &[u8],
    width: u32,
    height: u32,
    bits: u8,
    colours: Colours,
) -> Result<(Vec<u8>, Vec<u8>), String> {
    use std::io::{Read, Write};

    if bits != 8 && bits != 16 {
        return Err(format!(
            "{bits} bits a component with an alpha channel is not something a PNG writes"
        ));
    }
    let mut pixels = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut pixels)
        .map_err(|e| format!("the pixels are not a zlib stream: {e}"))?;

    let sample = bits as usize / 8;
    let components = colours.components();
    // A pixel is its colour and one more component for the alpha.
    let pixel = (components + 1) * sample;
    let row = width as usize * pixel;
    let expected = (row + 1) * height as usize;
    if pixels.len() < expected {
        return Err(format!(
            "the pixels are {} bytes where {expected} were promised",
            pixels.len()
        ));
    }

    let mut colour_out = Vec::with_capacity(expected);
    let mut alpha_out = Vec::with_capacity(height as usize * (width as usize * sample + 1));
    let mut previous = vec![0u8; row];
    let mut current = vec![0u8; row];
    for y in 0..height as usize {
        let at = y * (row + 1);
        let filter = pixels[at];
        current.copy_from_slice(&pixels[at + 1..at + 1 + row]);
        unfilter(filter, &mut current, &previous, pixel)?;

        // Filtered with nothing, so what follows each of these bytes is the
        // pixels themselves.
        colour_out.push(0);
        alpha_out.push(0);
        for x in 0..width as usize {
            let from = x * pixel;
            colour_out.extend_from_slice(&current[from..from + components * sample]);
            alpha_out.extend_from_slice(&current[from + components * sample..from + pixel]);
        }
        previous.copy_from_slice(&current);
    }

    let deflate = |bytes: &[u8]| -> Result<Vec<u8>, String> {
        let mut out = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        out.write_all(bytes).map_err(|e| e.to_string())?;
        out.finish().map_err(|e| e.to_string())
    };
    Ok((deflate(&colour_out)?, deflate(&alpha_out)?))
}

/// Undo one row's filter, in place.
///
/// §9.2 of the PNG specification: `a` is the byte one pixel to the left, `b`
/// the one above, `c` the one above and to the left, and a byte before the
/// start of the picture is zero.
fn unfilter(filter: u8, row: &mut [u8], previous: &[u8], pixel: usize) -> Result<(), String> {
    for i in 0..row.len() {
        let a = match i >= pixel {
            true => row[i - pixel] as i32,
            false => 0,
        };
        let b = previous[i] as i32;
        let c = match i >= pixel {
            true => previous[i - pixel] as i32,
            false => 0,
        };
        let value = row[i] as i32;
        row[i] = match filter {
            0 => value,
            1 => value + a,
            2 => value + b,
            3 => value + (a + b) / 2,
            4 => {
                // Paeth: whichever of the three neighbours the gradient is
                // nearest to.
                let p = a + b - c;
                let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                value
                    + match (pa <= pb && pa <= pc, pb <= pc) {
                        (true, _) => a,
                        (_, true) => b,
                        _ => c,
                    }
            }
            other => return Err(format!("{other} is not a PNG row filter")),
        } as u8;
    }
    Ok(())
}

impl Image {
    /// How many bytes a row of pixels takes, before compression -- which is
    /// what a reader needs to undo the predictor.
    pub fn row_bytes(&self) -> usize {
        let bits = self.width as usize * self.colours.components() * self.bits as usize;
        bits.div_ceil(8)
    }

    /// A summary a person reads.
    pub fn summary(&self) -> String {
        let mut out = format!("size          {} by {} pixels\n", self.width, self.height);
        out.push_str(&format!(
            "colours       {:?}, {} bits a component\n",
            self.colours, self.bits
        ));
        if let Some(palette) = &self.palette {
            out.push_str(&format!("palette       {} colours\n", palette.len() / 3));
        }
        out.push_str(&format!(
            "compression   {}\n",
            match self.compression {
                Compression::Flate => "flate, as a PDF wants it",
                Compression::Dct => "jpeg, as a PDF wants it",
            }
        ));
        out.push_str(&format!("data          {} bytes\n", self.data.len()));
        if let Some(alpha) = &self.alpha {
            out.push_str(&format!("soft mask     {} bytes\n", alpha.len()));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every row filter, undone.
    ///
    /// A real PNG uses whichever filter compressed each row best, so a picture
    /// from a rendering tool may never use the awkward ones -- and Average and
    /// Paeth are exactly the ones with arithmetic in them. So this builds a
    /// picture whose five rows use the five filters, filtering it here with
    /// the definitions read the other way round, and requires the split to
    /// give the pixels back.
    #[test]
    fn every_row_filter_gives_the_pixels_back() {
        use std::io::Write;

        // Five rows of four RGBA pixels, with values that differ in every
        // direction: a filter undone against the wrong neighbour cannot come
        // out right by accident.
        let width = 4usize;
        let height = 5usize;
        let pixel = 4usize;
        let row = width * pixel;
        let wanted: Vec<u8> = (0..height * row)
            .map(|i| ((i * 37 + i / row * 11) % 251) as u8)
            .collect();

        // Filter a row the way a PNG writer does, which is the definition in
        // §9.2 read forwards.
        let filtered = |filter: u8, current: &[u8], previous: &[u8]| -> Vec<u8> {
            (0..current.len())
                .map(|i| {
                    let a = match i >= pixel {
                        true => current[i - pixel] as i32,
                        false => 0,
                    };
                    let b = previous[i] as i32;
                    let c = match i >= pixel {
                        true => previous[i - pixel] as i32,
                        false => 0,
                    };
                    let value = current[i] as i32;
                    (match filter {
                        0 => value,
                        1 => value - a,
                        2 => value - b,
                        3 => value - (a + b) / 2,
                        _ => {
                            let p = a + b - c;
                            let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                            value
                                - match (pa <= pb && pa <= pc, pb <= pc) {
                                    (true, _) => a,
                                    (_, true) => b,
                                    _ => c,
                                }
                        }
                    }) as u8
                })
                .collect()
        };

        let mut stream = Vec::new();
        let zero = vec![0u8; row];
        for y in 0..height {
            let filter = y as u8;
            let current = &wanted[y * row..(y + 1) * row];
            let previous = match y {
                0 => &zero[..],
                _ => &wanted[(y - 1) * row..y * row],
            };
            stream.push(filter);
            stream.extend(filtered(filter, current, previous));
        }
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&stream).expect("deflate");
        let data = encoder.finish().expect("deflate");

        let (colour, alpha) =
            split_alpha(&data, width as u32, height as u32, 8, Colours::Rgb).expect("the split");

        // Both come back as rows filtered with nothing, so inflating them
        // gives the pixels with a zero in front of each row.
        let inflate = |bytes: &[u8]| {
            let mut out = Vec::new();
            std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(bytes), &mut out)
                .expect("inflate");
            out
        };
        let colour = inflate(&colour);
        let alpha = inflate(&alpha);

        for y in 0..height {
            assert_eq!(
                colour[y * (width * 3 + 1)],
                0,
                "row {y} says it was filtered"
            );
            assert_eq!(alpha[y * (width + 1)], 0, "row {y} of the mask");
            for x in 0..width {
                let from = y * row + x * pixel;
                assert_eq!(
                    &colour[y * (width * 3 + 1) + 1 + x * 3..][..3],
                    &wanted[from..from + 3],
                    "the colour at {x},{y}"
                );
                assert_eq!(
                    alpha[y * (width + 1) + 1 + x],
                    wanted[from + 3],
                    "the alpha at {x},{y}"
                );
            }
        }
    }

    /// Paeth, against numbers worked out by hand.
    ///
    /// The round trip above cannot catch an error in the Paeth predictor that
    /// the test's own filtering makes too -- both would be wrong the same way
    /// and cancel. So this is the one case with the answer written down: with
    /// a row of 200 above and 0 filtered, the predictor must choose the byte
    /// above and give 200 then 0. Adding the corner instead of subtracting it
    /// chooses the byte to the left the second time and gives 200 twice.
    #[test]
    fn the_paeth_predictor_chooses_the_neighbour_the_gradient_is_nearest() {
        let previous = [200u8, 0];
        let mut row = [0u8, 0];
        unfilter(4, &mut row, &previous, 1).expect("a filter");
        assert_eq!(row, [200, 0]);

        // And the three simple ones, which are addition and nothing else.
        let mut row = [5u8, 5];
        unfilter(1, &mut row, &[0, 0], 1).expect("sub");
        assert_eq!(row, [5, 10], "each byte adds the one to its left");

        let mut row = [5u8, 5];
        unfilter(2, &mut row, &[10, 20], 1).expect("up");
        assert_eq!(row, [15, 25], "each byte adds the one above");

        let mut row = [5u8, 5];
        unfilter(3, &mut row, &[10, 20], 1).expect("average");
        assert_eq!(row, [10, 20], "and the average of the two");

        // A filter that does not exist is refused rather than guessed at.
        assert!(unfilter(9, &mut row, &[0, 0], 1).is_err());
    }

    /// A PNG from the installation: chunks, header, pixels.
    #[test]
    fn a_png_says_how_big_it_is_and_where_its_pixels_are() {
        // A palette PNG, which is the common kind for a diagram.
        let made = std::process::Command::new("gs")
            .args(["-dNOPAUSE", "-dBATCH", "-dQUIET", "-sDEVICE=png16m", "-r36"])
            .arg("-sOutputFile=/dev/stdout")
            .arg("-c")
            .arg("0 0 moveto 100 100 lineto stroke showpage")
            .output();
        let Ok(made) = made else { return };
        if !made.status.success() || made.stdout.is_empty() {
            return;
        }
        let image = read(&made.stdout).expect("the png reads");

        assert!(image.width > 0 && image.height > 0);
        assert_eq!(image.bits, 8);
        assert_eq!(image.colours, Colours::Rgb);
        assert_eq!(image.compression, Compression::Flate);
        assert!(!image.data.is_empty());
        // The data is a zlib stream, which begins with its own two-byte header.
        assert_eq!(image.data[0] & 0x0f, 8, "zlib's compression method is 8");
        // A row is three bytes a pixel.
        assert_eq!(image.row_bytes(), image.width as usize * 3);
    }

    /// What is not a picture, and the pictures this will not take.
    #[test]
    fn what_cannot_be_copied_is_refused_by_name() {
        assert!(read(b"").is_err());
        assert!(read(b"not a picture")
            .unwrap_err()
            .contains("PNG or a JPEG"));

        // A PNG header and nothing after it.
        let mut bare = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        assert!(read(&bare).unwrap_err().contains("IHDR"));

        // One that says it carries an alpha channel and then carries no
        // pixels: the alpha comes out of the pixels, so there have to be some.
        bare.extend([0, 0, 0, 13]);
        bare.extend(b"IHDR");
        bare.extend(10u32.to_be_bytes());
        bare.extend(10u32.to_be_bytes());
        bare.extend([8, 6, 0, 0, 0]); // depth 8, colour type 6 (RGBA)
        bare.extend([0, 0, 0, 0]); // checksum
        let e = read(&bare).unwrap_err();
        assert!(e.contains("pixels"), "{e}");

        // And an interlaced one, whose rows are not in order.
        let mut interlaced = bare.clone();
        let at = interlaced.len() - 5;
        interlaced[at] = 1;
        interlaced[at - 3] = 2; // colour type 2, so the alpha is not the reason
        let e = read(&interlaced).unwrap_err();
        assert!(e.contains("interlaced"), "{e}");
    }

    /// A JPEG from the installation.
    #[test]
    fn a_jpeg_says_its_size_from_its_frame_header() {
        let path = "/usr/local/texlive/2026/texmf-dist/doc/eplain/xhyper.jpg";
        let Ok(bytes) = std::fs::read(path) else {
            return;
        };
        let image = read(&bytes).expect("the jpeg reads");

        // What sips says about the same file: 212 by 233, three components.
        assert_eq!((image.width, image.height), (212, 233));
        assert_eq!(image.colours, Colours::Rgb);
        assert_eq!(image.bits, 8);
        assert_eq!(image.compression, Compression::Dct);
        // A JPEG goes into a PDF whole.
        assert_eq!(image.data.len(), bytes.len());

        // A file that begins like a JPEG and holds no frame header.
        let e = read(&[0xff, 0xd8, 0xff, 0xda, 0x00, 0x02]).unwrap_err();
        assert!(e.contains("frame header"), "{e}");
    }
}
