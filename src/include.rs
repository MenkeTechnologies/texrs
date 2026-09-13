//! Putting a page of one PDF onto a page of another, ported from
//! `pdf_include_page` in tectonic's `xdvipdfmx`.
//!
//! This is what [`crate::pdfread`] was for. The commonest figure in a LaTeX
//! document is another PDF -- a plot, a diagram, a scan -- and
//! `\includegraphics{figure.pdf}` means taking one page out of that file and
//! drawing it on a page of this one.
//!
//! A page cannot simply be copied. Its content stream refers to fonts and
//! pictures by name, those names are resolved through the page's resource
//! dictionary, and everything in that dictionary refers to objects elsewhere
//! in the source file by number -- numbers that mean something else in the
//! destination. So including a page is copying an object *graph*: the page's
//! resources, whatever they point at, whatever those point at, with every
//! reference rewritten to the number the object was given here. A font
//! reached that way brings its descriptor, which brings its embedded file.
//!
//! What comes out is a Form XObject, which is PDF's word for a drawing kept
//! somewhere and used by name. It carries its own resources and its own
//! bounding box, and the page that uses it decides where it goes, so a figure
//! can be drawn twice at two sizes without being copied twice.

use std::collections::BTreeMap;

use crate::pdf::{Dict, Object, Pdf};
use crate::pdfread::Document;

/// A page taken out of `source` and put into `pdf`, as a Form XObject.
///
/// `number` counts from one, as a person counts pages.
pub fn page(pdf: &mut Pdf, source: &Document, number: usize) -> Result<Object, String> {
    let pages = source.pages();
    let page = pages
        .get(number.saturating_sub(1))
        .ok_or_else(|| format!("the file has {} pages, not {number}", pages.len()))?;

    // §8.10.2: a form's BBox is what of it is drawn, in its own coordinates.
    // A page states its size in a MediaBox, and may state a smaller CropBox,
    // which is the part a reader shows -- so that is the part to draw.
    let box_ = page
        .get("CropBox")
        .or_else(|| page.get("MediaBox"))
        .map(|it| source.resolve(it))
        .unwrap_or(Object::Null);
    let bounds = match &box_ {
        Object::Array(items) if items.len() == 4 => items
            .iter()
            .map(|item| source.resolve(item))
            .collect::<Vec<Object>>(),
        // A page with no box at all is US Letter, which is what a reader
        // assumes.
        _ => vec![
            Object::Integer(0),
            Object::Integer(0),
            Object::Integer(612),
            Object::Integer(792),
        ],
    };

    let mut copier = Copier {
        source,
        pdf,
        copied: BTreeMap::new(),
    };
    let resources = match page.get("Resources") {
        Some(resources) => copier.copy(resources, 0),
        // A page with no resources of its own draws nothing that needs any.
        None => Object::Dict(Dict::new()),
    };
    let content = source.content(page)?;

    let mut dict = Dict::from([
        ("Type", Object::name("XObject")),
        ("Subtype", Object::name("Form")),
        ("FormType", Object::Integer(1)),
        ("BBox", Object::Array(bounds.clone())),
        ("Resources", resources),
    ]);
    // A page whose box does not begin at the origin is drawn shifted, so the
    // form's own coordinates start where the page's do.
    if let (Object::Integer(x), Object::Integer(y)) = (&bounds[0], &bounds[1]) {
        if *x != 0 || *y != 0 {
            dict.insert(
                "Matrix",
                Object::Array(vec![
                    Object::Integer(1),
                    Object::Integer(0),
                    Object::Integer(0),
                    Object::Integer(1),
                    Object::Integer(-x),
                    Object::Integer(-y),
                ]),
            );
        }
    }
    Ok(pdf.add(Object::Stream {
        dict,
        data: content,
    }))
}

/// How big a page is, for a caller deciding where to put it.
pub fn size(source: &Document, number: usize) -> Option<(f64, f64)> {
    let pages = source.pages();
    let page = pages.get(number.saturating_sub(1))?;
    let box_ = source.resolve(page.get("CropBox").or_else(|| page.get("MediaBox"))?);
    let Object::Array(items) = box_ else {
        return None;
    };
    let number = |at: usize| match source.resolve(items.get(at)?) {
        Object::Integer(value) => Some(value as f64),
        Object::Real(value) => Some(value),
        _ => None,
    };
    Some((number(2)? - number(0)?, number(3)? - number(1)?))
}

/// The copy in progress, and what it has already brought across.
struct Copier<'a> {
    source: &'a Document,
    pdf: &'a mut Pdf,
    /// The number an object had there, and the number it has here.
    copied: BTreeMap<u32, u32>,
}

impl Copier<'_> {
    /// Copy an object, and everything it refers to.
    ///
    /// A reference is copied once: the second time it is met, the number it
    /// was given is handed back. That is not only for size -- a page's
    /// resources can refer to the page, and a copier that followed every
    /// reference every time would not stop.
    fn copy(&mut self, object: &Object, depth: usize) -> Object {
        if depth > 64 {
            return Object::Null;
        }
        match object {
            Object::Reference(number) => {
                if let Some(already) = self.copied.get(number) {
                    return Object::Reference(*already);
                }
                // The number is claimed before the object is copied, so
                // anything the object refers to that refers back finds it.
                let here = self.pdf.reserve();
                self.copied.insert(*number, here);
                let found = match self.source.object(*number) {
                    Some(found) => self.copy(&found, depth + 1),
                    // A reference to an object that is not there is null,
                    // which is what a reader makes of one.
                    None => Object::Null,
                };
                self.pdf.fill(here, found);
                Object::Reference(here)
            }
            Object::Array(items) => Object::Array(
                items
                    .iter()
                    .map(|item| self.copy(item, depth + 1))
                    .collect(),
            ),
            Object::Dict(dict) => Object::Dict(
                dict.iter()
                    .map(|(key, value)| (key.clone(), self.copy(value, depth + 1)))
                    .collect(),
            ),
            Object::Stream { dict, data } => {
                // A stream's bytes are copied as they are, with whatever
                // compression they came in: undoing it would only mean doing
                // it again, and a picture that was decoded and recompressed
                // would not be the picture.
                Object::Stream {
                    dict: dict
                        .iter()
                        .map(|(key, value)| (key.clone(), self.copy(value, depth + 1)))
                        .collect(),
                    data: data.clone(),
                }
            }
            other => other.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source file whose objects refer to each other the way a real one
    /// does: something used twice, and something that refers back.
    ///
    /// A figure written by a drawing program has both -- one font used by
    /// several pieces of text, and a form whose resources name the form. The
    /// documents a test makes with pdftex have neither, so the parts of the
    /// copier that exist for them are never reached by a page comparison: a
    /// copier that forgot what it had already copied, or claimed an object's
    /// number too late to stop a cycle, draws exactly the same page.
    fn shared_and_circular() -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] \
              /Resources 4 0 R /Contents 5 0 R >>"
                .to_string(),
            // The same object under two names, and a font beside it.
            "<< /XObject << /A 6 0 R /B 6 0 R >> /Font << /F1 7 0 R >> >>".to_string(),
            String::new(), // the content stream, written below
            // A form whose own resources are the page's, which refers to the
            // form: a circle.
            String::new(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];

        let mut out = String::from("%PDF-1.4\n");
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(out.len());
            let body = match i {
                4 => "<< /Length 10 >>\nstream\n/A Do /B Do\nendstream".to_string(),
                5 => "<< /Type /XObject /Subtype /Form /BBox [0 0 10 10] \
                      /Resources 4 0 R /Length 0 >>\nstream\n\nendstream"
                    .to_string(),
                _ => object.clone(),
            };
            out.push_str(&format!("{} 0 obj\n{body}\nendobj\n", i + 1));
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

    #[test]
    fn an_object_used_twice_is_copied_once_and_a_circle_does_not_spin() {
        let source = Document::read(shared_and_circular()).expect("the source reads");
        assert_eq!(source.pages().len(), 1);

        let mut pdf = Pdf::new();
        // A copier that claimed an object's number after copying what it
        // refers to would not return from this.
        let form = page(&mut pdf, &source, 1).expect("the page");
        let Object::Reference(_) = form else {
            panic!("the page did not come back as an object");
        };
        let written = pdf.finish();

        // Read it back and follow the names.
        let copy = Document::read(written).expect("what was written reads");
        let Object::Stream { dict, .. } = copy
            .object(match form {
                Object::Reference(number) => number,
                _ => unreachable!(),
            })
            .expect("the form")
        else {
            panic!("the form is not a stream");
        };
        let Object::Dict(resources) = copy.resolve(dict.get("Resources").expect("resources"))
        else {
            panic!("the resources are not a dictionary");
        };
        let Object::Dict(xobjects) = copy.resolve(resources.get("XObject").expect("an XObject"))
        else {
            panic!("the XObjects are not a dictionary");
        };

        // The same object under two names is one object here too: a copier
        // that forgot what it had copied would write it twice, and one that
        // did not remap the second reference would leave it pointing at a
        // number that means something else in this file.
        let a = xobjects.get("A").expect("A");
        let b = xobjects.get("B").expect("B");
        assert_eq!(a, b, "what was one object became two");
        let Object::Reference(number) = a else {
            panic!("A is not a reference");
        };
        assert!(
            copy.object(*number).is_some(),
            "A points at {number}, which is not in the file"
        );

        // And the circle closes: the inner form's resources are these
        // resources, not the source file's numbering.
        let Object::Stream { dict: inner, .. } = copy.resolve(a) else {
            panic!("A is not a stream");
        };
        assert_eq!(
            inner.get("Resources"),
            dict.get("Resources"),
            "the circle was copied twice instead of closing"
        );

        // The font came too, which is what makes the text draw.
        let Object::Dict(fonts) = copy.resolve(resources.get("Font").expect("a font")) else {
            panic!("the fonts are not a dictionary");
        };
        let Object::Dict(font) = copy.resolve(fonts.get("F1").expect("F1")) else {
            panic!("F1 is not a dictionary");
        };
        assert_eq!(
            font.get("BaseFont"),
            Some(&Object::Name("Helvetica".into()))
        );
    }

    /// A stream's dictionary may hold references too, and they need copying
    /// like any other.
    #[test]
    fn a_streams_dictionary_is_copied_with_the_stream() {
        let source = Document::read(shared_and_circular()).expect("the source reads");
        let mut pdf = Pdf::new();
        let form = page(&mut pdf, &source, 1).expect("the page");
        let written = pdf.finish();
        let copy = Document::read(written).expect("what was written reads");

        // The inner form's dictionary names its resources by reference. If the
        // dictionary had been copied as it was, that reference would still be
        // the source file's number -- which here is a font, not resources.
        let Object::Reference(number) = form else {
            unreachable!()
        };
        let Object::Stream { dict, .. } = copy.object(number).expect("the form") else {
            panic!("not a stream");
        };
        let Object::Dict(resources) = copy.resolve(dict.get("Resources").expect("resources"))
        else {
            panic!("the resources are not a dictionary");
        };
        let Object::Dict(xobjects) = copy.resolve(resources.get("XObject").expect("XObject"))
        else {
            panic!("not a dictionary");
        };
        let Object::Stream { dict: inner, .. } = copy.resolve(xobjects.get("A").expect("A")) else {
            panic!("A is not a stream");
        };
        let Object::Dict(theirs) = copy.resolve(inner.get("Resources").expect("resources")) else {
            panic!("the inner resources are not a dictionary");
        };
        assert!(
            theirs.get("XObject").is_some() && theirs.get("Font").is_some(),
            "the inner form's resources are not the resources: {theirs:?}"
        );
    }
}
