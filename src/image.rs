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
//! What is NOT here: an alpha channel. A PNG that carries one interleaves it
//! with the colour, and PDF wants it as a separate soft mask, so the pixels
//! have to be inflated, un-filtered, split and deflated again -- at which point
//! the copying is no longer copying. That is a piece of its own, and until it
//! is done such a PNG is refused by name rather than embedded wrongly.

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
    /// The pixels, exactly as they were in the file.
    pub data: Vec<u8>,
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

    let colours =
        match colour_type {
            0 => Colours::Gray,
            2 => Colours::Rgb,
            3 => Colours::Indexed,
            4 | 6 => return Err(
                "carries an alpha channel, which has to be taken out of the pixels rather than \
                 copied with them"
                    .into(),
            ),
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

    Ok(Image {
        width,
        height,
        bits,
        colours,
        palette,
        data,
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
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        // One that says it carries an alpha channel: refused by name, because
        // copying its pixels would put the alpha in the colour.
        bare.extend([0, 0, 0, 13]);
        bare.extend(b"IHDR");
        bare.extend(10u32.to_be_bytes());
        bare.extend(10u32.to_be_bytes());
        bare.extend([8, 6, 0, 0, 0]); // depth 8, colour type 6 (RGBA)
        bare.extend([0, 0, 0, 0]); // checksum
        let e = read(&bare).unwrap_err();
        assert!(e.contains("alpha"), "{e}");

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
