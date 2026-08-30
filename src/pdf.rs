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

/// One page: how big it is, and what is drawn on it.
#[derive(Debug, Clone)]
pub struct Page {
    /// The page's size in PDF points, which are 1/72 of an inch and not TeX's.
    pub width: f64,
    pub height: f64,
    /// The content stream: the operators that draw the page.
    pub content: String,
    /// The fonts the content names, as `(name in the content, base font)` --
    /// `("F1", "Helvetica")`.
    pub fonts: Vec<(String, String)>,
}

impl Page {
    /// A page of the size TeX defaults to: 8.5 by 11 inches.
    pub fn letter() -> Page {
        Page {
            width: 612.0,
            height: 792.0,
            content: String::new(),
            fonts: Vec::new(),
        }
    }

    /// Draw `text` at `(x, y)` in `size` points of `font`, where `y` is
    /// measured up from the bottom of the page -- PDF's axis, which points the
    /// other way from DVI's.
    pub fn text(&mut self, font: &str, size: f64, x: f64, y: f64, text: &str) {
        let name = format!("F{}", self.fonts.len() + 1);
        let name = match self.fonts.iter().find(|(_, base)| base == font) {
            Some((existing, _)) => existing.clone(),
            None => {
                self.fonts.push((name.clone(), font.to_string()));
                name
            }
        };
        let escaped: String = text
            .chars()
            .flat_map(|c| match c {
                '(' | ')' | '\\' => vec!['\\', c],
                other => vec![other],
            })
            .collect();
        let _ = write!(
            self.content,
            "BT /{name} {size} Tf 1 0 0 1 {x} {y} Tm ({escaped}) Tj ET\n"
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

    let mut kids = Vec::with_capacity(pages.len());
    for page in pages {
        let content = pdf.add(Object::Stream {
            dict: BTreeMap::new(),
            data: page.content.clone().into_bytes(),
        });
        let fonts: BTreeMap<String, Object> = page
            .fonts
            .iter()
            .map(|(name, base)| {
                let font = pdf.add(Object::dict([
                    ("Type", Object::name("Font")),
                    // A base-14 font is named rather than embedded: every
                    // reader has these, which is what makes a file like this
                    // three kilobytes instead of three hundred.
                    ("Subtype", Object::name("Type1")),
                    ("BaseFont", Object::name(base)),
                    ("Encoding", Object::name("WinAnsiEncoding")),
                ]));
                (name.clone(), font)
            })
            .collect();
        let resources = Object::dict([("Font", Object::Dict(fonts))]);
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
        assert_eq!(page.fonts[0], ("F1".to_string(), "Helvetica".to_string()));
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
