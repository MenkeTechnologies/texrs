//! Inline Rust in a TeX document: `\rust{ ... }` blocks.
//!
//! The work is fusevm's — [`fusevm::RustSugar`] rewrites the block at the source
//! level, and `fusevm::ffi` compiles it with `rustc`, `dlopen`s the result and
//! marshals calls. This module supplies the TeX-flavoured config and the
//! [`desugar`] entry the pipeline runs before the mouth ever sees the file.
//!
//! **Why before the mouth.** A `rust { … }` body is Rust, not TeX: it is full of
//! `#`, `{`, `}`, `&`, `_` and `$`, every one of which is a category code the
//! mouth would act on. Tokenising it would be meaningless, so the block is
//! lifted out textually first and replaced by a control sequence the engine does
//! understand. The replacement is padded with newlines so a diagnostic after the
//! block still lands on the right line.
//!
//! **The replacement carries no braces.** `\rustcompile <base64>\endrust` is
//! read correctly whatever the catcodes happen to be at that point in the file:
//! the escape character is the only category the emitted text depends on, and a
//! document that changed THAT would not be able to write a control sequence at
//! all. A brace-delimited form would break in a file whose `\catcode`\{=1` line
//! comes after the block.
//!
//! ```tex
//! \rust{
//!     #[no_mangle]
//!     pub extern "C" fn twice(n: i64) -> i64 { n * 2 }
//! }
//! \catcode`\{=1 \catcode`\}=2
//! \count1=21
//! \message{\rustcall twice \count1 \endrust}   % => 42
//! ```

use fusevm::RustSugar;

/// The control sequence a `\rust{ … }` block becomes, and the one that ends it.
pub const COMPILE_CS: &str = "rustcompile";
/// Terminator for both the compile blob and a call's argument list.
pub const END_CS: &str = "endrust";
/// The control sequence that calls into a compiled block.
pub const CALL_CS: &str = "rustcall";

/// The statement a `\rust{ … }` block desugars to.
///
/// The line number is not emitted: TeX has no comment syntax that survives a
/// catcode change (`%` is only a comment while its category says so), and the
/// engine already knows the line from the newline padding the desugarer keeps.
fn emit(b64: &str, _line: usize) -> String {
    format!("\\{COMPILE_CS} {b64}\\{END_CS} ")
}

/// TeX desugar config.
///
/// The keyword is the bare word every sibling frontend uses, because fusevm's
/// scanner matches an IDENTIFIER: a backslash cannot start one, so `\rust`
/// could never match. [`desugar`] rewrites the TeX spelling into this one
/// first, so a document may write either and the fleet keeps one block syntax.
/// Comments are `%` to end of line — under the default category codes, which is
/// the only assumption a pass that runs before the mouth can make.
/// `newline_boundary` is true because TeX is line-oriented: a construct begins
/// wherever a line does.
pub const SUGAR: RustSugar = RustSugar {
    keyword: "rust",
    line_comments: &["%"],
    block_comment: None,
    newline_boundary: true,
    emit,
};

/// Rewrite every `\rust{ … }` (or `rust { … }`) block into
/// `\rustcompile <base64>\endrust`.
///
/// A no-op — and a single substring scan — when the source has no `rust`.
pub fn desugar(src: &str) -> String {
    SUGAR.desugar(&tex_spelling(src))
}

/// Rewrite the TeX spelling `\rust{` into the fleet's `rust {`.
///
/// The two are the same length, so nothing downstream shifts: fusevm's scanner
/// tracks lines, and a replacement that changed the byte count would move every
/// diagnostic after it. Only `\rust` immediately followed by a brace is
/// rewritten, so a document that defines a macro called `\rustle` is untouched.
fn tex_spelling(src: &str) -> String {
    if !src.contains("\\rust") {
        return src.to_string();
    }
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(at) = rest.find("\\rust") {
        let (before, from) = rest.split_at(at);
        out.push_str(before);
        let after = &from["\\rust".len()..];
        match after.starts_with('{') {
            true => {
                out.push_str("rust ");
                rest = after;
            }
            false => {
                out.push_str("\\rust");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}
