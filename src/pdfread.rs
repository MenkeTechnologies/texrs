//! Reading a PDF, ported from `pdfparse.c` and the reading half of `pdfobj.c`
//! in tectonic's `xdvipdfmx`.
//!
//! [`crate::pdf`] writes one. This reads one, and the reason a driver needs to
//! is `\includegraphics{figure.pdf}`: the commonest figure in a LaTeX document
//! is another PDF, and putting one page of it on a page of this one means
//! taking the file apart -- its cross-reference, its objects, its streams --
//! and copying what that page depends on across.
//!
//! Two things make a modern PDF harder to read than the one this crate writes.
//! Since PDF 1.5 the cross-reference may itself be a compressed stream rather
//! than the table of twenty-byte lines, and objects may be packed together
//! inside an *object stream*, so an object's location is not a byte offset at
//! all but a place in another object. pdftex writes both, so neither is
//! optional: the file this was first pointed at had 46 of its 50 objects
//! inside object streams.
//!
//! The object model is [`crate::pdf::Object`], the one the writer emits. A
//! reader and a writer that disagreed about what a PDF object is would be two
//! programs; sharing the type is what makes including a page a copy rather
//! than a translation.

use std::collections::BTreeMap;

use crate::pdf::{Dict, Object};

/// Where an object is: at a byte offset, or inside another object.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Where {
    At(usize),
    /// In the object stream numbered `stream`, as its `index`th object.
    In {
        stream: u32,
        index: usize,
    },
}

/// A PDF, read.
pub struct Document {
    bytes: Vec<u8>,
    places: BTreeMap<u32, Where>,
    /// The trailer, which says where the catalogue is.
    pub trailer: Dict,
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("bytes", &self.bytes.len())
            .field("objects", &self.places.len())
            .field(
                "trailer",
                &self.trailer.iter().map(|(key, _)| key).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl Document {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Document, String> {
        let path = path.as_ref();
        let bytes =
            std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Document::read(bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Read a file that is already in memory.
    pub fn read(bytes: Vec<u8>) -> Result<Document, String> {
        if !bytes.starts_with(b"%PDF-") {
            return Err("does not begin with %PDF-".into());
        }
        // §7.5.5: the last thing in the file says where the cross-reference is,
        // and the cross-reference says where everything else is. A PDF is read
        // backwards.
        let tail = bytes.len().saturating_sub(2048);
        let text = String::from_utf8_lossy(&bytes[tail..]).into_owned();
        let at = text
            .rfind("startxref")
            .ok_or("the file does not say where its cross-reference is")?;
        let start: usize = text[at + 9..]
            .split_whitespace()
            .next()
            .and_then(|word| word.parse().ok())
            .ok_or("the startxref is not a number")?;

        let mut document = Document {
            bytes,
            places: BTreeMap::new(),
            trailer: Dict::new(),
        };
        let mut seen = Vec::new();
        let mut next = Some(start);
        // A file that has been edited has several cross-references, each
        // pointing back at the one before it. The newest wins, so what is read
        // first is kept.
        while let Some(at) = next {
            if seen.contains(&at) || at >= document.bytes.len() {
                break;
            }
            seen.push(at);
            next = document.read_section(at)?;
        }
        if document.places.is_empty() {
            return Err("the cross-reference names no objects".into());
        }
        Ok(document)
    }

    /// One cross-reference section, in whichever of the two forms it is, and
    /// where the one before it is.
    fn read_section(&mut self, at: usize) -> Result<Option<usize>, String> {
        let mut lexer = Lexer::new(&self.bytes, at);
        // The old form begins with the word `xref`.
        if lexer.word() == Some("xref".to_string()) {
            return self.read_table(lexer.at);
        }
        // The new form is an object holding a stream.
        let (_, object) = read_indirect(&self.bytes, at)?;
        let Object::Stream { dict, data } = &object else {
            return Err(format!("what is at {at} is neither a table nor a stream"));
        };
        let data = decode(dict, data)?;
        self.read_xref_stream(dict, &data)?;
        for (key, value) in dict.iter() {
            if self.trailer.get(key).is_none() {
                self.trailer.insert(key.clone(), value.clone());
            }
        }
        Ok(dict.get("Prev").and_then(as_usize))
    }

    /// The old form: subsections of twenty-byte lines.
    fn read_table(&mut self, at: usize) -> Result<Option<usize>, String> {
        let mut lexer = Lexer::new(&self.bytes, at);
        loop {
            let mark = lexer.at;
            match lexer.word().as_deref() {
                Some("trailer") => break,
                _ => lexer.at = mark,
            }
            let Some(first) = lexer.number() else { break };
            let Some(count) = lexer.number() else { break };
            for i in 0..count as u32 {
                let Some(offset) = lexer.number() else { break };
                let Some(_generation) = lexer.number() else {
                    break;
                };
                let kind = lexer.word().unwrap_or_default();
                // `n` is in use and `f` is free, and a free entry names no
                // object.
                if kind == "n" {
                    self.places
                        .entry(first as u32 + i)
                        .or_insert(Where::At(offset as usize));
                }
            }
        }
        // Past the word `trailer` is the dictionary.
        let mut lexer = Lexer::new(&self.bytes, lexer.at);
        let trailer = lexer.object().ok_or("the trailer is not a dictionary")?;
        let Object::Dict(trailer) = trailer else {
            return Err("the trailer is not a dictionary".into());
        };
        let previous = trailer.get("Prev").and_then(as_usize);
        // A file may have both forms: a table with an /XRefStm beside it.
        if let Some(hybrid) = trailer.get("XRefStm").and_then(as_usize) {
            self.read_section(hybrid)?;
        }
        for (key, value) in trailer.iter() {
            if self.trailer.get(key).is_none() {
                self.trailer.insert(key.clone(), value.clone());
            }
        }
        Ok(previous)
    }

    /// The new form: §7.5.8, a stream of fixed-width fields whose widths the
    /// dictionary states.
    fn read_xref_stream(&mut self, dict: &Dict, data: &[u8]) -> Result<(), String> {
        let widths: Vec<usize> = match dict.get("W") {
            Some(Object::Array(items)) => items.iter().filter_map(as_usize).collect(),
            _ => return Err("the cross-reference stream states no field widths".into()),
        };
        if widths.len() < 3 {
            return Err(format!("{} field widths is not three", widths.len()));
        }
        let size = dict.get("Size").and_then(as_usize).unwrap_or(0);
        // /Index says which objects the stream describes, in pairs; without
        // one it describes all of them from zero.
        let index: Vec<usize> = match dict.get("Index") {
            Some(Object::Array(items)) => items.iter().filter_map(as_usize).collect(),
            _ => vec![0, size],
        };

        let row = widths.iter().sum::<usize>();
        if row == 0 {
            return Err("the cross-reference stream's rows are empty".into());
        }
        let mut at = 0usize;
        for pair in index.chunks(2) {
            let [first, count] = pair else { break };
            for i in 0..*count {
                if at + row > data.len() {
                    break;
                }
                let mut field = [0u64; 3];
                let mut cursor = at;
                for (which, width) in widths.iter().enumerate().take(3) {
                    field[which] = data[cursor..cursor + width]
                        .iter()
                        .fold(0u64, |value, &b| (value << 8) | b as u64);
                    // §7.5.8.2: a width of zero means the field takes its
                    // default, and the first field's default is 1.
                    if *width == 0 && which == 0 {
                        field[0] = 1;
                    }
                    cursor += width;
                }
                at += row;
                let number = (first + i) as u32;
                match field[0] {
                    1 => {
                        self.places
                            .entry(number)
                            .or_insert(Where::At(field[1] as usize));
                    }
                    2 => {
                        self.places.entry(number).or_insert(Where::In {
                            stream: field[1] as u32,
                            index: field[2] as usize,
                        });
                    }
                    // Type 0 is a free object, which names nothing.
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// How many objects the cross-reference names.
    pub fn len(&self) -> usize {
        self.places.len()
    }

    pub fn is_empty(&self) -> bool {
        self.places.is_empty()
    }

    /// The object with this number.
    pub fn object(&self, number: u32) -> Option<Object> {
        match self.places.get(&number)? {
            Where::At(at) => {
                let (found, object) = read_indirect(&self.bytes, *at).ok()?;
                // An offset that names the wrong object is a file that has
                // been edited badly; better nothing than the wrong thing.
                (found == number).then_some(object)
            }
            Where::In { stream, index } => self.packed_object(*stream, *index),
        }
    }

    /// Follow a reference, however many deep, to something that is not one.
    pub fn resolve(&self, object: &Object) -> Object {
        let mut object = object.clone();
        for _ in 0..32 {
            match object {
                Object::Reference(number) => match self.object(number) {
                    Some(found) => object = found,
                    None => return Object::Null,
                },
                other => return other,
            }
        }
        Object::Null
    }

    /// An object packed inside another, §7.5.7. The stream begins with a table
    /// of object numbers and where each one starts.
    fn packed_object(&self, stream: u32, index: usize) -> Option<Object> {
        let Object::Stream { dict, data } = self.object_at(stream)? else {
            return None;
        };
        let data = decode(&dict, &data).ok()?;
        let count = dict.get("N").and_then(as_usize)?;
        let first = dict.get("First").and_then(as_usize)?;
        if index >= count {
            return None;
        }
        let mut lexer = Lexer::new(&data, 0);
        let mut at = None;
        for i in 0..count {
            let _number = lexer.number()?;
            let offset = lexer.number()?;
            if i == index {
                at = Some(first + offset as usize);
            }
        }
        let mut lexer = Lexer::new(&data, at?);
        lexer.object()
    }

    /// The object at a number, without going through an object stream -- which
    /// is what reading an object stream itself needs.
    fn object_at(&self, number: u32) -> Option<Object> {
        match self.places.get(&number)? {
            Where::At(at) => read_indirect(&self.bytes, *at).ok().map(|(_, it)| it),
            Where::In { .. } => None,
        }
    }

    /// The catalogue, which is where a reader starts.
    pub fn catalog(&self) -> Option<Dict> {
        match self.resolve(self.trailer.get("Root")?) {
            Object::Dict(dict) => Some(dict),
            _ => None,
        }
    }

    /// Every page, in order, as its dictionary.
    ///
    /// The pages are a tree, and a page inherits what its parents say: a
    /// `/MediaBox` on the root is the size of every page under it, which is how
    /// a document of two hundred pages states its size once.
    pub fn pages(&self) -> Vec<Dict> {
        let mut out = Vec::new();
        let Some(catalog) = self.catalog() else {
            return out;
        };
        let Some(root) = catalog.get("Pages") else {
            return out;
        };
        self.walk(root, &Dict::new(), &mut out, 0);
        out
    }

    fn walk(&self, node: &Object, inherited: &Dict, out: &mut Vec<Dict>, depth: usize) {
        if depth > 32 || out.len() > 10_000 {
            return;
        }
        let Object::Dict(dict) = self.resolve(node) else {
            return;
        };
        let mut carried = inherited.clone();
        // §7.7.3.4: these four are inherited by everything below.
        for key in ["Resources", "MediaBox", "CropBox", "Rotate"] {
            if let Some(value) = dict.get(key) {
                carried.insert(key.to_string(), value.clone());
            }
        }
        match dict.get("Kids") {
            Some(kids) => {
                if let Object::Array(kids) = self.resolve(kids) {
                    for kid in kids {
                        self.walk(&kid, &carried, out, depth + 1);
                    }
                }
            }
            None => {
                let mut page = dict.clone();
                for (key, value) in carried.iter() {
                    if page.get(key).is_none() {
                        page.insert(key.clone(), value.clone());
                    }
                }
                out.push(page);
            }
        }
    }

    /// A page's content, with its streams joined -- a page may carry several,
    /// and they are one stream cut into pieces wherever the writer felt like
    /// it.
    pub fn content(&self, page: &Dict) -> Result<Vec<u8>, String> {
        let Some(contents) = page.get("Contents") else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        let streams = match self.resolve(contents) {
            Object::Array(items) => items,
            other => vec![other],
        };
        for stream in streams {
            if let Object::Stream { dict, data } = self.resolve(&stream) {
                out.extend(decode(&dict, &data)?);
                out.push(b'\n');
            }
        }
        Ok(out)
    }

    /// A summary a person reads.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        let version = String::from_utf8_lossy(&self.bytes[..8.min(self.bytes.len())]).to_string();
        out.push_str(&format!(
            "version       {}\n",
            version.trim_start_matches('%')
        ));
        out.push_str(&format!("objects       {}\n", self.places.len()));
        let packed = self
            .places
            .values()
            .filter(|place| matches!(place, Where::In { .. }))
            .count();
        out.push_str(&format!("in streams    {packed}\n"));
        let pages = self.pages();
        out.push_str(&format!("pages         {}\n", pages.len()));
        for (number, page) in pages.iter().enumerate() {
            let size = match page.get("MediaBox").map(|it| self.resolve(it)) {
                Some(Object::Array(items)) => items
                    .iter()
                    .map(|item| match item {
                        Object::Integer(value) => value.to_string(),
                        Object::Real(value) => format!("{value}"),
                        _ => "?".into(),
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => "unstated".into(),
            };
            let content = self.content(page).map(|it| it.len()).unwrap_or(0);
            out.push_str(&format!(
                "  page {}     {size}, {content} bytes of content\n",
                number + 1
            ));
        }
        out
    }
}

/// `n g obj … endobj` at `at`, and which object it turned out to be.
fn read_indirect(bytes: &[u8], at: usize) -> Result<(u32, Object), String> {
    let mut lexer = Lexer::new(bytes, at);
    let number = lexer.number().ok_or("no object number")? as u32;
    let _generation = lexer.number().ok_or("no generation")?;
    match lexer.word().as_deref() {
        Some("obj") => {}
        other => return Err(format!("{other:?} is not obj")),
    }
    let object = lexer.object().ok_or("no object")?;
    // A dictionary followed by `stream` is a stream, and its length may be a
    // reference -- which cannot be followed from here, so a length that is not
    // a number is found by looking for `endstream`.
    // A dictionary followed by the word `stream` is a stream; anything else
    // means the object was the dictionary and the word belongs to whatever
    // comes next, so nothing is consumed.
    let mark = lexer.at;
    let is_stream = lexer.word().as_deref() == Some("stream");
    if !is_stream {
        lexer.at = mark;
    }
    if is_stream {
        let Object::Dict(dict) = object else {
            return Err("a stream whose dictionary is not one".into());
        };
        // §7.3.8.1: the keyword is followed by a newline, optionally after a
        // carriage return, and the data begins after it.
        let mut start = lexer.at;
        if bytes.get(start) == Some(&b'\r') {
            start += 1;
        }
        if bytes.get(start) == Some(&b'\n') {
            start += 1;
        }
        let length = match dict.get("Length").and_then(as_usize) {
            Some(length) if start + length <= bytes.len() => length,
            _ => find(bytes, b"endstream", start)
                .map(|end| end.saturating_sub(start))
                .ok_or("a stream with no end")?,
        };
        let data = bytes[start..start + length].to_vec();
        return Ok((number, Object::Stream { dict, data }));
    }
    Ok((number, object))
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (from..=haystack.len() - needle.len()).find(|&at| &haystack[at..at + needle.len()] == needle)
}

fn as_usize(object: &Object) -> Option<usize> {
    match object {
        Object::Integer(value) => usize::try_from(*value).ok(),
        Object::Real(value) => Some(*value as usize),
        _ => None,
    }
}

/// A stream's bytes, with whatever filter it names undone.
///
/// Only Flate is undone, which is what every PDF this reads uses for anything
/// a driver has to look inside. A picture's own compression is left alone: it
/// is copied across as it is, which is the point of [`crate::image`].
pub fn decode(dict: &Dict, data: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Read;

    let filters: Vec<String> = match dict.get("Filter") {
        Some(Object::Name(name)) => vec![name.clone()],
        Some(Object::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                Object::Name(name) => Some(name.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    if filters.is_empty() {
        return Ok(data.to_vec());
    }
    if filters.len() > 1 || filters[0] != "FlateDecode" {
        return Err(format!(
            "{} is not a filter this undoes",
            filters.join(" and ")
        ));
    }

    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .map_err(|e| format!("the stream does not inflate: {e}"))?;

    // A predictor means the bytes were made easier to compress by writing each
    // one as a difference from the one above, which has to be undone. This is
    // PNG's scheme, and a cross-reference stream is nearly always written with
    // it.
    let Some(Object::Dict(parms)) = dict.get("DecodeParms").cloned() else {
        return Ok(out);
    };
    let predictor = parms.get("Predictor").and_then(as_usize).unwrap_or(1);
    if predictor < 2 {
        return Ok(out);
    }
    let colours = parms.get("Colors").and_then(as_usize).unwrap_or(1);
    let bits = parms
        .get("BitsPerComponent")
        .and_then(as_usize)
        .unwrap_or(8);
    let columns = parms.get("Columns").and_then(as_usize).unwrap_or(1);
    let pixel = (colours * bits).div_ceil(8).max(1);
    let row = (columns * colours * bits).div_ceil(8);

    let mut undone = Vec::with_capacity(out.len());
    let mut previous = vec![0u8; row];
    let mut at = 0usize;
    while at + 1 + row <= out.len() + 1 && at < out.len() {
        let filter = out[at];
        let end = (at + 1 + row).min(out.len());
        let mut current = out[at + 1..end].to_vec();
        current.resize(row, 0);
        crate::image::unfilter_row(filter, &mut current, &previous, pixel)?;
        undone.extend_from_slice(&current);
        previous = current;
        at = end;
    }
    Ok(undone)
}

/// A PDF's tokens, §7.2.
struct Lexer<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Lexer<'a> {
    fn new(bytes: &'a [u8], at: usize) -> Lexer<'a> {
        Lexer { bytes, at }
    }

    /// Past whitespace and comments, which run to the end of a line.
    fn skip(&mut self) {
        while let Some(&byte) = self.bytes.get(self.at) {
            match byte {
                b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0' => self.at += 1,
                b'%' => {
                    while self
                        .bytes
                        .get(self.at)
                        .is_some_and(|&b| b != b'\n' && b != b'\r')
                    {
                        self.at += 1;
                    }
                }
                _ => return,
            }
        }
    }

    fn word(&mut self) -> Option<String> {
        self.skip();
        let start = self.at;
        while self
            .bytes
            .get(self.at)
            .is_some_and(|&b| b.is_ascii_alphabetic())
        {
            self.at += 1;
        }
        (start < self.at).then(|| String::from_utf8_lossy(&self.bytes[start..self.at]).into_owned())
    }

    fn number(&mut self) -> Option<f64> {
        self.skip();
        let start = self.at;
        while self
            .bytes
            .get(self.at)
            .is_some_and(|&b| b.is_ascii_digit() || b == b'+' || b == b'-' || b == b'.')
        {
            self.at += 1;
        }
        String::from_utf8_lossy(&self.bytes[start..self.at])
            .parse()
            .ok()
    }

    /// One object, whatever it is.
    fn object(&mut self) -> Option<Object> {
        self.skip();
        match *self.bytes.get(self.at)? {
            b'<' if self.bytes.get(self.at + 1) == Some(&b'<') => self.dictionary(),
            b'<' => self.hex_string(),
            b'(' => self.string(),
            b'/' => self.name(),
            b'[' => {
                self.at += 1;
                let mut items = Vec::new();
                loop {
                    self.skip();
                    match self.bytes.get(self.at) {
                        Some(b']') => {
                            self.at += 1;
                            return Some(Object::Array(items));
                        }
                        None => return Some(Object::Array(items)),
                        _ => items.push(self.object()?),
                    }
                }
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => self.number_or_reference(),
            _ => match self.word()?.as_str() {
                "true" => Some(Object::Boolean(true)),
                "false" => Some(Object::Boolean(false)),
                "null" => Some(Object::Null),
                // Anything else is a keyword this does not read as an object:
                // `endobj`, `stream`, an operator in a content stream.
                _ => None,
            },
        }
    }

    /// A number, or the `n g R` that is a reference to another object.
    fn number_or_reference(&mut self) -> Option<Object> {
        let value = self.number()?;
        let mark = self.at;
        if value >= 0.0 && value.fract() == 0.0 {
            if let Some(generation) = self.number() {
                if generation.fract() == 0.0 && self.word().as_deref() == Some("R") {
                    return Some(Object::Reference(value as u32));
                }
            }
        }
        self.at = mark;
        Some(match value.fract() == 0.0 {
            true => Object::Integer(value as i64),
            false => Object::Real(value),
        })
    }

    fn name(&mut self) -> Option<Object> {
        self.at += 1;
        let mut out = String::new();
        while let Some(&byte) = self.bytes.get(self.at) {
            match byte {
                // §7.3.5: a `#` and two hexadecimal digits are one character.
                b'#' => {
                    let text = String::from_utf8_lossy(self.bytes.get(self.at + 1..self.at + 3)?)
                        .into_owned();
                    out.push(u8::from_str_radix(&text, 16).ok()? as char);
                    self.at += 3;
                }
                b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0' | b'/' | b'<' | b'>' | b'['
                | b']' | b'(' | b')' | b'%' => break,
                other => {
                    out.push(other as char);
                    self.at += 1;
                }
            }
        }
        Some(Object::Name(out))
    }

    /// A string in parentheses, which may hold parentheses of its own so long
    /// as they balance.
    fn string(&mut self) -> Option<Object> {
        self.at += 1;
        let mut out = String::new();
        let mut depth = 1usize;
        while let Some(&byte) = self.bytes.get(self.at) {
            self.at += 1;
            match byte {
                b'\\' => {
                    let escaped = *self.bytes.get(self.at)?;
                    self.at += 1;
                    out.push(match escaped {
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        // A backslash and up to three octal digits are one
                        // character.
                        b'0'..=b'7' => {
                            let mut value = (escaped - b'0') as u32;
                            for _ in 0..2 {
                                match self.bytes.get(self.at) {
                                    Some(&digit @ b'0'..=b'7') => {
                                        value = value * 8 + (digit - b'0') as u32;
                                        self.at += 1;
                                    }
                                    _ => break,
                                }
                            }
                            char::from_u32(value)?
                        }
                        // A backslash at the end of a line joins it to the
                        // next.
                        b'\n' => continue,
                        other => other as char,
                    });
                }
                b'(' => {
                    depth += 1;
                    out.push('(');
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(Object::Str(out));
                    }
                    out.push(')');
                }
                other => out.push(other as char),
            }
        }
        Some(Object::Str(out))
    }

    fn hex_string(&mut self) -> Option<Object> {
        self.at += 1;
        let mut digits = String::new();
        while let Some(&byte) = self.bytes.get(self.at) {
            self.at += 1;
            match byte {
                b'>' => break,
                b if b.is_ascii_hexdigit() => digits.push(b as char),
                _ => {}
            }
        }
        // §7.3.4.3: an odd number of digits is padded with a zero.
        if digits.len() % 2 == 1 {
            digits.push('0');
        }
        let mut out = String::new();
        for pair in digits.as_bytes().chunks(2) {
            let text = String::from_utf8_lossy(pair).into_owned();
            out.push(u8::from_str_radix(&text, 16).ok()? as char);
        }
        Some(Object::Str(out))
    }

    fn dictionary(&mut self) -> Option<Object> {
        self.at += 2;
        let mut out = Dict::new();
        loop {
            self.skip();
            match self.bytes.get(self.at) {
                Some(b'>') if self.bytes.get(self.at + 1) == Some(&b'>') => {
                    self.at += 2;
                    return Some(Object::Dict(out));
                }
                Some(b'/') => {
                    let Some(Object::Name(key)) = self.name() else {
                        return Some(Object::Dict(out));
                    };
                    let value = self.object()?;
                    out.insert(key, value);
                }
                // Anything else in a dictionary is a file that is wrong; what
                // was read is still worth having.
                _ => return Some(Object::Dict(out)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PDF built here, byte by byte, to reach what a real file does not.
    ///
    /// The files a producer writes are all alike: pdftex never writes an octal
    /// escape, Ghostscript never writes a hex string with an odd number of
    /// digits, and neither writes a cross-reference with a predictor on it.
    /// Those paths exist in the format and so must be read, which means
    /// writing the files that hold them.
    fn pdf_of(body: &str) -> Vec<u8> {
        // One object, one page, and a table pointing at them, laid out here so
        // the offsets are real.
        let mut out = String::from("%PDF-1.7\n");
        let mut offsets = Vec::new();
        let objects = [
            format!("<< /Type /Catalog /Pages 2 0 R /Marker {body} >>"),
            "<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] >>".to_string(),
        ];
        for (i, object) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.push_str(&format!("{} 0 obj\n{object}\nendobj\n", i + 1));
        }
        let xref = out.len();
        out.push_str(&format!(
            "xref\n0 {}\n0000000000 65535 f \n",
            objects.len() + 1
        ));
        for offset in &offsets {
            out.push_str(&format!("{offset:010} 00000 n \n"));
        }
        out.push_str(&format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        ));
        out.into_bytes()
    }

    /// What the marker in that file came out as.
    fn marker(body: &str) -> Object {
        let document = Document::read(pdf_of(body)).expect("the pdf reads");
        document
            .catalog()
            .expect("a catalogue")
            .get("Marker")
            .cloned()
            .expect("the marker")
    }

    #[test]
    fn a_string_is_read_with_its_escapes_undone() {
        assert_eq!(marker("(plain)"), Object::Str("plain".into()));
        // Parentheses nest, so long as they balance.
        assert_eq!(marker("(a (b) c)"), Object::Str("a (b) c".into()));
        // A backslash escapes the next character, and \n is a newline.
        assert_eq!(marker("(a\\)b)"), Object::Str("a)b".into()));
        assert_eq!(marker("(one\\ntwo)"), Object::Str("one\ntwo".into()));
        // §7.3.4.2: a backslash and up to three octal digits are one
        // character, which is how a PDF writes a byte it would rather not
        // write plainly.
        assert_eq!(marker("(\\101\\102\\103)"), Object::Str("ABC".into()));
        assert_eq!(
            marker("(\\0601)"),
            Object::Str("01".into()),
            "three digits, then a 1"
        );
        // A backslash at the end of a line joins it to the next.
        assert_eq!(marker("(one\\\ntwo)"), Object::Str("onetwo".into()));
    }

    #[test]
    fn a_hex_string_is_read_in_pairs() {
        assert_eq!(marker("<414243>"), Object::Str("ABC".into()));
        // §7.3.4.3: an odd number of digits is padded with a zero, so <4> is
        // 0x40 and not 0x04.
        assert_eq!(marker("<414>"), Object::Str("A@".into()));
        // Whitespace inside a hex string is nothing.
        assert_eq!(marker("<41 42\n43>"), Object::Str("ABC".into()));
    }

    #[test]
    fn a_name_may_hold_anything_if_it_is_written_in_hexadecimal() {
        assert_eq!(marker("/Simple"), Object::Name("Simple".into()));
        // §7.3.5: `#` and two digits are one character, which is the only way
        // to put a space or a slash in a name.
        assert_eq!(marker("/A#20B"), Object::Name("A B".into()));
        assert_eq!(marker("/A#2FB"), Object::Name("A/B".into()));
        assert_eq!(marker("/#48ello"), Object::Name("Hello".into()));
    }

    /// A cross-reference stream with a predictor on it, which is how a file
    /// that has been through a tool that compresses well arrives.
    #[test]
    fn a_cross_reference_with_a_predictor_is_undone() {
        use std::io::Write;

        // Three objects, laid out first so their offsets are known.
        let mut body = String::from("%PDF-1.5\n");
        let mut offsets = Vec::new();
        for (number, object) in [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Count 1 /Kids [3 0 R] >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] >>",
        ]
        .iter()
        .enumerate()
        {
            offsets.push(body.len());
            body.push_str(&format!("{} 0 obj\n{object}\nendobj\n", number + 1));
        }

        // The rows: type, offset in two bytes, generation. The free object
        // first, then the three, then the cross-reference stream itself.
        let xref_at = body.len();
        let rows: Vec<[u8; 4]> = std::iter::once([0, 0, 0, 255])
            .chain(offsets.iter().map(|&at| [1, (at >> 8) as u8, at as u8, 0]))
            .chain(std::iter::once([1, (xref_at >> 8) as u8, xref_at as u8, 0]))
            .collect();

        // Filtered with PNG's `up`, which is predictor 12: each byte is the
        // difference from the byte above it.
        let mut filtered = Vec::new();
        let mut previous = [0u8; 4];
        for row in &rows {
            filtered.push(2);
            for (i, byte) in row.iter().enumerate() {
                filtered.push(byte.wrapping_sub(previous[i]));
            }
            previous = *row;
        }
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&filtered).expect("deflate");
        let data = encoder.finish().expect("deflate");

        let mut out = body.into_bytes();
        out.extend(
            format!(
                "4 0 obj\n<< /Type /XRef /Size 5 /W [1 2 1] /Root 1 0 R \
                 /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 4 >> \
                 /Length {} >>\nstream\n",
                data.len()
            )
            .into_bytes(),
        );
        out.extend(&data);
        out.extend(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").into_bytes());

        let document = Document::read(out).expect("the pdf reads");
        assert_eq!(document.len(), 4, "{}", document.summary());
        assert_eq!(document.pages().len(), 1, "{}", document.summary());
        assert_eq!(
            document.catalog().expect("a catalogue").get("Type"),
            Some(&Object::Name("Catalog".into()))
        );
    }

    /// A field the stream leaves out takes its default, and the default of the
    /// first field is 1 -- an object at an offset. A file whose every object
    /// is at an offset need not say so five hundred times.
    #[test]
    fn a_field_left_out_of_the_cross_reference_takes_its_default() {
        use std::io::Write;

        let mut body = String::from("%PDF-1.5\n");
        let mut offsets = Vec::new();
        for (number, object) in [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Count 1 /Kids [3 0 R] >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] >>",
        ]
        .iter()
        .enumerate()
        {
            offsets.push(body.len());
            body.push_str(&format!("{} 0 obj\n{object}\nendobj\n", number + 1));
        }
        let xref_at = body.len();

        // /W [0 2 0]: no type field and no generation, so every entry is an
        // object at an offset.
        let mut rows = vec![0u8, 0];
        for at in offsets.iter().chain(std::iter::once(&xref_at)) {
            rows.extend([(at >> 8) as u8, *at as u8]);
        }
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&rows).expect("deflate");
        let data = encoder.finish().expect("deflate");

        let mut out = body.into_bytes();
        out.extend(
            format!(
                "4 0 obj\n<< /Type /XRef /Size 5 /W [0 2 0] /Root 1 0 R \
                 /Filter /FlateDecode /Length {} >>\nstream\n",
                data.len()
            )
            .into_bytes(),
        );
        out.extend(&data);
        out.extend(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").into_bytes());

        let document = Document::read(out).expect("the pdf reads");
        assert_eq!(document.pages().len(), 1, "{}", document.summary());
    }
}
