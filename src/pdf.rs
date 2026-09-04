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
//! it says where every object in the file is, and a reader trusts it
//! absolutely. An entry out by one and the file is refused. So the writer
//! numbers objects as it takes them, records where each one lands as it writes
//! it, and builds the table from what it did rather than from what it meant to
//! do.
//!
//! The table is not a table of byte offsets, because most objects have no byte
//! offset of their own. What `finish` writes is PDF 1.5's pair of structures,
//! which is what LuaTeX writes: the objects that may be packed live inside one
//! compressed `/ObjStm` (§7.5.7), and the table is an `/XRef` stream (§7.5.8)
//! whose entries say either "at this offset" or "at this index of that object
//! stream". Anything reading such a file back has to undo that first, because
//! the structure is no longer in the file's own bytes: `inflate_streams` is
//! enough to find a key, and `unpacked` puts the `N 0 obj` headers back for a
//! reader that walks the graph.
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
                    // §7.3.5: a name carries its bytes as they are except for
                    // the delimiters, whitespace, `#` itself, and anything
                    // outside the graphic range, which go as `#` and two
                    // hexadecimal digits. Escaping more than that is not
                    // wrong for a reader, but it is not what the file should
                    // say: `@` is a graphic character and PGF names a
                    // graphics state `/pgf@CA0.5`, which this wrote as
                    // `/pgf#40CA0.5` -- measured against the same picture set
                    // by lualatex, whose content stream has the `@`.
                    const DELIMITERS: &[u8] = b"()<>[]{}/%#";
                    match byte.is_ascii_graphic() && !DELIMITERS.contains(&byte) {
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
    // `<< /Key value >>`, spaced the way LuaTeX spaces it. The spec allows any
    // whitespace at all between the brackets and the first key, so this is a
    // writer's habit rather than a rule -- and two writers with different
    // habits produce different bytes for identical content, which is the
    // difference the parity ladder's BYTES rung would otherwise report forever.
    out.extend(b"<<");
    for (key, value) in pairs {
        out.push(b' ');
        Object::Name(key.clone()).write(out);
        out.push(b' ');
        value.write(out);
    }
    out.extend(b" >>");
}

/// Where an object ended up once the file was written.
#[derive(Clone, Copy)]
enum Where {
    /// At this byte offset in the file.
    File(usize),
    /// At this index inside the object stream.
    Packed(usize),
    /// A number `reserve` handed out that nothing was ever filled into. It
    /// still needs an entry: the table is indexed by object number, so a gap
    /// in it would move every object after the gap.
    Nowhere,
}

/// Deflate, which is how a PDF compresses anything that is not a picture.
fn deflate(data: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut out = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    out.write_all(data).expect("deflate into a vector");
    out.finish().expect("deflate into a vector")
}

/// How many bytes it takes to write `value`, at least one.
fn bytes_for(value: usize) -> usize {
    ((usize::BITS - value.leading_zeros()) as usize)
        .div_ceil(8)
        .max(1)
}

/// Append `value` as `width` bytes, most significant first, dropping anything
/// that does not fit -- which is what turns the free entry's 65535 into the
/// 255 LuaTeX writes when the field is one byte wide.
fn put(out: &mut Vec<u8>, value: usize, width: usize) {
    for shift in (0..width).rev() {
        out.push((value >> (shift * 8)) as u8);
    }
}

/// Inflate every deflated `stream ... endstream` in a PDF, leaving the rest.
///
/// PDF 1.5 puts a document's structure inside compressed object streams, so
/// anything that wants to READ what a file says -- how many pages it declares,
/// which fonts it carries, what a test asserts about a dictionary -- has to
/// come through here first. `pdf_parity` needs the same thing for LuaTeX's
/// output and calls this.
pub fn inflate_streams(pdf: &[u8]) -> Vec<u8> {
    use std::io::Read;
    let mut out = Vec::with_capacity(pdf.len());
    let mut i = 0;
    while i < pdf.len() {
        let Some(at) = find(&pdf[i..], b"stream").map(|a| i + a) else {
            out.extend_from_slice(&pdf[i..]);
            break;
        };
        let mut data = at + b"stream".len();
        if pdf.get(data) == Some(&b'\r') {
            data += 1;
        }
        if pdf.get(data) == Some(&b'\n') {
            data += 1;
        }
        let Some(end) = find(&pdf[data..], b"endstream").map(|e| data + e) else {
            out.extend_from_slice(&pdf[i..]);
            break;
        };
        out.extend_from_slice(&pdf[i..data]);
        let raw = &pdf[data..end];
        let mut plain = Vec::new();
        match flate2::read::ZlibDecoder::new(raw).read_to_end(&mut plain) {
            Ok(_) => out.extend_from_slice(&plain),
            // Not deflated, or deflated in a way this cannot read: the bytes as
            // they stand are what the engine wrote, and stand in for themselves.
            Err(_) => out.extend_from_slice(raw),
        }
        out.extend_from_slice(b"endstream");
        i = end + b"endstream".len();
    }
    out
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// A PDF's objects as they would read if none of them had been packed.
///
/// `inflate_streams` is enough to FIND a key -- the packed bodies are in the
/// inflated data -- but not to read the file as a graph, because a packed
/// object has no `N 0 obj` header any more: it is a body at an offset named by
/// its object stream's own header. This puts the headers back, so a reader that
/// walks `N 0 obj ... endobj` and follows references between them sees the same
/// document a PDF reader does.
///
/// What comes out is not a valid PDF -- every offset in it is wrong and the
/// table still names the object stream that is no longer there. It is for
/// reading, and the tests that assert on what a document SAYS use it.
pub fn unpacked(pdf: &[u8]) -> Vec<u8> {
    let plain = inflate_streams(pdf);
    let mut out = Vec::with_capacity(plain.len());
    let mut i = 0;
    while i < plain.len() {
        let Some(key) = find(&plain[i..], b"/Type /ObjStm").map(|at| i + at) else {
            out.extend_from_slice(&plain[i..]);
            break;
        };
        // The object this key is in: back to its `N 0 obj` header, forward to
        // the data `inflate_streams` left between `stream` and `endstream`.
        let head = find_last(&plain[i..key], b" 0 obj").map(|at| i + at);
        let data = find(&plain[key..], b"stream\n").map(|at| key + at + 7);
        let (Some(head), Some(data)) = (head, data) else {
            out.extend_from_slice(&plain[i..]);
            break;
        };
        let Some(end) = find(&plain[data..], b"endstream").map(|at| data + at) else {
            out.extend_from_slice(&plain[i..]);
            break;
        };
        let start = find_last(&plain[i..head], b"\n")
            .map(|at| i + at + 1)
            .unwrap_or(head);
        out.extend_from_slice(&plain[i..start]);

        // `First` says where the pairs stop; the pairs say where each object
        // begins, measured from there.
        let dict = String::from_utf8_lossy(&plain[head..data]).into_owned();
        let first: usize = match number_after(&dict, "/First ") {
            Some(first) => first,
            None => {
                out.extend_from_slice(&plain[start..]);
                break;
            }
        };
        let body = &plain[data..end];
        let pairs: Vec<usize> = String::from_utf8_lossy(&body[..first.min(body.len())])
            .split_whitespace()
            .filter_map(|word| word.parse().ok())
            .collect();
        for (index, pair) in pairs.chunks(2).enumerate() {
            let [number, at] = pair else { continue };
            let from = first + at;
            let to = match pairs.get(index * 2 + 3) {
                Some(next) => first + next,
                None => body.len(),
            };
            if from > to || to > body.len() {
                continue;
            }
            out.extend(format!("{number} 0 obj\n").as_bytes());
            out.extend_from_slice(trim_end(&body[from..to]));
            out.extend(b"\nendobj\n");
        }
        // Past the object stream's own `endobj`, which is now redundant.
        i = match find(&plain[end..], b"endobj") {
            Some(at) => end + at + b"endobj".len(),
            None => plain.len(),
        };
    }
    out
}

fn find_last(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}

fn trim_end(body: &[u8]) -> &[u8] {
    let mut end = body.len();
    while end > 0 && body[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &body[..end]
}

/// The number written straight after `key`, if there is one.
fn number_after(text: &str, key: &str) -> Option<usize> {
    text.split_once(key)?
        .1
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()
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

    /// Serialise the file, in the PDF 1.5 form LuaTeX writes.
    ///
    /// The cross-reference table is why this cannot be done a piece at a time:
    /// it says where every object is, so where each one landed has to be
    /// recorded while writing and the table written after everything else.
    ///
    /// Two structures rather than one, because §7.5.7 and §7.5.8 are one
    /// decision. The objects that MAY be packed -- everything that is not
    /// itself a stream -- go into a single `/Type /ObjStm` whose data is one
    /// deflated run, and the table that says where they are cannot then be a
    /// table of byte offsets, because they no longer have any: it becomes a
    /// `/Type /XRef` stream, whose entries say either "at this offset" or "at
    /// this index of that object stream". The trailer goes with the table, its
    /// keys moving into the stream's own dictionary.
    ///
    /// Measured on `Hello world.` through `luatex`: seven objects packed into
    /// one `/ObjStm`, three standing alone (two streams and the document
    /// information), and a `/Type /XRef /W [1 2 1]` object last. What is NOT
    /// reachable this way is LuaTeX's object NUMBERING -- 3, 9, 11, 6, 12 for
    /// that file, which is the order its own allocator handed numbers out in
    /// and not a property of the format.
    pub fn finish(&self) -> Vec<u8> {
        let mut out = Vec::new();
        // §7.5.2: the header, and a comment of high bytes that tells anything
        // moving the file about that it is binary and must not be translated.
        // The spec asks only for four bytes over 127; WHICH four is the
        // writer's own. These are LuaTeX's, so that a document both engines set
        // the same way is the same file here too rather than differing in its
        // second line -- the parity ladder's BYTES rung compares what was drawn,
        // and a header comment is not a difference in drawing.
        out.extend(b"%PDF-1.7\n");
        out.extend([
            b'%', 0xcc, 0xd5, 0xc1, 0xd4, 0xc5, 0xd8, 0xd0, 0xc4, 0xc6, b'\n',
        ]);

        // §7.5.7: a stream may not be packed. A reader finds a stream's data
        // by the `Length` in its dictionary and then reads that many bytes of
        // the FILE, and there are no such bytes for an object that lives
        // inside somebody else's compressed data.
        let packable: Vec<bool> = self
            .objects
            .iter()
            .map(|object| !matches!(object, Object::Stream { .. }))
            .collect();
        let objects = self.objects.len() as u32;
        let objstm = packable.iter().any(|&yes| yes).then_some(objects + 1);
        let table_number = objstm.unwrap_or(objects) + 1;

        // Where each object ended up, which is what the table has to say.
        let mut located: Vec<Where> = vec![Where::Nowhere; self.objects.len()];
        for (i, object) in self.objects.iter().enumerate() {
            if packable[i] {
                continue;
            }
            located[i] = Where::File(out.len());
            out.extend(format!("{} 0 obj\n", i + 1).as_bytes());
            object.write(&mut out);
            out.extend(b"\nendobj\n");
        }

        // The object stream: `N` pairs of object number and offset, then the
        // objects themselves, one after another. `First` is where the pairs
        // stop -- the offsets are measured from there and not from the start
        // of the data, so a reader adds the two.
        let mut objstm_offset = 0;
        if let Some(number) = objstm {
            let mut head = String::new();
            let mut body = Vec::new();
            let mut packed = 0usize;
            for (i, object) in self.objects.iter().enumerate() {
                if !packable[i] {
                    continue;
                }
                let space = match packed {
                    0 => "",
                    _ => " ",
                };
                let _ = write!(head, "{space}{} {}", i + 1, body.len());
                object.write(&mut body);
                body.push(b'\n');
                located[i] = Where::Packed(packed);
                packed += 1;
            }
            head.push('\n');
            let first = head.len();
            let mut data = head.into_bytes();
            data.append(&mut body);
            objstm_offset = out.len();
            out.extend(format!("{number} 0 obj\n").as_bytes());
            Object::Stream {
                dict: BTreeMap::from([
                    ("Type".to_string(), Object::name("ObjStm")),
                    ("N".to_string(), Object::Integer(packed as i64)),
                    ("First".to_string(), Object::Integer(first as i64)),
                    ("Filter".to_string(), Object::name("FlateDecode")),
                ]),
                data: deflate(&data),
            }
            .write(&mut out);
            out.extend(b"\nendobj\n");
        }

        // §7.5.8: the table, as one row an object of three fields -- what kind
        // of entry it is, and two fields meaning different things per kind.
        // Object zero heads the free list, as it does in a classic table.
        let mut rows: Vec<(u8, usize, usize)> = vec![(0, 0, 0xffff)];
        for place in &located {
            rows.push(match place {
                Where::File(offset) => (1, *offset, 0),
                Where::Packed(index) => (2, objstm.unwrap_or(0) as usize, *index),
                // Nothing reaches this today -- an unfilled `reserve` is a
                // `Null`, which packs like anything else. A free entry is what
                // an object number that is not in use is spelled as, and it is
                // the one answer here that cannot send a reader somewhere
                // wrong.
                Where::Nowhere => (0, 0, 0xffff),
            });
        }
        if objstm.is_some() {
            rows.push((1, objstm_offset, 0));
        }
        let table_offset = out.len();
        rows.push((1, table_offset, 0));

        // How wide each field has to be. The second holds a byte offset, so it
        // grows with the file -- LuaTeX's `[1 2 1]` is what a file under 64 kB
        // needs and a book needs more. The third is an index into an object
        // stream, and the free entry's 65535 is truncated to it rather than
        // widening it, which is what LuaTeX writes too.
        let widest = |field: fn(&(u8, usize, usize)) -> Option<usize>| {
            rows.iter().filter_map(field).max().unwrap_or(0)
        };
        let second = widest(|row| (row.0 != 0).then_some(row.1)).max(0xffff);
        let third = widest(|row| (row.0 == 2).then_some(row.2));
        let (second, third) = (bytes_for(second), bytes_for(third));
        let mut data = Vec::with_capacity(rows.len() * (1 + second + third));
        for (kind, one, two) in &rows {
            data.push(*kind);
            put(&mut data, *one, second);
            put(&mut data, *two, third);
        }

        // The trailer's keys, in the table's own dictionary: this IS the
        // trailer now, and `startxref` points at the object holding it.
        let mut dict = BTreeMap::from([
            ("Type".to_string(), Object::name("XRef")),
            (
                "Index".to_string(),
                Object::Array(vec![Object::Integer(0), Object::Integer(rows.len() as i64)]),
            ),
            ("Size".to_string(), Object::Integer(rows.len() as i64)),
            (
                "W".to_string(),
                Object::Array(vec![
                    Object::Integer(1),
                    Object::Integer(second as i64),
                    Object::Integer(third as i64),
                ]),
            ),
            ("Filter".to_string(), Object::name("FlateDecode")),
        ]);
        if let Some(catalog) = self.catalog {
            dict.insert("Root".to_string(), Object::Reference(catalog));
        }
        out.extend(format!("{table_number} 0 obj\n").as_bytes());
        Object::Stream {
            dict,
            data: deflate(&data),
        }
        .write(&mut out);
        out.extend(b"\nendobj\n");
        let mut tail = String::new();
        let _ = write!(tail, "startxref\n{table_offset}\n%%EOF\n");
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
    /// A face carried in the file and addressed by GLYPH rather than by code.
    ///
    /// This is what a per-glyph fallback needs and a simple font cannot give:
    /// `\setmainfont{Arimo}` embeds Arimo whole, WinAnsi addresses 224 of its
    /// glyphs, and no code in that encoding means U+2500. A face that HAS the
    /// box drawing is written here instead as a composite font -- `/Type0` with
    /// `/Identity-H`, where a code is two bytes and is the glyph id itself --
    /// so any glyph in the file can be drawn, whatever Unicode calls it.
    Glyphs {
        /// The name the file calls it by.
        name: String,
        /// The font program, embedded as `FontFile2` -- subsetted to the glyphs
        /// below, since a broad face is tens of megabytes and a document
        /// borrows a handful of glyphs from it.
        bytes: Vec<u8>,
        /// The glyphs the document actually asks this face for: the glyph id,
        /// the character it stands for, and its advance in 1/1000 em. Only
        /// these, because a broad face has fifty thousand glyphs and a `/W`
        /// array of all of them is a quarter of a megabyte of widths for a
        /// document that drew nine of them.
        glyphs: Vec<(u16, char, i64)>,
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
            Font::TrueType { name, .. } | Font::Glyphs { name, .. } => name.clone(),
        }
    }

    /// Whether a code in this font is two bytes rather than one.
    ///
    /// PDF's word spacing (S9.3.3) applies to the single-byte code 32 and to
    /// nothing else, so a `Tw` written for a composite font is ignored by the
    /// reader while the driver goes on advancing by it. Nothing here sets a
    /// two-byte run to a width, and this is what says so.
    pub fn is_composite(&self) -> bool {
        matches!(self, Font::Glyphs { .. })
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
    /// Which codes each of those fonts was actually asked to draw, as
    /// `("F1", {'H', 'e', 'l', 'o'})`.
    ///
    /// A face has thousands of glyphs and a book draws perhaps two hundred of
    /// them, so embedding the file whole puts a megabyte of outlines nobody
    /// looks at into every document; this is the record of which ones were
    /// looked at, kept as the page is drawn because that is the only moment
    /// that knows. It is per resource NAME rather than per font because the
    /// name is what the content stream says; `document` joins the two back
    /// together across the pages, since a font is written once for the file.
    pub used: BTreeMap<String, std::collections::BTreeSet<u8>>,
    /// The graphics-state dictionaries the content names, as `(name, key,
    /// value)` -- `("pgf@CA0.5", "CA", 0.5)`.
    ///
    /// Constant alpha is the one drawing parameter PDF gives no operator for:
    /// §11.6.4.4 puts it in a dictionary, and `gs` (§8.4.4, Table 57) sets it
    /// by NAME out of the page's `/ExtGState`. So `0.5 CA` does not exist and
    /// `/pgf@CA0.5 gs` is the whole of how a half-transparent stroke is asked
    /// for -- which means a page emitting that operator and carrying no such
    /// resource is a page whose transparency a reader silently drops, the name
    /// resolving to nothing.
    pub ext_gstates: Vec<(String, String, f64)>,
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
            used: BTreeMap::new(),
            ext_gstates: Vec::new(),
        }
    }

    /// Name a graphics-state dictionary this page's content stream uses, so
    /// the page's `/Resources` carries it.
    ///
    /// Naming the same one twice is what a picture with two half-transparent
    /// paths does, and the second is dropped rather than written again: the
    /// resource dictionary is keyed by the name, so a duplicate is at best a
    /// wasted entry and at worst two entries a reader may pick between.
    pub fn ext_gstate(&mut self, name: &str, key: &str, value: f64) {
        if self.ext_gstates.iter().any(|(used, _, _)| used == name) {
            return;
        }
        self.ext_gstates
            .push((name.to_string(), key.to_string(), value));
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
        // A composite font's codes are two bytes and none of them is the
        // single-byte 32 that `Tw` widens, so there is no space here to share
        // the room between: 0x20 inside a glyph id is half a glyph id.
        let spaces = match font.is_composite() {
            true => 0,
            false => text.chars().filter(|c| *c == ' ').count(),
        };
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
        // What this font is asked for, so the file can carry those glyphs and
        // leave the rest of the face behind. A code is one byte -- the same
        // byte the escape below writes -- and a two-byte code of a composite
        // font is recorded as its two halves, which is what a `/CIDToGIDMap
        // /Identity` font's own glyph list already says more exactly.
        self.used
            .entry(name.clone())
            .or_default()
            .extend(text.chars().map(|c| (c as u32 & 0xFF) as u8));
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

    // Which codes each font was asked for anywhere in the document. A font is
    // written once, so what it has to carry is the union over the pages -- and
    // it has to be known BEFORE the first page is written, since that is when
    // the font program goes into the file.
    let mut drawn: Vec<(&Font, std::collections::BTreeSet<u8>)> = Vec::new();
    for page in pages {
        for (name, font) in &page.fonts {
            let codes = page.used.get(name).cloned().unwrap_or_default();
            match drawn.iter_mut().find(|(seen, _)| *seen == font) {
                Some((_, all)) => all.extend(codes),
                None => drawn.push((font, codes)),
            }
        }
    }
    let codes_of = |font: &Font| {
        drawn
            .iter()
            .find(|(seen, _)| *seen == font)
            .map(|(_, codes)| codes.clone())
            .unwrap_or_default()
    };

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
                    let object = add_font(&mut pdf, font, &codes_of(font));
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
        let mut resources = BTreeMap::from([("Font".to_string(), Object::Dict(fonts))]);
        if !images.is_empty() {
            resources.insert("XObject".to_string(), Object::Dict(images));
        }
        // The graphics states the content named. Built up rather than matched
        // on, because a page may have any combination of the three.
        if !page.ext_gstates.is_empty() {
            let states: BTreeMap<String, Object> = page
                .ext_gstates
                .iter()
                .map(|(name, key, value)| {
                    (
                        name.clone(),
                        Object::Dict(BTreeMap::from([(key.clone(), Object::Real(*value))])),
                    )
                })
                .collect();
            resources.insert("ExtGState".to_string(), Object::Dict(states));
        }
        let resources = Object::Dict(resources);
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

/// The six letters a subsetted font's name begins with (S9.6.4).
///
/// "The tag shall consist of exactly six uppercase letters; the choice of
/// letters is arbitrary, but different subsets in the same PDF file shall have
/// different tags." So it is a hash of what the subset HOLDS: two subsets of
/// the same face with the same glyphs are the same font and may share a tag,
/// and two that differ in one glyph differ in their tags. FNV-1a, because a
/// hash whose value is fixed by the standard is one that gives the same file
/// twice for the same document.
fn subset_tag(name: &str, glyphs: impl IntoIterator<Item = u16>) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |byte: u8| {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    for byte in name.bytes() {
        eat(byte);
    }
    for glyph in glyphs {
        eat((glyph >> 8) as u8);
        eat((glyph & 0xFF) as u8);
    }
    (0..6)
        .map(|i| (b'A' + ((hash >> (i * 5)) % 26) as u8) as char)
        .collect()
}

/// A TrueType program cut down to the codes a document drew, and the tag to
/// name it by.
///
/// The codes are WinAnsi -- that is the encoding these fonts are written with
/// -- and the face's `cmap` is indexed by Unicode, so the two are joined by
/// what a code MEANS, exactly as `embed_file` joined them to find the widths.
///
/// `None` when the font cannot be cut: a face whose `glyf` or `loca` will not
/// read is one to embed whole rather than to embed wrongly.
fn cut_to_codes(
    name: &str,
    bytes: &[u8],
    codes: &std::collections::BTreeSet<u8>,
) -> Option<(String, Vec<u8>)> {
    let sfnt = crate::sfnt::Sfnt::parse(bytes.to_vec()).ok()?;
    let cmap = sfnt.cmap().ok()?;
    let keep: std::collections::BTreeSet<u16> = codes
        .iter()
        .filter_map(|code| crate::typeset::winansi_unicode(*code))
        .filter_map(|ch| cmap.get(&(ch as u32)).copied())
        .collect();
    // A font nothing was drawn in is one this cannot measure the use of, and
    // carrying it whole is what it did before there was a subset at all.
    if keep.is_empty() {
        return None;
    }
    let cut = sfnt.subset_encoded(&keep).ok()?;
    Some((subset_tag(name, keep.iter().copied()), cut))
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
///
/// `codes` is what the document actually drew in this font, which is what says
/// how much of it has to go into the file.
fn add_font(pdf: &mut Pdf, font: &Font, codes: &std::collections::BTreeSet<u8>) -> Object {
    if let Font::Glyphs {
        name,
        bytes,
        glyphs,
        bbox,
        ascent,
        descent,
    } = font
    {
        // This program was cut down to `glyphs` when it was loaded, so the
        // name it goes into the file under is a subset's name: the tag says so,
        // and says which subset, so a reader never takes two different cuts of
        // one face for the same font.
        let name = &format!(
            "{}+{name}",
            subset_tag(name, glyphs.iter().map(|(gid, _, _)| *gid))
        );
        let file = pdf.add(Object::Stream {
            dict: BTreeMap::from([("Length1".to_string(), Object::Integer(bytes.len() as i64))]),
            data: bytes.clone(),
        });
        let descriptor = pdf.add(Object::dict([
            ("Type", Object::name("FontDescriptor")),
            ("FontName", Object::name(name)),
            // 4 is the symbolic flag: a code here is a glyph id and not a
            // character in any standard encoding, which is what symbolic means.
            ("Flags", Object::Integer(4)),
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
        // `/W` (S9.7.4.3): a CID, then the widths of the CIDs running from it.
        // Written one CID to an entry rather than in runs, because the glyphs a
        // document borrows from a fallback are scattered across the face and a
        // run of consecutive ids is the exception.
        let widths: Vec<Object> = glyphs
            .iter()
            .flat_map(|(gid, _, w)| {
                [
                    Object::Integer(i64::from(*gid)),
                    Object::Array(vec![Object::Integer(*w)]),
                ]
            })
            .collect();
        let descendant = pdf.add(Object::dict([
            ("Type", Object::name("Font")),
            ("Subtype", Object::name("CIDFontType2")),
            ("BaseFont", Object::name(name)),
            (
                "CIDSystemInfo",
                Object::dict([
                    ("Registry", Object::string("Adobe")),
                    ("Ordering", Object::string("Identity")),
                    ("Supplement", Object::Integer(0)),
                ]),
            ),
            ("FontDescriptor", descriptor),
            ("DW", Object::Integer(1000)),
            ("W", Object::Array(widths)),
            // Identity: the CID a code names IS the glyph to draw, which is
            // what makes the two-byte code above a glyph id.
            ("CIDToGIDMap", Object::name("Identity")),
        ]));
        let meanings: Vec<(u16, String)> = glyphs
            .iter()
            .map(|(gid, ch, _)| (*gid, ch.to_string()))
            .collect();
        let to_unicode = pdf.add(Object::Stream {
            dict: BTreeMap::new(),
            data: crate::agl::to_unicode_wide(name, &meanings).into_bytes(),
        });
        return pdf.add(Object::dict([
            ("Type", Object::name("Font")),
            ("Subtype", Object::name("Type0")),
            ("BaseFont", Object::name(name)),
            ("Encoding", Object::name("Identity-H")),
            ("DescendantFonts", Object::Array(vec![descendant])),
            ("ToUnicode", to_unicode),
        ]));
    }
    if let Font::TrueType {
        name,
        bytes,
        widths,
        bbox,
        ascent,
        descent,
    } = font
    {
        // The font program, cut to the glyphs the document set. A book sets
        // perhaps two hundred of a face's several thousand, and carrying the
        // rest is half a megabyte a face of outlines nothing ever draws: what
        // `lualatex` writes for these same books is a third the size, and this
        // is most of the difference. A face that will not cut goes in whole,
        // which is what every face did before.
        let (tag, program) = match cut_to_codes(name, bytes, codes) {
            Some((tag, cut)) => (format!("{tag}+"), cut),
            None => (String::new(), bytes.clone()),
        };
        // S9.6.4: a subsetted font is named for its subset, so a reader does
        // not take one cut of a face for another.
        let name = &format!("{tag}{name}");
        let file = pdf.add(Object::Stream {
            dict: BTreeMap::from([("Length1".to_string(), Object::Integer(program.len() as i64))]),
            data: program,
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
    // What the font says about itself, where it says anything. `/ItalicAngle`
    // is in the cleartext header and `/StdVW` in the Private DICT, and both
    // were written as constants here before they were read.
    //
    // Ascent, descent, cap height and x-height are asked for and are in NONE of
    // a Type 1 font: they are in the `.afm` beside it, which `Type1::open`
    // reads. Measured, that is where LuaTeX gets them -- for CMR10 it writes
    // `/Ascent 694 /CapHeight 683 /Descent -194 /XHeight 431`, which is
    // cmr10.afm's `Ascender`, `CapHeight`, `Descender` and `XHeight` to the
    // digit and none of which is anywhere in cmr10.pfb.
    //
    // Without the metrics the bounding box is what there is to say, and it says
    // something else: the box's top and bottom are the extremes of the OUTLINES
    // -- 750 and -250 for CMR10, where the letters reach 694 and -194.
    let heights = type1.afm_metrics;
    let mut entries = vec![
        ("Type", Object::name("FontDescriptor")),
        ("FontName", Object::name(&type1.font_name)),
        ("Flags", Object::Integer(4)),
        (
            "FontBBox",
            Object::Array(bbox.iter().map(|&n| Object::Real(n)).collect()),
        ),
        ("ItalicAngle", Object::Real(type1.italic_angle)),
        (
            "Ascent",
            Object::Real(heights.map(|m| m.ascender).unwrap_or(bbox[3])),
        ),
        (
            "Descent",
            Object::Real(heights.map(|m| m.descender).unwrap_or(bbox[1])),
        ),
        (
            "CapHeight",
            Object::Real(heights.map(|m| m.cap_height).unwrap_or(bbox[3])),
        ),
        // The font's own dominant stem width when it states one. LuaTeX writes
        // 69 whatever the face, which is a default rather than an answer.
        ("StemV", Object::Real(type1.stem_v.unwrap_or(80.0))),
        ("FontFile", file),
    ];
    // Only when it is known: §9.8.1 marks `/XHeight` optional, and a face whose
    // metrics could not be found has nothing to say about it.
    if let Some(metrics) = heights {
        entries.push(("XHeight", Object::Real(metrics.x_height)));
    }
    let descriptor = pdf.add(Object::dict(entries));

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
            "<< /Count 2 /Type /Page >>"
        );
        // Spaced the way LuaTeX spaces a dictionary. The spec allows any
        // whitespace here, so this pins a writer's habit -- and pinning it is
        // the point: two writers with different habits give different bytes for
        // identical content, which the parity ladder would report forever.
        assert_eq!(written(Object::Dict(BTreeMap::new())), "<< >>");
    }

    /// A stream's `Length` is what was written, not what was claimed.
    #[test]
    fn a_stream_carries_the_length_of_what_it_holds() {
        let text = written(Object::Stream {
            dict: BTreeMap::from([("Length".to_string(), Object::Integer(9999))]),
            data: b"12345".to_vec(),
        });
        assert!(text.starts_with("<< /Length 5 >>"), "{text}");
        assert!(text.contains("\nstream\n12345\nendstream"), "{text}");
    }

    /// The cross-reference table is the file's index, and every entry in it
    /// must land where it says -- at a byte offset, or at an index inside the
    /// object stream.
    ///
    /// The three objects here are all packable, so all three go into the
    /// `/ObjStm` and NONE of them has a byte offset any more. What is checked
    /// is the pair of indirections that replaced the one: the table says
    /// "index 2 of object 4", object 4's own header says where index 2 starts,
    /// and the object is there.
    #[test]
    fn every_entry_in_the_table_lands_where_it_says() {
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
        // Object zero, the three that were added, the object stream holding
        // them and the table itself.
        assert!(text.contains("/Size 6"), "{text}");
        // Nothing is written as a numbered object of its own except those two.
        assert_eq!(text.matches(" 0 obj\n").count(), 2, "{text}");

        // Read the table the way a reader does.
        let at = text.rfind("startxref\n").expect("a startxref");
        let start: usize = text[at + 10..]
            .lines()
            .next()
            .expect("an offset")
            .trim()
            .parse()
            .expect("a number");
        assert!(bytes[start..].starts_with(b"5 0 obj\n"), "{text}");
        let dict =
            String::from_utf8_lossy(&bytes[start..(start + 200).min(bytes.len())]).into_owned();
        assert!(dict.contains("/Type /XRef"), "{dict}");
        assert!(dict.contains("/W [1 2 1]"), "{dict}");

        let table = inflated_stream(&bytes, start);
        assert_eq!(table.len(), 6 * 4, "six entries of four bytes");
        let entry = |number: usize| {
            let row = &table[number * 4..number * 4 + 4];
            (
                row[0] as usize,
                (row[1] as usize) << 8 | row[2] as usize,
                row[3] as usize,
            )
        };
        assert_eq!(entry(0), (0, 0, 255), "object zero heads the free list");
        for number in 1..=3 {
            assert_eq!(
                entry(number),
                (2, 4, number - 1),
                "object {number} is not packed where the table says"
            );
        }
        assert_eq!(entry(5), (1, start, 0), "the table's own entry");

        // Object 4 is the object stream, at the offset the table gives, and
        // its header locates each packed object inside it.
        let (kind, offset, _) = entry(4);
        assert_eq!(kind, 1, "an object stream cannot be inside one");
        assert!(bytes[offset..].starts_with(b"4 0 obj\n"), "{text}");
        let head =
            String::from_utf8_lossy(&bytes[offset..(offset + 200).min(bytes.len())]).into_owned();
        assert!(head.contains("/Type /ObjStm"), "{head}");
        assert!(head.contains("/N 3"), "{head}");
        let data = inflated_stream(&bytes, offset);
        let first: usize = head
            .split("/First ")
            .nth(1)
            .expect("a /First")
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .expect("a number");
        let plain = String::from_utf8_lossy(&data).into_owned();
        let pairs: Vec<usize> = plain[..first]
            .split_whitespace()
            .map(|word| word.parse().expect("a number"))
            .collect();
        assert_eq!(pairs.len(), 6, "three objects, each a number and an offset");
        for (i, want) in ["1", "(a longer", "<< /Type /Catalog >>"]
            .iter()
            .enumerate()
        {
            assert_eq!(pairs[i * 2], i + 1, "the objects are in order");
            let at = first + pairs[i * 2 + 1];
            assert!(
                plain[at..].starts_with(want),
                "object {} is not at {at}: {:?}",
                i + 1,
                &plain[at..(at + 24).min(plain.len())]
            );
        }
    }

    /// The inflated data of the stream object beginning at `start`.
    fn inflated_stream(pdf: &[u8], start: usize) -> Vec<u8> {
        let at = find(&pdf[start..], b"stream\n").expect("a stream") + start + 7;
        let end = find(&pdf[at..], b"\nendstream").expect("an endstream") + at;
        let mut out = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&pdf[at..end]), &mut out)
            .expect("the stream inflates");
        out
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

    /// `unpacked` gives back every object the file holds, header and all.
    ///
    /// The point of it is that a reader which walks `N 0 obj ... endobj` and
    /// follows the references between them sees the whole document -- so this
    /// checks the count, and then that a reference INTO the object stream
    /// resolves: the catalogue names the page tree, and the page tree is one of
    /// the objects that came back.
    #[test]
    fn unpacking_gives_back_every_object_the_file_holds() {
        let mut first = Page::letter();
        first.text("Helvetica", 12.0, 72.0, 700.0, "one");
        let mut second = Page::letter();
        second.text("Times-Roman", 12.0, 72.0, 700.0, "two");
        let bytes = document(&[first, second]);

        // The page tree, two content streams, two fonts, two pages and the
        // catalogue are objects 1 to 8; 9 is the object stream that held six of
        // them and 10 is the table. Unpacked, all eight come back and the
        // object stream does not, because what it held stands in its place.
        let plain = String::from_utf8_lossy(&unpacked(&bytes)).into_owned();
        let numbers: Vec<u32> = plain
            .match_indices(" 0 obj\n")
            .filter_map(|(at, _)| plain[..at].rsplit('\n').next()?.parse().ok())
            .collect();
        let mut sorted = numbers.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), numbers.len(), "an object came back twice");
        assert_eq!(sorted, vec![1, 2, 3, 4, 5, 6, 7, 8, 10], "{plain}");

        // The object stream itself is gone: what it held is what stands in its
        // place.
        assert!(!plain.contains("/Type /ObjStm"), "{plain}");
        assert_eq!(plain.matches("/Type /Page ").count(), 2, "{plain}");
        assert_eq!(plain.matches("/Type /Pages").count(), 1, "{plain}");

        // And a reference resolves: the catalogue's /Pages names an object that
        // came back, and that object is the page tree.
        let catalog = plain
            .split("endobj")
            .find(|body| body.contains("/Type /Catalog"))
            .expect("a catalogue");
        let tree = number_after(catalog, "/Pages ").expect("a reference");
        let body = plain
            .split("endobj")
            .find(|body| body.contains(&format!("\n{tree} 0 obj\n")))
            .unwrap_or_else(|| panic!("object {tree} did not come back: {plain}"));
        assert!(body.contains("/Type /Pages"), "{body}");
    }
}
