//! OpenType fonts as luaotfload's node mode loads them.
//!
//! `mode=node` is luaotfload's default and fontspec's: the font is read by
//! the ConTeXt fontloader that luaotfload carries (`fontloader-font-otr.lua`
//! and the files around it, merged into `fontloader-2023-12-28.lua` in TeX
//! Live 2026), not by HarfBuzz, and its numbers are its own -- a glyph's
//! height is the top of a box that counts curve control points, a duplicate
//! code point is a copy of the glyph rather than an alias, and a glyph no
//! Unicode reaches lives at a private slot from U+F0000 on. This module is a
//! port of that loader, file by file:
//!
//! - [`stream`]: the byte reader;
//! - [`otr`]: the table directory and the tables the metrics read;
//! - [`cff`]: CFF dictionaries, charsets and charstring bounding boxes;
//! - [`dsp`]: GDEF, the GSUB and GPOS lookups, the old `kern` table, MATH;
//! - [`oup`]: the glyphs keyed by Unicode, duplicates, the Unicode a glyph is
//!   guessed to stand for, and the lookups compacted, rekeyed and expanded;
//! - [`tfm`]: the scaled characters and parameters.
//!
//! The order is `otf.load`'s (fontloader-font-otl.lua:132-290): read,
//! `compact`, `rehash` (unify the glyphs, then the lookups, copy the
//! duplicates, guess the missing Unicodes, strip the redundant names), then
//! after the cache round trip `expand` (heights, depths and the contextual
//! rules). A face is read once and shared by `Arc`; nothing in it changes
//! after loading.

pub mod cff;
pub mod dsp;
pub mod features;
pub mod nodes;
pub mod normalize;
pub mod otc;
pub mod otj;
pub mod otr;
pub mod ots;
pub mod ott;
pub mod oup;
pub mod stream;
pub mod tfm;

use std::collections::BTreeMap;
use std::sync::Arc;

use oup::Description;

/// A font file's face as node mode holds it: `otf.load`'s `data`, the
/// descriptions keyed by Unicode or private slot.
#[derive(Clone, Debug)]
pub struct Face {
    pub descriptions: BTreeMap<u32, Description>,
    pub metadata: tfm::Metadata,
    /// `opentype` (CFF outlines) or `truetype`.
    pub format: &'static str,
    /// Name to Unicode (`resources.unicodes`).
    pub unicodes: BTreeMap<String, u32>,
    /// The next unused private slot.
    pub private: u32,
    /// The lookups, marks and features, keyed by Unicode.
    pub layout: dsp::Layout,
    /// The contextual rules `expandlookups` builds.
    pub expanded: oup::Expanded,
}

impl Face {
    /// Load face `sub` (0 for the first) of the font at `path`.
    pub fn open(path: impl AsRef<std::path::Path>, sub: usize) -> Result<Arc<Face>, String> {
        let path = path.as_ref();
        let data =
            std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Face::from_bytes(&data, sub)
    }

    /// `otf.load` over a file already in memory.
    pub fn from_bytes(data: &[u8], sub: usize) -> Result<Arc<Face>, String> {
        let offsets = otr::face_offsets(data);
        let offset = *offsets
            .get(sub)
            .ok_or_else(|| "not an OpenType font, or no such face in the collection".to_string())?;
        let (mut fontdata, mut layout) = otr::read(data, offset);
        oup::compact(&mut layout);

        // readers.rehash(data, "unicodes") (font-oup.lua:1275-1295)
        let (mut descriptions, indices, private) = oup::unify_glyphs(&mut fontdata);
        let ix = |g: u32| indices.get(g as usize).copied();
        for (gid, mut m) in std::mem::take(&mut layout.math) {
            for list in [m.vvariants.as_mut(), m.hvariants.as_mut()]
                .into_iter()
                .flatten()
            {
                for g in list.iter_mut() {
                    *g = ix(*g).unwrap_or(*g);
                }
            }
            for parts in [m.vparts.as_mut(), m.hparts.as_mut()].into_iter().flatten() {
                for p in parts.iter_mut() {
                    p.glyph = ix(p.glyph).unwrap_or(p.glyph);
                }
            }
            if let Some(d) = ix(gid).and_then(|u| descriptions.get_mut(&u)) {
                d.math = Some(m);
            }
        }
        oup::unify_resources(&mut layout, &indices, &fontdata.duplicates);
        oup::copy_duplicates(&mut descriptions, &fontdata.duplicates);
        // unifymissing (font-oup.lua:748-772): the names of the glyphs that
        // have a real Unicode, then addtounicode with the lookups.
        let mut unicodes = BTreeMap::new();
        for (&u, d) in &descriptions {
            if u < otr::PRIVATE && !(0xE000..=0xF8FF).contains(&u) {
                if let Some(n) = &d.name {
                    unicodes.insert(n.clone(), u);
                }
            }
        }
        oup::add_tounicode(&mut descriptions, &mut unicodes, &oup::lookups_of(&layout));
        oup::strip_redundant(&mut descriptions, true);
        let unicodes = oup::unicode_table(&descriptions);

        // readers.expand (font-oup.lua:3129-3335)
        oup::expand_dimensions(&mut descriptions, 0.0);
        let mut expanded = oup::expand_lookups(&mut layout);
        // otfenhancers.apply: the "check extra features" enhancer
        otc::enhance(&descriptions, &mut layout, &mut expanded);

        let m = fontdata.windowsmetrics.clone().unwrap_or_default();
        let post = fontdata.postscript.clone().unwrap_or_default();
        let head = fontdata.fontheader.clone().unwrap_or_default();
        let metadata = tfm::Metadata {
            units: head.units as f64,
            designsize: layout.designsize,
            monospaced: post.monospaced > 0 || m.panose[3] == 9,
            averagewidth: Some(m.averagewidth as f64),
            xheight: m.xheight.map(|x| x as f64),
            capheight: m.capheight.map(|x| x as f64),
            ascender: Some(m.typoascender as f64),
            descender: Some(m.typodescender as f64),
            italicangle: post.italicangle,
            fontname: fontdata.getname("postscriptname"),
            fullname: fontdata.getname("fullname"),
        };
        Ok(Arc::new(Face {
            descriptions,
            metadata,
            format: fontdata.format,
            unicodes,
            private,
            layout,
            expanded,
        }))
    }

    /// The face at `size` scaled points.
    pub fn scale(&self, size: i64) -> tfm::Scaled {
        tfm::scale(&self.descriptions, &self.metadata, size)
    }

    /// `resources.classes[u]` (fontloader-font-otl.lua:275-285): the GDEF
    /// class of a description, `base` when it has none.
    pub fn class(&self, u: u32) -> &str {
        self.descriptions
            .get(&u)
            .and_then(|d| d.class.as_deref())
            .unwrap_or("base")
    }
}
