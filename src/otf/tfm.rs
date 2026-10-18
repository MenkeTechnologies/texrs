//! A node-mode font at a size: `copytotfm` (fontloader-font-otl.lua:314-549)
//! and `constructors.scale` (fontloader-font-con.lua:344-944), then the
//! `patch_font` callbacks luaotfload runs on the result
//! (luaotfload-auxiliary.lua) and the feature initialisers that touch the
//! parameters (`no_designsize`, `internal__variablespace`, `fixedspace`).
//!
//! Every number here is the Lua float the table carries; the engine rounds
//! it when the font is defined, as [`round`] does.

use std::collections::BTreeMap;

use super::oup::{Description, Unicode};

/// The engine's reading of a Lua float as a dimension: half up (measured in
/// `crate::shape`).
pub fn round(x: f64) -> i64 {
    (x + 0.5).floor() as i64
}

/// The metadata `copytotfm` reads (font-otr.lua:1953-1989's `getinfo`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Metadata {
    pub units: f64,
    pub designsize: Option<f64>,
    pub monospaced: bool,
    pub averagewidth: Option<f64>,
    pub xheight: Option<f64>,
    pub capheight: Option<f64>,
    pub ascender: Option<f64>,
    pub descender: Option<f64>,
    pub italicangle: f64,
    pub fontname: Option<String>,
    pub fullname: Option<String>,
}

/// One scaled character (font-con.lua:703-917).
#[derive(Clone, Debug, PartialEq)]
pub struct Character {
    pub index: u32,
    pub width: f64,
    pub height: f64,
    pub depth: Option<f64>,
    pub italic: Option<f64>,
    pub unicode: Option<Unicode>,
    /// `chr.tounicode`: the UTF-16 hex the PDF's `/ToUnicode` gets.
    pub tounicode: String,
}

/// The parameters a scaled font carries, by luaotfload's names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parameters {
    pub slant: f64,
    pub space: f64,
    pub space_stretch: f64,
    pub space_shrink: f64,
    pub x_height: f64,
    pub quad: f64,
    pub extra_space: f64,
    /// `parameters[8]`, XeTeX's cap height (`set_capheight`).
    pub capheight: Option<f64>,
    pub ascender: f64,
    pub descender: f64,
    pub size: f64,
    pub factor: f64,
}

impl Parameters {
    /// `\fontdimen1` to `\fontdimen8` as the engine stores them.
    pub fn fontdimens(&self) -> Vec<i64> {
        let mut v = vec![
            round(self.slant),
            round(self.space),
            round(self.space_stretch),
            round(self.space_shrink),
            round(self.x_height),
            round(self.quad),
            round(self.extra_space),
        ];
        if let Some(c) = self.capheight {
            v.push(round(c));
        }
        v
    }
}

/// `tounicode` (font-map.lua:118-137) for a description's Unicode.
pub fn tounicode(u: &Unicode) -> String {
    let one = |k: u32| -> String {
        if k < 0xD7FF || (k > 0xDFFF && k <= 0xFFFF) {
            format!("{k:04X}")
        } else {
            let k = k - 0x10000;
            format!("{:04X}{:04X}", k / 0x400 + 0xD800, k % 0x400 + 0xDC00)
        }
    };
    match u {
        Unicode::Many(list) => list.iter().map(|&k| one(k)).collect(),
        Unicode::One(k) => {
            let k = *k;
            if (0xE000..=0xF8FF).contains(&k)
                || (0x0F_0000..=0x0F_FFFF).contains(&k)
                || (0x10_0000..=0x10_FFFF).contains(&k)
                || (0xD7FF..=0xDFFF).contains(&k)
            {
                "FFFD".into()
            } else {
                one(k)
            }
        }
    }
}

/// A node-mode font at one size.
#[derive(Clone, Debug, PartialEq)]
pub struct Scaled {
    pub characters: BTreeMap<u32, Character>,
    pub parameters: Parameters,
    pub units: f64,
}

/// `copytotfm`'s parameters, then `constructors.scale` at `size` scaled
/// points, then the `patch_font` and initialiser adjustments.
pub fn scale(descriptions: &BTreeMap<u32, Description>, meta: &Metadata, size: i64) -> Scaled {
    // copytotfm (font-otl.lua:314-549)
    let mut units = meta.units;
    if units == 0.0 {
        units = 1000.0;
    }
    let space_desc = descriptions.get(&0x20);
    let emdash = descriptions.get(&0x2014);
    let mut spaceunits: Option<f64> = None;
    if let Some(d) = space_desc {
        spaceunits = d.width;
    }
    if spaceunits.is_none() {
        if let Some(d) = emdash {
            spaceunits = d.width.map(|w| match meta.monospaced {
                true => w,
                false => w / 2.0,
            });
        }
    }
    if spaceunits.is_none() {
        spaceunits = meta.averagewidth;
    }
    let spaceunits = spaceunits.unwrap_or(units / 2.0);
    let mut slant = 0.0;
    if meta.italicangle != 0.0 {
        slant = -(meta.italicangle * std::f64::consts::PI / 180.0).tan();
    }
    // `units/2` and `units/3` at first, replaced: a monospaced font does
    // not stretch, and `syncspace` ties the others to the space.
    let (space_stretch, space_shrink) = match meta.monospaced {
        true => (0.0, 0.0),
        false => (spaceunits / 2.0, spaceunits / 3.0),
    };
    let extra_space = space_shrink;
    let x_height = match meta.xheight {
        Some(x) => x,
        None => descriptions
            .get(&0x78)
            .and_then(|d| d.height)
            .unwrap_or(2.0 * units / 5.0),
    };
    let ascender = meta.ascender.unwrap_or(0.0).abs();
    let descender = meta.descender.unwrap_or(0.0).abs();

    // constructors.scale (font-con.lua:344-944): `calculatescale` is
    // size / units.
    let size_f = size as f64;
    let delta = size_f / units;
    let (hdelta, vdelta) = (delta, delta);
    let mut parameters = Parameters {
        slant: slant * 65536.0,
        space: spaceunits * hdelta,
        space_stretch: space_stretch * hdelta,
        space_shrink: space_shrink * hdelta,
        x_height: x_height * vdelta,
        quad: units * hdelta,
        extra_space: extra_space * hdelta,
        capheight: None,
        ascender: delta * ascender,
        descender: delta * descender,
        size: size_f,
        factor: delta,
    };
    let mut characters = BTreeMap::new();
    for (&unicode, d) in descriptions {
        let width = d.width.unwrap_or(0.0) * hdelta;
        let height = d.height.map_or(0.0, |h| vdelta * h);
        let depth = d.depth.filter(|&x| x != 0.0).map(|x| delta * x);
        let tounicode = match &d.unicode {
            Some(u) => tounicode(u),
            None => "FFFD".into(),
        };
        characters.insert(
            unicode,
            Character {
                index: d.index,
                width,
                height,
                depth,
                italic: None,
                unicode: d.unicode.clone(),
                tounicode,
            },
        );
    }

    // patch_font: set_capheight (luaotfload-auxiliary.lua:264-292)
    let measured = ['X', 'M', 'Ж', 'ξ']
        .iter()
        .find_map(|c| characters.get(&(*c as u32)).map(|ch| ch.height));
    let alleged = meta.capheight.map(|c| c * size_f / units);
    let asc = Some(parameters.ascender);
    parameters.capheight = alleged.or(asc).or(measured);
    // set_xheight (luaotfload-auxiliary.lua:296-305)
    if parameters.x_height == 0.0 {
        parameters.x_height = match characters.get(&120) {
            Some(c) => c.height,
            None => parameters.ascender / 2.0,
        };
    }
    // internal__variablespace would give a monospaced font its stretch back,
    // but returns early whenever `fixedspace` is set -- and its default is
    // `auto` (luaotfload-features.lua:798-847), so it never does.
    Scaled {
        characters,
        parameters,
        units,
    }
}
