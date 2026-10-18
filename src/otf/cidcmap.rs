//! The `/ToUnicode` CMap luatex writes for a CID font: `write_cid_tounicode`
//! of LuaTeX 1.24's font/tounicode.c. Each used glyph (CID) gets the
//! `tounicode` of the first used character, in character order, that is set
//! with it -- or, for a font without `tounicode` data, that character's
//! code -- and runs of consecutive codes become `bfrange`s, a hundred to a
//! block, the rest `bfchar`s.

use std::collections::BTreeMap;

/// What a CID maps to (`gtab[j]`).
#[derive(Clone, Debug, PartialEq)]
enum Code {
    /// A code point: the fallback to the character code.
    Code(u32),
    /// A `tounicode` hex string.
    Text(String),
}

/// `utf16be_str`: a code point as UTF-16BE hex.
fn utf16be(code: u32) -> String {
    if code < 0x10000 {
        format!("{code:04X}")
    } else {
        let v = code - 0x10000;
        format!("{:04X}{:04X}", v / 0x400 + 0xD800, v % 0x400 + 0xDC00)
    }
}

/// The CMap's text. `tag` is the subset tag (`UCS` without one), `fontname`
/// the font's name in the descriptor; `used` the characters set in the
/// font with their glyph index and `tounicode`, and `font_tounicode`
/// whether the font carries `tounicode` at all (`font_tounicode(f)`).
pub fn write_cid_tounicode(
    tag: Option<&str>,
    fontname: &str,
    used: &BTreeMap<u32, (u16, Option<String>)>,
    font_tounicode: bool,
) -> String {
    let buf = format!("{}-{fontname}", tag.unwrap_or("UCS"));
    let mut out = format!(
        "%!PS-Adobe-3.0 Resource-CMap\n\
         %%DocumentNeededResources: ProcSet (CIDInit)\n\
         %%IncludeResource: ProcSet (CIDInit)\n\
         %%BeginResource: CMap (TeX-{buf}-0)\n\
         %%Title: (TeX-{buf}-0 TeX {buf} 0)\n\
         %%Version: 1.000\n\
         %%EndComments\n\
         /CIDInit /ProcSet findresource begin\n\
         12 dict begin\n\
         begincmap\n\
         /CIDSystemInfo\n\
         << /Registry (TeX)\n\
         /Ordering ({buf})\n\
         /Supplement 0\n\
         >> def\n\
         /CMapName /TeX-Identity-{buf} def\n\
         /CMapType 2 def\n\
         1 begincodespacerange\n\
         <0000> <FFFF>\n\
         endcodespacerange\n"
    );
    // gtab: the first used character of each glyph decides
    let mut gtab: BTreeMap<u16, Code> = BTreeMap::new();
    for (&c, (glyph, tu)) in used {
        if gtab.contains_key(glyph) {
            continue;
        }
        if font_tounicode {
            if let Some(t) = tu {
                gtab.insert(*glyph, Code::Text(t.clone()));
            }
        } else {
            gtab.insert(*glyph, Code::Code(c));
        }
    }
    // range_size
    let get = |i: u32| gtab.get(&(i as u16));
    let mut range: BTreeMap<u32, u32> = BTreeMap::new();
    let mut i = 0u32;
    while i < 65536 {
        match get(i) {
            Some(Code::Text(_)) => {
                range.insert(i, 1);
                i += 1;
            }
            None => i += 1,
            Some(Code::Code(code)) => {
                let j = i;
                let mut k = i % 256;
                let mut code = *code;
                while i < 65535 && k < 255 {
                    match get(i + 1) {
                        Some(Code::Code(next)) if code + 1 == *next => {
                            code = *next;
                            i += 1;
                            k += 1;
                        }
                        _ => break,
                    }
                }
                i += 1;
                range.insert(j, i - j);
            }
        }
    }
    let ranges: Vec<(u32, u32)> = range
        .iter()
        .filter(|(_, &n)| n > 1)
        .map(|(&a, &n)| (a, n))
        .collect();
    let chars: Vec<u32> = range
        .iter()
        .filter(|(_, &n)| n == 1)
        .map(|(&a, _)| a)
        .collect();
    for block in ranges.chunks(100) {
        out.push_str(&format!("{} beginbfrange\n", block.len()));
        for &(a, n) in block {
            let Some(Code::Code(code)) = get(a) else {
                continue;
            };
            out.push_str(&format!(
                "<{a:04X}> <{:04X}> <{}>\n",
                a + n - 1,
                utf16be(*code)
            ));
        }
        out.push_str("endbfrange\n");
    }
    // the bfchar block is written once even when it is empty
    let blocks: Vec<&[u32]> = if chars.is_empty() {
        vec![&[]]
    } else {
        chars.chunks(100).collect()
    };
    for block in blocks {
        out.push_str(&format!("{} beginbfchar\n", block.len()));
        for &a in block {
            match get(a) {
                Some(Code::Text(t)) => out.push_str(&format!("<{a:04X}> <{t}>\n")),
                Some(Code::Code(c)) => out.push_str(&format!("<{a:04X}> <{}>\n", utf16be(*c))),
                None => {}
            }
        }
        out.push_str("endbfchar\n");
    }
    out.push_str(
        "endcmap\n\
         CMapName currentdict /CMap defineresource pop\n\
         end\n\
         end\n\
         %%EndResource\n\
         %%EOF\n",
    );
    out
}
