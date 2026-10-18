//! The runtime a standalone AOT binary links: the builtins hook, the entry, and
//! the page inputs embedded beside the compiled chunk.
//!
//! fusevm's AOT model puts the bincode-serialized `Chunk` in the object and, at
//! load, deserializes it and runs it on a `VM` (`fusevm_aot_run_embedded`).
//! Before running, it calls back into the frontend through the C symbol
//! `fusevm_aot_register_builtins` to install the frontend's builtins on that VM.
//!
//! The chunk is not the whole document. It produces the `\message` stream and
//! the TEXT a page is set from, but what the page builder sets that text with --
//! the font families, the page shape, the page colour -- is read while
//! lowering, and lives beside the chunk in [`crate::PdfProgram`], not in it. So
//! does what the lowering printed (a package that would not load) and whether
//! `\end` stopped the file, which decides how its `(./doc.tex …)` line closes.
//! [`Embedded`] carries all of it: `src/aot.rs` encodes it into the C entry
//! stub as a byte array, and [`texrs_aot_main`] decodes it, runs the chunk, and
//! typesets the text exactly as an ordinary run does after its VM returns.

use crate::colour::Rgb;
use crate::linebreak::Params;
use crate::typeset::{Families, FontFile, Layout, PageParams};
use fusevm::VM;

/// Install texrs's builtins on the AOT run's VM.
///
/// # Safety
///
/// Called by fusevm's AOT entry with the VM it is about to run.
#[no_mangle]
pub unsafe extern "C" fn fusevm_aot_register_builtins(vm: *mut VM) {
    if vm.is_null() {
        return;
    }
    crate::runtime::register_message_builtins(&mut *vm);
}

/// The `main` a linked texrs AOT binary calls, handed the [`Embedded`] bytes.
///
/// Runs the embedded chunk, then does what `texrs FILE` does after its VM
/// returns: typesets the text, writes the PDF when there is a page, and prints
/// the file line and the `Output written on …` line -- the compiled document has
/// to be observationally the same program, or "compiled ahead of time" would
/// mean "behaves differently".
///
/// # Safety
///
/// `data` points at `len` readable bytes: the array `src/aot.rs` wrote into the
/// entry stub.
#[no_mangle]
pub unsafe extern "C" fn texrs_aot_main(data: *const u8, len: usize) -> i64 {
    let bytes = std::slice::from_raw_parts(data, len);
    let Some(embedded) = Embedded::decode(bytes) else {
        eprintln!("texrs: aot: corrupt page inputs; rebuild the binary");
        return 1;
    };
    crate::kpse::prefetch_startup();
    // The lowering's reports went to stderr before its VM ran; here the
    // lowering happened at compile time, so they are replayed at the same point.
    crate::latex::release_reports(&embedded.reports);

    // fusevm prints a VM error itself and answers 1; a clean run answers 0.
    let code = fusevm::aot::fusevm_aot_run_embedded();
    if code != 0 {
        return code;
    }
    if let Err(fault) = crate::runtime::finish_run() {
        eprintln!("! {fault}.");
        return 1;
    }
    let msgs = crate::runtime::take_messages();
    let text = crate::runtime::take_text();

    let path = std::path::Path::new(&embedded.name);
    // The job is the document the binary was compiled from, whatever the
    // binary itself is called: the PDF's `/ID` is made from the job's name.
    if let Some(stem) = path.file_stem() {
        crate::lua::set_jobname(&stem.to_string_lossy());
    }
    let pdf = crate::typeset::to_pdf(
        &text,
        &embedded.families,
        &embedded.layout,
        embedded.page_colour,
        path.parent(),
    );
    let out = path.with_extension("pdf");
    if pdf.pages > 0 {
        if let Err(e) = std::fs::write(&out, &pdf.bytes) {
            eprintln!("! {}: {e}.", out.display());
            return 1;
        }
    }
    println!(
        "{}",
        crate::file_line(&embedded.name, &msgs.join(" "), embedded.ended)
    );
    println!("{}", crate::pdf_output_line(&out, pdf.pages, &pdf.bytes));
    0
}

/// Everything a `--aot` binary needs beside its chunk; see the module docs.
#[derive(Debug, Clone, PartialEq)]
pub struct Embedded {
    /// The document as it was named to `--aot`, which the file line prints and
    /// the PDF is named after.
    pub name: String,
    pub families: Families,
    pub layout: Layout,
    pub page_colour: Option<Rgb>,
    /// Whether `\end` stopped the file ([`crate::source_ends_run`]).
    pub ended: bool,
    /// The lines the lowering reported on stderr.
    pub reports: Vec<String>,
}

/// Leads the encoding, so a stub from an incompatible texrs is refused rather
/// than misread.
const MAGIC: &[u8] = b"texrs-aot-page-1\0";

impl Embedded {
    /// The bytes [`Embedded::decode`] reads back.
    ///
    /// Every struct is destructured without `..`, so a field added to any of
    /// them is a compile error here until it is carried -- a page input the
    /// binary silently dropped is exactly the bug this format exists to fix.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer(MAGIC.to_vec());
        let Embedded {
            name,
            families,
            layout,
            page_colour,
            ended,
            reports,
        } = self;
        w.str(name);
        let Families {
            main,
            main_file,
            sans,
            sans_file,
            mono,
            mono_file,
            fallbacks,
        } = families;
        w.opt_str(main);
        w.font_file(main_file);
        w.opt_str(sans);
        w.font_file(sans_file);
        w.opt_str(mono);
        w.font_file(mono_file);
        w.strs(fallbacks);
        let Layout {
            measure,
            height,
            leading,
            margin,
            size,
            breaking,
            paging,
        } = layout;
        for v in [measure, height, leading, margin, size] {
            w.f64(*v);
        }
        let Params {
            pretolerance,
            tolerance,
            line_penalty,
            hyphen_penalty,
            ex_hyphen_penalty,
            adj_demerits,
            double_hyphen_demerits,
            final_hyphen_demerits,
        } = breaking;
        for v in [
            pretolerance,
            tolerance,
            line_penalty,
            hyphen_penalty,
            ex_hyphen_penalty,
            adj_demerits,
            double_hyphen_demerits,
            final_hyphen_demerits,
        ] {
            w.f64(*v);
        }
        let PageParams {
            club,
            widow,
            broken,
        } = paging;
        for v in [club, widow, broken] {
            w.f64(*v);
        }
        match page_colour {
            Some((r, g, b)) => {
                w.0.push(1);
                for v in [r, g, b] {
                    w.f64(*v);
                }
            }
            None => w.0.push(0),
        }
        w.0.push(u8::from(*ended));
        w.strs(reports);
        w.0
    }

    /// Read [`Embedded::encode`]'s bytes; `None` for anything else.
    pub fn decode(bytes: &[u8]) -> Option<Embedded> {
        let mut r = Reader(bytes.strip_prefix(MAGIC)?);
        let name = r.str()?;
        let families = Families {
            main: r.opt_str()?,
            main_file: r.font_file()?,
            sans: r.opt_str()?,
            sans_file: r.font_file()?,
            mono: r.opt_str()?,
            mono_file: r.font_file()?,
            fallbacks: r.strs()?,
        };
        let layout = Layout {
            measure: r.f64()?,
            height: r.f64()?,
            leading: r.f64()?,
            margin: r.f64()?,
            size: r.f64()?,
            breaking: Params {
                pretolerance: r.f64()?,
                tolerance: r.f64()?,
                line_penalty: r.f64()?,
                hyphen_penalty: r.f64()?,
                ex_hyphen_penalty: r.f64()?,
                adj_demerits: r.f64()?,
                double_hyphen_demerits: r.f64()?,
                final_hyphen_demerits: r.f64()?,
            },
            paging: PageParams {
                club: r.f64()?,
                widow: r.f64()?,
                broken: r.f64()?,
            },
        };
        let page_colour = match r.byte()? {
            0 => None,
            1 => Some((r.f64()?, r.f64()?, r.f64()?)),
            _ => return None,
        };
        let ended = match r.byte()? {
            0 => false,
            1 => true,
            _ => return None,
        };
        let reports = r.strs()?;
        match r.0.is_empty() {
            true => Some(Embedded {
                name,
                families,
                layout,
                page_colour,
                ended,
                reports,
            }),
            false => None,
        }
    }
}

/// Little-endian, length-prefixed: a string is a `u32` byte count and its UTF-8.
struct Writer(Vec<u8>);

impl Writer {
    fn f64(&mut self, v: f64) {
        self.0.extend_from_slice(&v.to_bits().to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.0.extend_from_slice(&(s.len() as u32).to_le_bytes());
        self.0.extend_from_slice(s.as_bytes());
    }
    fn opt_str(&mut self, s: &Option<String>) {
        match s {
            Some(s) => {
                self.0.push(1);
                self.str(s);
            }
            None => self.0.push(0),
        }
    }
    fn strs(&mut self, v: &[String]) {
        self.0.extend_from_slice(&(v.len() as u32).to_le_bytes());
        for s in v {
            self.str(s);
        }
    }
    fn font_file(&mut self, f: &FontFile) {
        let FontFile {
            path,
            upright,
            bold,
            italic,
            scale,
            extension,
        } = f;
        for s in [path, upright, bold, italic, scale, extension] {
            self.opt_str(s);
        }
    }
}

/// [`Writer`]'s reader; every method is `None` on truncated or invalid input.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let (head, rest) = (self.0.get(..n)?, self.0.get(n..)?);
        self.0 = rest;
        Some(head)
    }
    fn byte(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn f64(&mut self) -> Option<f64> {
        Some(f64::from_bits(u64::from_le_bytes(
            self.take(8)?.try_into().ok()?,
        )))
    }
    fn str(&mut self) -> Option<String> {
        let n = self.u32()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
    fn opt_str(&mut self) -> Option<Option<String>> {
        match self.byte()? {
            0 => Some(None),
            1 => Some(Some(self.str()?)),
            _ => None,
        }
    }
    fn strs(&mut self) -> Option<Vec<String>> {
        let n = self.u32()?;
        (0..n).map(|_| self.str()).collect()
    }
    fn font_file(&mut self) -> Option<FontFile> {
        Some(FontFile {
            path: self.opt_str()?,
            upright: self.opt_str()?,
            bold: self.opt_str()?,
            italic: self.opt_str()?,
            scale: self.opt_str()?,
            extension: self.opt_str()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every field set off its default, and to a value no other field has, so
    /// a decode that reads two fields in the wrong order cannot round-trip.
    fn distinct() -> Embedded {
        let file = |k: &str| FontFile {
            path: Some(format!("{k}-path")),
            upright: Some(format!("{k}-up")),
            bold: None,
            italic: Some(format!("{k}-it")),
            scale: Some(format!("{k}-scale")),
            extension: Some(format!("{k}-ext")),
        };
        Embedded {
            name: "dir/doc.tex".into(),
            families: Families {
                main: Some("Main".into()),
                main_file: file("m"),
                sans: None,
                sans_file: file("s"),
                mono: Some("Mono".into()),
                mono_file: file("t"),
                fallbacks: vec!["A".into(), "B".into()],
            },
            layout: Layout {
                measure: 1.0,
                height: 2.0,
                leading: 3.0,
                margin: 4.0,
                size: 5.0,
                breaking: Params {
                    pretolerance: 6.0,
                    tolerance: 7.0,
                    line_penalty: 8.0,
                    hyphen_penalty: 9.0,
                    ex_hyphen_penalty: 10.0,
                    adj_demerits: 11.0,
                    double_hyphen_demerits: 12.0,
                    final_hyphen_demerits: 13.0,
                },
                paging: PageParams {
                    club: 14.0,
                    widow: 15.0,
                    broken: 16.0,
                },
            },
            page_colour: Some((0.25, 0.5, 0.75)),
            ended: true,
            reports: vec!["package x needs \\y".into(), String::new()],
        }
    }

    #[test]
    fn page_inputs_round_trip_field_for_field() {
        let e = distinct();
        assert_eq!(Embedded::decode(&e.encode()), Some(e));
    }

    #[test]
    fn truncated_or_foreign_page_inputs_are_refused() {
        let bytes = distinct().encode();
        for cut in [0, MAGIC.len(), bytes.len() / 2, bytes.len() - 1] {
            assert_eq!(Embedded::decode(&bytes[..cut]), None, "cut at {cut}");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert_eq!(Embedded::decode(&longer), None);
        assert_eq!(Embedded::decode(b"not a texrs stub"), None);
    }
}
