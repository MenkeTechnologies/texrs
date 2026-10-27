//! OpenType shaping as LuaHBTeX's `mode=harf` does it.
//!
//! A `\font` that luaotfload loads with `mode=harf` is not a TeX font in the
//! `.tfm` sense: its characters table is built from the font file by
//! `luaotfload-harf-define.lua`, and every list that uses it is handed, run by
//! run, to HarfBuzz by `luaotfload-harf-plug.lua` before the line breaker or
//! `hpack` sees it. Those two files (luaotfload 3.29, TeX Live 2026) are the
//! specification of this module, and it is a port of them rather than a shaper
//! that happens to call HarfBuzz: which slot a glyph node gets, when a font
//! kern is inserted and how big it is, how an interword glue's width moves,
//! and how a discretionary is reshaped are all theirs, cited by line.
//!
//! The two halves:
//!
//! - [`HarfFace`] and [`HarfFont`] are `loadfont` and `scalefont`
//!   (harf-define.lua:107 and :377): the metrics of every glyph in design
//!   units, read once per file, and the same metrics at a size.
//! - [`process`] is `process` (harf-plug.lua:1043): `itemize` cuts a list into
//!   runs, `shape` sends each to HarfBuzz and reshapes the discretionaries it
//!   crosses, and `tonodes` turns the glyphs back into nodes.
//!
//! The list is luatex's, not texrs's: [`Item`] carries what the Lua code reads
//! and writes on a node (a glyph's character and offsets, a glue's subtype and
//! width, a discretionary's three lists), because a stomach that wants
//! luatex's output has to give the shaper luatex's input. The port works on a
//! doubly linked arena (`List`) since the Lua code edits a linked list in
//! place -- splicing a discretionary's replacement in, inserting kerns before
//! or after a glyph, copying stretches of nodes into a new `pre` -- and a port
//! that rewrote that into slice operations would be a different algorithm.
//!
//! HarfBuzz itself is harfrust, the HarfBuzz project's Rust port; LuaHBTeX
//! 1.24.0 links HarfBuzz 12.3.2.
//!
//! Every dimension a Lua table hands the engine as a float is turned into a
//! scaled point the way the engine turns it: `floor(x + 0.5)`. Measured, with
//! `lmroman10-regular.otf` at 500sp, where the halves are exact: the space is
//! 333 units, 166.5sp, and luatex's glue is 167; the `AV` kern is -111 units,
//! -55.5sp, and luatex's kern is -55 -- half up, not half away from zero.
//!
//! Nothing here is mutable once built: a face is shared by `Arc` between every
//! size of it and every thread that shapes with it, and the one piece of
//! luaotfload that IS order-dependent state -- the first `/ToUnicode` a glyph
//! is given (harf-plug.lua:969-976) -- is left to [`ToUnicode`], which the
//! caller walks in document order, so a parallel run cannot change the PDF.

use std::collections::BTreeMap;
use std::sync::Arc;

use harfrust::{
    Buffer, ClusterLevel, Direction, Feature, GlyphId, Language, Script, ShapeOptions, ShaperFont,
};
use read_fonts::{FontRef, TableProvider};

/// `gid_offset` (harf-define.lua:335): the slot of a glyph that no Unicode
/// character maps to is this plus its index.
pub const GID_OFFSET: u32 = 0x12_0000;

/// U+FFFC, the code a node that is not a character of the font is shaped as
/// (harf-plug.lua:198) -- unless the font itself maps U+FFFC, in which case
/// harf-define.lua:470 uses a lone surrogate no font can map.
const OBJECT_REPLACEMENT: u32 = 0xFFFC;

/// What the engine does with a float a Lua table hands it as a dimension.
fn round(x: f64) -> i64 {
    (x + 0.5).floor() as i64
}

/// Why a font could not be loaded or shaped with, in luaotfload's words where
/// it has any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapeError(pub String);

impl std::fmt::Display for ShapeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One glyph's metrics in design units, as `loadfont` records them
/// (harf-define.lua:270-288). Floats because the fallbacks are `upem * .8`
/// and `upem * .2`, which need not be whole.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphUnits {
    pub width: f64,
    pub height: f64,
    pub depth: f64,
    pub italic: f64,
}

/// Which outlines the font carries, as harf-define.lua:221-235 decides it --
/// the engine reads this to know whether there is a CFF table to embed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outlines {
    /// `CFF ` or `CFF2`: luaotfload's `"opentype"`.
    Cff,
    /// `glyf`: luaotfload's `"truetype"`.
    TrueType,
}

/// A font file, read once: `loadfont`'s `cached` table (harf-define.lua:331).
pub struct HarfFace {
    /// The shaping face; `Font` clones share one parse.
    font: harfrust::Font,
    data: Arc<Vec<u8>>,
    index: u32,
    pub upem: u16,
    /// The GPOS `size` feature's design size in scaled points, or 10pt.
    pub designsize: i64,
    pub outlines: Option<Outlines>,
    /// The advance of the glyph U+0020 maps to (glyph 0's when none does).
    pub real_space: f64,
    /// `real_space` when the font has a space, else half an em.
    pub tex_space: f64,
    pub xheight: f64,
    pub capheight: f64,
    /// `post`'s italic angle as `-tan(angle) * 65536`, which is how `slant`
    /// is stored in a TeX font's parameters.
    pub slant: f64,
    /// Every glyph, by index.
    pub glyphs: Vec<GlyphUnits>,
    /// Glyph to the character the engine calls it (`nominals`): the LAST
    /// Unicode, in increasing order, that maps to it, because
    /// harf-define.lua:293-299 overwrites as it walks the sorted set.
    nominals: BTreeMap<u32, u32>,
    /// Character to glyph (`unicodes`).
    unicodes: BTreeMap<u32, u32>,
    pub psname: String,
    pub fullname: String,
}

impl std::fmt::Debug for HarfFace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HarfFace")
            .field("psname", &self.psname)
            .field("upem", &self.upem)
            .field("glyphs", &self.glyphs.len())
            .finish()
    }
}

/// `get_designsize` (harf-define.lua:55-80): the first `size` feature's
/// design size, in decipoints, times 2^16/10 -- or 10pt when there is no
/// GPOS or no such feature.
fn designsize(gpos: Option<&[u8]>) -> i64 {
    let be16 = |b: &[u8], at: usize| -> Option<usize> {
        Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]) as usize)
    };
    let found = (|| {
        let buf = gpos.filter(|b| !b.is_empty())?;
        let feature_off = be16(buf, 6)?;
        let count = be16(buf, feature_off)?;
        for k in 0..count {
            let rec = feature_off + 2 + 6 * k;
            if buf.get(rec..rec + 4)? == b"size" {
                let feature = feature_off + be16(buf, rec + 4)?;
                let params = feature + be16(buf, feature)?;
                return Some((be16(buf, params)? as f64 * 6553.6 + 0.5).floor() as i64);
            }
        }
        None
    })();
    found.unwrap_or(655_360)
}

impl HarfFace {
    /// Read the font at `path`, face `sub` of a collection (0 for the first,
    /// which is luaotfload's `sub - 1`).
    pub fn open(path: impl AsRef<std::path::Path>, sub: u32) -> Result<Arc<HarfFace>, ShapeError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)
            .map_err(|e| ShapeError(format!("cannot read {}: {e}", path.display())))?;
        HarfFace::from_bytes(bytes, sub)
    }

    /// `loadfont` over bytes already in memory.
    pub fn from_bytes(bytes: Vec<u8>, sub: u32) -> Result<Arc<HarfFace>, ShapeError> {
        let data = Arc::new(bytes);
        let blob: Arc<dyn AsRef<[u8]> + Send + Sync> = data.clone();
        // harf-define.lua:207-210: a face with no tables is not a font
        // HarfBuzz can handle, and the request fails.
        let font = harfrust::Font::new(blob, sub)
            .ok_or_else(|| ShapeError("not an OpenType font HarfBuzz can read".into()))?;
        let raw = FontRef::from_index(&data, sub)
            .map_err(|e| ShapeError(format!("not an OpenType font: {e}")))?;
        let table = |tag: &[u8; 4]| raw.data_for_tag(read_fonts::types::Tag::new(tag));

        let upem = font.units_per_em();
        let outlines = match () {
            _ if table(b"CFF ").is_some() || table(b"CFF2").is_some() => Some(Outlines::Cff),
            _ if table(b"glyf").is_some() => Some(Outlines::TrueType),
            _ => None,
        };

        let shaper = ShaperFont::new(&font);
        // harf-define.lua:237-239: the font's horizontal extents, falling back
        // to 0.8 and 0.2 of an em.
        let (ascender, descender) =
            h_extents(&raw).unwrap_or((upem as f64 * 0.8, upem as f64 * 0.2));

        let space_gid = shaper.nominal_glyph(0x20);
        let real_space = shaper.glyph_h_advance(space_gid.unwrap_or(GlyphId::new(0))) as f64;
        let tex_space = match space_gid {
            Some(_) => real_space,
            None => upem as f64 / 2.0,
        };

        // harf-define.lua:245-256: `post` version 3.0 or earlier carries the
        // italic angle as a 16.16 fixed number at offset 4.
        let mut slant = 0.0;
        if let Some(post) = table(b"post") {
            let post = post.as_bytes();
            if post.len() >= 32 {
                let version = i32::from_be_bytes([post[0], post[1], post[2], post[3]]);
                if version <= 0x0003_0000 {
                    let angle =
                        i32::from_be_bytes([post[4], post[5], post[6], post[7]]) as f64 / 65536.0;
                    if angle != 0.0 {
                        slant = -(angle * std::f64::consts::PI / 180.0).tan() * 65536.0;
                    }
                }
            }
        }

        // harf-define.lua:267-288, every glyph up front.
        let count = font.num_glyphs();
        let autoitalic = (slant != 0.0).then_some(20.0);
        let glyphs: Vec<GlyphUnits> = (0..count)
            .map(|gid| {
                let g = GlyphId::new(gid);
                let width = shaper.glyph_h_advance(g) as f64;
                let (mut height, mut depth, mut italic) = (None, None, None);
                if let Some(e) = shaper.glyph_extents(g) {
                    height = Some(e.y_bearing as f64);
                    depth = Some((e.y_bearing + e.height) as f64);
                    let right_bearing = (e.x_bearing + e.width) as f64 - width;
                    if let Some(a) = autoitalic {
                        if right_bearing > -a {
                            italic = Some(right_bearing + a);
                        }
                    }
                }
                GlyphUnits {
                    width,
                    height: height.unwrap_or(ascender),
                    depth: -(depth.unwrap_or(descender)),
                    italic: italic.unwrap_or(0.0),
                }
            })
            .collect();

        // harf-define.lua:290-299.
        let mut unicodes = BTreeMap::new();
        let mut nominals = BTreeMap::new();
        let charmap = font.charmap();
        let mut codes: Vec<u32> = charmap.iter_unicodes().map(|(u, _)| u).collect();
        codes.sort_unstable();
        codes.dedup();
        for uni in codes {
            if let Some(g) = shaper.nominal_glyph(uni) {
                unicodes.insert(uni, g.to_u32());
                nominals.insert(g.to_u32(), uni);
            }
        }

        // harf-define.lua:301-329: OS/2 from version 2 on states both
        // heights; otherwise the x's and the X's glyph heights, then the
        // ascender.
        let (mut xheight, mut capheight) = (0.0, 0.0);
        if let Some(os2) = table(b"OS/2") {
            let os2 = os2.as_bytes();
            if os2.len() >= 96 && u16::from_be_bytes([os2[0], os2[1]]) > 1 {
                xheight = u16::from_be_bytes([os2[86], os2[87]]) as f64;
                capheight = u16::from_be_bytes([os2[88], os2[89]]) as f64;
            }
        }
        if xheight == 0.0 {
            xheight = match unicodes.get(&120) {
                Some(&g) => glyphs[g as usize].height,
                None => ascender / 2.0,
            };
        }
        if capheight == 0.0 {
            capheight = match unicodes.get(&88) {
                Some(&g) => glyphs[g as usize].height,
                None => ascender,
            };
        }

        let name = |id: u16| {
            font.names()
                .get(read_fonts::types::NameId::new(id))
                .english_or_first()
                .map(|s| s.name().to_string())
                .unwrap_or_default()
        };
        let psname = name(6);
        let fullname = name(4);
        let designsize = designsize(table(b"GPOS").map(|d| d.as_bytes()));
        drop(shaper);
        Ok(Arc::new(HarfFace {
            font,
            data,
            index: sub,
            upem,
            designsize,
            outlines,
            real_space,
            tex_space,
            xheight,
            capheight,
            slant,
            glyphs,
            nominals,
            unicodes,
            psname,
            fullname,
        }))
    }

    /// The file's bytes, for the subsetter.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Which face of a collection this is.
    pub fn index(&self) -> u32 {
        self.index
    }

    /// The glyph `uni` maps to.
    pub fn glyph_of(&self, uni: u32) -> Option<u32> {
        self.unicodes.get(&uni).copied()
    }

    /// The character the engine calls glyph `gid` (`nominals[gid]`).
    pub fn nominal(&self, gid: u32) -> Option<u32> {
        self.nominals.get(&gid).copied()
    }

    /// The slot a glyph node for `gid` carries: its nominal character, or
    /// `GID_OFFSET + gid` (harf-plug.lua:801).
    pub fn slot(&self, gid: u32) -> u32 {
        self.nominal(gid).unwrap_or(GID_OFFSET + gid)
    }

    /// The glyph a slot names: a character the font maps, or an offset glyph
    /// index (harf-define.lua:402-416's two kinds of key).
    pub fn glyph_of_slot(&self, slot: u32) -> Option<u32> {
        match slot.checked_sub(GID_OFFSET) {
            Some(g) if (g as usize) < self.glyphs.len() => Some(g),
            _ => self.glyph_of(slot),
        }
    }
}

/// `hb_ot_get_font_h_extents`: `OS/2`'s typographic ascender and descender
/// when `fsSelection` asks for them (bit 7, USE_TYPO_METRICS), `hhea`'s
/// otherwise, the ascender made positive and the descender negative
/// (`_fix_ascender_descender`). `None` when neither table is there.
fn h_extents(raw: &FontRef) -> Option<(f64, f64)> {
    let fix = |a: i16, d: i16| ((a as i32).abs() as f64, -((d as i32).abs() as f64));
    if let Ok(os2) = raw.os2() {
        if os2.fs_selection().bits() & (1 << 7) != 0 {
            return Some(fix(os2.s_typo_ascender(), os2.s_typo_descender()));
        }
    }
    let hhea = raw.hhea().ok()?;
    Some(fix(hhea.ascender().to_i16(), hhea.descender().to_i16()))
}

/// One character of a scaled font (harf-define.lua:403-409), in scaled
/// points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CharMetrics {
    pub index: u32,
    pub width: i64,
    pub height: i64,
    pub depth: i64,
    pub italic: i64,
}

/// The OpenType script and language a request named, already in HarfBuzz's
/// form (harf-define.lua:496-513).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ScriptLang {
    pub script: Option<Script>,
    pub language: Option<Language>,
}

/// `luaotfload-scripts.lua`'s `script_to_iso`: an OpenType script tag as the
/// ISO 15924 tag HarfBuzz is given.
fn script_to_iso(tag: &str) -> String {
    let tag = tag.to_lowercase();
    let canonical = |t: &str| -> Option<&'static str> {
        Some(match t {
            "dflt" => "DFLT",
            "hira" => "kana",
            "laoo" | "lao " => "lao",
            "yiii" | "yi  " => "yi",
            "nkoo" | "nko " => "nko",
            "yaii" | "vai " => "vai",
            _ => return None,
        })
    };
    let versioned = |t: &str| -> Option<&'static str> {
        Some(match t {
            "mym" => "mymr",
            "mymr" => "mym",
            "bng" => "beng",
            "beng" => "bng",
            "dev" => "deva",
            "deva" => "dev",
            "gjr" => "gujr",
            "gujr" => "gjr",
            "gur" => "guru",
            "guru" => "gur",
            "knd" => "knda",
            "knda" => "knd",
            "mlm" => "mlym",
            "mlym" => "mlm",
            "ory" => "orya",
            "orya" => "ory",
            "tml" => "taml",
            "taml" => "tml",
            "tel" => "telu",
            "telu" => "tel",
            _ => return None,
        })
    };
    let mut tag = canonical(&tag).map(str::to_string).unwrap_or(tag);
    // `tag:gsub("%d$", "")`: a trailing digit is a shaping-engine version.
    if tag.ends_with(|c: char| c.is_ascii_digit()) {
        let stripped = &tag[..tag.len() - 1];
        if let Some(v) = versioned(stripped) {
            tag = v.to_string();
        }
        // A digit with no versioned twin keeps the tag as it was.
    }
    match tag.len() {
        4 => tag,
        // "I promise you, I am not making this one up": pad by repeating the
        // last character.
        0 => tag,
        n => {
            let last = tag[n - 1..].to_string();
            tag.clone() + &last.repeat(4 - n.min(4))
        }
    }
}

impl ScriptLang {
    /// harf-define.lua:496-513 from the request's `script=` and `language=`.
    pub fn from_request(script: Option<&str>, language: Option<&str>) -> ScriptLang {
        match (script, language) {
            (Some(script), language) => {
                let mut script = script.to_lowercase();
                if script == "dflt" {
                    script = "DFLT".into();
                }
                let mut lang = language.unwrap_or("dflt").to_uppercase();
                if lang == "DFLT" {
                    lang = "dflt".into();
                }
                // `to_harfbuzz` (luaotfload-scripts.lua:76-82).
                let iso = script_to_iso(&script);
                let hb_lang = format!("x-hbot{lang}-hbsc{script}");
                ScriptLang {
                    script: iso.parse().ok(),
                    language: hb_lang.parse().ok(),
                }
            }
            (None, Some(language)) => {
                let lang = language.to_uppercase();
                let lang = if lang == "DFLT" {
                    "dflt".to_string()
                } else {
                    lang
                };
                ScriptLang {
                    script: None,
                    language: lang.parse().ok(),
                }
            }
            (None, None) => ScriptLang::default(),
        }
    }
}

/// A face at a size, with the request's features: `scalefont`'s `tfmdata`
/// (harf-define.lua:377-485) reduced to what shaping and measuring read.
#[derive(Clone)]
pub struct HarfFont {
    pub face: Arc<HarfFace>,
    /// The size in scaled points, after a negative (`scaled`) size has been
    /// resolved against the design size.
    pub size: i64,
    /// `size / upem`: everything HarfBuzz returns is in design units.
    pub scale: f64,
    features: Vec<Feature>,
    pub script_lang: ScriptLang,
    /// `hb.space` (harf-define.lua:463): the real space, scaled and rounded.
    space: f64,
    /// `hb.obj_repl` (harf-define.lua:470).
    obj_repl: u32,
    /// `extend` / 1000 (harf-plug.lua:758); 1 unless the request extends.
    hfactor: f64,
}

impl std::fmt::Debug for HarfFont {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HarfFont")
            .field("face", &self.face.psname)
            .field("size", &self.size)
            .finish()
    }
}

/// A feature as a request writes it, after luaotfload has read the request:
/// `+liga` is `liga = true`, `-kern` is `kern = false`, `ss01=2` a number.
#[derive(Clone, Debug, PartialEq)]
pub enum FeatureValue {
    Bool(bool),
    Number(f64),
    Text(String),
}

impl HarfFont {
    /// `scalefont` for `size` (negative: a `scaled` factor times -1000) and
    /// the request's raw features, which are where harf-define.lua:514-527
    /// finds the HarfBuzz features: every four-letter key whose value is a
    /// boolean or a number.
    pub fn new(
        face: Arc<HarfFace>,
        size: i64,
        raw: &BTreeMap<String, FeatureValue>,
        script_lang: ScriptLang,
    ) -> HarfFont {
        let size = match size < 0 {
            true => round(size as f64 * face.designsize as f64 / -1000.0),
            false => size,
        };
        let scale = size as f64 / face.upem as f64;
        let mut features = Vec::new();
        for (key, value) in raw {
            if key.len() != 4 {
                continue;
            }
            let setting = match value {
                FeatureValue::Bool(on) => format!("{}{key}", if *on { '+' } else { '-' }),
                FeatureValue::Number(n) => format!("+{key}={}", *n as i64),
                FeatureValue::Text(t) => match t.parse::<f64>() {
                    Ok(n) => format!("+{key}={}", n as i64),
                    Err(_) => continue,
                },
            };
            if let Ok(f) = setting.parse::<Feature>() {
                features.push(f);
            }
        }
        let obj_repl = match face.glyph_of(OBJECT_REPLACEMENT) {
            Some(_) => 0xD800,
            None => OBJECT_REPLACEMENT,
        };
        let space = ((face.real_space * scale + 0.5) / 1.0).floor();
        let hfactor = match raw.get("extend") {
            Some(FeatureValue::Number(e)) => e / 1000.0,
            _ => 1.0,
        };
        HarfFont {
            face,
            size,
            scale,
            features,
            script_lang,
            space,
            obj_repl,
            hfactor,
        }
    }

    /// The character at `slot`, in scaled points: the engine's view of
    /// `characters[slot]`.
    pub fn char_metrics(&self, slot: u32) -> Option<CharMetrics> {
        let gid = self.face.glyph_of_slot(slot)?;
        let g = self.face.glyphs.get(gid as usize)?;
        Some(CharMetrics {
            index: gid,
            width: round(g.width * self.scale),
            height: round(g.height * self.scale),
            depth: round(g.depth * self.scale),
            italic: round(g.italic * self.scale),
        })
    }

    /// The font's parameters (harf-define.lua:452-461) as `\fontdimen1..8`,
    /// in scaled points.
    pub fn parameters(&self) -> [i64; 8] {
        let f = &self.face;
        let s = self.scale;
        [
            round(f.slant),
            round(f.tex_space * s),
            round(f.tex_space * s / 2.0),
            round(f.tex_space * s / 3.0),
            round(f.xheight * s),
            self.size,
            round(f.tex_space * s / 3.0),
            round(f.capheight * s),
        ]
    }
}

/// One glyph HarfBuzz returned, in design units, in LOGICAL order
/// (harf-plug.lua:416 reverses a right-to-left buffer).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct HbGlyph {
    pub codepoint: u32,
    pub cluster: usize,
    pub x_advance: i32,
    pub y_advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
    pub unsafe_to_break: bool,
}

/// `hb.shape_full` on `codes[offset..offset+len]`, the rest of `codes` being
/// the context HarfBuzz sees either side (`add_codepoints(codes, offset,
/// len)`, harf-plug.lua:372, which is `hb_buffer_add_codepoints`: up to five
/// codes before, nearest first, and five after). `extra` are the per-glyph
/// features harf-plug.lua:378-411 adds for a stretch of the run.
pub fn shape_codes(
    font: &HarfFont,
    codes: &[u32],
    offset: usize,
    len: usize,
    rtl: bool,
    extra: &[Feature],
) -> Result<Vec<HbGlyph>, ShapeError> {
    let shaper = ShaperFont::new(&font.face.font);
    let mut buf = Buffer::new();
    buf.set_direction(match rtl {
        true => Direction::RightToLeft,
        false => Direction::LeftToRight,
    });
    buf.set_script(font.script_lang.script);
    buf.set_language(font.script_lang.language.clone());
    buf.set_cluster_level(ClusterLevel::MonotoneCharacters);
    let pre: Vec<u32> = codes[..offset].iter().rev().take(5).copied().collect();
    buf.set_pre_context_codepoints(&pre);
    for (i, &c) in codes.iter().enumerate().skip(offset).take(len) {
        buf.push(c, i as u32);
    }
    let end = (offset + len).min(codes.len());
    let post: Vec<u32> = codes[end..].iter().take(5).copied().collect();
    buf.set_post_context_codepoints(&post);

    let mut features = font.features.clone();
    features.extend_from_slice(extra);
    harfrust::shape(&shaper, &mut buf, ShapeOptions::new().features(&features)).map_err(|_| {
        ShapeError(format!(
            "luaotfload | harf : All shapers failed for font \"{}\".",
            font.face.fullname
        ))
    })?;
    if rtl {
        buf.reverse();
    }
    Ok(buf
        .glyph_infos()
        .iter()
        .zip(buf.glyph_positions())
        .map(|(info, pos)| HbGlyph {
            codepoint: info.glyph_id,
            cluster: info.cluster as usize,
            x_advance: pos.x_advance,
            y_advance: pos.y_advance,
            x_offset: pos.x_offset,
            y_offset: pos.y_offset,
            unsafe_to_break: info.unsafe_to_break(),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// The node list luaotfload edits.
// ---------------------------------------------------------------------------

/// luatex's glue subtype for an interword space, `spaceskip` (13): the one
/// glue harf-plug.lua:213 shapes as U+0020.
pub const SPACESKIP: u16 = 13;
/// A kern node's `fontkern` subtype (0), which every kern the shaper inserts
/// carries (harf-plug.lua:150-154).
pub const FONTKERN: u16 = 0;
/// `italiccorrection` (3): a `\/` whose amount the shaper recomputes.
pub const ITALICCORR: u16 = 3;

/// Properties luaotfload hangs on a node (its `properties` table).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Props {
    /// `glyph_info` (harf-plug.lua:956): the text the cluster this glyph
    /// starts stands for, which is what the engine prints for it.
    pub glyph_info: Option<String>,
    /// `luaotfload_startactualtext`: the UTF-16 hex an `/ActualText` span
    /// opened before this node carries.
    pub start_actual: Option<String>,
    /// `luaotfload_endactualtext`: the span closes after this node.
    pub end_actual: bool,
    /// `zwnj`: shape this node as U+200C.
    pub zwnj: bool,
    /// `glyph_features`: features this glyph alone is shaped with.
    pub glyph_features: Option<BTreeMap<String, FeatureValue>>,
}

/// One node of a horizontal list, as much of it as the shaper reads.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// A glyph node. `font` is the engine's font id; `char` the slot, which
    /// may be past Unicode (`GID_OFFSET + gid`). `processed` is luatex's
    /// "protected" bit: a glyph a font handler has already done, which
    /// `is_char` no longer answers for.
    Glyph {
        char: u32,
        font: usize,
        xoffset: i64,
        yoffset: i64,
        processed: bool,
        props: Props,
    },
    Glue {
        subtype: u16,
        width: i64,
        stretch: i64,
        shrink: i64,
        props: Props,
    },
    Kern {
        subtype: u16,
        kern: i64,
        props: Props,
    },
    /// A discretionary with luatex's three lists. `subtype` is luatex's:
    /// 1 explicit, 2 automatic, 3 regular, 4 first, 5 second.
    Disc {
        subtype: u16,
        pre: Vec<Item>,
        post: Vec<Item>,
        replace: Vec<Item>,
    },
    /// A direction node: `dir` 0 for left to right, 1 for right to left;
    /// `cancel` for the node that ends one.
    Dir { dir: u8, cancel: bool },
    /// The `local_par` node a paragraph starts with, carrying its direction.
    LocalPar { dir: u8 },
    /// A page-mode `pdf_literal` whatsit, which is what an `/ActualText` span
    /// is made of (harf-plug.lua:1059-1064).
    Literal(String),
    /// Anything else -- a penalty, a rule, a box, a math node, a whatsit --
    /// carried through untouched. `id` is whatever the caller needs to put it
    /// back.
    Other { id: u32 },
}

type Ix = usize;

#[derive(Clone, Debug)]
enum Kind {
    Glyph {
        char: u32,
        font: usize,
        xoffset: i64,
        yoffset: i64,
        processed: bool,
    },
    Glue {
        subtype: u16,
        width: i64,
        stretch: i64,
        shrink: i64,
    },
    Kern {
        subtype: u16,
        kern: i64,
    },
    Disc {
        subtype: u16,
        pre: Option<Ix>,
        post: Option<Ix>,
        replace: Option<Ix>,
    },
    Dir {
        dir: u8,
        cancel: bool,
    },
    LocalPar {
        dir: u8,
    },
    Literal(String),
    Other {
        id: u32,
    },
}

#[derive(Clone, Debug)]
struct Node {
    kind: Kind,
    props: Props,
    prev: Option<Ix>,
    next: Option<Ix>,
}

/// luatex's node memory: nodes by index, each with its own `prev` and
/// `next`. A freed node is simply never reached again.
#[derive(Default)]
struct List {
    nodes: Vec<Node>,
}

impl List {
    fn alloc(&mut self, kind: Kind, props: Props) -> Ix {
        self.nodes.push(Node {
            kind,
            props,
            prev: None,
            next: None,
        });
        self.nodes.len() - 1
    }

    /// Build a list from items; returns its head.
    fn build(&mut self, items: &[Item]) -> Option<Ix> {
        let mut head = None;
        let mut last: Option<Ix> = None;
        for item in items {
            let (kind, props) = match item {
                Item::Glyph {
                    char,
                    font,
                    xoffset,
                    yoffset,
                    processed,
                    props,
                } => (
                    Kind::Glyph {
                        char: *char,
                        font: *font,
                        xoffset: *xoffset,
                        yoffset: *yoffset,
                        processed: *processed,
                    },
                    props.clone(),
                ),
                Item::Glue {
                    subtype,
                    width,
                    stretch,
                    shrink,
                    props,
                } => (
                    Kind::Glue {
                        subtype: *subtype,
                        width: *width,
                        stretch: *stretch,
                        shrink: *shrink,
                    },
                    props.clone(),
                ),
                Item::Kern {
                    subtype,
                    kern,
                    props,
                } => (
                    Kind::Kern {
                        subtype: *subtype,
                        kern: *kern,
                    },
                    props.clone(),
                ),
                Item::Disc {
                    subtype,
                    pre,
                    post,
                    replace,
                } => {
                    let pre = self.build(pre);
                    let post = self.build(post);
                    let replace = self.build(replace);
                    (
                        Kind::Disc {
                            subtype: *subtype,
                            pre,
                            post,
                            replace,
                        },
                        Props::default(),
                    )
                }
                Item::Dir { dir, cancel } => (
                    Kind::Dir {
                        dir: *dir,
                        cancel: *cancel,
                    },
                    Props::default(),
                ),
                Item::LocalPar { dir } => (Kind::LocalPar { dir: *dir }, Props::default()),
                Item::Literal(s) => (Kind::Literal(s.clone()), Props::default()),
                Item::Other { id } => (Kind::Other { id: *id }, Props::default()),
            };
            let n = self.alloc(kind, props);
            match last {
                Some(l) => self.link(Some(l), Some(n)),
                None => head = Some(n),
            }
            last = Some(n);
        }
        head
    }

    /// Read a list back out, from `head` to its end.
    fn items(&self, head: Option<Ix>) -> Vec<Item> {
        let mut out = Vec::new();
        let mut n = head;
        while let Some(i) = n {
            let node = &self.nodes[i];
            let props = node.props.clone();
            out.push(match &node.kind {
                Kind::Glyph {
                    char,
                    font,
                    xoffset,
                    yoffset,
                    processed,
                } => Item::Glyph {
                    char: *char,
                    font: *font,
                    xoffset: *xoffset,
                    yoffset: *yoffset,
                    processed: *processed,
                    props,
                },
                Kind::Glue {
                    subtype,
                    width,
                    stretch,
                    shrink,
                } => Item::Glue {
                    subtype: *subtype,
                    width: *width,
                    stretch: *stretch,
                    shrink: *shrink,
                    props,
                },
                Kind::Kern { subtype, kern } => Item::Kern {
                    subtype: *subtype,
                    kern: *kern,
                    props,
                },
                Kind::Disc {
                    subtype,
                    pre,
                    post,
                    replace,
                } => Item::Disc {
                    subtype: *subtype,
                    pre: self.items(*pre),
                    post: self.items(*post),
                    replace: self.items(*replace),
                },
                Kind::Dir { dir, cancel } => Item::Dir {
                    dir: *dir,
                    cancel: *cancel,
                },
                Kind::LocalPar { dir } => Item::LocalPar { dir: *dir },
                Kind::Literal(s) => Item::Literal(s.clone()),
                Kind::Other { id } => Item::Other { id: *id },
            });
            n = node.next;
        }
        out
    }

    fn next(&self, n: Option<Ix>) -> Option<Ix> {
        n.and_then(|i| self.nodes[i].next)
    }

    fn prev(&self, n: Option<Ix>) -> Option<Ix> {
        n.and_then(|i| self.nodes[i].prev)
    }

    fn set_next(&mut self, n: Ix, to: Option<Ix>) {
        self.nodes[n].next = to;
    }

    fn set_prev(&mut self, n: Ix, to: Option<Ix>) {
        self.nodes[n].prev = to;
    }

    /// `setlink(a, b)`: either may be nil.
    fn link(&mut self, a: Option<Ix>, b: Option<Ix>) {
        if let Some(a) = a {
            self.nodes[a].next = b;
        }
        if let Some(b) = b {
            self.nodes[b].prev = a;
        }
    }

    /// `setlink(a, b, c)`.
    fn link3(&mut self, a: Option<Ix>, b: Option<Ix>, c: Option<Ix>) {
        self.link(a, b);
        self.link(b, c);
    }

    /// `tail(head)`.
    fn tail(&self, head: Option<Ix>) -> Option<Ix> {
        let mut n = head?;
        while let Some(next) = self.nodes[n].next {
            n = next;
        }
        Some(n)
    }

    /// `length(head)`.
    fn length(&self, head: Option<Ix>) -> usize {
        let mut k = 0;
        let mut n = head;
        while let Some(i) = n {
            k += 1;
            n = self.nodes[i].next;
        }
        k
    }

    /// `copy(n)`: one node, unlinked, its lists copied with it.
    fn copy(&mut self, n: Ix) -> Ix {
        let node = self.nodes[n].clone();
        let kind = match node.kind {
            Kind::Disc {
                subtype,
                pre,
                post,
                replace,
            } => {
                let pre = self.copy_list(pre, None);
                let post = self.copy_list(post, None);
                let replace = self.copy_list(replace, None);
                Kind::Disc {
                    subtype,
                    pre,
                    post,
                    replace,
                }
            }
            k => k,
        };
        self.alloc(kind, node.props)
    }

    /// `copy_list(from, upto)`: the nodes from `from` up to but excluding
    /// `upto` (to the end when `upto` is nil).
    fn copy_list(&mut self, from: Option<Ix>, upto: Option<Ix>) -> Option<Ix> {
        let mut head = None;
        let mut last = None;
        let mut n = from;
        while let Some(i) = n {
            if Some(i) == upto {
                break;
            }
            let c = self.copy(i);
            match last {
                Some(l) => self.link(Some(l), Some(c)),
                None => head = Some(c),
            }
            last = Some(c);
            n = self.nodes[i].next;
        }
        head
    }

    /// `insert_before(head, current, new)`: returns the new head.
    fn insert_before(&mut self, head: Option<Ix>, current: Option<Ix>, new: Ix) -> Option<Ix> {
        let Some(cur) = current else {
            // luatex appends to the tail when there is no current node.
            let t = self.tail(head);
            self.link(t, Some(new));
            return head.or(Some(new));
        };
        let prev = self.nodes[cur].prev;
        self.link3(prev, Some(new), Some(cur));
        match head == Some(cur) {
            true => Some(new),
            false => head,
        }
    }

    /// `insert_after(head, current, new)`: returns the head and `new`.
    fn insert_after(
        &mut self,
        head: Option<Ix>,
        current: Option<Ix>,
        new: Ix,
    ) -> (Option<Ix>, Option<Ix>) {
        let Some(cur) = current else {
            let t = self.tail(head);
            self.link(t, Some(new));
            return (head.or(Some(new)), Some(new));
        };
        let next = self.nodes[cur].next;
        self.link3(Some(cur), Some(new), next);
        (head, Some(new))
    }

    /// `remove(head, current)`: unlinks `current`; returns the new head and
    /// the node after it.
    fn remove(&mut self, head: Option<Ix>, current: Option<Ix>) -> (Option<Ix>, Option<Ix>) {
        let Some(cur) = current else {
            return (head, None);
        };
        let (prev, next) = (self.nodes[cur].prev, self.nodes[cur].next);
        self.link(prev, next);
        if prev.is_none() {
            if let Some(nx) = next {
                self.nodes[nx].prev = None;
            }
        }
        self.nodes[cur].prev = None;
        self.nodes[cur].next = None;
        let head = match head == Some(cur) {
            true => next,
            false => head,
        };
        (head, next)
    }

    /// `is_char(n, font)`: the slot of a glyph in `font` that no handler has
    /// processed yet.
    fn is_char(&self, n: Option<Ix>, font: usize) -> Option<u32> {
        match &self.nodes[n?].kind {
            Kind::Glyph {
                char,
                font: f,
                processed: false,
                ..
            } if *f == font => Some(*char),
            _ => None,
        }
    }

    /// `is_char(n)` without a font: the slot and the font of an unprocessed
    /// glyph.
    fn is_char_any(&self, n: Option<Ix>) -> Option<(u32, usize)> {
        match &self.nodes[n?].kind {
            Kind::Glyph {
                char,
                font,
                processed: false,
                ..
            } => Some((*char, *font)),
            _ => None,
        }
    }

    /// `uses_font(n, font)` for a discretionary: whether any glyph of its
    /// three lists is in `font`.
    fn uses_font(&self, n: Ix, font: usize) -> bool {
        let Kind::Disc {
            pre, post, replace, ..
        } = &self.nodes[n].kind
        else {
            return false;
        };
        [*pre, *post, *replace].iter().any(|&l| {
            let mut m = l;
            while let Some(i) = m {
                if matches!(&self.nodes[i].kind, Kind::Glyph { font: f, .. } if *f == font) {
                    return true;
                }
                m = self.nodes[i].next;
            }
            false
        })
    }

    fn disc(&self, n: Ix) -> (Option<Ix>, Option<Ix>, Option<Ix>) {
        match &self.nodes[n].kind {
            Kind::Disc {
                pre, post, replace, ..
            } => (*pre, *post, *replace),
            _ => (None, None, None),
        }
    }

    fn set_disc(&mut self, n: Ix, pre: Option<Ix>, post: Option<Ix>, replace: Option<Ix>) {
        for l in [pre, post, replace].into_iter().flatten() {
            self.nodes[l].prev = None;
        }
        if let Kind::Disc {
            pre: p,
            post: q,
            replace: r,
            ..
        } = &mut self.nodes[n].kind
        {
            *p = pre;
            *q = post;
            *r = replace;
        }
    }

    fn set_replace(&mut self, n: Ix, replace: Option<Ix>) {
        if let Kind::Disc { replace: r, .. } = &mut self.nodes[n].kind {
            *r = replace;
        }
    }

    /// `newnode(glyph)` inheriting from `base` (`inherit`, harf-plug.lua:141):
    /// luaotfload copies the attributes and gives the node a property table
    /// that falls back to `base`'s, which for the fields this port reads is a
    /// copy of them.
    fn new_glyph_like(&mut self, base: Option<Ix>, props: Option<&Props>) -> Ix {
        let props = props.cloned().unwrap_or_default();
        let _ = base;
        self.alloc(
            Kind::Glyph {
                char: 0,
                font: 0,
                xoffset: 0,
                yoffset: 0,
                processed: false,
            },
            props,
        )
    }

    /// `newkern(v, n)` (harf-plug.lua:150).
    fn new_kern(&mut self, v: f64, base: Ix) -> Ix {
        let props = self.nodes[base].props.clone();
        self.alloc(
            Kind::Kern {
                subtype: FONTKERN,
                kern: round(v),
            },
            Props {
                start_actual: None,
                end_actual: false,
                ..props
            },
        )
    }
}

/// One run `itemize` cuts (harf-plug.lua:264-272).
#[derive(Clone, Debug)]
struct Run {
    /// 1-based index of the run's first code, as in the Lua.
    start: usize,
    len: usize,
    rtl: bool,
    skip: bool,
    /// The node after the run.
    after: Option<Ix>,
    /// The first of the run's discretionaries, chained by `next`.
    discs: Option<usize>,
    fordisc: bool,
}

/// A discretionary the run crosses (harf-plug.lua:221-225).
#[derive(Clone, Debug)]
struct DiscRec {
    disc: Ix,
    anchor_cluster: i64,
    after_cluster: i64,
    next: Option<usize>,
}

/// A glyph `shape` hands `tonodes`: HarfBuzz's, or the discretionary that
/// stands where a reshaped discretionary was (harf-plug.lua:535-550).
#[derive(Clone, Debug)]
struct Glyph {
    codepoint: u32,
    /// 0-based, into the run's codes.
    cluster: i64,
    nextcluster: Option<i64>,
    x_advance: i32,
    x_offset: i32,
    y_offset: i32,
    unsafe_to_break: bool,
    nglyphs: usize,
    tounicode: Option<String>,
    string: Option<String>,
    endactual: bool,
    replace: Option<Box<Sub>>,
    pre: Option<Box<Sub>>,
    post: Option<Box<Sub>>,
}

impl Glyph {
    fn from_hb(g: &HbGlyph) -> Glyph {
        Glyph {
            codepoint: g.codepoint,
            cluster: g.cluster as i64,
            nextcluster: None,
            x_advance: g.x_advance,
            x_offset: g.x_offset,
            y_offset: g.y_offset,
            unsafe_to_break: g.unsafe_to_break,
            nglyphs: 0,
            tounicode: None,
            string: None,
            endactual: false,
            replace: None,
            pre: None,
            post: None,
        }
    }
}

/// `makesub`'s result and a replacement branch (harf-plug.lua:319, :536).
#[derive(Clone, Debug)]
struct Sub {
    glyphs: Vec<Glyph>,
    run: Run,
    head: Option<Ix>,
}

/// harf-plug.lua:166-175.
fn to_utf16_hex(uni: u32) -> String {
    if uni < 0x10000 {
        format!("{uni:04X}")
    } else {
        let u = uni - 0x10000;
        format!("{:04X}{:04X}", 0xD800 + u / 0x400, 0xDC00 + u % 0x400)
    }
}

/// The fonts a list may use, by engine font id: the shaper asks which ids are
/// harf fonts and for their data.
pub trait Fonts {
    fn harf(&self, font: usize) -> Option<&HarfFont>;
}

impl Fonts for BTreeMap<usize, HarfFont> {
    fn harf(&self, font: usize) -> Option<&HarfFont> {
        self.get(&font)
    }
}

struct Shaper<'a> {
    list: List,
    font: &'a HarfFont,
    fontid: usize,
    discs: Vec<DiscRec>,
    /// `codes`, shared by every run of one `process` call (harf-plug.lua:186).
    codes: Vec<u32>,
}

impl<'a> Shaper<'a> {
    /// `itemize` (harf-plug.lua:179-291). Returns the new head and the runs.
    fn itemize(&mut self, head: Option<Ix>, direction: u8) -> (Option<Ix>, Vec<Run>) {
        let obj_repl = self.font.obj_repl;
        let fontid = self.fontid;
        let mut runs: Vec<Run> = Vec::new();
        let mut dirstack: Vec<u8> = Vec::new();
        let mut currdir = direction;
        let mut lastskip = true;
        let mut lastdir: Option<u8> = None;
        // The dummy run is index usize::MAX; its `after` is the new head.
        let mut dummy_after: Option<Ix> = None;
        let mut lastrun: Option<usize> = None;
        let mut lastdisc: Option<usize> = None;
        let mut in_disc: Option<Ix> = None;

        // `traverse(head)`: the iterator steps to the `next` of the node it
        // last returned, whatever the body did to the loop variable.
        let mut cursor = head;
        while let Some(orig) = cursor {
            let mut n = Some(orig);
            if in_disc == n {
                in_disc = None;
            }
            let mut disc: Option<usize> = None;
            let mut code = Some(obj_repl);
            let mut skip = lastskip;
            let props_zwnj = self.list.nodes[orig].props.zwnj;
            let kind = self.list.nodes[orig].kind.clone();
            let lastrun_start = lastrun.map(|r| runs[r].start).unwrap_or(0) as i64;

            if props_zwnj {
                code = Some(0x200C);
            } else {
                match kind {
                    Kind::Glyph { .. } => match self.list.is_char(n, fontid) {
                        Some(c) => {
                            code = Some(c);
                            skip = false;
                        }
                        None => skip = true,
                    },
                    Kind::Glue { subtype, .. } if subtype == SPACESKIP => code = Some(0x20),
                    Kind::Disc { .. } => {
                        let (pre, post, rep) = self.list.disc(orig);
                        let empty = pre.is_none() && post.is_none() && rep.is_none();
                        if self.list.uses_font(orig, fontid) || !lastskip && empty {
                            let rep_tail = self.list.tail(rep);
                            self.list.set_replace(orig, None);
                            let (prev, next) =
                                (self.list.nodes[orig].prev, self.list.nodes[orig].next);
                            in_disc = next;
                            let anchor = self.codes.len() as i64 - lastrun_start + 1;
                            self.discs.push(DiscRec {
                                disc: orig,
                                anchor_cluster: anchor,
                                after_cluster: anchor + self.list.length(rep) as i64,
                                next: None,
                            });
                            disc = Some(self.discs.len() - 1);
                            match rep {
                                Some(r) => {
                                    self.list.link(rep_tail, next);
                                    // "just to keep the loop going"
                                    self.list.set_next(orig, Some(r));
                                    self.list.set_prev(r, None);
                                    n = Some(r);
                                }
                                None => n = next,
                            }
                            self.list.link(prev, n);
                            if prev.is_none() {
                                if let Some(x) = n {
                                    self.list.set_prev(x, None);
                                }
                            }
                            code = None;
                            skip = false;
                        } else {
                            skip = true;
                        }
                    }
                    Kind::Dir { dir, cancel } => {
                        if cancel {
                            currdir = dirstack.pop().unwrap_or(0);
                        } else {
                            dirstack.push(currdir);
                            currdir = dir;
                        }
                    }
                    Kind::LocalPar { dir } => currdir = dir,
                    _ => {}
                }
            }

            let ncodes = self.codes.len();
            if let Some(c) = code {
                self.codes.push(c);
            }

            if (disc.is_some() || in_disc.is_none())
                && (lastdir != Some(currdir) || lastskip != skip)
            {
                if let Some(d) = disc {
                    let rec = &mut self.discs[d];
                    rec.after_cluster -= rec.anchor_cluster;
                    rec.anchor_cluster = 0;
                }
                match lastrun {
                    Some(r) => runs[r].after = n,
                    None => dummy_after = n,
                }
                runs.push(Run {
                    start: ncodes + 1,
                    len: code.map_or(0, |_| 1),
                    rtl: currdir == 1,
                    skip,
                    after: None,
                    discs: disc,
                    fordisc: false,
                });
                lastrun = Some(runs.len() - 1);
                lastdir = Some(currdir);
                lastskip = skip;
                lastdisc = disc;
            } else if code.is_some() {
                if let Some(r) = lastrun {
                    runs[r].len += 1;
                }
            } else if let Some(d) = disc {
                match lastdisc {
                    Some(ld) => {
                        if let Some(r) = lastrun {
                            if runs[r].len == 0 && r >= 1 {
                                runs[r - 1].after = n;
                            }
                        }
                        self.discs[ld].next = Some(d);
                        lastdisc = Some(d);
                    }
                    None => {
                        if let Some(r) = lastrun {
                            runs[r].discs = Some(d);
                        }
                        lastdisc = Some(d);
                    }
                }
            }
            cursor = self.list.nodes[orig].next;
        }
        (dummy_after, runs)
    }

    /// `makesub` (harf-plug.lua:304-320).
    fn makesub(&mut self, run: &Run, codes: Vec<u32>, nodelist: Option<Ix>) -> Sub {
        let subrun = Run {
            start: 1,
            len: codes.len(),
            rtl: run.rtl,
            skip: false,
            after: None,
            discs: None,
            fordisc: true,
        };
        let saved = std::mem::replace(&mut self.codes, codes);
        let (head, _, glyphs, _) = self.shape(nodelist, nodelist, &subrun);
        self.codes = saved;
        Sub {
            glyphs: glyphs.expect("Shaping discretionary list failed. This shouldn't happen."),
            run: subrun,
            head,
        }
    }

    /// The per-glyph features of the run (harf-plug.lua:378-411).
    fn local_features(&self, first: Option<Ix>, offset: usize, len: usize) -> Vec<Feature> {
        let mut out = Vec::new();
        let mut current: BTreeMap<String, (u32, usize)> = BTreeMap::new();
        let mut n = first;
        for i in offset..offset + len {
            let props = n.map(|x| &self.list.nodes[x].props);
            let empty = BTreeMap::new();
            let local = props
                .and_then(|p| p.glyph_features.as_ref())
                .unwrap_or(&empty);
            let value_of = |v: &FeatureValue| -> u32 {
                match v {
                    FeatureValue::Number(x) => *x as u32,
                    FeatureValue::Text(t) => t.parse::<f64>().map(|x| x as u32).unwrap_or(1),
                    FeatureValue::Bool(b) => *b as u32,
                }
            };
            let tags: Vec<String> = current.keys().cloned().collect();
            for tag in tags {
                let (value, start) = current[&tag];
                let loc = local.get(&tag).map(value_of);
                if Some(value) != loc {
                    if let Ok(t) = tag.parse::<harfrust::Tag>() {
                        out.push(Feature::new(t, value, start..i));
                    }
                    current.remove(&tag);
                }
            }
            for (tag, v) in local {
                current.entry(tag.clone()).or_insert((value_of(v), i));
            }
            n = self.list.next(n);
        }
        for (tag, (value, start)) in current {
            if let Ok(t) = tag.parse::<harfrust::Tag>() {
                out.push(Feature::new(t, value, start..));
            }
        }
        out
    }

    /// `shape` (harf-plug.lua:341-692). Returns the head, the first node,
    /// the glyphs and how many codes the run lost to discretionaries.
    fn shape(
        &mut self,
        mut head: Option<Ix>,
        mut firstnode: Option<Ix>,
        run: &Run,
    ) -> (Option<Ix>, Option<Ix>, Option<Vec<Glyph>>, i64) {
        let fontid = self.fontid;
        let mut node = firstnode;
        let offset = run.start as i64 - 1;
        let mut len = run.len as i64;
        let obj_repl = self.font.obj_repl;
        let mut cluster: Option<i64> = None;
        let mut discs = match run.fordisc {
            true => None,
            false => run.discs,
        };

        let extra = self.local_features(node, offset as usize, len as usize);
        let hb = match shape_codes(
            self.font,
            &self.codes,
            offset as usize,
            len as usize,
            run.rtl,
            &extra,
        ) {
            Ok(g) => g,
            Err(_) => return (head, firstnode, None, 0),
        };
        let mut glyphs: Vec<Glyph> = hb.iter().map(Glyph::from_hb).collect();
        // Lua indexes from 1; `codes[k]` there is `code(k)` here.
        let code = |codes: &Vec<u32>, k: i64| -> Option<u32> {
            (k >= 1)
                .then(|| codes.get(k as usize - 1).copied())
                .flatten()
        };

        let mut break_glyph: usize = 1;
        let mut break_cluster = offset;
        let mut break_node = node;
        let mut disc_glyph: usize = 0;
        let mut disc_cluster: Option<i64> = None;
        let mut disc_node: Option<Ix> = None;
        let mut i: usize = 0;
        loop {
            // `repeat i = i+1 ... until not glyph or glyph.cluster ~= cluster`
            loop {
                i += 1;
                match glyphs.get(i - 1) {
                    None => break,
                    Some(g) if Some(g.cluster) != cluster => break,
                    _ => {}
                }
            }
            let has_glyph = i <= glyphs.len();
            {
                let oldcluster = cluster;
                let c = match has_glyph {
                    true => glyphs[i - 1].cluster,
                    false => offset + len,
                };
                cluster = Some(c);
                if let Some(old) = oldcluster {
                    for _ in old + 1..=c {
                        node = self.list.next(node);
                    }
                }
            }
            // The Lua reads `cluster` afresh at every use, and a reshaped
            // discretionary moves it (`cluster = disc_cluster + 1`), so this
            // copy is kept in step with it.
            let mut cl = cluster.unwrap();

            let safe = !has_glyph
                || code(&self.codes, cl) == Some(0x20)
                || code(&self.codes, cl + 1) == Some(0x20)
                || code(&self.codes, cl) == Some(obj_repl)
                || code(&self.codes, cl + 1) == Some(obj_repl)
                || !glyphs[i - 1].unsafe_to_break;
            if let (Some(d), true) = (discs, safe) {
                let mut d = d;
                let mut anchor_cluster = offset + self.discs[d].anchor_cluster;
                let mut after_cluster = offset + self.discs[d].after_cluster;
                let mut stop = false;
                while disc_cluster.is_some() && after_cluster <= cl
                    || disc_cluster.is_none() && anchor_cluster <= cl
                {
                    if let Some(dc) = disc_cluster {
                        // harf-plug.lua:463-478: the glyphs that stand for the
                        // discretionary's replacement come out of the run.
                        let mut rep_glyphs: Vec<Glyph> = glyphs[disc_glyph - 1..i - 1].to_vec();
                        for g in &mut rep_glyphs {
                            g.cluster -= dc;
                            if let Some(nc) = g.nextcluster.as_mut() {
                                *nc -= dc;
                            }
                        }
                        let cluster_offset = dc - cl + 1;
                        for g in glyphs.iter_mut().skip(i - 1) {
                            g.cluster += cluster_offset;
                        }
                        len += cluster_offset;
                        // table.move(glyphs, i, #glyphs + i - disc_glyph,
                        // disc_glyph + 1): glyphs i.. move to just after the
                        // slot the discretionary will take, and the nils read
                        // from past the end clear what is left behind. When the
                        // discretionary IS glyph i the move is one place up.
                        let len_now = glyphs.len();
                        let last = len_now + i - disc_glyph;
                        let moved: Vec<Glyph> = (i..=last)
                            .filter_map(|k| glyphs.get(k - 1).cloned())
                            .collect();
                        glyphs.truncate(disc_glyph);
                        // The slot itself is assigned below.
                        while glyphs.len() < disc_glyph {
                            glyphs.push(Glyph::from_hb(&HbGlyph::default()));
                        }
                        glyphs.extend(moved);
                        let mut later = self.discs[d].next;
                        while let Some(l) = later {
                            self.discs[l].anchor_cluster += cluster_offset;
                            self.discs[l].after_cluster += cluster_offset;
                            later = self.discs[l].next;
                        }

                        let disc_n = self.discs[d].disc;
                        let (pre0, post0, _) = self.list.disc(disc_n);
                        let mut pre = pre0;
                        let mut post = post0;
                        let lastpost = self.list.tail(post0);
                        // precodes: codes[disc_cluster+1 .. anchor_cluster],
                        // then the pre list's own characters.
                        let mut precodes: Vec<u32> = Vec::new();
                        for k in dc + 1..=anchor_cluster {
                            if let Some(c) = code(&self.codes, k) {
                                precodes.push(c);
                            }
                        }
                        let mut m = pre;
                        while let Some(x) = m {
                            precodes.push(self.list.is_char(Some(x), fontid).unwrap_or(obj_repl));
                            m = self.list.nodes[x].next;
                        }
                        let mut postcodes: Vec<u32> = Vec::new();
                        let mut m = post;
                        while let Some(x) = m {
                            postcodes.push(self.list.is_char(Some(x), fontid).unwrap_or(obj_repl));
                            m = self.list.nodes[x].next;
                        }
                        for k in after_cluster + 1..=cl {
                            if let Some(c) = code(&self.codes, k) {
                                postcodes.push(c);
                            }
                        }
                        // harf-plug.lua:504-505: the discretionary becomes one
                        // object replacement code in place of what it spans.
                        {
                            let from = (dc as usize).min(self.codes.len());
                            let to = (cl as usize).min(self.codes.len());
                            self.codes.splice(from..to, [obj_repl]);
                        }

                        {
                            let mut iter = disc_node;
                            for _ in dc..anchor_cluster {
                                iter = self.list.next(iter);
                            }
                            if iter != disc_node {
                                let newpre = self.list.copy_list(disc_node, iter);
                                let t = self.list.tail(newpre);
                                self.list.link(t, pre);
                                pre = newpre;
                            }
                            for _ in anchor_cluster..after_cluster {
                                iter = self.list.next(iter);
                            }
                            match post {
                                Some(_) => {
                                    let copied = self.list.copy_list(iter, node);
                                    self.list.link(lastpost, copied);
                                }
                                None => post = self.list.copy_list(iter, node),
                            }
                        }
                        let prev = self.list.prev(disc_node);
                        if dc != cl {
                            if let Some(dn) = disc_node {
                                self.list.set_prev(dn, None);
                            }
                            if let Some(p) = self.list.prev(node) {
                                self.list.set_next(p, None);
                            }
                        }
                        self.list.link3(prev, Some(disc_n), node);
                        if prev.is_none() {
                            self.list.set_prev(disc_n, None);
                        }
                        if disc_node == firstnode {
                            firstnode = Some(disc_n);
                            if head == disc_node {
                                head = firstnode;
                            }
                        }
                        // The replacement keeps its own glyphs; pre and post
                        // are shaped afresh, each on its own.
                        self.list.set_disc(disc_n, pre, post, None);
                        let rep_head = match disc_node != node {
                            true => disc_node,
                            false => None,
                        };
                        let pre_sub = self.makesub(run, precodes, pre);
                        let post_sub = self.makesub(run, postcodes, post);
                        glyphs[disc_glyph - 1] = Glyph {
                            codepoint: obj_repl,
                            cluster: dc,
                            nextcluster: Some(dc + 1),
                            x_advance: 0,
                            x_offset: 0,
                            y_offset: 0,
                            unsafe_to_break: false,
                            nglyphs: 0,
                            tounicode: None,
                            string: None,
                            endactual: false,
                            replace: Some(Box::new(Sub {
                                glyphs: rep_glyphs,
                                head: rep_head,
                                run: Run {
                                    start: 1,
                                    len: 0,
                                    rtl: run.rtl,
                                    skip: false,
                                    after: None,
                                    discs: None,
                                    fordisc: true,
                                },
                            })),
                            pre: Some(Box::new(pre_sub)),
                            post: Some(Box::new(post_sub)),
                        };
                        i = disc_glyph + 1;
                        cluster = Some(dc + 1);
                        cl = dc + 1;

                        disc_cluster = None;
                        let mut nd = self.discs[d].next;
                        while let Some(x) = nd {
                            if self.discs[x].anchor_cluster + offset < dc + 1 {
                                nd = self.discs[x].next;
                            } else {
                                break;
                            }
                        }
                        discs = nd;
                        match nd {
                            None => {
                                stop = true;
                                break;
                            }
                            Some(x) => {
                                d = x;
                                anchor_cluster = offset + self.discs[d].anchor_cluster;
                                after_cluster = offset + self.discs[d].after_cluster;
                            }
                        }
                    } else if anchor_cluster == cl {
                        disc_glyph = i;
                        disc_cluster = Some(cl);
                        disc_node = node;
                    } else {
                        disc_glyph = break_glyph;
                        disc_cluster = Some(break_cluster);
                        disc_node = break_node;
                    }
                }
                let _ = stop;
                let cl_now = cluster.unwrap();
                break_glyph = i;
                break_cluster = cl_now;
                break_node = node;
            }

            if i > glyphs.len() {
                break;
            }
            let cl = cluster.unwrap();

            // harf-plug.lua:640-652.
            let mut nextcluster = None;
            for j in i + 1..=glyphs.len() {
                let c = glyphs[j - 1].cluster;
                if c != cl {
                    glyphs[i - 1].nglyphs = j - i;
                    nextcluster = Some(c);
                    break;
                }
            }
            let nextcluster = match nextcluster {
                Some(c) => c,
                None => {
                    glyphs[i - 1].nglyphs = glyphs.len() + 1 - i;
                    offset + len
                }
            };
            glyphs[i - 1].nextcluster = Some(nextcluster);

            // harf-plug.lua:657-672: the characters of the cluster.
            let mut hex = String::new();
            let mut s = String::new();
            let mut m = node;
            for _ in cl + 1..=nextcluster {
                if let Some(c) = self.list.is_char(m, fontid) {
                    hex.push_str(&to_utf16_hex(c));
                    if let Some(ch) = char::from_u32(c) {
                        s.push(ch);
                    }
                }
                m = self.list.next(m);
            }
            glyphs[i - 1].tounicode = Some(hex);
            glyphs[i - 1].string = Some(s);
        }
        (head, firstnode, Some(glyphs), run.len as i64 - len)
    }

    /// `tonodes` (harf-plug.lua:751-1028), colour layers and bitmap glyphs
    /// aside (see the note on [`process`]).
    fn tonodes(
        &mut self,
        mut head: Option<Ix>,
        mut node: Option<Ix>,
        run: &Run,
        glyphs: &mut [Glyph],
        unicode: &mut ToUnicode,
    ) -> (Option<Ix>, Option<Ix>) {
        let font = self.font;
        let fontid = self.fontid;
        let face = &font.face;
        let rtl = run.rtl;
        let scale = font.scale;
        let mut nodeindex = run.start as i64;
        let mut lastprops: Option<Props> = None;
        let mut nextcluster: i64 = 0;

        let n_glyphs = glyphs.len();
        for gi in 0..n_glyphs {
            let glyph_cluster = glyphs[gi].cluster;
            if glyph_cluster + 1 >= nodeindex {
                nextcluster = glyphs[gi].nextcluster.expect("nextcluster");
                for _ in nodeindex..=glyph_cluster {
                    let (h, nx) = self.list.remove(head, node);
                    head = h;
                    node = nx;
                }
                lastprops = node.map(|x| self.list.nodes[x].props.clone());
                nodeindex = glyph_cluster + 1;
            } else if nextcluster + 1 == nodeindex {
                nodeindex -= 1;
                let new;
                match node {
                    Some(_) => {
                        let base = self.list.prev(node);
                        new = self.list.new_glyph_like(base, lastprops.as_ref());
                        head = self.list.insert_before(head, node, new);
                        node = Some(new);
                    }
                    None => {
                        node = self.list.tail(head);
                        new = self.list.new_glyph_like(node, lastprops.as_ref());
                        let (h, nx) = self.list.insert_after(head, node, new);
                        head = h;
                        node = nx;
                    }
                }
                if let Kind::Glyph { font: f, .. } = &mut self.list.nodes[new].kind {
                    *f = fontid;
                }
            }
            let gid = glyphs[gi].codepoint;
            let mut char = face.slot(gid);
            let orig_char = self.list.is_char(node, fontid);

            if glyphs[gi].replace.is_some() {
                let (rep, pre, post) = (
                    glyphs[gi].replace.take().unwrap(),
                    glyphs[gi].pre.take().unwrap(),
                    glyphs[gi].post.take().unwrap(),
                );
                let Sub {
                    glyphs: mut pg,
                    run: prun,
                    head: phead,
                } = *pre;
                let Sub {
                    glyphs: mut qg,
                    run: qrun,
                    head: qhead,
                } = *post;
                let Sub {
                    glyphs: mut rg,
                    run: rrun,
                    head: rhead,
                } = *rep;
                let p = self.tonodes(phead, phead, &prun, &mut pg, unicode).0;
                let q = self.tonodes(qhead, qhead, &qrun, &mut qg, unicode).0;
                let r = self.tonodes(rhead, rhead, &rrun, &mut rg, unicode).0;
                if let Some(nd) = node {
                    self.list.set_disc(nd, p, q, r);
                }
                node = self.list.next(node);
                nodeindex += 1;
                continue;
            }

            let zwnj_skip =
                lastprops.as_ref().is_some_and(|p| p.zwnj) && nodeindex == glyph_cluster + 1;
            if zwnj_skip {
                // harf-plug.lua:815: a ZWNJ's own node is left alone.
            } else if let Some(orig) = orig_char {
                let nd = node.expect("a character has a node");
                let _ = &mut char;
                let character = font.char_metrics(char);
                let oldcharacter = font.char_metrics(orig);
                match oldcharacter {
                    None => {
                        if gid == 0 {
                            let new = self.list.copy(nd);
                            let (h, nx) = self.list.insert_after(head, node, new);
                            head = h;
                            node = nx;
                        }
                        self.set_char(node.unwrap(), char);
                    }
                    Some(old) => {
                        if character.map(|c| c.index) != Some(old.index) {
                            self.set_char(nd, char);
                        }
                    }
                }
                let nd = node.unwrap();
                let g = &glyphs[gi];
                let xoffset = (if rtl { -g.x_offset } else { g.x_offset }) as f64 * scale;
                let yoffset = g.y_offset as f64 * scale;
                if let Kind::Glyph {
                    xoffset: xo,
                    yoffset: yo,
                    ..
                } = &mut self.list.nodes[nd].kind
                {
                    *xo = round(xoffset);
                    *yo = round(yoffset);
                }
                self.list.nodes[nd].props.glyph_info = Some(g.string.clone().unwrap_or_default());

                // harf-plug.lua:967-980: one glyph for the whole cluster is
                // the font's /ToUnicode, the first time it is seen; any other
                // arrangement is an /ActualText span.
                if let Some(tu) = g.tounicode.clone() {
                    let first = unicode.first(face, gid);
                    if g.nglyphs == 1 && first.is_none() {
                        unicode.set(face, gid, tu);
                    } else if Some(&tu) != first.as_ref() {
                        self.list.nodes[nd].props.start_actual = Some(tu);
                        let last = gi + g.nglyphs.max(1) - 1;
                        if let Some(l) = glyphs.get_mut(last) {
                            l.endactual = true;
                        }
                    }
                }
                if glyphs[gi].endactual {
                    self.list.nodes[nd].props.end_actual = true;
                }
                if face.outlines.is_some() {
                    unicode.used(face, gid);
                }

                let x_advance = glyphs[gi].x_advance as f64;
                let width = face.glyphs.get(gid as usize).map_or(0.0, |g| g.width) * font.hfactor;
                if width != x_advance {
                    let kern = self.list.new_kern((x_advance - width) * scale, nd);
                    if rtl {
                        head = self.list.insert_before(head, Some(nd), kern);
                    } else {
                        let (h, nx) = self.list.insert_after(head, Some(nd), kern);
                        head = h;
                        node = nx;
                    }
                }
            } else if let Some(nd) = node {
                match self.list.nodes[nd].kind.clone() {
                    Kind::Glue { subtype, width, .. } if subtype == SPACESKIP => {
                        let w = glyphs[gi].x_advance as f64 * scale;
                        if font.space != w {
                            if let Kind::Glue { width: gw, .. } = &mut self.list.nodes[nd].kind {
                                *gw = round(width as f64 - font.space + w);
                            }
                        }
                    }
                    Kind::Kern { subtype, .. } if subtype == ITALICCORR => {
                        let prev = self.list.prev(Some(nd));
                        if let Some((pc, pf)) = self.list.is_char_any(prev) {
                            if pf == fontid && pc > 0 {
                                if let Some(m) = font.char_metrics(pc) {
                                    if let Kind::Kern { kern, .. } = &mut self.list.nodes[nd].kind {
                                        *kern = m.italic;
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            node = self.list.next(node);
            nodeindex += 1;
        }
        while node.is_some() && node != run.after {
            let (h, nx) = self.list.remove(head, node);
            head = h;
            node = nx;
        }
        (head, node)
    }

    fn set_char(&mut self, n: Ix, c: u32) {
        if let Kind::Glyph { char, .. } = &mut self.list.nodes[n].kind {
            *char = c;
        }
    }

    /// luatex marks a glyph the font handler has finished with, so the next
    /// handler's `is_char` passes over it.
    fn protect(&mut self, head: Option<Ix>) {
        let mut n = head;
        while let Some(i) = n {
            let fontid = self.fontid;
            match &mut self.list.nodes[i].kind {
                Kind::Glyph {
                    font, processed, ..
                } if *font == fontid => *processed = true,
                Kind::Disc {
                    pre, post, replace, ..
                } => {
                    let lists = [*pre, *post, *replace];
                    for l in lists {
                        self.protect(l);
                    }
                }
                _ => {}
            }
            n = self.list.nodes[i].next;
        }
    }
}

/// Which glyphs of which font have been set and what the font's `/ToUnicode`
/// says each stands for: the `used` and `tounicode` fields harf-plug.lua
/// writes into the shared glyph table, and `set_tounicode` (:1125-1151) reads
/// at the end of the run.
///
/// The FIRST cluster a glyph is the whole of fixes its entry (:969-972), so
/// this is order-dependent and kept apart from the fonts: a caller shaping
/// paragraphs in parallel replays [`process`]'s calls in document order
/// against one of these.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToUnicode {
    entries: BTreeMap<(usize, u32), String>,
    used: std::collections::BTreeSet<(usize, u32)>,
}

/// The shared glyph table is the FACE's (`hbshared.glyphs`, one per file and
/// instance, harf-define.lua:331), so every size of one file shares it.
fn face_key(face: &HarfFace) -> usize {
    face as *const HarfFace as usize
}

impl ToUnicode {
    fn first(&self, face: &HarfFace, gid: u32) -> Option<String> {
        self.entries.get(&(face_key(face), gid)).cloned()
    }

    fn set(&mut self, face: &HarfFace, gid: u32, hex: String) {
        self.entries.insert((face_key(face), gid), hex);
    }

    fn used(&mut self, face: &HarfFace, gid: u32) {
        self.used.insert((face_key(face), gid));
    }

    /// `set_tounicode` for one font: every glyph set, with the UTF-16 hex
    /// its `/ToUnicode` entry carries -- the first whole cluster it stood
    /// for, else its nominal character, else U+FFFD.
    pub fn entries_of(&self, face: &HarfFace) -> Vec<(u32, String)> {
        let font = face_key(face);
        self.used
            .range((font, 0)..=(font, u32::MAX))
            .map(|&(_, gid)| {
                let hex = self
                    .entries
                    .get(&(font, gid))
                    .cloned()
                    .or_else(|| face.nominal(gid).map(to_utf16_hex))
                    .unwrap_or_else(|| "FFFD".into());
                (gid, hex)
            })
            .collect()
    }
}

/// luaotfload's harf plugin on one list for one font (harf-plug.lua:1043):
/// every run of `font`'s glyphs is shaped and replaced by what HarfBuzz made
/// of it. `direction` is the list's starting direction (0 left to right, 1
/// right to left). Glyphs of other fonts, and glyphs already processed, are
/// passed over.
///
/// Not ported, and refused rather than approximated by never being reached:
/// colour fonts (`COLR`/`CPAL` layers and `sbix`/`CBDT` bitmaps,
/// harf-plug.lua:822-922) -- a font with either draws its glyphs as
/// outlines here -- and the `shaper=` request option, which picks a HarfBuzz
/// back end harfrust does not have.
pub fn process(
    items: &[Item],
    fonts: &dyn Fonts,
    fontid: usize,
    direction: u8,
    unicode: &mut ToUnicode,
) -> Result<Vec<Item>, ShapeError> {
    let Some(font) = fonts.harf(fontid) else {
        return Ok(items.to_vec());
    };
    let mut sh = Shaper {
        list: List::default(),
        font,
        fontid,
        discs: Vec::new(),
        codes: Vec::new(),
    };
    let head = sh.list.build(items);
    let (newhead, runs) = sh.itemize(head, direction);
    let mut newhead = newhead;
    let mut current = newhead;
    let mut offset: i64 = 0;
    for mut run in runs {
        run.start = (run.start as i64 - offset) as usize;
        if run.skip {
            current = run.after;
            continue;
        }
        let (h, c, glyphs, new_offset) = sh.shape(newhead, current, &run);
        let Some(mut glyphs) = glyphs else {
            return Err(ShapeError(format!(
                "luaotfload | harf : All shapers failed for font \"{}\".",
                font.face.fullname
            )));
        };
        let (h, c) = sh.tonodes(h, c, &run, &mut glyphs, unicode);
        newhead = h;
        current = c;
        offset += new_offset;
    }
    let newhead = newhead.or(head);
    sh.protect(newhead);
    Ok(sh.list.items(newhead))
}

/// `post_process` (harf-plug.lua:1066-1093): an `/ActualText` span opened
/// before every glyph that starts one and closed after every glyph that ends
/// one, as page-mode literals. luaotfload runs this from `hpack_filter` on a
/// box's list and from `post_linebreak_filter` on each line.
pub fn post_process(items: Vec<Item>) -> Vec<Item> {
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        match item {
            Item::Glyph {
                char,
                font,
                xoffset,
                yoffset,
                processed,
                mut props,
            } => {
                let start = props.start_actual.take();
                let end = std::mem::take(&mut props.end_actual);
                if let Some(hex) = start {
                    out.push(Item::Literal(format!("/Span<</ActualText<FEFF{hex}>>>BDC")));
                }
                out.push(Item::Glyph {
                    char,
                    font,
                    xoffset,
                    yoffset,
                    processed,
                    props,
                });
                if end {
                    out.push(Item::Literal("EMC".into()));
                }
            }
            Item::Disc {
                subtype,
                pre,
                post,
                replace,
            } => out.push(Item::Disc {
                subtype,
                pre,
                post,
                replace: post_process(replace),
            }),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A face is shared between every size of it and every thread that
    /// shapes with it; that has to stay true of the types, not just of how
    /// they are used today.
    #[test]
    fn faces_and_fonts_cross_threads() {
        fn shareable<T: Send + Sync>() {}
        shareable::<HarfFace>();
        shareable::<HarfFont>();
        shareable::<ToUnicode>();
    }

    /// The engine's rounding of a Lua float, measured on luahbtex: -55.5sp
    /// is a kern of -55 and 166.5sp a glue of 167 (half up).
    #[test]
    fn floats_round_half_up() {
        assert_eq!(round(-55.5), -55);
        assert_eq!(round(166.5), 167);
        assert_eq!(round(-72744.96), -72745);
        assert_eq!(round(218234.88), 218235);
    }

    /// `script_to_iso`: the versioned Indic tags name their ISO script, a
    /// short tag is padded with its last letter, and the canonical names
    /// are applied first.
    #[test]
    fn opentype_script_tags_become_iso_ones() {
        assert_eq!(script_to_iso("dev2"), "deva");
        assert_eq!(script_to_iso("bng2"), "beng");
        assert_eq!(script_to_iso("mym2"), "mymr");
        assert_eq!(script_to_iso("latn"), "latn");
        assert_eq!(script_to_iso("DFLT"), "DFLT");
        assert_eq!(script_to_iso("lao "), "laoo");
        assert_eq!(script_to_iso("nko "), "nkoo");
        assert_eq!(script_to_iso("hira"), "kana");
    }

    /// `to_harfbuzz`: a request's script and language reach HarfBuzz as an
    /// ISO script and a private-use language tag that pins both OpenType
    /// tags; a language without a script is a plain BCP 47 tag.
    #[test]
    fn script_and_language_reach_harfbuzz_pinned() {
        let sl = ScriptLang::from_request(Some("arab"), Some("urd"));
        assert_eq!(sl.script, "Arab".parse().ok());
        assert_eq!(sl.language, "x-hbotURD-hbscarab".parse().ok());
        let sl = ScriptLang::from_request(Some("latn"), None);
        assert_eq!(sl.language, "x-hbotdflt-hbsclatn".parse().ok());
        let sl = ScriptLang::from_request(None, Some("TRK"));
        assert_eq!(sl.script, None);
        assert_eq!(sl.language, "trk".parse().ok());
        assert_eq!(ScriptLang::from_request(None, None), ScriptLang::default());
    }

    /// `get_designsize` reads the first `size` feature's FeatureParams, in
    /// decipoints; with none the design size is 10pt.
    #[test]
    fn design_size_comes_from_the_size_feature() {
        // GPOS 1.0: ScriptList at 10, FeatureList at 12 holding one record,
        // `size` -> feature table at +8 from the list, whose FeatureParams
        // (offset 4) give a design size of 120 decipoints.
        let mut gpos = vec![0u8, 1, 0, 0, 0, 10, 0, 12, 0, 0, 0, 0];
        gpos.extend_from_slice(&[0, 1, b's', b'i', b'z', b'e', 0, 8]);
        gpos.extend_from_slice(&[0, 4, 0, 0, 0, 120, 0, 0]);
        assert_eq!(
            designsize(Some(&gpos)),
            (120.0f64 * 6553.6 + 0.5).floor() as i64
        );
        assert_eq!(designsize(None), 655_360);
        assert_eq!(designsize(Some(&[])), 655_360);
    }

    #[test]
    fn utf16_for_actualtext() {
        assert_eq!(to_utf16_hex(0x41), "0041");
        assert_eq!(to_utf16_hex(0x1D400), "D835DC00");
    }

    /// `post_process`: a span opened before the glyph that starts it and
    /// closed after the one that ends it, inside a replacement list too.
    #[test]
    fn actualtext_spans_become_literals() {
        let glyph = |c: u32, start: Option<&str>, end: bool| Item::Glyph {
            char: c,
            font: 1,
            xoffset: 0,
            yoffset: 0,
            processed: true,
            props: Props {
                start_actual: start.map(str::to_string),
                end_actual: end,
                ..Props::default()
            },
        };
        let out = post_process(vec![
            glyph(0x120001, Some("0915094D"), false),
            glyph(0x120002, None, true),
            Item::Disc {
                subtype: 3,
                pre: vec![],
                post: vec![],
                replace: vec![glyph(0x120003, Some("0041"), true)],
            },
        ]);
        let literals: Vec<String> = out
            .iter()
            .filter_map(|i| match i {
                Item::Literal(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(literals, ["/Span<</ActualText<FEFF0915094D>>>BDC", "EMC"]);
        let Item::Disc { replace, .. } = &out[out.len() - 1] else {
            panic!("the discretionary is last");
        };
        assert!(matches!(&replace[0], Item::Literal(s) if s.ends_with("<FEFF0041>>>BDC")));
        assert!(matches!(&replace[2], Item::Literal(s) if s == "EMC"));
    }
}
