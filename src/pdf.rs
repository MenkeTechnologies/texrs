//! Writing PDF, ported from `pdfobj.c` and `pdfdoc.c` in tectonic's
//! `xdvipdfmx`.
//!
//! Everything else in this crate reads. This writes, and it is the far end of
//! the chain the rest of the font work built: a document lays out boxes, the
//! boxes name glyphs in fonts, and what a reader opens is a PDF. `xdvipdfmx`
//! is where tectonic turns a page into one, and the part of it that is a port
//! rather than a typesetter is the object model and the file it serialises to.
//!
//! A PDF is a graph of objects with a table saying where each one starts. That
//! table -- the cross-reference -- is the whole difficulty: it is written last,
//! it holds a byte offset for every object in the file, and a reader trusts it
//! absolutely. An offset out by one and the file is refused. So the writer
//! numbers objects as it takes them, records where each one lands as it writes
//! it, and builds the table from what it did rather than from what it meant to
//! do.
//!
//! What is here is the model and the writer, and enough of the document
//! structure to make a page: a catalogue, a page tree, a content stream and a
//! resource dictionary. What is not here is the typesetting -- there is nothing
//! yet to lay a page out -- nor font embedding, which is the next piece and
//! wants the Type 1 and OpenType readers this crate already has.

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// One PDF object.
#[derive(Debug, Clone, PartialEq)]
pub enum Object {
    Null,
    Boolean(bool),
    /// PDF has one number type in the file, written with or without a point.
    Integer(i64),
    Real(f64),
    /// A string, written in parentheses with what must be escaped escaped.
    Str(String),
    /// `/Name`.
    Name(String),
    Array(Vec<Object>),
    Dict(BTreeMap<String, Object>),
    /// A dictionary with bytes after it. `Length` is filled in by the writer,
    /// because it is the length of what was written and not of what was meant.
    Stream {
        dict: BTreeMap<String, Object>,
        data: Vec<u8>,
    },
    /// `12 0 R`: a reference to an object written elsewhere in the file.
    Reference(u32),
}

impl Object {
    /// A dictionary from pairs, which is most of what building a PDF is.
    pub fn dict(pairs: impl IntoIterator<Item = (&'static str, Object)>) -> Object {
        Object::Dict(
            pairs
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        )
    }

    pub fn name(text: &str) -> Object {
        Object::Name(text.to_string())
    }

    pub fn string(text: &str) -> Object {
        Object::Str(text.to_string())
    }

    /// Write this object into `out`.
    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Object::Null => out.extend(b"null"),
            Object::Boolean(true) => out.extend(b"true"),
            Object::Boolean(false) => out.extend(b"false"),
            Object::Integer(value) => out.extend(value.to_string().as_bytes()),
            Object::Real(value) => {
                // PDF has no exponent notation, and a reader is entitled to
                // refuse `1e-7`. Six places is what xdvipdfmx writes.
                let mut text = format!("{value:.6}");
                while text.contains('.') && (text.ends_with('0') || text.ends_with('.')) {
                    text.pop();
                }
                out.extend(
                    match text.is_empty() || text == "-0" {
                        true => "0".to_string(),
                        false => text,
                    }
                    .as_bytes(),
                )
            }
            Object::Str(text) => {
                out.push(b'(');
                for byte in text.bytes() {
                    // A parenthesis or a backslash inside a string has to be
                    // escaped or the string ends early.
                    match byte {
                        b'(' | b')' | b'\\' => {
                            out.push(b'\\');
                            out.push(byte);
                        }
                        b'\n' => out.extend(b"\\n"),
                        b'\r' => out.extend(b"\\r"),
                        b'\t' => out.extend(b"\\t"),
                        other => out.push(other),
                    }
                }
                out.push(b')');
            }
            Object::Name(text) => {
                out.push(b'/');
                for byte in text.bytes() {
                    // Anything outside the ordinary characters is written as
                    // `#` and two hexadecimal digits.
                    match byte.is_ascii_alphanumeric() || b"-_.+".contains(&byte) {
                        true => out.push(byte),
                        false => out.extend(format!("#{byte:02X}").as_bytes()),
                    }
                }
            }
            Object::Array(items) => {
                out.push(b'[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(b' ');
                    }
                    item.write(out);
                }
                out.push(b']');
            }
            Object::Dict(pairs) => write_dict(pairs, out),
            Object::Stream { dict, data } => {
                let mut dict = dict.clone();
                // The length is what was written, so it is set here rather
                // than trusted from the caller.
                dict.insert("Length".into(), Object::Integer(data.len() as i64));
                write_dict(&dict, out);
                out.extend(b"\nstream\n");
                out.extend(data);
                out.extend(b"\nendstream");
            }
            Object::Reference(number) => out.extend(format!("{number} 0 R").as_bytes()),
        }
    }
}

fn write_dict(pairs: &BTreeMap<String, Object>, out: &mut Vec<u8>) {
    out.extend(b"<<");
    for (key, value) in pairs {
        Object::Name(key.clone()).write(out);
        out.push(b' ');
        value.write(out);
        out.push(b' ');
    }
    // A dictionary with nothing in it still needs its brackets apart.
    if pairs.is_empty() {
        out.push(b' ');
    }
    out.pop();
    out.extend(b">>");
}

/// A PDF being written.
pub struct Pdf {
    objects: Vec<Object>,
    /// What the trailer's `Root` points at.
    catalog: Option<u32>,
}

impl Default for Pdf {
    fn default() -> Self {
        Pdf::new()
    }
}

impl Pdf {
    pub fn new() -> Pdf {
        Pdf {
            objects: Vec::new(),
            catalog: None,
        }
    }

    /// Take an object and hand back the reference to it. Objects are numbered
    /// from one, because zero is the head of the free list and always has been.
    pub fn add(&mut self, object: Object) -> Object {
        self.objects.push(object);
        Object::Reference(self.objects.len() as u32)
    }

    /// Reserve a number for an object not yet built -- a page has to name the
    /// page tree that has to list the page.
    pub fn reserve(&mut self) -> u32 {
        self.objects.push(Object::Null);
        self.objects.len() as u32
    }

    /// Fill in what `reserve` promised.
    pub fn fill(&mut self, number: u32, object: Object) {
        if let Some(slot) = self.objects.get_mut(number as usize - 1) {
            *slot = object;
        }
    }

    /// Say which object is the catalogue, which is where a reader starts.
    pub fn set_catalog(&mut self, number: u32) {
        self.catalog = Some(number);
    }

    /// Serialise the file.
    ///
    /// The cross-reference table is why this cannot be done a piece at a time:
    /// it holds the byte offset of every object, so the offsets have to be
    /// recorded while writing and the table written after everything else.
    pub fn finish(&self) -> Vec<u8> {
        let mut out = Vec::new();
        // §7.5.2: the header, and a comment of high bytes that tells anything
        // moving the file about that it is binary and must not be translated.
        out.extend(b"%PDF-1.7\n");
        out.extend([b'%', 0xe2, 0xe3, 0xcf, 0xd3, b'\n']);

        let mut offsets = Vec::with_capacity(self.objects.len());
        for (i, object) in self.objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n", i + 1).as_bytes());
            object.write(&mut out);
            out.extend(b"\nendobj\n");
        }

        let xref = out.len();
        out.extend(format!("xref\n0 {}\n", self.objects.len() + 1).as_bytes());
        // Object zero is the head of the free list, and its entry is fixed.
        out.extend(b"0000000000 65535 f \n");
        for offset in &offsets {
            // Every entry is exactly twenty bytes, which is what lets a reader
            // find one by multiplying.
            out.extend(format!("{offset:010} 00000 n \n").as_bytes());
        }

        let mut trailer = BTreeMap::new();
        trailer.insert(
            "Size".to_string(),
            Object::Integer(self.objects.len() as i64 + 1),
        );
        if let Some(catalog) = self.catalog {
            trailer.insert("Root".to_string(), Object::Reference(catalog));
        }
        out.extend(b"trailer\n");
        write_dict(&trailer, &mut out);
        let mut tail = String::new();
        let _ = write!(tail, "\nstartxref\n{xref}\n%%EOF\n");
        out.extend(tail.as_bytes());
        out
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }
}

/// A font a page sets text in.
#[derive(Debug, Clone, PartialEq)]
pub enum Font {
    /// One of the fourteen every reader has, named rather than embedded --
    /// which is what makes a file like that three kilobytes instead of three
    /// hundred.
    Base14(String),
    /// A Type 1 font carried in the file, so the document reads the same
    /// wherever it is opened. This is what a TeX document needs: nobody has
    /// Computer Modern installed.
    Embedded(Box<crate::type1::Type1>),
    /// A TrueType or OpenType font carried in the file.
    ///
    /// What `\setmainfont{Arimo}` actually asks for. Naming one of the
    /// fourteen gets the right WIDTHS -- Arimo's are Arial's are Helvetica's --
    /// and the wrong shapes; this is the font itself, so the page is set in the
    /// face the document named.
    TrueType {
        /// The name the file calls it by.
        name: String,
        /// The whole font file, embedded as `FontFile2`.
        bytes: Vec<u8>,
        /// Advance widths for codes 32..=255, in 1/1000 em.
        widths: Vec<i64>,
        /// `[xMin, yMin, xMax, yMax]`, in the same units.
        bbox: [i64; 4],
        /// From `hhea`, for the descriptor a reader needs.
        ascent: i64,
        /// From `hhea`. Negative, the way the table stores it.
        descent: i64,
    },
}

impl Font {
    /// What the font calls itself.
    pub fn name(&self) -> String {
        match self {
            Font::Base14(name) => name.clone(),
            Font::Embedded(font) => font.font_name.clone(),
            Font::TrueType { name, .. } => name.clone(),
        }
    }
}

/// What a run of text is set at: what its glyphs come to, and what it is to
/// occupy.
///
/// One argument rather than two because it is one decision, and because two
/// bare widths side by side at a call site are two things to get the wrong way
/// round. `natural` is what the caller measured this same text at, in the same
/// font at the same size; the difference between the two is what is shared out
/// over the spaces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Set {
    pub natural: f64,
    pub width: f64,
}

/// One page: how big it is, and what is drawn on it.
#[derive(Debug, Clone)]
pub struct Page {
    /// The page's size in PDF points, which are 1/72 of an inch and not TeX's.
    pub width: f64,
    pub height: f64,
    /// The content stream: the operators that draw the page.
    pub content: String,
    /// The pictures the content names, as `(name in the content, picture)`.
    pub images: Vec<(String, crate::image::Image)>,
    /// The fonts the content names, as `(name in the content, font)` --
    /// `("F1", Helvetica)`.
    pub fonts: Vec<(String, Font)>,
}

impl Page {
    /// A page of the size TeX defaults to: 8.5 by 11 inches.
    pub fn letter() -> Page {
        Page {
            width: 612.0,
            height: 792.0,
            content: String::new(),
            fonts: Vec::new(),
            images: Vec::new(),
        }
    }

    /// Draw `text` at `(x, y)` in `size` points of `font`, where `y` is
    /// measured up from the bottom of the page -- PDF's axis, which points the
    /// other way from DVI's.
    pub fn text(&mut self, font: &str, size: f64, x: f64, y: f64, text: &str) {
        self.text_in(Font::Base14(font.to_string()), size, x, y, text);
    }

    /// The same, in a font carried in the file.
    ///
    /// The text is bytes rather than characters: a code in a TeX font means
    /// whatever the font's own encoding says, and 11 is an `ff` ligature and
    /// not a vertical tab.
    pub fn text_in(&mut self, font: Font, size: f64, x: f64, y: f64, text: &str) {
        self.draw(font, size, x, y, text, 0.0);
    }

    /// The same, set to occupy exactly the width asked for rather than whatever
    /// the glyphs happen to come to.
    ///
    /// Without this the renderer could only draw a run at its natural width,
    /// and a line breaker that prices a line over glue -- every one of them
    /// does -- had nothing to hand its answer to: a line it chose to shrink was
    /// still drawn at its natural width, out past the measure into the margin.
    ///
    /// PDF spells it `Tw` (S9.3.3): a number of unscaled text-space units added
    /// to the advance of every code 32 in a simple font, which all three of the
    /// fonts here are. Text with no space in it cannot be set to a width at
    /// all, and is drawn where it stands rather than drawn wrong.
    pub fn text_set(&mut self, font: Font, size: f64, x: f64, y: f64, text: &str, set: Set) {
        let spaces = text.chars().filter(|c| *c == ' ').count();
        let word_space = match spaces {
            0 => 0.0,
            n => (set.width - set.natural) / n as f64,
        };
        self.draw(font, size, x, y, text, word_space);
    }

    /// One `BT ... ET`, with `word_space` added to every space in it.
    fn draw(&mut self, font: Font, size: f64, x: f64, y: f64, text: &str, word_space: f64) {
        let name = format!("F{}", self.fonts.len() + 1);
        let name = match self.fonts.iter().find(|(_, used)| used == &font) {
            Some((existing, _)) => existing.clone(),
            None => {
                self.fonts.push((name.clone(), font));
                name
            }
        };
        // A code above 126 is written as its octal escape, because a content
        // stream is BYTES and a `char` pushed into a Rust string is written as
        // UTF-8: the arrow at code 0xAE went into the file as three bytes, a
        // reader drew the three letters that font has at 0xE2, 0x86 and 0x92,
        // and `pdftotext` read the line back with three wrong characters in it.
        // `\256` is the one byte the font's encoding was asked for.
        let escaped: String = text
            .chars()
            .flat_map(|c| match c {
                '(' | ')' | '\\' => vec!['\\', c],
                c if (c as u32) < 32 || (c as u32) > 126 => {
                    format!("\\{:03o}", (c as u32) & 0xFF).chars().collect()
                }
                other => vec![other],
            })
            .collect();
        // Word spacing is TEXT STATE and outlives the `BT ... ET` it was set in
        // (S9.3.1), so a line set to the measure would go on stretching every
        // line drawn after it. It is set inside the block and put back before
        // the block closes, which leaves a run drawn at its natural width byte
        // for byte the operators it was before.
        //
        // It goes AFTER the `Tm` and not before it, which is the same to a
        // reader and not the same to everything that reads this stream back:
        // `Tm`'s six operands are found by counting from the `Tf`, so a `Tw`
        // between the two moves the position a line was drawn at by two words.
        let (open, close) = match word_space == 0.0 {
            true => (String::new(), String::new()),
            false => (format!("{word_space} Tw "), " 0 Tw".to_string()),
        };
        let _ = writeln!(
            self.content,
            "BT /{name} {size} Tf 1 0 0 1 {x} {y} Tm {open}({escaped}) Tj{close} ET"
        );
    }

    /// Draw a picture, with its bottom left corner at `(x, y)` and the size
    /// given in points.
    ///
    /// A picture is drawn by a matrix rather than by coordinates: PDF puts an
    /// image in the unit square and the matrix says where that square lands,
    /// which is why the width and the height go into the matrix and the
    /// drawing is one operator.
    pub fn image(&mut self, image: crate::image::Image, x: f64, y: f64, width: f64, height: f64) {
        let name = match self.images.iter().find(|(_, used)| used == &image) {
            Some((existing, _)) => existing.clone(),
            None => {
                let name = format!("I{}", self.images.len() + 1);
                self.images.push((name.clone(), image));
                name
            }
        };
        // Saved and restored, so the matrix does not follow the picture into
        // whatever is drawn next.
        let _ = writeln!(
            self.content,
            "q {width} 0 0 {height} {x} {y} cm /{name} Do Q"
        );
    }

    /// A filled rectangle, which is what a rule is.
    pub fn rule(&mut self, x: f64, y: f64, width: f64, height: f64) {
        let _ = writeln!(self.content, "{x} {y} {width} {height} re f");
    }
}

/// Write a document of `pages`.
///
/// This is `pdfdoc.c`'s shape: a catalogue naming a page tree, a page tree
/// listing pages, and each page naming its own content and resources. The page
/// tree has to be numbered before the pages so a page can point at its parent
/// and the tree at its children.
pub fn document(pages: &[Page]) -> Vec<u8> {
    let mut pdf = Pdf::new();
    let tree = pdf.reserve();

    // A font is written ONCE and referred to from every page that uses it.
    // Adding it per page instead is correct PDF and unusable in practice: a
    // 144-page book carrying an embedded Arimo came out at 72 MB, one whole
    // copy of the font per page.
    let mut font_objects: Vec<(&Font, Object)> = Vec::new();
    let mut image_objects: Vec<(&crate::image::Image, Object)> = Vec::new();

    let mut kids = Vec::with_capacity(pages.len());
    for page in pages {
        let content = pdf.add(Object::Stream {
            dict: BTreeMap::new(),
            data: page.content.clone().into_bytes(),
        });
        let mut fonts: BTreeMap<String, Object> = BTreeMap::new();
        for (name, font) in &page.fonts {
            let object = match font_objects.iter().find(|(seen, _)| *seen == font) {
                Some((_, object)) => object.clone(),
                None => {
                    let object = add_font(&mut pdf, font);
                    font_objects.push((font, object.clone()));
                    object
                }
            };
            fonts.insert(name.clone(), object);
        }
        let mut images: BTreeMap<String, Object> = BTreeMap::new();
        for (name, image) in &page.images {
            let object = match image_objects.iter().find(|(seen, _)| *seen == image) {
                Some((_, object)) => object.clone(),
                None => {
                    let object = add_image(&mut pdf, image);
                    image_objects.push((image, object.clone()));
                    object
                }
            };
            images.insert(name.clone(), object);
        }
        let resources = match images.is_empty() {
            true => Object::dict([("Font", Object::Dict(fonts))]),
            false => Object::dict([
                ("Font", Object::Dict(fonts)),
                ("XObject", Object::Dict(images)),
            ]),
        };
        kids.push(pdf.add(Object::dict([
            ("Type", Object::name("Page")),
            ("Parent", Object::Reference(tree)),
            (
                "MediaBox",
                Object::Array(vec![
                    Object::Integer(0),
                    Object::Integer(0),
                    Object::Real(page.width),
                    Object::Real(page.height),
                ]),
            ),
            ("Resources", resources),
            ("Contents", content),
        ])));
    }

    pdf.fill(
        tree,
        Object::dict([
            ("Type", Object::name("Pages")),
            ("Count", Object::Integer(kids.len() as i64)),
            ("Kids", Object::Array(kids)),
        ]),
    );
    let catalog = pdf.add(Object::dict([
        ("Type", Object::name("Catalog")),
        ("Pages", Object::Reference(tree)),
    ]));
    if let Object::Reference(number) = catalog {
        pdf.set_catalog(number);
    }
    pdf.finish()
}

/// Add a picture to the file, as `pdf_ximage_load_image` does.
///
/// The pixels are not decoded and not recompressed: PDF's own compressions are
/// PNG's and JPEG's, so a JPEG becomes a `/DCTDecode` stream and a PNG's data
/// becomes a `/FlateDecode` stream with the predictor PNG filtered its rows
/// with. What the dictionary says is what the header said.
fn add_image(pdf: &mut Pdf, image: &crate::image::Image) -> Object {
    use crate::image::{Colours, Compression};

    let space = match (image.colours, &image.palette) {
        (Colours::Indexed, Some(palette)) => Object::Array(vec![
            Object::name("Indexed"),
            Object::name("DeviceRGB"),
            // The highest index the palette defines, which is one less than
            // the number of colours in it.
            Object::Integer(palette.len() as i64 / 3 - 1),
            Object::Str(palette.iter().map(|&b| b as char).collect()),
        ]),
        (Colours::Gray, _) => Object::name("DeviceGray"),
        (Colours::Cmyk, _) => Object::name("DeviceCMYK"),
        _ => Object::name("DeviceRGB"),
    };

    let mut dict = BTreeMap::from([
        ("Type".to_string(), Object::name("XObject")),
        ("Subtype".to_string(), Object::name("Image")),
        ("Width".to_string(), Object::Integer(image.width as i64)),
        ("Height".to_string(), Object::Integer(image.height as i64)),
        (
            "BitsPerComponent".to_string(),
            Object::Integer(image.bits as i64),
        ),
        ("ColorSpace".to_string(), space),
    ]);
    match image.compression {
        Compression::Dct => {
            dict.insert("Filter".to_string(), Object::name("DCTDecode"));
        }
        Compression::Flate => {
            dict.insert("Filter".to_string(), Object::name("FlateDecode"));
            // §7.4.4: predictor 15 is PNG's row filters, which is what is
            // inside a PNG's data and the reason it can be copied.
            dict.insert(
                "DecodeParms".to_string(),
                Object::dict([
                    ("Predictor", Object::Integer(15)),
                    ("Colors", Object::Integer(image.colours.components() as i64)),
                    ("BitsPerComponent", Object::Integer(image.bits as i64)),
                    ("Columns", Object::Integer(image.width as i64)),
                ]),
            );
        }
    }
    // A picture that carried an alpha channel carries it here as a picture of
    // its own, in grey, the same size: §8.9.5.4, which is how PDF says
    // transparency for an image.
    if let Some(alpha) = &image.alpha {
        let mask = pdf.add(Object::Stream {
            dict: BTreeMap::from([
                ("Type".to_string(), Object::name("XObject")),
                ("Subtype".to_string(), Object::name("Image")),
                ("Width".to_string(), Object::Integer(image.width as i64)),
                ("Height".to_string(), Object::Integer(image.height as i64)),
                (
                    "BitsPerComponent".to_string(),
                    Object::Integer(image.bits as i64),
                ),
                ("ColorSpace".to_string(), Object::name("DeviceGray")),
                ("Filter".to_string(), Object::name("FlateDecode")),
                (
                    "DecodeParms".to_string(),
                    Object::dict([
                        ("Predictor", Object::Integer(15)),
                        ("Colors", Object::Integer(1)),
                        ("BitsPerComponent", Object::Integer(image.bits as i64)),
                        ("Columns", Object::Integer(image.width as i64)),
                    ]),
                ),
            ]),
            data: alpha.clone(),
        });
        dict.insert("SMask".to_string(), mask);
    }

    pdf.add(Object::Stream {
        dict,
        data: image.data.clone(),
    })
}

/// Add a font to the file, embedding it when it is not one of the fourteen.
///
/// Ported from `pdf_font_load_type1`. Embedding is four objects that have to
/// agree: the font dictionary, an encoding saying what each code means, a
/// descriptor with the measurements a reader needs to substitute or lay out
/// the font, and the file itself as a stream. The stream is the font exactly as
/// it was read, because a Type 1 font that was rebuilt would no longer
/// decrypt, and `Length1`, `Length2` and `Length3` say where its three parts
/// end.
fn add_font(pdf: &mut Pdf, font: &Font) -> Object {
    if let Font::TrueType {
        name,
        bytes,
        widths,
        bbox,
        ascent,
        descent,
    } = font
    {
        // The font program, whole. A subsetted one would be smaller; a whole
        // one is correct, and correctness is what was missing.
        let file = pdf.add(Object::Stream {
            dict: BTreeMap::from([("Length1".to_string(), Object::Integer(bytes.len() as i64))]),
            data: bytes.clone(),
        });
        let descriptor = pdf.add(Object::dict([
            ("Type", Object::name("FontDescriptor")),
            ("FontName", Object::name(name)),
            // 32 is the non-symbolic flag: the font uses a standard encoding,
            // which is what the WinAnsi below makes true.
            ("Flags", Object::Integer(32)),
            (
                "FontBBox",
                Object::Array(bbox.iter().map(|v| Object::Integer(*v)).collect()),
            ),
            ("ItalicAngle", Object::Integer(0)),
            ("Ascent", Object::Integer(*ascent)),
            ("Descent", Object::Integer(*descent)),
            ("CapHeight", Object::Integer(*ascent)),
            ("StemV", Object::Integer(80)),
            ("FontFile2", file),
        ]));
        return pdf.add(Object::dict([
            ("Type", Object::name("Font")),
            ("Subtype", Object::name("TrueType")),
            ("BaseFont", Object::name(name)),
            ("FirstChar", Object::Integer(32)),
            ("LastChar", Object::Integer(255)),
            (
                "Widths",
                Object::Array(widths.iter().map(|w| Object::Integer(*w)).collect()),
            ),
            ("Encoding", Object::name("WinAnsiEncoding")),
            ("FontDescriptor", descriptor),
        ]));
    }
    let Font::Embedded(type1) = font else {
        let name = font.name();
        // Symbol and ZapfDingbats are the two of the fourteen that are NOT in
        // any standard encoding: their codes mean what their own built-in
        // encoding says, and overlaying WinAnsi on one of them tells a reader
        // that code 174 is a guillemot where the font draws a right arrow.
        // Every other one of the fourteen is written with WinAnsi, which is the
        // encoding this driver addresses them in.
        let built_in = name == "Symbol" || name == "ZapfDingbats";
        let mut entries = vec![
            ("Type", Object::name("Font")),
            ("Subtype", Object::name("Type1")),
            ("BaseFont", Object::name(&name)),
        ];
        if !built_in {
            entries.push(("Encoding", Object::name("WinAnsiEncoding")));
        }
        // What each of the fallback font's codes MEANS, as against which glyph
        // it draws. Without it a page that draws an arrow is a page nobody can
        // search for one -- `pdftotext` read the first of these back with the
        // arrows simply absent from the text. The same thing is written beside
        // an embedded Type 1 below, out of the same `agl`; only the table of
        // what the codes mean is the font's own, and it lives beside the
        // metrics it was read from.
        if name == "Symbol" {
            let meanings: Vec<(u8, String)> = (0u8..=255)
                .filter_map(|code| Some((code, crate::typeset::symbol_unicode(code)?.to_string())))
                .collect();
            let map = pdf.add(Object::Stream {
                dict: BTreeMap::new(),
                data: crate::agl::to_unicode(&name, &meanings).into_bytes(),
            });
            entries.push(("ToUnicode", map));
        }
        return pdf.add(Object::dict(entries));
    };

    let (bytes, clear, binary, trailer) = type1.embeddable();
    let file = pdf.add(Object::Stream {
        dict: BTreeMap::from([
            ("Length1".to_string(), Object::Integer(clear as i64)),
            ("Length2".to_string(), Object::Integer(binary as i64)),
            ("Length3".to_string(), Object::Integer(trailer as i64)),
        ]),
        data: bytes,
    });

    // The measurements a reader needs. A TeX font is not in any of the
    // standard encodings, so `Symbolic` is set: that is what tells a reader to
    // believe the font's own encoding rather than overlay a standard one.
    let bbox = type1.font_bbox;
    let descriptor = pdf.add(Object::dict([
        ("Type", Object::name("FontDescriptor")),
        ("FontName", Object::name(&type1.font_name)),
        ("Flags", Object::Integer(4)),
        (
            "FontBBox",
            Object::Array(bbox.iter().map(|&n| Object::Real(n)).collect()),
        ),
        ("ItalicAngle", Object::Integer(0)),
        // Ascent, descent and cap height are asked for and are not in a Type 1
        // font; the bounding box is what there is to say.
        ("Ascent", Object::Real(bbox[3])),
        ("Descent", Object::Real(bbox[1])),
        ("CapHeight", Object::Real(bbox[3])),
        ("StemV", Object::Integer(80)),
        ("FontFile", file),
    ]));

    // The codes the font's own encoding uses, and what each is called. A
    // reader that was told nothing would use its own idea of what code 11 is,
    // which in a TeX font is an `ff` ligature.
    let mut first = 256usize;
    let mut last = 0usize;
    for code in 0..=255usize {
        if type1.encoded(code as u8).is_some() {
            first = first.min(code);
            last = last.max(code);
        }
    }
    let (first, last) = match first > last {
        true => (0, 0),
        false => (first, last),
    };

    let mut differences = Vec::new();
    let mut expected = usize::MAX;
    for code in first..=last {
        let Some(glyph) = type1.encoded(code as u8) else {
            continue;
        };
        // A run of consecutive codes is written once with its first code, so
        // the array is `1 /a /b /c 40 /x` rather than a code per name.
        if code != expected {
            differences.push(Object::Integer(code as i64));
        }
        differences.push(Object::name(&glyph.name));
        expected = code + 1;
    }
    let encoding = pdf.add(Object::dict([
        ("Type", Object::name("Encoding")),
        ("Differences", Object::Array(differences)),
    ]));

    // The widths, in thousandths of an em, which is what the font is drawn in.
    let widths: Vec<Object> = (first..=last)
        .map(|code| {
            Object::Real(
                type1
                    .encoded(code as u8)
                    .map(|glyph| glyph.width)
                    .unwrap_or(0.0),
            )
        })
        .collect();

    // What each code MEANS, as against which glyph it draws. A PDF says only
    // the second, so a reader asked to copy a paragraph out has a glyph called
    // `ff` and no idea it is two f's. Some readers guess from the name; this
    // says, which is what `xdvipdfmx` writes and what makes a TeX document's
    // text searchable rather than nearly searchable.
    let meanings: Vec<(u8, String)> = (first..=last)
        .filter_map(|code| {
            let glyph = type1.encoded(code as u8)?;
            Some((code as u8, crate::agl::unicode(&glyph.name)?))
        })
        .collect();
    let to_unicode = pdf.add(Object::Stream {
        dict: BTreeMap::new(),
        data: crate::agl::to_unicode(&type1.font_name, &meanings).into_bytes(),
    });

    pdf.add(Object::dict([
        ("Type", Object::name("Font")),
        ("Subtype", Object::name("Type1")),
        ("BaseFont", Object::name(&type1.font_name)),
        ("FirstChar", Object::Integer(first as i64)),
        ("LastChar", Object::Integer(last as i64)),
        ("Widths", Object::Array(widths)),
        ("Encoding", encoding),
        ("ToUnicode", to_unicode),
        ("FontDescriptor", descriptor),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(object: Object) -> String {
        let mut out = Vec::new();
        object.write(&mut out);
        String::from_utf8_lossy(&out).into_owned()
    }

    /// Every kind of object, written the way §7.3 says.
    #[test]
    fn an_object_is_written_the_way_the_specification_says() {
        assert_eq!(written(Object::Null), "null");
        assert_eq!(written(Object::Boolean(true)), "true");
        assert_eq!(written(Object::Integer(-42)), "-42");
        assert_eq!(written(Object::Reference(12)), "12 0 R");

        // A real is written without an exponent, because a reader is entitled
        // to refuse one, and without trailing zeros.
        assert_eq!(written(Object::Real(1.5)), "1.5");
        assert_eq!(written(Object::Real(612.0)), "612");
        assert_eq!(written(Object::Real(0.0000001)), "0");
        assert_eq!(written(Object::Real(-0.25)), "-0.25");

        // A string ends at its closing parenthesis unless the parenthesis is
        // escaped, which is the classic way to write a broken PDF.
        assert_eq!(written(Object::string("plain")), "(plain)");
        assert_eq!(written(Object::string("a (b) c")), "(a \\(b\\) c)");
        assert_eq!(written(Object::string("back\\slash")), "(back\\\\slash)");
        assert_eq!(written(Object::string("two\nlines")), "(two\\nlines)");

        // A name escapes anything unusual as #xx.
        assert_eq!(written(Object::name("Helvetica")), "/Helvetica");
        assert_eq!(written(Object::name("a b")), "/a#20b");
        assert_eq!(written(Object::name("A#B")), "/A#23B");

        assert_eq!(
            written(Object::Array(vec![Object::Integer(1), Object::name("x")])),
            "[1 /x]"
        );
        assert_eq!(
            written(Object::dict([
                ("Type", Object::name("Page")),
                ("Count", Object::Integer(2))
            ])),
            "<</Count 2 /Type /Page>>"
        );
        assert_eq!(written(Object::Dict(BTreeMap::new())), "<<>>");
    }

    /// A stream's `Length` is what was written, not what was claimed.
    #[test]
    fn a_stream_carries_the_length_of_what_it_holds() {
        let text = written(Object::Stream {
            dict: BTreeMap::from([("Length".to_string(), Object::Integer(9999))]),
            data: b"12345".to_vec(),
        });
        assert!(text.starts_with("<</Length 5>>"), "{text}");
        assert!(text.contains("\nstream\n12345\nendstream"), "{text}");
    }

    /// The cross-reference table is the file's index, and every offset in it
    /// must be where the object really is.
    #[test]
    fn every_offset_in_the_table_is_where_the_object_is() {
        let mut pdf = Pdf::new();
        pdf.add(Object::Integer(1));
        pdf.add(Object::string(
            "a longer object, to move the next one along",
        ));
        let third = pdf.add(Object::dict([("Type", Object::name("Catalog"))]));
        if let Object::Reference(number) = third {
            pdf.set_catalog(number);
        }
        let bytes = pdf.finish();
        // The offsets in the table are byte offsets, and the header's binary
        // comment is not UTF-8 -- so all of this is done on the bytes. Doing
        // it on a lossy string moves every offset along by two per replaced
        // byte, which is a mistake worth making once.
        let text = String::from_utf8_lossy(&bytes);

        assert!(bytes.starts_with(b"%PDF-1.7\n"), "{text}");
        assert!(bytes.ends_with(b"%%EOF\n"), "{text}");
        assert!(text.contains("/Root 3 0 R"), "{text}");
        assert!(text.contains("/Size 4"), "the count includes object zero");

        // Read the table the way a reader does, and go to each offset.
        let tail = String::from_utf8_lossy(&bytes[bytes.len() - 64..]).into_owned();
        let at = tail.rfind("startxref\n").expect("a startxref");
        let start: usize = tail[at + 10..]
            .lines()
            .next()
            .expect("an offset")
            .trim()
            .parse()
            .expect("a number");
        assert_eq!(&bytes[start..start + 4], b"xref");
        let table_text = String::from_utf8_lossy(&bytes[start..]).into_owned();
        // Past `xref`, past the `0 4` that says which objects follow, and past
        // object zero's fixed free entry.
        let table: Vec<&str> = table_text.lines().skip(3).take(3).collect();
        for (i, line) in table.iter().enumerate() {
            // Every entry is twenty bytes, which is what lets a reader find
            // one by multiplying rather than by counting lines.
            assert_eq!(line.len() + 1, 20, "{line:?}");
            let offset: usize = line[..10].parse().expect("an offset");
            assert!(
                bytes[offset..].starts_with(format!("{} 0 obj", i + 1).as_bytes()),
                "object {} is not at {offset}: {:?}",
                i + 1,
                String::from_utf8_lossy(&bytes[offset..(offset + 20).min(bytes.len())])
            );
        }
    }

    /// A page carries what was drawn on it.
    #[test]
    fn a_page_holds_its_text_and_its_fonts() {
        let mut page = Page::letter();
        page.text("Helvetica", 12.0, 72.0, 700.0, "Hello (world)");
        page.text("Helvetica", 12.0, 72.0, 680.0, "again");
        page.text("Times-Roman", 10.0, 72.0, 660.0, "and again");
        page.rule(72.0, 650.0, 100.0, 1.0);

        // A font is named once however often it is used.
        assert_eq!(page.fonts.len(), 2);
        assert_eq!(
            page.fonts[0],
            ("F1".to_string(), Font::Base14("Helvetica".to_string()))
        );
        assert_eq!(page.content.matches("/F1").count(), 2);
        // The parentheses in the text are escaped, or the stream ends early.
        assert!(
            page.content.contains("(Hello \\(world\\)) Tj"),
            "{}",
            page.content
        );
        assert!(
            page.content.contains("72 650 100 1 re f"),
            "{}",
            page.content
        );
    }
}
