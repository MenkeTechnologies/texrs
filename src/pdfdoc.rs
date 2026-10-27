//! The file a document's pages go into, laid out as LuaTeX lays it out: a port
//! of the object machinery of `pdfgen.c` (`pdf_create_obj`, `pdf_begin_obj`,
//! the object streams, `pdf_dict_add_streaminfo`, `pdf_finish_file`), of
//! `pdfpagetree.c`, and of the order `writefont.c` writes a font's objects in.
//!
//! LuaTeX numbers an object when something first needs to NAME it and writes
//! it when it is complete, and the two orders differ. Measured on `Hello
//! world.`: the page's resources are 1, the page 2 and its content 3 (all
//! numbered as the page begins), the font 4 (when the first glyph needs it),
//! the page tree 5 (as the page ends), the object stream 6 (when the first
//! object is packed into it, which is the page), and the widths 7, the
//! descriptor 8 and the font program 9 when the fonts are written at the end;
//! then the catalogue 10, the information dictionary 11 and the cross
//! reference stream 12. In the file: the content stream, the font program,
//! the information dictionary (which LuaTeX never packs), the object stream
//! holding 2 1 7 8 4 5 10 in that order, and the cross reference stream.
//!
//! What goes INTO the objects -- the content of a page, the font program --
//! is decided elsewhere; this decides where each object goes, what it is
//! numbered, and how its dictionary and stream are spelled, down to the
//! padding LuaTeX leaves after a stream's `/Length` when it seeks back to
//! write the number in.

use crate::pdf::{Dict, Font, Object, Page};
use std::collections::{BTreeMap, BTreeSet};

/// The `\pdfvariable`s the writer reads, at the values LuaTeX's own format
/// sets (`pdftexconfig.tex`, read by `luatexconfig.tex` when `luatex.fmt` and
/// `lualatex.fmt` are made): compression at 9, object streams at 2, three
/// decimal digits, PDF 1.7, no ToUnicode maps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Variables {
    /// `compresslevel`: zlib's level for every stream; 0 writes them plain.
    pub compress_level: u32,
    /// `objcompresslevel`: an object goes into an object stream when this is
    /// at least the threshold its writer asks for (`OBJSTM_ALWAYS` is 1, the
    /// information dictionary's 3).
    pub objcompress_level: u32,
    /// `decimaldigits`: how many places of a big point a position keeps.
    pub decimal_digits: u32,
    /// `gentounicode`: whether a font gets a `/ToUnicode` map.
    pub gen_tounicode: bool,
    pub major_version: u32,
    pub minor_version: u32,
}

impl Default for Variables {
    fn default() -> Self {
        Variables {
            compress_level: 9,
            objcompress_level: 2,
            decimal_digits: 3,
            gen_tounicode: false,
            major_version: 1,
            minor_version: 7,
        }
    }
}

/// `OBJSTM_ALWAYS` and the information dictionary's threshold
/// (`pdf_print_info` calls `pdf_begin_obj(pdf, k, 3)`).
const OBJSTM_ALWAYS: u32 = 1;
const INFO_THRESHOLD: u32 = 3;
/// `PDF_OS_MAX_OBJS` (`pdfgen.h`): an object stream is written out when it
/// holds this many.
const OS_MAX_OBJS: usize = 100;
/// `PAGES_TREE_KIDSMAX` (`pdfpagetree.c`).
const PAGES_KIDS_MAX: usize = 10;

/// Deflate `data` with zlib itself at `level` -- `write_zip` in `pdfgen.c`,
/// which feeds `deflate` with the default window and memory level. The
/// output is the same however the input is split, so one call is the same
/// bytes as LuaTeX's buffer-sized ones.
pub fn deflate_at(level: u32, data: &[u8]) -> Vec<u8> {
    use libz_sys::{compress2, compressBound, uLong, Z_OK};
    let mut size: uLong = unsafe { compressBound(data.len() as uLong) };
    let mut out = vec![0u8; size as usize];
    // SAFETY: `out` has the room `compressBound` says the worst case needs,
    // `size` says so going in and holds the length written coming out, and
    // `data` is read for exactly its length.
    let status = unsafe {
        compress2(
            out.as_mut_ptr(),
            &mut size,
            data.as_ptr(),
            data.len() as uLong,
            level.min(9) as i32,
        )
    };
    assert_eq!(status, Z_OK, "zlib could not compress into its own bound");
    out.truncate(size as usize);
    out
}

/// MD5, which both `make_subset_tag` and `print_ID` are defined by.
fn md5(parts: &[&[u8]]) -> [u8; 16] {
    use md5::Digest as _;
    let mut d = md5::Md5::new();
    for part in parts {
        d.update(part);
    }
    d.finalize().into()
}

/// `make_subset_tag` (`utils.c`) for a simple font: MD5 over the glyph names
/// in `strcmp` order, each followed by a space, then the font's name and the
/// collision counter as a C `int`, and six letters folded out of the digest.
///
/// Measured: luatex 1.24.0 names the subset of CMR10 that draws `Hello
/// world.` and its folio `KJJYRX`.
pub fn subset_tag(glyphs: &BTreeSet<String>, font_name: &str) -> String {
    let mut names = Vec::new();
    for glyph in glyphs {
        names.extend_from_slice(glyph.as_bytes());
        names.push(b' ');
    }
    let round = 0i32.to_le_bytes();
    let digest = md5(&[&names, font_name.as_bytes(), &round]);
    let mut a = [0i32; 6];
    a[0] = digest[..13].iter().map(|&b| i32::from(b)).sum();
    for i in 1..6 {
        a[i] = a[i - 1] - i32::from(digest[i - 1]) + i32::from(digest[(i + 12) % 16]);
    }
    a.iter().map(|&x| (b'A' + (x % 26) as u8) as char).collect()
}

/// `print_ID`'s system-provided identifier: MD5 of the start time as
/// `%Y%m%dT%H%M%SZ` in UT, the working directory, a slash and the output
/// file's name -- both halves of `/ID` the same.
pub fn trailer_id(epoch: i64, cwd: &str, file_name: &str) -> [u8; 16] {
    let (days, secs) = (epoch.div_euclid(86_400), epoch.rem_euclid(86_400));
    let (y, mo, d) = crate::pdf::civil(days);
    let time = format!(
        "{y:04}{mo:02}{d:02}T{:02}{:02}{:02}Z",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60
    );
    md5(&[time.as_bytes(), cwd.as_bytes(), b"/", file_name.as_bytes()])
}

/// Where an object went.
#[derive(Clone, Copy, Debug)]
enum Place {
    /// Numbered and not yet written (`set_obj_fresh`).
    Fresh,
    /// At this byte offset in the file.
    File(usize),
    /// At this index of that object stream.
    Packed { stream: u32, index: usize },
}

/// The object stream being filled (`os_struct`).
struct ObjStm {
    number: u32,
    body: Vec<u8>,
    /// Each object's number and offset into `body`.
    entries: Vec<(u32, usize)>,
}

/// LuaTeX's object table and output buffer.
pub struct Writer {
    pub vars: Variables,
    out: Vec<u8>,
    /// Indexed by object number; 0 is the head of the free list.
    places: Vec<Place>,
    objstm: Option<ObjStm>,
    /// `pdf.settrailerid`'s `/ID`, written in place of `print_ID`'s.
    pub trailer_id: Option<String>,
    /// The numbers object streams were given (`obj_type_objstm`).
    objstms: BTreeSet<u32>,
}

/// `pdf_dict_add_streaminfo`'s `/Length`: LuaTeX reserves room, writes the
/// rest of the dictionary, and seeks back to write the number, so what is
/// left after `N >>` is spaces: fourteen characters from the number on.
fn length_and_close(len: usize) -> String {
    format!("{:<14}", format!("{len} >>"))
}

impl Writer {
    /// A file begun: `%PDF-1.N` and the comment of high bytes LuaTeX writes
    /// (`LUATEXPDF` with the top bit set).
    pub fn new(vars: Variables) -> Writer {
        let mut out = format!("%PDF-{}.{}\n", vars.major_version, vars.minor_version).into_bytes();
        out.extend([
            b'%', 0xcc, 0xd5, 0xc1, 0xd4, 0xc5, 0xd8, 0xd0, 0xc4, 0xc6, b'\n',
        ]);
        Writer {
            vars,
            out,
            places: vec![Place::Fresh],
            objstm: None,
            trailer_id: None,
            objstms: BTreeSet::new(),
        }
    }

    /// How many numbers have been handed out.
    pub fn count(&self) -> u32 {
        (self.places.len() - 1) as u32
    }

    /// How many of them are written (`is_obj_written`).
    pub fn written(&self) -> u32 {
        self.places
            .iter()
            .skip(1)
            .filter(|p| !matches!(p, Place::Fresh))
            .count() as u32
    }

    /// Whether `n` is an object stream this file opened.
    pub fn is_objstm(&self, n: u32) -> bool {
        self.objstms.contains(&n)
    }

    /// `pdf_create_obj`: the next number.
    pub fn create(&mut self) -> u32 {
        self.places.push(Place::Fresh);
        (self.places.len() - 1) as u32
    }

    /// `pdf_begin_obj` ... `pdf_end_obj` for an object that is not a stream:
    /// into the object stream when `objcompresslevel` reaches `threshold`,
    /// into the file otherwise.
    pub fn object(&mut self, number: u32, threshold: u32, body: &[u8]) {
        if self.vars.objcompress_level >= threshold {
            self.pack(number, body);
        } else {
            self.places[number as usize] = Place::File(self.out.len());
            self.out.extend(format!("{number} 0 obj\n").as_bytes());
            self.out.extend(body);
            self.out.extend(b"\nendobj\n");
        }
    }

    fn pack(&mut self, number: u32, body: &[u8]) {
        if self.objstm.is_none() {
            let stream = self.create();
            self.objstms.insert(stream);
            self.objstm = Some(ObjStm {
                number: stream,
                body: Vec::new(),
                entries: Vec::new(),
            });
        }
        let Some(os) = self.objstm.as_mut() else {
            return;
        };
        let index = os.entries.len();
        os.entries.push((number, os.body.len()));
        os.body.extend(body);
        let stream = os.number;
        self.places[number as usize] = Place::Packed { stream, index };
        // "Adobe Reader seems to need this": a line end after each object but
        // the one that fills the stream, which goes out at once.
        if index + 1 == OS_MAX_OBJS {
            self.flush_objstm();
        } else if let Some(os) = self.objstm.as_mut() {
            os.body.push(b'\n');
        }
    }

    /// A stream object, written into the file now: `keys` is the dictionary
    /// before `/Filter` and `/Length` (already spelled, `""` for none), and the
    /// data is deflated when `compresslevel` asks and `deflate` allows -- a
    /// picture that carries its own filter is copied as it is.
    pub fn stream(&mut self, number: u32, keys: &str, data: &[u8], deflate: bool) {
        let packed;
        let (filter, data) = match deflate && self.vars.compress_level > 0 {
            true => {
                packed = deflate_at(self.vars.compress_level, data);
                (" /Filter /FlateDecode", packed.as_slice())
            }
            false => ("", data),
        };
        self.places[number as usize] = Place::File(self.out.len());
        let space = match keys.is_empty() {
            true => "",
            false => " ",
        };
        self.out.extend(
            format!(
                "{number} 0 obj\n<<{space}{keys}{filter} /Length {}\nstream\n",
                length_and_close(data.len())
            )
            .as_bytes(),
        );
        self.out.extend(data);
        self.out.extend(b"\nendstream\nendobj\n");
    }

    /// `pdf_os_write_objstream`: the pairs of number and offset, ten to a
    /// line, then the objects, as one stream into the file.
    fn flush_objstm(&mut self) {
        let Some(os) = self.objstm.take() else {
            return;
        };
        let mut head = String::new();
        for (i, (number, at)) in os.entries.iter().enumerate() {
            head.push_str(&format!("{number} {at}"));
            let end = i % 10 == 9 || i + 1 == os.entries.len();
            head.push(if end { '\n' } else { ' ' });
        }
        let first = head.len();
        let mut data = head.into_bytes();
        data.extend(&os.body);
        let keys = format!("/Type /ObjStm /N {} /First {first}", os.entries.len());
        self.stream(os.number, &keys, &data, true);
    }

    /// `pdf_finish_file`'s tail: the last object stream, then the cross
    /// reference stream, which is the trailer.
    pub fn finish(mut self, root: u32, info: Option<u32>, trailer: &str, id: [u8; 16]) -> Vec<u8> {
        self.flush_objstm();
        let xref = self.create();
        let xref_offset = self.out.len();
        self.places[xref as usize] = Place::File(xref_offset);
        // The second field is as wide as the offset of the last object needs.
        let width = match xref_offset {
            o if o / 256 > 16_777_215 => 5,
            o if o > 16_777_215 => 4,
            o if o > 65_535 => 3,
            _ => 2,
        };
        // `build_free_object_list`: every number never written, chained.
        let mut next_free = vec![0usize; self.places.len()];
        let mut last = 0usize;
        for (k, place) in self.places.iter().enumerate().skip(1) {
            if matches!(place, Place::Fresh) {
                next_free[last] = k;
                last = k;
            }
        }
        next_free[last] = 0;
        let mut rows = Vec::with_capacity(self.places.len() * (2 + width));
        let put = |rows: &mut Vec<u8>, value: usize| {
            for shift in (0..width).rev() {
                rows.push((value >> (shift * 8)) as u8);
            }
        };
        for (k, place) in self.places.iter().enumerate() {
            match (k, place) {
                (0, _) | (_, Place::Fresh) => {
                    rows.push(0);
                    put(&mut rows, next_free[k]);
                    rows.push(255);
                }
                (_, Place::File(at)) => {
                    rows.push(1);
                    put(&mut rows, *at);
                    rows.push(0);
                }
                (_, Place::Packed { stream, index }) => {
                    rows.push(2);
                    put(&mut rows, *stream as usize);
                    rows.push(*index as u8);
                }
            }
        }
        let size = self.places.len();
        let hex: String = id.iter().map(|b| format!("{b:02X}")).collect();
        let mut keys = format!(
            "/Type /XRef /Index [ 0 {size} ] /Size {size} /W [ 1 {width} 1 ] /Root {root} 0 R"
        );
        if let Some(info) = info {
            keys.push_str(&format!(" /Info {info} 0 R"));
        }
        // `\pdfextension trailer`, between the information dictionary and the
        // identifier.
        if !trailer.is_empty() {
            keys.push(' ');
            keys.push_str(trailer);
        }
        match &self.trailer_id {
            Some(id) => keys.push_str(&format!(" /ID {id}")),
            None => keys.push_str(&format!(" /ID [ <{hex}> <{hex}> ]")),
        }
        self.stream(xref, &keys, &rows, true);
        self.out
            .extend(format!("startxref\n{xref_offset}\n%%EOF\n").as_bytes());
        self.out
    }

    /// An object built as `crate::pdf::Object`, numbered and written now:
    /// a stream into the file, deflated unless it names its own filter, and
    /// anything else into the object stream. The way the writers of the
    /// fonts and pictures texrs carries beside LuaTeX's add what they make.
    pub fn add(&mut self, object: Object) -> Object {
        let number = self.create();
        match object {
            Object::Stream { dict, data } => {
                let own_filter = dict.get("Filter").is_some();
                let keys: Dict = dict
                    .iter()
                    .filter(|(k, _)| k.as_str() != "Length")
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                self.stream(number, &spelled_keys(&keys), &data, !own_filter);
            }
            other => {
                let mut body = Vec::new();
                other.write_into(&mut body);
                self.object(number, OBJSTM_ALWAYS, &body);
            }
        }
        Object::Reference(number)
    }
}

/// A dictionary's entries as `/Key value` pairs joined by spaces, without the
/// brackets: what goes before `/Filter` in a stream dictionary.
fn spelled_keys(dict: &Dict) -> String {
    let mut body = Vec::new();
    Object::Dict(dict.clone()).write_into(&mut body);
    let text = String::from_utf8_lossy(&body).into_owned();
    text.trim_start_matches("<<")
        .trim_end_matches(">>")
        .trim()
        .to_string()
}

impl crate::pdf::Sink for Writer {
    fn add(&mut self, object: Object) -> Object {
        Writer::add(self, object)
    }
}

/// `divide_scaled(s, m, dd)` (`utils.c`): `s / m` to `dd` decimal places,
/// rounded, as an integer of that many places.
fn divide_scaled(s: i64, m: i64, dd: u32) -> i64 {
    let sign = if (s < 0) != (m < 0) { -1 } else { 1 };
    let (s, m) = (s.abs(), m.abs());
    if m == 0 {
        return 0;
    }
    let (mut q, mut r) = (s / m, s % m);
    for _ in 0..dd {
        q = 10 * q + (10 * r) / m;
        r = (10 * r) % m;
    }
    if 2 * r >= m {
        q += 1;
    }
    sign * q
}

/// A TeX font's object as the document has used it: what number it has, and
/// which codes were drawn in it.
struct UsedFont<'a> {
    font: &'a Font,
    number: u32,
    codes: BTreeSet<u8>,
}

/// One `/Pages` node (`pages_entry`).
struct PagesNode {
    number: u32,
    kids: Vec<u32>,
    pages: usize,
}

thread_local! {
    /// What the run that just lowered gave the backend, for the file it is
    /// about to write: see [`prepare`].
    static PREPARED: std::cell::RefCell<Option<crate::backend::Prepared>> =
        const { std::cell::RefCell::new(None) };
}

/// Hand the next document written on this thread what its run gave the
/// backend: its variables, the objects it already wrote, its `info`,
/// `catalog`, `names` and `trailer`, its outlines and the destinations they
/// name.
pub fn prepare(prepared: crate::backend::Prepared) {
    PREPARED.with(|p| *p.borrow_mut() = Some(prepared));
}

/// What [`prepare`] handed over, once.
pub fn take_prepared() -> Option<crate::backend::Prepared> {
    PREPARED.with(|p| p.borrow_mut().take())
}

/// The file for `pages`, whose content streams are `contents`, one a page,
/// with LuaTeX's object numbering and order, and what the run gave the
/// backend (`prepared`): the file it began with the objects it wrote while
/// reading the document, and what it asked the catalogue, the information
/// dictionary, the page tree and each page to carry.
pub fn write(pages: &[Page], contents: Vec<String>, prepared: crate::backend::Prepared) -> Vec<u8> {
    let mut prepared = prepared;
    let vars = prepared.vars;
    let mut w = prepared.writer.take().unwrap_or_else(|| Writer::new(vars));
    w.vars = vars;
    let pictures = crate::pdf::picture_classes(pages);

    // The content streams are deflated before any is written: each reads only
    // its own page, and that is most of the work of a long document.
    let streams: Vec<Vec<u8>> = {
        use rayon::prelude::*;
        let level = vars.compress_level;
        let squeeze = |c: String| match level {
            0 => c.into_bytes(),
            _ => deflate_at(level, c.as_bytes()),
        };
        match pages.len() >= crate::pdf::PARALLEL_PAGES {
            true => contents.into_par_iter().map(squeeze).collect(),
            false => contents.into_iter().map(squeeze).collect(),
        }
    };

    let mut fonts: Vec<UsedFont> = Vec::new();
    let mut last_page = 0u32;
    let mut images: Vec<Option<u32>> = Vec::new();
    let mut tree: Vec<PagesNode> = Vec::new();
    for ((page, classes), stream) in pages.iter().zip(&pictures).zip(streams) {
        // A picture is numbered when `\saveimageresource` reads it, which is
        // before the page that first uses it begins.
        for &class in classes {
            if class >= images.len() {
                images.resize(class + 1, None);
            }
            if images[class].is_none() {
                images[class] = Some(w.create());
            }
        }
        // `pdf_begin_page`: the resources, the page, the content.
        let resources = w.create();
        // `pdf_get_obj(pdf, obj_type_page, total_pages + 1, 0)`: a page
        // something asked for before it existed was numbered then.
        let page_number = match prepared.pages.get(&(tree_pages(&tree) as i64 + 1)) {
            Some(&k) => k,
            None => w.create(),
        };
        let content = w.create();
        let content_at = w.out.len();
        w.places[content as usize] = Place::File(content_at);
        let filter = match vars.compress_level {
            0 => "",
            _ => " /Filter /FlateDecode",
        };
        w.out.extend(
            format!(
                "{content} 0 obj\n<<{filter} /Length {}\nstream\n",
                length_and_close(stream.len())
            )
            .as_bytes(),
        );
        w.out.extend(&stream);
        w.out.extend(b"\nendstream\nendobj\n");
        // `pdf_init_font`: a font is numbered the first time a glyph of it is
        // set, which is while the content is written.
        let mut font_refs = Dict::new();
        for (name, font) in &page.fonts {
            let font: &Font = font;
            let codes = page.used.get(name).cloned().unwrap_or_default();
            let at = match fonts.iter().position(|u| crate::pdf::same(u.font, font)) {
                Some(at) => at,
                None => {
                    let number = w.create();
                    fonts.push(UsedFont {
                        font,
                        number,
                        codes: BTreeSet::new(),
                    });
                    fonts.len() - 1
                }
            };
            fonts[at].codes.extend(codes);
            font_refs.insert(name.clone(), Object::Reference(fonts[at].number));
        }
        // `pdf_do_page_divert`: the page goes into the last `/Pages` node,
        // and a new node is begun when it is full.
        if tree.last().is_none_or(|n| n.kids.len() == PAGES_KIDS_MAX) {
            let number = w.create();
            tree.push(PagesNode {
                number,
                kids: Vec::new(),
                pages: 0,
            });
        }
        let parent = {
            let node = tree.last_mut().expect("a node was just made");
            node.kids.push(page_number);
            node.pages += 1;
            node.number
        };
        // `pdf_end_page`'s page dictionary, `\pdfvariable pageattr` after the
        // media box.
        let mut body = format!(
            "<< /Type /Page /Contents {content} 0 R /Resources {resources} 0 R \
             /MediaBox [ 0 0 {} {} ]",
            bp_text(page.width, vars.decimal_digits),
            bp_text(page.height, vars.decimal_digits)
        );
        for extra in [&prepared.page_attr, &prepared.lua.page_attributes] {
            if !extra.is_empty() {
                body.push(' ');
                body.push_str(extra);
            }
        }
        body.push_str(&format!(" /Parent {parent} 0 R >>"));
        w.object(page_number, OBJSTM_ALWAYS, body.as_bytes());
        // The pictures this page used for the first time, as `pdf_end_page`
        // writes pending images.
        let mut procset = BTreeSet::new();
        let mut xobjects = Dict::new();
        for ((name, image), &class) in page.images.iter().zip(classes) {
            let Some(number) = images[class] else {
                continue;
            };
            if matches!(w.places[number as usize], Place::Fresh) {
                crate::pdf::write_image_as(&mut w, number, image);
            }
            procset.extend(image_procset(image));
            xobjects.insert(name.clone(), Object::Reference(number));
        }
        // The resources last: what a page-resources setting adds first (the
        // graphics states and shadings a picture asked for), then the fonts,
        // the pictures and the `/ProcSet`.
        let mut res = crate::pdf::extra_resources(page);
        if !font_refs.is_empty() {
            res.insert("Font".to_string(), Object::Dict(font_refs.clone()));
        }
        if !xobjects.is_empty() {
            res.insert("XObject".to_string(), Object::Dict(xobjects));
        }
        if vars.major_version == 1 {
            let mut set = vec![Object::name("PDF")];
            if !font_refs.is_empty() {
                set.push(Object::name("Text"));
            }
            for name in ["ImageB", "ImageC", "ImageI"] {
                if procset.contains(name) {
                    set.push(Object::name(name));
                }
            }
            res.insert("ProcSet".to_string(), Object::Array(set));
        }
        // `\pdfvariable pageresources` first, then what the page drew with.
        let mut body = Vec::new();
        Object::Dict(res).write_into(&mut body);
        let first: Vec<&str> = [&prepared.page_resources, &prepared.lua.page_resources]
            .into_iter()
            .filter(|s| !s.is_empty())
            .map(String::as_str)
            .collect();
        if !first.is_empty() {
            let rest = String::from_utf8_lossy(&body[2..]).into_owned();
            body = format!("<< {}{rest}", first.join(" ")).into_bytes();
        }
        w.object(resources, OBJSTM_ALWAYS, &body);
        last_page = page_number;
    }

    // `check_nonexisting_destinations`: a destination something named and no
    // page set goes to the last page, newest first as `head_tab` lists them.
    let mut dests: Vec<u32> = prepared.dests.values().copied().collect();
    dests.sort_unstable();
    for &d in dests.iter().rev() {
        let body = format!("[ {last_page} 0 R /Fit ]");
        w.object(d, OBJSTM_ALWAYS, body.as_bytes());
    }

    write_fonts(&mut w, &fonts);
    let pages_attr = [&prepared.pages_attr, &prepared.lua.pages_attributes]
        .into_iter()
        .filter(|s| !s.is_empty())
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ");
    let root_pages = write_page_tree(&mut w, tree, &pages_attr);
    let outlines = write_outlines(&mut w, &prepared);
    let names = write_name_tree(&mut w, &prepared);

    // The catalogue: the page tree, the outlines, the name tree, then what
    // `\pdfextension catalog` added and its `openaction`.
    let catalog = w.create();
    let mut body = String::from("<< /Type /Catalog");
    if let Some(pages) = root_pages {
        body.push_str(&format!(" /Pages {pages} 0 R"));
        if let Some(o) = outlines {
            body.push_str(&format!(" /Outlines {o} 0 R"));
        }
        if let Some(n) = names {
            body.push_str(&format!(" /Names {n} 0 R"));
        }
        if !prepared.catalog.is_empty() {
            body.push(' ');
            body.push_str(&prepared.catalog);
        }
    }
    if let Some(a) = prepared.open_action {
        body.push_str(&format!(" /OpenAction {a} 0 R"));
    }
    if !prepared.lua.catalog.is_empty() {
        body.push(' ');
        body.push_str(&prepared.lua.catalog);
    }
    body.push_str(" >>");
    w.object(catalog, OBJSTM_ALWAYS, body.as_bytes());

    // `pdf_print_info`: what `\pdfextension info` said first, then each key
    // it did not give.
    let info = w.create();
    let date = crate::pdf::pdf_date(crate::pdf::stamp());
    let given = |key: &str| prepared.info.contains(key) || prepared.lua.info.contains(key);
    let mut body = String::from("<<");
    for extra in [&prepared.info, &prepared.lua.info] {
        if !extra.is_empty() {
            body.push(' ');
            body.push_str(extra);
        }
    }
    let v = env!("CARGO_PKG_VERSION");
    if !given("/Producer") {
        body.push_str(&format!(" /Producer (texrs-{v})"));
    }
    if !given("/Creator") {
        body.push_str(" /Creator (TeX)");
    }
    if !given("/CreationDate") {
        body.push_str(&format!(" /CreationDate ({date})"));
    }
    if !given("/ModDate") {
        body.push_str(&format!(" /ModDate ({date})"));
    }
    if !given("/Trapped") {
        body.push_str(" /Trapped /False");
    }
    body.push_str(&format!(
        " /PTEX.FullBanner (This is texrs, Version {v}) >>"
    ));
    w.object(info, INFO_THRESHOLD, body.as_bytes());

    let cwd = std::env::current_dir()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    let id = trailer_id(
        crate::pdf::stamp(),
        &cwd,
        &format!("{}.pdf", crate::lua::jobname()),
    );
    let trailer = [&prepared.trailer, &prepared.lua.trailer]
        .into_iter()
        .filter(|s| !s.is_empty())
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ");
    w.trailer_id = (!prepared.lua.trailer_id.is_empty()).then(|| prepared.lua.trailer_id.clone());
    w.finish(catalog, Some(info), &trailer, id)
}

/// How many pages the tree holds so far, which is the number of the page
/// being begun, less one.
fn tree_pages(tree: &[PagesNode]) -> usize {
    tree.iter().map(|n| n.pages).sum()
}

/// `print_outlines`: the outline dictionary and every entry, newest entry
/// first as `head_tab` lists them.
fn write_outlines(w: &mut Writer, p: &crate::backend::Prepared) -> Option<u32> {
    if p.first_outline == 0 {
        return None;
    }
    let mut entries = p.outlines.clone();
    let find = |entries: &[crate::backend::OutlineEntry], k: u32| {
        entries.iter().position(|e| e.number == k)
    };
    // `open_subentries`: count each entry's open descendants, set its
    // parent and its last child, and make a closed entry's count negative.
    fn open(entries: &mut Vec<crate::backend::OutlineEntry>, at: usize) -> i64 {
        let mut k = 0;
        let mut l = entries[at].first;
        let me = entries[at].number;
        while l != 0 {
            let Some(i) = entries.iter().position(|e| e.number == l) else {
                break;
            };
            k += 1;
            let c = open(entries, i);
            if entries[i].count > 0 {
                k += c;
            }
            entries[i].parent = me;
            let r = entries[i].next;
            if r == 0 {
                entries[at].last = l;
            }
            l = r;
        }
        entries[at].count = match entries[at].count > 0 {
            true => k,
            false => -k,
        };
        k
    }
    let outlines = w.create();
    let mut k = 0;
    let mut l = p.first_outline;
    while let Some(i) = find(&entries, l) {
        k += 1;
        let a = open(&mut entries, i);
        if entries[i].count > 0 {
            k += a;
        }
        // LuaTeX sets a top-level entry's parent to `pdf->obj_ptr`, which
        // is the outline dictionary just numbered.
        entries[i].parent = outlines;
        l = entries[i].next;
        if l == 0 {
            break;
        }
    }
    let body = format!(
        "<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count {k} >>",
        p.first_outline, p.last_outline
    );
    w.object(outlines, OBJSTM_ALWAYS, body.as_bytes());
    for e in &entries {
        let mut body = format!("<< /Title {} 0 R /A {} 0 R", e.title, e.action);
        for (key, v) in [
            ("Parent", e.parent),
            ("Prev", e.prev),
            ("Next", e.next),
            ("First", e.first),
            ("Last", e.last),
        ] {
            if v != 0 {
                body.push_str(&format!(" /{key} {v} 0 R"));
            }
        }
        if e.count != 0 {
            body.push_str(&format!(" /Count {}", e.count));
        }
        if let Some(attr) = &e.attr {
            body.push_str(&format!("\n{attr}\n"));
        }
        body.push_str(" >>");
        w.object(e.number, OBJSTM_ALWAYS, body.as_bytes());
    }
    Some(outlines)
}

/// `dest_cmp`: two destination names in the order a name tree needs, a
/// hexadecimal string compared as the bytes it stands for.
fn dest_order(a: &str, b: &str) -> std::cmp::Ordering {
    let bytes = |s: &str| -> Vec<u8> {
        let raw = s.as_bytes();
        let hex = raw.len() >= 2
            && raw[0] == b'<'
            && raw[raw.len() - 1] == b'>'
            && raw.len().is_multiple_of(2)
            && raw[1..raw.len() - 1].iter().all(u8::is_ascii_hexdigit);
        match hex {
            true => raw[1..raw.len() - 1]
                .chunks(2)
                .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap_or("0"), 16).unwrap_or(0))
                .collect(),
            false => raw.to_vec(),
        }
    };
    let empty = |s: &str| s.is_empty() || s == "<>";
    match (empty(a), empty(b)) {
        (true, false) => return std::cmp::Ordering::Less,
        (false, true) => return std::cmp::Ordering::Greater,
        _ => {}
    }
    bytes(a).cmp(&bytes(b))
}

/// `output_name_tree`: the named destinations in leaves of 32, the levels
/// above them, and the `/Names` dictionary with `\pdfextension names` in it.
fn write_name_tree(w: &mut Writer, p: &crate::backend::Prepared) -> Option<u32> {
    const KIDS_MAX: usize = 32;
    let mut names = p.dest_names.clone();
    names.sort_by(|a, b| dest_order(&a.0, &b.0));
    let mut dests = 0;
    if !names.is_empty() {
        // (number, first name, last name) of each node of the level built.
        let mut level: Vec<(u32, String, String)> = Vec::new();
        for chunk in names.chunks(KIDS_MAX) {
            let l = w.create();
            let pairs: String = chunk
                .iter()
                .map(|(n, k)| format!(" {} {k} 0 R", crate::backend::pdf_string(n)))
                .collect();
            let (first, last) = (chunk[0].0.clone(), chunk[chunk.len() - 1].0.clone());
            let body = format!(
                "<< /Names [{pairs} ] /Limits [ {} {} ] >>",
                crate::backend::pdf_string(&first),
                crate::backend::pdf_string(&last)
            );
            w.object(l, OBJSTM_ALWAYS, body.as_bytes());
            level.push((l, first, last));
        }
        while level.len() > 1 {
            let mut above = Vec::new();
            for chunk in level.chunks(KIDS_MAX) {
                let l = w.create();
                let kids: String = chunk.iter().map(|(k, ..)| format!(" {k} 0 R")).collect();
                let (first, last) = (chunk[0].1.clone(), chunk[chunk.len() - 1].2.clone());
                let body = format!(
                    "<< /Kids [{kids} ] /Limits [ {} {} ] >>",
                    crate::backend::pdf_string(&first),
                    crate::backend::pdf_string(&last)
                );
                w.object(l, OBJSTM_ALWAYS, body.as_bytes());
                above.push((l, first, last));
            }
            level = above;
        }
        dests = level[0].0;
    }
    if dests == 0 && p.names.is_empty() && p.lua.names.is_empty() {
        return None;
    }
    let m = w.create();
    let mut body = String::from("<<");
    if dests != 0 {
        body.push_str(&format!(" /Dests {dests} 0 R"));
    }
    for extra in [&p.names, &p.lua.names] {
        if !extra.is_empty() {
            body.push(' ');
            body.push_str(extra);
        }
    }
    body.push_str(" >>");
    w.object(m, OBJSTM_ALWAYS, body.as_bytes());
    Some(m)
}

/// A page dimension given in big points, as `pdf_add_bp` prints one.
fn bp_text(points: f64, decimal_digits: u32) -> String {
    let scale = 10f64.powi(decimal_digits as i32);
    let mut out = Vec::new();
    crate::pdfpage::print_pdffloat(
        &mut out,
        crate::pdfpage::PdfFloat::new((points * scale).round() as i64, decimal_digits),
    );
    String::from_utf8_lossy(&out).into_owned()
}

/// The `/ProcSet` names a picture adds to its page (`writepng.c`,
/// `writejpg.c`): `ImageB` for grey, `ImageC` for colour, and `ImageC` with
/// `ImageI` for a palette.
fn image_procset(image: &crate::image::Image) -> Vec<&'static str> {
    use crate::image::Colours;
    match image.colours {
        Colours::Gray => vec!["ImageB"],
        Colours::Indexed => vec!["ImageC", "ImageI"],
        _ => vec!["ImageC"],
    }
}

/// `output_pages_tree`: the leaf nodes, and the nodes above them while there
/// is more than one, written bottom up; the root's number.
fn write_page_tree(w: &mut Writer, leaves: Vec<PagesNode>, attr: &str) -> Option<u32> {
    let mut level = leaves;
    loop {
        match level.len() {
            0 => return None,
            1 => {
                let root = &level[0];
                write_pages_node(w, root, None, attr);
                return Some(root.number);
            }
            _ => {
                let mut parents: Vec<PagesNode> = vec![PagesNode {
                    number: w.create(),
                    kids: Vec::new(),
                    pages: 0,
                }];
                for node in &level {
                    if parents
                        .last()
                        .is_some_and(|q| q.kids.len() == PAGES_KIDS_MAX)
                    {
                        let number = w.create();
                        parents.push(PagesNode {
                            number,
                            kids: Vec::new(),
                            pages: 0,
                        });
                    }
                    let q = parents.last_mut().expect("a parent exists");
                    q.kids.push(node.number);
                    q.pages += node.pages;
                    let parent = q.number;
                    write_pages_node(w, node, Some(parent), attr);
                }
                level = parents;
            }
        }
    }
}

/// `write_pages`. The root carries `\pdfvariable pagesattr`, and is spelled
/// with the spaces LuaTeX leaves: one after the attributes when there are
/// any, one after the (empty) Lua table's, and the next key's own -- so two
/// before `/Count` when there are none.
fn write_pages_node(w: &mut Writer, node: &PagesNode, parent: Option<u32>, attr: &str) {
    let kids: String = node.kids.iter().map(|k| format!(" {k} 0 R")).collect();
    let head = match (parent, attr.is_empty()) {
        (None, true) => "/Type /Pages  ".to_string(),
        (None, false) => format!("/Type /Pages {attr}   "),
        (Some(p), _) => format!("/Type /Pages /Parent {p} 0 R "),
    };
    let body = format!("<< {head}/Count {} /Kids [{kids} ] >>", node.pages);
    w.object(node.number, OBJSTM_ALWAYS, body.as_bytes());
}

/// The font objects, in `pdf_finish_file`'s order: each font's widths as
/// `do_pdf_font` makes them, newest font first (`head_tab` is a list each new
/// font is pushed onto the front of); then the descriptors with their font
/// programs, in the order of the programs' file names; then the encodings;
/// then the font dictionaries, in the order of the fonts' TFM names.
fn write_fonts(w: &mut Writer, fonts: &[UsedFont]) {
    // What `create_fontdictionary` leaves for the second half.
    struct Tex<'a> {
        tex: &'a crate::pdf::TexFont,
        number: u32,
        first: u8,
        last: u8,
        widths: u32,
        encoding: Option<(u32, Vec<Object>)>,
        glyphs: BTreeSet<String>,
    }
    let mut tex: Vec<Tex> = Vec::new();
    for used in fonts.iter().rev() {
        let Font::Tex(t) = used.font else {
            // A face LuaTeX would load through luaotfload, or one of the
            // fourteen: written by texrs's own writer, under the number it was
            // given when first used.
            crate::pdf::write_font_as(w, used.number, used.font, &used.codes);
            continue;
        };
        let codes: Vec<u8> = used
            .codes
            .iter()
            .copied()
            .filter(|&c| t.exists(c))
            .collect();
        let (Some(&first), Some(&last)) = (codes.first(), codes.last()) else {
            continue;
        };
        // A code drawn that the program's own encoding does not put the glyph
        // at is re-encoded, as a map line naming an `.enc` file would be. A
        // TeX font set through its own codes never is; the code a driver
        // addressing it in WinAnsi used is (see `TexFont::glyph_for`).
        let mut differences = Vec::new();
        let mut expected = None;
        let mut glyphs = BTreeSet::new();
        for &code in &codes {
            let Some(name) = t.glyph_for(code) else {
                continue;
            };
            glyphs.insert(name.clone());
            if t.program.encoded(code).map(|g| g.name.as_str()) == Some(name.as_str()) {
                continue;
            }
            if expected != Some(code) {
                differences.push(Object::Integer(i64::from(code)));
            }
            differences.push(Object::name(&name));
            expected = Some(code.wrapping_add(1));
        }
        let encoding = (!differences.is_empty()).then(|| (w.create(), differences));
        // `write_charwidth_array`.
        let widths = w.create();
        let mut body = b"[".to_vec();
        let mut j = first;
        for (n, &code) in codes.iter().enumerate() {
            if n > 0 {
                body.push(b' ');
            }
            while j.wrapping_add(1) < code && j < code {
                body.extend(b"0 ");
                j += 1;
            }
            j = code;
            crate::pdfpage::print_pdffloat(
                &mut body,
                crate::pdfpage::PdfFloat::new(t.char_width_e1(code), 1),
            );
        }
        body.extend(b" ]");
        w.object(widths, OBJSTM_ALWAYS, &body);
        tex.push(Tex {
            tex: t,
            number: used.number,
            first,
            last,
            widths,
            encoding,
            glyphs,
        });
    }

    // `write_fontdescriptors`: one per program file, in `strcmp` order of its
    // name, the glyphs of every font sharing it joined.
    let mut by_file: BTreeMap<&str, (BTreeSet<String>, Vec<usize>)> = BTreeMap::new();
    for (i, t) in tex.iter().enumerate() {
        let entry = by_file.entry(t.tex.program_file.as_str()).or_default();
        entry.0.extend(t.glyphs.iter().cloned());
        entry.1.push(i);
    }
    let mut descriptors: BTreeMap<&str, (u32, String)> = BTreeMap::new();
    for (file, (glyphs, users)) in &by_file {
        let t = tex[users[0]].tex;
        let number = w.create();
        let tag = subset_tag(glyphs, &t.program.font_name);
        let name = format!("{tag}+{}", t.program.font_name);
        // `writet1` cuts the program to the glyphs; then the stream.
        let cut = t.program.subset(glyphs, &tag);
        let program = cut.as_ref().unwrap_or(&t.program);
        let (bytes, clear, binary, trailer) = program.embeddable();
        let file_number = w.create();
        w.stream(
            file_number,
            &format!("/Length1 {clear} /Length2 {binary} /Length3 {trailer}"),
            &bytes,
            true,
        );
        let bbox = program.font_bbox;
        let charset: String = glyphs.iter().map(|g| format!(" /{g}")).collect();
        let body = format!(
            "<< /Type /FontDescriptor /FontName /{name} /Flags 4 /FontBBox [ {} {} {} {} ] \
             /Ascent {} /CapHeight {} /Descent {} /ItalicAngle {} /StemV {} /XHeight {} \
             /CharSet({charset}) /FontFile {file_number} 0 R >>",
            bbox[0] as i64,
            bbox[1] as i64,
            bbox[2] as i64,
            bbox[3] as i64,
            t.ascent,
            t.cap_height,
            t.descent,
            program.italic_angle as i64,
            program.stem_v.map(|v| v as i64).unwrap_or(t.stem_v),
            t.x_height,
        );
        w.object(number, OBJSTM_ALWAYS, body.as_bytes());
        descriptors.insert(file, (number, name));
    }

    // `write_fontencodings`.
    for t in &tex {
        if let Some((number, differences)) = &t.encoding {
            let mut body = Vec::new();
            Object::dict([
                ("Type", Object::name("Encoding")),
                ("Differences", Object::Array(differences.clone())),
            ])
            .write_into(&mut body);
            w.object(*number, OBJSTM_ALWAYS, &body);
        }
    }

    // `write_fontdictionaries`, in `strcmp` order of the TFM name; a ToUnicode
    // map first when `gentounicode` asks for one.
    let mut order: Vec<&Tex> = tex.iter().collect();
    order.sort_by(|a, b| a.tex.tfm_name.cmp(&b.tex.tfm_name));
    for t in order {
        let (fd, name) = &descriptors[t.tex.program_file.as_str()];
        let to_unicode = match w.vars.gen_tounicode {
            true => {
                let meanings: Vec<(u8, String)> = (t.first..=t.last)
                    .filter_map(|c| Some((c, crate::agl::unicode(&t.tex.glyph_for(c)?)?)))
                    .collect();
                let number = w.create();
                w.stream(
                    number,
                    "",
                    crate::agl::to_unicode(&t.tex.tfm_name, &meanings).as_bytes(),
                    true,
                );
                Some(number)
            }
            false => None,
        };
        let mut body = format!(
            "<< /Type /Font /Subtype /Type1 /BaseFont /{name} /FontDescriptor {fd} 0 R \
             /FirstChar {} /LastChar {} /Widths {} 0 R",
            t.first, t.last, t.widths
        );
        if let Some((number, _)) = t.encoding {
            body.push_str(&format!(" /Encoding {number} 0 R"));
        }
        if let Some(number) = to_unicode {
            body.push_str(&format!(" /ToUnicode {number} 0 R"));
        }
        body.push_str(" >>");
        w.object(t.number, OBJSTM_ALWAYS, body.as_bytes());
    }
}

/// What a font's descriptor and widths need of its TFM, read at its design
/// size the way `preset_fontmetrics` reads them.
pub fn tex_metrics(tfm: &crate::tfm::Tfm) -> (i64, Vec<Option<i64>>, [i64; 5]) {
    let size = (tfm.design_size * 65536.0).round() as i64;
    let widths: Vec<Option<i64>> = (0..=255u8)
        .map(|c| tfm.char(c).map(|m| crate::tfm::store_scaled(m.width, size)))
        .collect();
    let height = |c: u8| {
        tfm.char(c)
            .map(|m| crate::tfm::store_scaled(m.height, size))
            .unwrap_or(0)
    };
    let depth = |c: u8| {
        tfm.char(c)
            .map(|m| crate::tfm::store_scaled(m.depth, size))
            .unwrap_or(0)
    };
    let ascent = divide_scaled(height(b'h'), size, 3);
    let cap_height = divide_scaled(height(b'H'), size, 3);
    let descent = -divide_scaled(depth(b'y'), size, 3);
    let descent = descent.min(0);
    let stem_v = divide_scaled(widths[usize::from(b'.')].unwrap_or(0) / 3, size, 3);
    let x_height = divide_scaled(tfm.param_at(5, size), size, 3);
    (
        size,
        widths,
        [ascent, cap_height, descent, stem_v, x_height],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// luatex's tag for the subset of CMR10 that `Hello world.` draws, with
    /// its folio: `KJJYRX`, measured.
    #[test]
    fn the_subset_tag_is_luatex_s() {
        let glyphs: BTreeSet<String> = ["H", "d", "e", "l", "o", "one", "period", "r", "w"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(subset_tag(&glyphs, "CMR10"), "KJJYRX");
    }

    /// `print_ID`'s recipe: MD5 of the UT start time spelled
    /// `%Y%m%dT%H%M%SZ`, the directory, `/` and the file's name. (Measured on
    /// `Hello world.`, luatex 1.24.0's `/ID` is exactly this digest for the
    /// directory it ran in and `case.pdf`.) The digests here are `md5(1)`'s
    /// of the same strings.
    #[test]
    fn the_trailer_id_is_print_id_s() {
        let hex = |id: [u8; 16]| id.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(
            hex(trailer_id(0, "/home/user/doc", "case.pdf")),
            "2115d72ebf4ae892795eb96688973995"
        );
        assert_eq!(
            hex(trailer_id(1_000_000_000, "/srv/build", "report.pdf")),
            "477e4375682cdbae010aadd9b8ef9007"
        );
    }

    /// zlib at level 9 is what luatex compresses with; a stream compressed
    /// here is the bytes luatex writes. Measured on the content stream of
    /// `Hello world.`: 99 bytes.
    #[test]
    fn a_stream_deflates_to_luatex_s_length() {
        let content = "BT\n/F1 9.96264 Tf\n1 0 0 1 91.925 710.037 Tm [(Hello)-333(w)27(orld.)]TJ\n\
                       1 0 0 1 303.509 55.29 Tm [(1)]TJ\nET\n";
        assert_eq!(deflate_at(9, content.as_bytes()).len(), 99);
    }

    /// The `/Length` LuaTeX seeks back to write leaves the rest of the room it
    /// reserved as spaces: fourteen characters from the number on.
    #[test]
    fn a_stream_length_is_padded_as_luatex_leaves_it() {
        assert_eq!(length_and_close(99), "99 >>         ");
        assert_eq!(length_and_close(10362), "10362 >>      ");
    }

    /// `divide_scaled` rounds half up, after the digits are long-divided out.
    #[test]
    fn divide_scaled_is_utils_c_s() {
        assert_eq!(divide_scaled(455111, 655360, 3), 694);
        assert_eq!(divide_scaled(-1, 2, 0), -1);
        assert_eq!(divide_scaled(1, 3, 3), 333);
    }

    /// The object machinery: numbers handed out in order, an object packed
    /// into the stream that is numbered when the first one goes in, a stream
    /// written to the file at once, a hundredth packed object flushing the
    /// stream, and the free list threading the numbers never written.
    #[test]
    fn objects_are_numbered_and_placed_as_luatex_places_them() {
        let mut w = Writer::new(Variables {
            compress_level: 0,
            ..Variables::default()
        });
        let a = w.create();
        let unused = w.create();
        w.object(a, OBJSTM_ALWAYS, b"<< /A 1 >>");
        // The object stream took the next number when `a` went into it.
        assert!(matches!(w.places[3], Place::Fresh));
        assert!(matches!(
            w.places[a as usize],
            Place::Packed {
                stream: 3,
                index: 0
            }
        ));
        let s = w.create();
        w.stream(s, "/K 1", b"data", true);
        let text = String::from_utf8_lossy(&w.out).into_owned();
        assert!(
            text.contains(
                "4 0 obj\n<< /K 1 /Length 4 >>          \nstream\ndata\nendstream\nendobj\n"
            ),
            "{text}"
        );
        for _ in 1..OS_MAX_OBJS {
            let n = w.create();
            w.object(n, OBJSTM_ALWAYS, b"null");
        }
        // Full, so written, and the next packed object starts another.
        assert!(w.objstm.is_none());
        let text = String::from_utf8_lossy(&w.out).into_owned();
        assert!(
            text.contains("3 0 obj\n<< /Type /ObjStm /N 100 /First"),
            "{text}"
        );
        let out = w.finish(a, None, "", [0; 16]);
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(text.contains("/W [ 1 2 1 ]"), "{text}");
        // The table's rows, four bytes each: object 0 heads the free list and
        // points at 2, the one number never written, which ends it; 1 is the
        // first object in stream 3; 4 is in the file.
        let find = |needle: &[u8], from: usize| {
            out[from..]
                .windows(needle.len())
                .position(|w| w == needle)
                .map(|at| at + from)
        };
        let table = find(b"/Type /XRef", 0).expect("a table");
        let rows_at = find(b"stream\n", table).expect("its rows") + 7;
        let row = |k: usize| &out[rows_at + 4 * k..rows_at + 4 * k + 4];
        assert_eq!(row(0), &[0, 0, unused as u8, 255]);
        assert_eq!(row(unused as usize), &[0, 0, 0, 255]);
        assert_eq!(row(a as usize), &[2, 0, 3, 0]);
        assert_eq!(row(4)[0], 1);
    }
}
