//! Variable fonts as the fontloader reads them (font-dsp.lua's `stat`,
//! `avar`, `fvar`, `hvar` and `mvar` readers and its variation helpers,
//! merged as fontloader-2023-12-28.lua:13395-13585 and :16180-16479), with
//! the `getfactors` luaotfload replaces the fontloader's with
//! (luaotfload-features.lua:480-530): an instance request -- a named
//! instance, or `axis` values like `wght=700` -- becomes one normalised
//! factor per axis, and the variation stores turn factors into deltas.

use super::otr::FontData;
use super::stream::Stream;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Axis {
    pub tag: String,
    pub minimum: f64,
    pub default: f64,
    pub maximum: f64,
    pub flags: u32,
    /// The axis name, lowercased (`weight` for `wght`).
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Instance {
    pub subfamily: Option<String>,
    pub psname: Option<String>,
    /// One value per axis, in axis order.
    pub values: Vec<f64>,
}

/// A STAT axis value (`variants` of a design axis).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AxisValue {
    pub flags: u32,
    pub name: String,
    pub value: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesignAxis {
    pub tag: String,
    pub name: String,
    pub variants: Vec<AxisValue>,
}

/// `fontdata.variabledata`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VariableData {
    pub axis: Option<Vec<Axis>>,
    pub instances: Option<Vec<Instance>>,
    /// `avar`'s segment maps per axis, `None` where the map is not usable.
    pub segments: Option<Vec<Option<Vec<(f64, f64)>>>>,
    pub designaxis: Option<Vec<DesignAxis>>,
    pub fallbackname: Option<String>,
    /// HVAR gave the advances, so `gvar`'s phantom points do not.
    pub hvarwidths: bool,
}

fn name_of(fontdata: &FontData, id: u32) -> Option<String> {
    fontdata
        .extras
        .get(&id)
        .map(|c| String::from_utf8_lossy(c).into_owned())
}

/// `readtag`: four bytes, stripped and lowercased.
fn tag(f: &mut Stream) -> String {
    let b = f.bytes(4);
    String::from_utf8_lossy(&b).trim().to_ascii_lowercase()
}

/// `readers.stat` (merged :16180-16252).
pub fn read_stat(f: &mut Stream, fontdata: &mut FontData) {
    let Some(t) = fontdata.tables.get("stat").copied() else {
        return;
    };
    f.set(t.offset);
    let _version = f.u32();
    let _axissize = f.u16();
    let nofaxis = f.u16() as usize;
    let axisoffset = f.u32() as usize;
    let nofvalues = f.u16() as usize;
    let valuesoffset = f.u32() as usize;
    let fallbackname = name_of(fontdata, f.u16());
    let mut axis: Vec<(DesignAxis, u32)> = Vec::new();
    f.set(t.offset + axisoffset);
    for _ in 0..nofaxis {
        let tg = tag(f);
        let name = name_of(fontdata, f.u16()).map_or(tg.clone(), |n| n.to_ascii_lowercase());
        let ordering = f.u16();
        axis.push((
            DesignAxis {
                tag: tg,
                name,
                variants: Vec::new(),
            },
            ordering,
        ));
    }
    f.set(t.offset + valuesoffset);
    let values = f.u16s(nofvalues);
    for v in values {
        f.set(t.offset + valuesoffset + v as usize);
        let format = f.u16();
        let index = f.u16() as usize;
        let flags = f.u16();
        let name = name_of(fontdata, f.u16()).map_or("no name".into(), |n| n.to_ascii_lowercase());
        let value = f.fixed();
        // format 4 makes no variant, and inserting nil adds nothing
        if (1..=3).contains(&format) {
            if let Some((a, _)) = axis.get_mut(index) {
                a.variants.push(AxisValue { flags, name, value });
            }
        }
    }
    // `sort` is not stable in Lua; this one is
    axis.sort_by_key(|(_, o)| *o);
    let mut axis: Vec<DesignAxis> = axis.into_iter().map(|(a, _)| a).collect();
    for a in &mut axis {
        a.variants.sort_by(|x, y| x.name.cmp(&y.name));
    }
    fontdata
        .variabledata
        .get_or_insert_with(VariableData::default)
        .designaxis = Some(axis);
    fontdata
        .variabledata
        .get_or_insert_with(VariableData::default)
        .fallbackname = fallbackname;
}

/// `readers.avar` (merged :16253-16297).
pub fn read_avar(f: &mut Stream, fontdata: &mut FontData) {
    let Some(t) = fontdata.tables.get("avar").copied() else {
        return;
    };
    f.set(t.offset);
    let _version = f.u32();
    let _reserved = f.u16();
    let nofaxis = f.u16() as usize;
    let mut segments = Vec::new();
    for _ in 0..nofaxis {
        let nofvalues = f.u16() as usize;
        let mut values: Vec<(f64, f64)> = Vec::new();
        let (mut lastfrom, mut lastto): (Option<f64>, Option<f64>) = (None, None);
        for _ in 0..nofvalues {
            let from = f.f2dot14();
            let to = f.f2dot14();
            if lastfrom.is_some_and(|l| from <= l) || lastto.is_some_and(|l| to >= l) {
                continue;
            }
            values.push((from, to));
            lastfrom = Some(from);
            lastto = Some(to);
        }
        let n = values.len();
        let usable = n > 2
            && values[0] == (-1.0, -1.0)
            && values[n - 1] == (1.0, 1.0)
            && values[1..n - 1].iter().any(|v| *v == (0.0, 0.0));
        segments.push(usable.then_some(values));
    }
    fontdata
        .variabledata
        .get_or_insert_with(VariableData::default)
        .segments = Some(segments);
}

/// `readers.fvar` (merged :16298-16365).
pub fn read_fvar(f: &mut Stream, fontdata: &mut FontData) {
    let Some(t) = fontdata.tables.get("fvar").copied() else {
        return;
    };
    f.set(t.offset);
    let _version = f.u32();
    let offsettoaxis = t.offset + f.u16() as usize;
    f.skip_u16(1);
    let nofaxis = f.u16() as usize;
    let sizeofaxis = f.u16() as usize;
    let nofinstances = f.u16() as usize;
    let sizeofinstances = f.u16() as usize;
    let mut axis = Vec::new();
    f.set(offsettoaxis);
    for _ in 0..nofaxis {
        let tg = tag(f);
        let minimum = f.fixed();
        let default = f.fixed();
        let maximum = f.fixed();
        let flags = f.u16();
        let name = name_of(fontdata, f.u16())
            .unwrap_or("bad name".into())
            .to_ascii_lowercase();
        axis.push(Axis {
            tag: tg,
            minimum,
            default,
            maximum,
            flags,
            name,
        });
        if sizeofaxis > 20 {
            f.set(f.pos() + sizeofaxis - 20);
        }
    }
    let nofbytes = 2 + 2 + 2 + nofaxis * 4;
    let readpsname = nofbytes <= sizeofinstances;
    let skippable = sizeofinstances as i64 - nofbytes as i64;
    let mut instances = Vec::new();
    for _ in 0..nofinstances {
        let mut subfamid = Some(f.u16());
        let _flags = f.u16();
        let values: Vec<f64> = (0..nofaxis).map(|_| f.fixed()).collect();
        let mut psnameid = if readpsname { Some(f.u16()) } else { None };
        subfamid = subfamid.filter(|&s| s == 2 || s == 17 || (s > 256 && s < 32768));
        psnameid = psnameid.filter(|&p| p == 6 || (p > 256 && p < 32768));
        instances.push(Instance {
            subfamily: subfamid.and_then(|s| name_of(fontdata, s)),
            psname: psnameid.and_then(|p| name_of(fontdata, p)),
            values,
        });
        if skippable > 0 {
            f.set(f.pos() + skippable as usize);
        }
    }
    let vd = fontdata
        .variabledata
        .get_or_insert_with(VariableData::default);
    vd.axis = Some(axis);
    vd.instances = Some(instances);
}

/// The instances `readdata` makes up for an `fvar` without any
/// (font-otr.lua, merged :7952-7981): the default, minimum and maximum of
/// each axis.
pub fn complete_instances(vd: &mut VariableData) {
    let Some(axis) = vd.axis.clone() else {
        return;
    };
    if vd.instances.as_ref().is_some_and(|i| !i.is_empty()) {
        return;
    }
    let mut instances = Vec::new();
    for (n, a) in axis.iter().enumerate() {
        for (what, value) in [
            ("default", a.default),
            ("minimum", a.minimum),
            ("maximum", a.maximum),
        ] {
            instances.push(Instance {
                subfamily: Some(format!("{what}{}", a.tag)),
                psname: None,
                values: axis
                    .iter()
                    .enumerate()
                    .map(|(i, x)| if i == n { value } else { x.default })
                    .collect(),
            });
        }
    }
    vd.instances = Some(instances);
}

/// `getaxisscale` (merged :13415-13455). Its segment test asks whether a
/// segment map is a number, which none is, so a font with `avar` maps gets
/// the plain normalisation back from the first one.
fn getaxisscale(
    segments: &Option<Vec<Option<Vec<(f64, f64)>>>>,
    minimum: f64,
    default: f64,
    maximum: f64,
    user: f64,
) -> f64 {
    let user = user.clamp(minimum, maximum);
    let d = if user < default {
        -(default - user) / (default - minimum)
    } else if user > default {
        (user - default) / (maximum - default)
    } else {
        0.0
    };
    let _ = segments;
    d
}

/// `cleanname`: lowercase letters and digits only.
fn cleanname(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// `axistofactors` (`settings_to_hash` with numbers made numbers).
fn axistofactors(s: &str) -> Vec<(String, String)> {
    s.split(',')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

/// luaotfload's `helpers.getfactors` (luaotfload-features.lua:495-530):
/// `None` for the default (`true`) request.
pub fn getfactors(vd: &VariableData, instance: Option<&str>) -> Option<Vec<f64>> {
    let axis = vd.axis.as_ref()?;
    let segments = &vd.segments;
    let Some(instance) = instance else {
        return Some(
            axis.iter()
                .map(|a| getaxisscale(segments, a.minimum, a.default, a.maximum, a.default))
                .collect(),
        );
    };
    let want = cleanname(instance);
    if let Some(inst) = vd
        .instances
        .iter()
        .flatten()
        .find(|i| i.subfamily.as_deref().is_some_and(|s| cleanname(s) == want))
    {
        return Some(
            axis.iter()
                .zip(&inst.values)
                .map(|(a, v)| getaxisscale(segments, a.minimum, a.default, a.maximum, *v))
                .collect(),
        );
    }
    let values = axistofactors(instance);
    let get = |k: &str| {
        values
            .iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.as_str())
    };
    Some(
        axis.iter()
            .map(|a| {
                let raw = get(&a.name).or_else(|| get(&a.tag));
                let value = match raw {
                    Some(v) => v.parse::<f64>().ok().or_else(|| {
                        // a named design value of the axis, from STAT
                        let d = vd
                            .designaxis
                            .as_ref()?
                            .iter()
                            .find(|d| cleanname(&d.tag) == cleanname(&a.tag))?;
                        d.variants
                            .iter()
                            .find(|x| cleanname(&x.name) == cleanname(v))
                            .map(|x| x.value)
                    }),
                    None => None,
                };
                getaxisscale(
                    segments,
                    a.minimum,
                    a.default,
                    a.maximum,
                    value.unwrap_or(a.default),
                )
            })
            .collect(),
    )
}

/// One axis of a variation region.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegionAxis {
    pub start: f64,
    pub peak: f64,
    pub stop: f64,
}

/// `getscales` (merged :13506-13533).
pub fn getscales(regions: &[Vec<RegionAxis>], factors: &[f64]) -> Vec<f64> {
    regions
        .iter()
        .map(|region| {
            let mut s = 1.0;
            for (j, a) in region.iter().enumerate() {
                let f = factors.get(j).copied().unwrap_or(0.0);
                if a.start > a.peak || a.peak > a.stop {
                } else if a.start < 0.0 && a.stop > 0.0 && a.peak != 0.0 {
                } else if a.peak == 0.0 {
                } else if f < a.start || f > a.stop {
                    s = 0.0;
                    break;
                } else if f < a.peak {
                    s = s * (f - a.start) / (a.peak - a.start);
                } else if f > a.peak {
                    s = s * (a.stop - f) / (a.stop - a.peak);
                }
            }
            s
        })
        .collect()
}

/// One item variation data subtable: its deltas per row, and the scale of
/// each region it uses at the instance.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeltaSet {
    pub deltas: Vec<Vec<i32>>,
    pub scales: Vec<f64>,
}

impl DeltaSet {
    /// The delta of row `inner`, summed over the regions until a region has
    /// no delta (`if di then ... else break`).
    pub fn delta(&self, inner: usize) -> Option<f64> {
        let d = self.deltas.get(inner)?;
        let mut dd = 0.0;
        for (i, s) in self.scales.iter().enumerate() {
            match d.get(i) {
                Some(&di) => dd += s * di as f64,
                None => break,
            }
        }
        Some(dd)
    }
}

/// `readvariationdata` (merged :13538-13585), leaving the stream where it
/// was.
pub fn readvariationdata(f: &mut Stream, storeoffset: usize, factors: &[f64]) -> Vec<DeltaSet> {
    let position = f.pos();
    f.set(storeoffset);
    let _format = f.u16();
    let regionoffset = storeoffset + f.u32() as usize;
    let nofdeltadata = f.u16() as usize;
    let deltadata = f.u32s(nofdeltadata);
    f.set(regionoffset);
    let nofaxis = f.u16() as usize;
    let nofregions = f.u16() as usize;
    let regions: Vec<Vec<RegionAxis>> = (0..nofregions)
        .map(|_| {
            (0..nofaxis)
                .map(|_| RegionAxis {
                    start: f.f2dot14(),
                    peak: f.f2dot14(),
                    stop: f.f2dot14(),
                })
                .collect()
        })
        .collect();
    let mut out = Vec::new();
    for d in deltadata {
        f.set(storeoffset + d as usize);
        let nofdeltasets = f.u16() as usize;
        let nofshorts = f.u16() as usize;
        let nofused = f.u16() as usize;
        let used: Vec<Vec<RegionAxis>> = (0..nofused)
            .map(|_| regions.get(f.u16() as usize).cloned().unwrap_or_default())
            .collect();
        let deltas = (0..nofdeltasets)
            .map(|_| {
                let mut t: Vec<i32> = (0..nofshorts).map(|_| f.i16()).collect();
                for _ in nofshorts..nofused {
                    t.push(f.u8() as u8 as i8 as i32);
                }
                t
            })
            .collect();
        out.push(DeltaSet {
            deltas,
            scales: getscales(&used, factors),
        });
    }
    f.set(position);
    out
}

/// `math.round` as the fontloader defines it (fontloader-l-math.lua:15-29):
/// half away from zero.
pub fn round(x: f64) -> f64 {
    if x < 0.0 {
        -(-x + 0.5).floor()
    } else {
        (x + 0.5).floor()
    }
}

/// `readers.hvar` (merged :16366-16434): with an advance map, every glyph's
/// width gets its delta, rounded.
pub fn read_hvar(f: &mut Stream, fontdata: &mut FontData, factors: &[f64]) {
    let Some(t) = fontdata.tables.get("hvar").copied() else {
        return;
    };
    f.set(t.offset);
    let _version = f.u32();
    // Both offsets are added to the table's before they are tested, so
    // neither test fails: an absent advance map is read from the header
    // itself, which as a map says "no entries" and so gives every glyph
    // the implicit outer 0, inner index.
    let variationoffset = t.offset + f.u32() as usize;
    let advanceoffset = t.offset + f.u32() as usize;
    let deltas = readvariationdata(f, variationoffset, factors);
    f.set(advanceoffset);
    let format = f.u16();
    let mapcount = f.u16() as usize;
    let entrysize = ((format & 0x0030) >> 4) + 1;
    let innerbits = (format & 0x000F) + 1;
    let innermask = (1u32 << innerbits) - 1;
    let map: Vec<(u32, u32)> = (0..mapcount)
        .map(|_| {
            let m = match entrysize {
                1 => f.u8(),
                2 => f.u16(),
                3 => f.u24(),
                _ => f.u32(),
            };
            (m >> innerbits, m & innermask)
        })
        .collect();
    fontdata
        .variabledata
        .get_or_insert_with(VariableData::default)
        .hvarwidths = true;
    for i in 0..fontdata.nofglyphs {
        let Some(width) = fontdata.glyph(i as u32).width else {
            continue;
        };
        let (outer, inner) = map.get(i).copied().unwrap_or((0, i as u32));
        if let Some(d) = deltas
            .get(outer as usize)
            .and_then(|d| d.delta(inner as usize))
        {
            fontdata.glyph(i as u32).width = Some(width + round(d));
        }
    }
}

/// `readers.mvar` (merged :16440-16479): the font-wide metrics the instance
/// moves.
pub fn read_mvar(f: &mut Stream, fontdata: &mut FontData, factors: &[f64]) {
    let Some(t) = fontdata.tables.get("mvar").copied() else {
        return;
    };
    f.set(t.offset);
    let _version = f.u32();
    f.skip_u16(1);
    let recordsize = f.u16() as usize;
    let nofrecords = f.u16() as usize;
    let offsettostore = t.offset + f.u16() as usize;
    let deltas = readvariationdata(f, offsettostore, factors);
    for _ in 0..nofrecords {
        let tg = tag(f);
        let outer = f.u16() as usize;
        let inner = f.u16() as usize;
        if let Some(d) = deltas.get(outer).and_then(|d| {
            // the Lua sums every region, a missing delta being an error
            d.deltas.get(inner).map(|row| {
                d.scales
                    .iter()
                    .zip(row)
                    .map(|(s, x)| s * *x as f64)
                    .sum::<f64>()
            })
        }) {
            let d = round(d) as i32;
            let m = fontdata.windowsmetrics.get_or_insert_with(Default::default);
            match tg.as_str() {
                "hasc" => m.typoascender += d,
                "hdsc" => m.typodescender += d,
                "hlgp" => m.typolinegap += d,
                "hcla" => m.winascent = (m.winascent as i64 + d as i64) as u32,
                "hcld" => m.windescent = (m.windescent as i64 + d as i64) as u32,
                "xhgt" => m.xheight = m.xheight.map(|x| x + d),
                "cpht" => m.capheight = m.capheight.map(|x| x + d),
                "unds" => {
                    if let Some(p) = fontdata.postscript.as_mut() {
                        p.underlineposition += d;
                    }
                }
                "undo" => {
                    if let Some(p) = fontdata.postscript.as_mut() {
                        p.underlinethickness += d;
                    }
                }
                _ => {}
            }
        }
        // (a record larger than 8 bytes would throw in the Lua: its
        // `skipbytes` is not given the file)
        let _ = recordsize;
    }
}
