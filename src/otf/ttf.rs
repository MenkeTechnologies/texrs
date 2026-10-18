//! The TrueType outlines of `fontloader-font-ttf.lua` that a variable
//! font's instance needs (merged as fontloader-2023-12-28.lua:11979-13180):
//! each glyph's points read from `glyf`, moved by its `gvar` deltas at the
//! instance (with the untouched points interpolated), composites merged out
//! of their components, and the bounding box measured again from the
//! resulting outline, as `contours2outlines_shaped` measures it -- through the
//! cubic control points its quadratic curves become, and keeping the `glyf`
//! header's xmin.

use super::otr::FontData;
use super::stream::Stream;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
    pub on: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Component {
    pub index: u32,
    /// xscale, xrotate, yrotate, yscale, xoffset, yoffset
    pub matrix: [f64; 6],
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Nothing,
    Glyph {
        points: Vec<Point>,
        /// the 1-based index of each contour's last point
        contours: Vec<usize>,
        nofpoints: usize,
    },
    Composite {
        components: Vec<Component>,
        /// after `mergecomposites`
        merged: Option<(Vec<Point>, Vec<usize>)>,
    },
}

impl Shape {
    fn outline(&self) -> Option<(&[Point], &[usize])> {
        match self {
            Shape::Glyph {
                points, contours, ..
            } => Some((points, contours)),
            Shape::Composite {
                merged: Some((p, c)),
                ..
            } => Some((p, c)),
            _ => None,
        }
    }
}

/// `readglyph` (merged :12719-12782).
fn read_glyph(f: &mut Stream, nofcontours: usize) -> Shape {
    let contours: Vec<usize> = (0..nofcontours).map(|_| (f.i16() + 1) as usize).collect();
    let nofpoints = contours.last().copied().unwrap_or(0);
    let nofinstructions = f.u16() as usize;
    f.set(f.pos() + nofinstructions);
    let mut flags = vec![0u32; nofpoints + 1];
    let mut i = 1;
    while i <= nofpoints {
        let flag = f.u8();
        flags[i] = flag;
        if flag & 0x08 != 0 {
            let n = f.u8();
            for _ in 0..n {
                i += 1;
                if i <= nofpoints {
                    flags[i] = flag;
                } else {
                    flags.push(flag);
                }
            }
        }
        i += 1;
    }
    let mut points = Vec::with_capacity(nofpoints);
    let mut x = 0.0;
    for &flag in &flags[1..=nofpoints] {
        if flag & 0x02 != 0 {
            if flag & 0x10 != 0 {
                x += f.u8() as f64;
            } else {
                x -= f.u8() as f64;
            }
        } else if flag & 0x10 == 0 {
            x += f.i16() as f64;
        }
        points.push(Point {
            x,
            y: 0.0,
            on: flag & 0x01 != 0,
        });
    }
    let mut y = 0.0;
    for (p, &flag) in points.iter_mut().zip(&flags[1..=nofpoints]) {
        if flag & 0x04 != 0 {
            if flag & 0x20 != 0 {
                y += f.u8() as f64;
            } else {
                y -= f.u8() as f64;
            }
        } else if flag & 0x20 == 0 {
            y += f.i16() as f64;
        }
        p.y = y;
    }
    Shape::Glyph {
        points,
        contours,
        nofpoints,
    }
}

/// `readcomposite` (merged :12783-12861).
fn read_composite(f: &mut Stream) -> Shape {
    let mut components = Vec::new();
    loop {
        let flags = f.u16();
        let index = f.u16();
        let xyarg = flags & 0x0002 != 0;
        let scaled_offset = flags & 0x0800 != 0;
        let (mut xscale, mut xrotate, mut yrotate, mut yscale) = (1.0, 0.0, 0.0, 1.0);
        let (mut xoffset, mut yoffset) = (0.0, 0.0);
        let words = flags & 0x0001 != 0;
        let mut arg = || {
            if words {
                f.i16() as f64
            } else {
                f.u8() as u8 as i8 as f64
            }
        };
        let (a, b) = (arg(), arg());
        if xyarg {
            xoffset = a;
            yoffset = b;
        }
        if flags & 0x0008 != 0 {
            xscale = f.f2dot14();
            yscale = xscale;
            if xyarg && scaled_offset {
                xoffset *= xscale;
                yoffset *= yscale;
            }
        } else if flags & 0x0040 != 0 {
            xscale = f.f2dot14();
            yscale = f.f2dot14();
            if xyarg && scaled_offset {
                xoffset *= xscale;
                yoffset *= yscale;
            }
        } else if flags & 0x0080 != 0 {
            xscale = f.f2dot14();
            xrotate = f.f2dot14();
            yrotate = f.f2dot14();
            yscale = f.f2dot14();
            if xyarg && scaled_offset {
                xoffset *= (xscale * xscale + yrotate * yrotate).sqrt();
                yoffset *= (xrotate * xrotate + yscale * yscale).sqrt();
            }
        }
        components.push(Component {
            index,
            matrix: [xscale, xrotate, yrotate, yscale, xoffset, yoffset],
        });
        if flags & 0x0020 == 0 {
            break;
        }
    }
    Shape::Composite {
        components,
        merged: None,
    }
}

/// The shapes `readers.glyf` reads when an instance is asked for.
pub fn read_shapes(f: &mut Stream, fontdata: &FontData, locations: &[usize]) -> Vec<Option<Shape>> {
    let mut shapes = Vec::new();
    for index in 0..fontdata.nofglyphs {
        let location = locations[index];
        let next = locations.get(index + 1).copied().unwrap_or(location);
        if location >= fontdata.filesize {
            break;
        }
        if next > location {
            f.set(location);
            let nofcontours = f.i16();
            f.skip_u16(4);
            shapes.push(Some(match nofcontours {
                0 => Shape::Nothing,
                n if n > 0 => read_glyph(f, n as usize),
                _ => read_composite(f),
            }));
        } else {
            shapes.push(Some(Shape::Nothing));
        }
    }
    shapes
}

/// `readpoints` (merged :12967-12992): 1-based point numbers, or none for
/// "all points".
fn read_points(f: &mut Stream) -> (Option<Vec<usize>>, usize) {
    let mut count = f.u8() as usize;
    if count == 0 {
        return (None, 0);
    }
    if count >= 128 {
        count = (count & 0x7F) * 256 + f.u8() as usize;
    }
    let mut points = Vec::new();
    let mut n = 1usize;
    while points.len() < count {
        let control = f.u8();
        let words = control & 0x80 != 0;
        for _ in 0..=(control & 0x7F) {
            n += if words { f.u16() } else { f.u8() } as usize;
            points.push(n);
        }
    }
    let p = points.len();
    (Some(points), p)
}

/// `readdeltas` (merged :12993-13022).
fn read_deltas(f: &mut Stream, nofpoints: usize) -> Option<Vec<f64>> {
    let mut deltas = Vec::new();
    let mut left = nofpoints as i64;
    while left > 0 {
        let control = f.u8();
        let runlength = (control & 0x3F) as usize + 1;
        if control & 0x80 != 0 {
            deltas.extend(std::iter::repeat_n(0.0, runlength));
        } else {
            let words = control & 0x40 != 0;
            for _ in 0..runlength {
                deltas.push(if words {
                    f.i16() as f64
                } else {
                    f.u8() as u8 as i8 as f64
                });
            }
        }
        left -= runlength as i64;
    }
    (!deltas.is_empty()).then_some(deltas)
}

struct DeltaSet {
    factor: f64,
    points: Option<Vec<usize>>,
    xvalues: Option<Vec<f64>>,
    yvalues: Option<Vec<f64>>,
}

/// `applyaxis` (merged :12074-12266): the deltas moved onto the points,
/// those without one interpolated between the nearest ones on their contour;
/// and when `dowidth`, the advance from the phantom points.
fn apply_axis(
    width: &mut Option<f64>,
    points: &mut [Point],
    contours: &[usize],
    deltas: &[DeltaSet],
    dowidth: bool,
) {
    let nofpoints = points.len();
    let (mut dw, mut dl) = (0.0, 0.0);
    let mut xv = vec![0.0; nofpoints + 1];
    let mut yv = vec![0.0; nofpoints + 1];
    for ds in deltas {
        let (Some(xvalues), Some(yvalues)) = (&ds.xvalues, &ds.yvalues) else {
            continue;
        };
        let factor = ds.factor;
        let at = |v: &Vec<f64>, i: usize| v.get(i.wrapping_sub(1)).copied();
        match &ds.points {
            Some(dpoints) => {
                let mut cnt = dpoints.len() as i64;
                if dowidth {
                    cnt -= 4;
                }
                if cnt > 0 {
                    let cnt = cnt as usize;
                    let dp = |j: usize| dpoints[j - 1];
                    let mut first = 1usize;
                    let mut firstindex = 1usize;
                    for &last in contours {
                        if last >= first {
                            let mut lastindex = cnt;
                            if firstindex < cnt {
                                for currentindex in firstindex..=cnt {
                                    let found = dp(currentindex);
                                    if found <= first {
                                        firstindex = currentindex;
                                    }
                                    if found == last {
                                        lastindex = currentindex;
                                        break;
                                    } else if found > last {
                                        while lastindex > 1 && dp(lastindex) > last {
                                            lastindex -= 1;
                                        }
                                        break;
                                    }
                                }
                            }
                            // find(i): the delta at i, or the ones around it
                            let find = |i: usize| -> (Option<usize>, Option<usize>, Option<usize>) {
                                let mut prv = lastindex;
                                for j in firstindex..=lastindex {
                                    let nxt = dp(j);
                                    if nxt == i {
                                        return (None, Some(j), None);
                                    } else if nxt > i {
                                        return (Some(prv), None, Some(j));
                                    }
                                    prv = j;
                                }
                                (Some(prv), None, Some(firstindex))
                            };
                            for point in first..=last {
                                let (d1, d2, d3) = find(point);
                                if let Some(d2) = d2 {
                                    xv[point] = at(xvalues, d2).unwrap_or(0.0);
                                    yv[point] = at(yvalues, d2).unwrap_or(0.0);
                                    continue;
                                }
                                let (d1, d3) = (d1.unwrap(), d3.unwrap());
                                let n1 = dp(d1).min(nofpoints);
                                let n3 = dp(d3).min(nofpoints);
                                let (p1, p2, p3) =
                                    (points[n1 - 1], points[point - 1], points[n3 - 1]);
                                let (x1, y1) = (
                                    at(xvalues, d1).unwrap_or(0.0),
                                    at(yvalues, d1).unwrap_or(0.0),
                                );
                                let (x3, y3) = (
                                    at(xvalues, d3).unwrap_or(0.0),
                                    at(yvalues, d3).unwrap_or(0.0),
                                );
                                let interpolate =
                                    |a1: f64, a2: f64, a3: f64, v1: f64, v3: f64| -> f64 {
                                        if a1 == a3 {
                                            if v1 == v3 {
                                                v1
                                            } else {
                                                0.0
                                            }
                                        } else if a2 <= a1.min(a3) {
                                            if a1 < a3 {
                                                v1
                                            } else {
                                                v3
                                            }
                                        } else if a2 >= a1.max(a3) {
                                            if a1 > a3 {
                                                v1
                                            } else {
                                                v3
                                            }
                                        } else {
                                            let t = (a2 - a1) / (a3 - a1);
                                            (1.0 - t) * v1 + t * v3
                                        }
                                    };
                                xv[point] = interpolate(p1.x, p2.x, p3.x, x1, x3);
                                yv[point] = interpolate(p1.y, p2.y, p3.y, y1, y3);
                            }
                            if lastindex < cnt {
                                firstindex = lastindex + 1;
                            }
                        }
                        first = last + 1;
                    }
                    for (i, p) in points.iter_mut().enumerate() {
                        let (fx, fy) = (xv[i + 1], yv[i + 1]);
                        if fx != 0.0 {
                            p.x += factor * fx;
                        }
                        if fy != 0.0 {
                            p.y += factor * fy;
                        }
                    }
                }
            }
            None => {
                for (i, p) in points.iter_mut().enumerate() {
                    let Some(&x) = xvalues.get(i) else {
                        break;
                    };
                    let y = yvalues.get(i).copied().unwrap_or(0.0);
                    if x != 0.0 {
                        p.x += factor * x;
                    }
                    if y != 0.0 {
                        p.y += factor * y;
                    }
                }
            }
        }
        if dowidth {
            if let Some(&x) = xvalues.get(nofpoints + 1) {
                dw += factor * x;
            }
            if let Some(&x) = xvalues.get(nofpoints) {
                dl += factor * x;
            }
        }
    }
    if dowidth {
        *width = Some(width.unwrap_or(0.0) + dw - dl);
    }
}

/// `readers.gvar` (merged :13023-13180) at `factors`. Composite glyphs keep
/// their deltas unapplied, as the Lua keeps them (`shape.deltas`, which
/// nothing reads).
pub fn gvar(
    f: &mut Stream,
    fontdata: &mut FontData,
    shapes: &mut [Option<Shape>],
    factors: &[f64],
) {
    let Some(t) = fontdata.tables.get("gvar").copied() else {
        return;
    };
    f.set(t.offset);
    let _version = f.u32();
    let nofaxis = f.u16() as usize;
    let noftuples = f.u16() as usize;
    let tupleoffset = t.offset + f.u32() as usize;
    let nofglyphs = f.u16() as usize;
    let flags = f.u16();
    let dataoffset = t.offset + f.u32() as usize;
    let data: Vec<usize> = if flags & 0x0001 != 0 {
        (0..=nofglyphs)
            .map(|_| dataoffset + f.u32() as usize)
            .collect()
    } else {
        (0..=nofglyphs)
            .map(|_| dataoffset + 2 * f.u16() as usize)
            .collect()
    };
    let read_tuple = |f: &mut Stream| -> Vec<f64> { (0..nofaxis).map(|_| f.f2dot14()).collect() };
    f.set(tupleoffset);
    let tuples: Vec<Vec<f64>> = (0..noftuples).map(|_| read_tuple(f)).collect();
    let dowidth = !fontdata.variabledata.as_ref().is_some_and(|v| v.hvarwidths);
    for i in 0..nofglyphs {
        let (start, next) = (data[i], data[i + 1]);
        if start == next {
            continue;
        }
        let Some(Some(shape)) = shapes.get_mut(i) else {
            continue;
        };
        f.set(start);
        let tflags = f.u16();
        let count = (tflags & 0x0FFF) as usize;
        let mut offset = start + f.u16() as usize;
        let allpoints = match shape {
            Shape::Glyph { nofpoints, .. } => *nofpoints,
            _ => 0,
        };
        let (mut shared, mut nofshared) = (None, 0);
        if tflags & 0x8000 != 0 {
            let current = f.pos();
            f.set(offset);
            (shared, nofshared) = read_points(f);
            offset = f.pos();
            f.set(current);
        }
        let mut deltas = Vec::new();
        for _ in 0..count {
            let size = f.u16() as usize;
            let vflags = f.u16();
            let index = (vflags & 0x0FFF) as usize;
            let peak = if vflags & 0x8000 != 0 {
                read_tuple(f)
            } else {
                tuples.get(index).cloned().unwrap_or_default()
            };
            let (istart, istop) = if vflags & 0x4000 != 0 {
                (Some(read_tuple(f)), Some(read_tuple(f)))
            } else {
                (None, None)
            };
            let mut points = shared.clone();
            let mut nofpoints = nofshared;
            let (mut xvalues, mut yvalues) = (None, None);
            if size > 0 {
                let current = f.pos();
                f.set(offset);
                if vflags & 0x2000 != 0 {
                    (points, nofpoints) = read_points(f);
                }
                if nofpoints == 0 {
                    nofpoints = allpoints + 4;
                }
                xvalues = read_deltas(f, nofpoints);
                yvalues = read_deltas(f, nofpoints);
                offset += size;
                f.set(current);
            }
            if xvalues.is_none() && yvalues.is_none() {
                points = None;
            }
            let mut s = 1.0;
            for (a, &fa) in factors.iter().enumerate().take(nofaxis) {
                let peak = peak.get(a).copied().unwrap_or(0.0);
                let start = istart
                    .as_ref()
                    .map_or(if peak < 0.0 { peak } else { 0.0 }, |v| v[a]);
                let stop = istop
                    .as_ref()
                    .map_or(if peak > 0.0 { peak } else { 0.0 }, |v| v[a]);
                if start > peak || peak > stop {
                } else if start < 0.0 && stop > 0.0 && peak != 0.0 {
                } else if peak == 0.0 {
                } else if fa < start || fa > stop {
                    s = 0.0;
                    break;
                } else if fa < peak {
                    s = s * (fa - start) / (peak - start);
                } else if fa > peak {
                    s = s * (stop - fa) / (stop - peak);
                }
            }
            if s != 0.0 {
                deltas.push(DeltaSet {
                    factor: s,
                    points,
                    xvalues,
                    yvalues,
                });
            }
        }
        if let Shape::Glyph {
            points, contours, ..
        } = shape
        {
            let mut width = fontdata.glyph(i as u32).width;
            apply_axis(&mut width, points, contours, &deltas, dowidth);
            fontdata.glyph(i as u32).width = width;
        }
    }
}

/// `mergecomposites` (merged :11979-12060): a composite's outline is its
/// components' points, transformed, one after the other.
pub fn merge_composites(shapes: &mut [Option<Shape>]) {
    fn merge(
        index: usize,
        shapes: &mut [Option<Shape>],
        depth: usize,
    ) -> Option<(Vec<Point>, Vec<usize>)> {
        if depth > 64 {
            return None;
        }
        let components = match &shapes[index] {
            Some(Shape::Composite {
                components,
                merged: None,
            }) => components.clone(),
            Some(s) => return s.outline().map(|(p, c)| (p.to_vec(), c.to_vec())),
            None => return None,
        };
        let mut points = Vec::new();
        let mut contours = Vec::new();
        let mut offset = 0;
        for c in &components {
            let sub = c.index as usize;
            if sub >= shapes.len() {
                continue;
            }
            let Some((subpoints, subcontours)) = merge(sub, shapes, depth + 1) else {
                continue;
            };
            let [xscale, xrotate, yrotate, yscale, xoffset, yoffset] = c.matrix;
            let plain = xscale == 1.0 && yscale == 1.0 && xrotate == 0.0 && yrotate == 0.0;
            for p in &subpoints {
                points.push(match plain {
                    true => Point {
                        x: p.x + xoffset,
                        y: p.y + yoffset,
                        on: p.on,
                    },
                    false => Point {
                        x: xscale * p.x + xrotate * p.y + xoffset,
                        y: yscale * p.y + yrotate * p.x + yoffset,
                        on: p.on,
                    },
                });
            }
            contours.extend(subcontours.iter().map(|&e| offset + e));
            offset += subpoints.len();
        }
        if let Some(Shape::Composite { merged, .. }) = &mut shapes[index] {
            *merged = Some((points.clone(), contours.clone()));
        }
        Some((points, contours))
    }
    for i in 0..shapes.len() {
        if matches!(shapes[i], Some(Shape::Composite { merged: None, .. })) {
            merge(i, shapes, 0);
        }
    }
}

/// `curveto` (merged :12066-12071): a quadratic segment's cubic control
/// points.
fn curveto(mx: f64, my: f64, lx: f64, ly: f64, rx: f64, ry: f64) -> (f64, f64, f64, f64, f64, f64) {
    (
        lx + 2.0 / 3.0 * (mx - lx),
        ly + 2.0 / 3.0 * (my - ly),
        rx + 2.0 / 3.0 * (mx - rx),
        ry + 2.0 / 3.0 * (my - ry),
        rx,
        ry,
    )
}

/// `contours2outlines_shaped`'s bounding box (merged :12396-12580) for the
/// glyphs `#glyphs - 1` reaches: every one but the last.
pub fn bounding_boxes(fontdata: &mut FontData, shapes: &[Option<Shape>]) {
    let n = fontdata.nofglyphs.saturating_sub(1);
    for index in 0..n {
        let Some(Some(shape)) = shapes.get(index) else {
            continue;
        };
        let Some((points, contours)) = shape.outline() else {
            continue;
        };
        if contours.is_empty() {
            continue;
        }
        let (mut xmin, mut ymin, mut xmax, mut ymax, mut done) = (0.0, 0.0, 0.0, 0.0, false);
        let grow =
            |x: f64, y: f64, xmin: &mut f64, ymin: &mut f64, xmax: &mut f64, ymax: &mut f64| {
                if x < *xmin {
                    *xmin = x;
                } else if x > *xmax {
                    *xmax = x;
                }
                if y < *ymin {
                    *ymin = y;
                } else if y > *ymax {
                    *ymax = y;
                }
            };
        let (mut px, mut py);
        let mut first = 1usize;
        for &last in contours {
            if last >= first && last != first {
                let pt = |i: usize| points[i - 1];
                let mut first_pt = pt(first);
                let last_pt = pt(last);
                let mut control: Option<Point> = None;
                // the Lua compares tables: first_pt is last_pt only when it
                // was made so here
                let mut first_is_last = false;
                if !first_pt.on {
                    if last_pt.on {
                        first_pt = last_pt;
                        first_is_last = true;
                    } else {
                        first_pt = Point {
                            x: (first_pt.x + last_pt.x) / 2.0,
                            y: (first_pt.y + last_pt.y) / 2.0,
                            on: false,
                        };
                    }
                    control = Some(first_pt);
                }
                let (x, y) = (first_pt.x, first_pt.y);
                if !done {
                    (xmin, ymin, xmax, ymax) = (x, y, x, y);
                    done = true;
                } else {
                    grow(x, y, &mut xmin, &mut ymin, &mut xmax, &mut ymax);
                }
                px = x;
                py = y;
                let mut previous = first_pt;
                for i in first..=last {
                    let current = pt(i);
                    if previous.on {
                        if current.on {
                            grow(
                                current.x, current.y, &mut xmin, &mut ymin, &mut xmax, &mut ymax,
                            );
                            px = current.x;
                            py = current.y;
                        } else {
                            control = Some(current);
                        }
                    } else if current.on {
                        let c = control.unwrap_or(current);
                        let (x1, y1, x2, y2, nx, ny) =
                            curveto(c.x, c.y, px, py, current.x, current.y);
                        for (a, b) in [(x1, y1), (x2, y2), (nx, ny)] {
                            grow(a, b, &mut xmin, &mut ymin, &mut xmax, &mut ymax);
                        }
                        px = nx;
                        py = ny;
                        control = None;
                    } else {
                        let x2 = (previous.x + current.x) / 2.0;
                        let y2 = (previous.y + current.y) / 2.0;
                        let c = control.unwrap_or(current);
                        let (x1, y1, cx2, cy2, nx, ny) = curveto(c.x, c.y, px, py, x2, y2);
                        for (a, b) in [(x1, y1), (cx2, cy2), (nx, ny)] {
                            grow(a, b, &mut xmin, &mut ymin, &mut xmax, &mut ymax);
                        }
                        px = nx;
                        py = ny;
                        control = Some(current);
                    }
                    previous = current;
                }
                // the closing segment
                if !first_is_last {
                    if let Some(c) = control {
                        grow(c.x, c.y, &mut xmin, &mut ymin, &mut xmax, &mut ymax);
                        let (_, _, x2, y2, nx, ny) =
                            curveto(c.x, c.y, px, py, first_pt.x, first_pt.y);
                        grow(x2, y2, &mut xmin, &mut ymin, &mut xmax, &mut ymax);
                        grow(nx, ny, &mut xmin, &mut ymin, &mut xmax, &mut ymax);
                    }
                }
            }
            first = last + 1;
        }
        let g = fontdata.glyph(index as u32);
        let header_xmin = g.boundingbox.map_or(0.0, |b| b[0]);
        // `math.round`: half away from zero (fontloader-l-math.lua:15-29)
        let r = super::var::round;
        g.boundingbox = Some([r(header_xmin), r(ymin), r(xmax), r(ymax)]);
    }
}
