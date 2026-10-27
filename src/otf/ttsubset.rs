//! A TrueType font's subset as luatex embeds it for a CID font
//! (`/FontFile2`): `make_tt_subset` of writetype2.c, with the glyph table
//! rebuilding of tt_glyf.c (`tt_build_tables`), the table readers and
//! packers of tt_table.c and the file writer of sfnt.c
//! (`sfnt_create_FontFile_stream`), all from LuaTeX 1.24's `font/`.
//!
//! The CIDs are the glyph indices: every used glyph keeps its index, glyph
//! 0 is always there, a composite's components come along at the next free
//! indices, and the tables a viewer needs are written in the order the
//! font's own directory has them, the rebuilt ones in place.

use std::collections::BTreeSet;

/// The pieces luatex writes for one subset: the font program, the
/// `/CIDSet` bits, and the stem width it puts in the descriptor.
#[derive(Clone, Debug, PartialEq)]
pub struct TtSubset {
    pub fontfile: Vec<u8>,
    pub cidset: Vec<u8>,
    /// `font_dim[STEMV_CODE]` from OS/2's weight class, when there is OS/2.
    pub stemv: Option<i64>,
}

struct Reader<'a> {
    data: &'a [u8],
    loc: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, String> {
        let b = *self
            .data
            .get(self.loc)
            .ok_or("type 2: the file ended prematurely")?;
        self.loc += 1;
        Ok(b)
    }
    fn ushort(&mut self) -> Result<u16, String> {
        Ok(((self.byte()? as u16) << 8) | self.byte()? as u16)
    }
    fn short(&mut self) -> Result<i16, String> {
        Ok(self.ushort()? as i16)
    }
    fn ulong(&mut self) -> Result<u32, String> {
        Ok(((self.ushort()? as u32) << 16) | self.ushort()? as u32)
    }
}

struct Table {
    tag: [u8; 4],
    check_sum: u32,
    offset: u32,
    length: u32,
    data: Option<Vec<u8>>,
    required: bool,
}

struct Directory {
    version: u32,
    tables: Vec<Table>,
    num_kept: u16,
}

impl Directory {
    fn find(&self, tag: &str) -> Option<usize> {
        self.tables.iter().position(|t| t.tag == tag.as_bytes())
    }
    fn pos(&self, tag: &str) -> u32 {
        self.find(tag).map_or(0, |i| self.tables[i].offset)
    }
    /// `sfnt_set_table`: replace the table or add it at the end.
    fn set(&mut self, tag: &str, data: Vec<u8>) {
        let check_sum = checksum(&data);
        let length = data.len() as u32;
        match self.find(tag) {
            Some(i) => {
                let t = &mut self.tables[i];
                t.check_sum = check_sum;
                t.offset = 0;
                t.length = length;
                t.data = Some(data);
            }
            None => {
                let mut t4 = [0u8; 4];
                t4.copy_from_slice(&tag.as_bytes()[..4]);
                self.tables.push(Table {
                    tag: t4,
                    check_sum,
                    offset: 0,
                    length,
                    data: Some(data),
                    required: false,
                });
            }
        }
    }
    /// `sfnt_require_table`.
    fn require(&mut self, tag: &str, must_exist: bool) -> Result<(), String> {
        match self.find(tag) {
            Some(i) => {
                self.tables[i].required = true;
                self.num_kept += 1;
                Ok(())
            }
            None if must_exist => Err("type 2: some required TrueType table does not exist".into()),
            None => Ok(()),
        }
    }
}

/// `sfnt_calc_checksum`.
fn checksum(data: &[u8]) -> u32 {
    let mut sum: u32 = 0;
    for (i, &b) in data.iter().enumerate() {
        sum = sum.wrapping_add((b as u32) << (8 * (3 - (i & 3))));
    }
    sum
}

fn read_directory(r: &mut Reader, offset: usize) -> Result<Directory, String> {
    r.loc = offset;
    let version = r.ulong()?;
    let n = r.ushort()?;
    r.ushort()?;
    r.ushort()?;
    r.ushort()?;
    let mut tables = Vec::new();
    for _ in 0..n {
        let tag = r.ulong()?.to_be_bytes();
        tables.push(Table {
            tag,
            check_sum: r.ulong()?,
            offset: r.ulong()?,
            length: r.ulong()?,
            data: None,
            required: false,
        });
    }
    Ok(Directory {
        version,
        tables,
        num_kept: 0,
    })
}

fn put16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}

fn put32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}

#[derive(Clone, Default)]
struct Glyph {
    gid: u16,
    ogid: u16,
    advw: u16,
    lsb: i16,
    length: u32,
    data: Vec<u8>,
}

/// `tt_add_glyph`.
fn add_glyph(
    gd: &mut Vec<Glyph>,
    used: &mut [u8],
    last_gid: &mut u16,
    gid: u16,
    new_gid: u16,
) -> u16 {
    let bit = 1u8 << (7 - (new_gid % 8));
    if used[new_gid as usize / 8] & bit == 0 {
        gd.push(Glyph {
            gid: new_gid,
            ogid: gid,
            ..Glyph::default()
        });
        used[new_gid as usize / 8] |= bit;
    }
    if new_gid > *last_gid {
        *last_gid = new_gid;
    }
    new_gid
}

/// `make_tt_subset` for `used` glyph indices of the face (`subfont`
/// counting from 1 in a collection, as `fm->subfont` does).
pub fn make_tt_subset(data: &[u8], subfont: u32, used: &BTreeSet<u16>) -> Result<TtSubset, String> {
    let mut r = Reader { data, loc: 0 };
    let kind = r.ulong()?;
    let offset = if kind == 0x7474_6366 {
        // ttc_read_offset
        r.loc = 8;
        let num = r.ulong()?;
        let i = subfont.saturating_sub(1);
        if i >= num {
            0
        } else {
            r.loc = 12 + 4 * i as usize;
            r.ulong()? as usize
        }
    } else {
        0
    };
    if !(kind == 0x0001_0000 || kind == 0x7472_7565 || kind == 0x7474_6366) {
        return Err("ttf: invalid font type".into());
    }
    let mut dir = read_directory(&mut r, offset)?;
    if kind == 0x7474_6366 && dir.pos("CFF ") != 0 {
        return Err("type 2: a CFF collection is not a TrueType font".into());
    }
    let locate = |dir: &Directory, tag: &str| -> Result<usize, String> {
        match dir.pos(tag) {
            0 => Err(format!("ttf: {tag} table not found")),
            p => Ok(p as usize),
        }
    };

    // the glyphs: .notdef, then the used ones as their own CIDs
    let mut gd: Vec<Glyph> = Vec::new();
    let mut used_slot = vec![0u8; 8192];
    let mut last_gid = 0u16;
    add_glyph(&mut gd, &mut used_slot, &mut last_gid, 0, 0);
    let last_cid = used.iter().next_back().copied().unwrap_or(0);
    for &cid in used.iter().filter(|&&c| c >= 1) {
        add_glyph(&mut gd, &mut used_slot, &mut last_gid, cid, cid);
    }
    if gd.len() == 1 {
        return Err("type 2: there are no glyphs in the subset".into());
    }

    // tt_build_tables: head, hhea, maxp
    r.loc = locate(&dir, "head")?;
    let head_version = r.ulong()?;
    let font_revision = r.ulong()?;
    let _checksum_adjustment = r.ulong()?;
    let magic = r.ulong()?;
    let flags = r.ushort()?;
    let units = r.ushort()?;
    let created: Vec<u8> = (0..8).map(|_| r.byte()).collect::<Result<_, _>>()?;
    let modified: Vec<u8> = (0..8).map(|_| r.byte()).collect::<Result<_, _>>()?;
    let bbox = [r.short()?, r.short()?, r.short()?, r.short()?];
    let mac_style = r.ushort()?;
    let lowest = r.ushort()?;
    let direction = r.short()?;
    let loc_format = r.short()?;
    let glyph_format = r.short()?;

    r.loc = locate(&dir, "hhea")?;
    let hhea_version = r.ulong()?;
    let hhea_shorts: Vec<i16> = (0..14).map(|_| r.short()).collect::<Result<_, _>>()?;
    let metric_format = r.short()?;
    if metric_format != 0 {
        return Err("ttf: unknown metricDataFormat".into());
    }
    let num_hmetrics = r.ushort()?;

    r.loc = locate(&dir, "maxp")?;
    let maxp_version = r.ulong()?;
    let num_glyphs = r.ushort()?;
    let maxp_rest: Vec<u16> = (0..13).map(|_| r.ushort()).collect::<Result<_, _>>()?;

    // hmtx as tt_read_longMetrics
    r.loc = locate(&dir, "hmtx")?;
    let mut hmtx = Vec::with_capacity(num_glyphs as usize);
    let mut last_adv = 0u16;
    for gid in 0..num_glyphs {
        if gid < num_hmetrics {
            last_adv = r.ushort()?;
        }
        hmtx.push((last_adv, r.short()?));
    }
    // OS/2: the stem width luatex derives from the weight class
    let stemv = match dir.pos("OS/2") {
        0 => None,
        p => {
            r.loc = p as usize + 4;
            let weight = r.ushort()? as i64;
            Some((weight / 65) * (weight / 65) + 50)
        }
    };
    r.loc = locate(&dir, "loca")?;
    let location: Vec<u32> = match loc_format {
        0 => (0..=num_glyphs)
            .map(|_| r.ushort().map(|x| 2 * x as u32))
            .collect::<Result<_, _>>()?,
        1 => (0..=num_glyphs)
            .map(|_| r.ulong())
            .collect::<Result<_, _>>()?,
        _ => return Err("ttf: unknown IndexToLocFormat".into()),
    };
    let glyf = locate(&dir, "glyf")?;

    // the glyph programs; a composite's components are added as found
    let mut i = 0;
    while i < gd.len() {
        let gid = gd[i].ogid;
        if gid >= num_glyphs {
            return Err(format!("ttf: invalid glyph index (gid {gid})"));
        }
        let loc = location[gid as usize];
        let len = location[gid as usize + 1].wrapping_sub(loc);
        let (advw, lsb) = hmtx[gid as usize];
        gd[i].advw = advw;
        gd[i].lsb = lsb;
        gd[i].length = len;
        if len == 0 {
            i += 1;
            continue;
        } else if len < 10 {
            return Err(format!("ttf: invalid glyph data (gid {gid})"));
        }
        let start = glyf + loc as usize;
        let mut p = data
            .get(start..start + len as usize)
            .ok_or("type 2: the file ended prematurely")?
            .to_vec();
        let contours = i16::from_be_bytes([p[0], p[1]]);
        if contours < 0 {
            let mut at = 10;
            loop {
                if at + 4 > p.len() {
                    return Err(format!("ttf: invalid glyph data (gid {gid}): {len} bytes"));
                }
                let flags = u16::from_be_bytes([p[at], p[at + 1]]);
                let cgid = u16::from_be_bytes([p[at + 2], p[at + 3]]);
                if cgid >= num_glyphs {
                    return Err(format!(
                        "ttf: invalid gid ({cgid} > {num_glyphs}) in composite glyph {gid}"
                    ));
                }
                // tt_find_glyph, else the first empty slot
                let mut new_gid = gd.iter().find(|g| g.ogid == cgid).map_or(0, |g| g.gid);
                if new_gid == 0 {
                    let slot = (0..65534u16)
                        .find(|s| used_slot[*s as usize / 8] & (1 << (7 - (s % 8))) == 0)
                        .ok_or("ttf: no empty glyph slot available.")?;
                    new_gid = add_glyph(&mut gd, &mut used_slot, &mut last_gid, cgid, slot);
                }
                p[at + 2..at + 4].copy_from_slice(&new_gid.to_be_bytes());
                at += 4;
                at += if flags & 0x0001 != 0 { 4 } else { 2 };
                if flags & 0x0008 != 0 {
                    at += 2;
                } else if flags & 0x0040 != 0 {
                    at += 4;
                } else if flags & 0x0080 != 0 {
                    at += 8;
                }
                if flags & 0x0020 == 0 {
                    break;
                }
            }
        }
        gd[i].data = p;
        i += 1;
    }
    gd.sort_by_key(|g| g.gid);

    // hmtx, loca and glyf rebuilt
    let n = gd.len();
    let last_advw = gd[n - 1].advw;
    let mut glyf_size = 0u32;
    let mut numberofhmetrics: Option<u16> = None;
    for g in gd.iter().rev() {
        glyf_size += g.length + (4 - g.length % 4) % 4;
        if numberofhmetrics.is_none() && last_advw != g.advw {
            numberofhmetrics = Some(g.gid + 2);
        }
    }
    let numberofhmetrics = numberofhmetrics.unwrap_or(1);
    let short_loca = glyf_size < 0x20000;
    let mut hmtx_out = Vec::new();
    let mut loca_out = Vec::new();
    let mut glyf_out = Vec::new();
    let mut offset = 0u32;
    let mut prev: i64 = 0;
    let put_loca = |v: &mut Vec<u8>, off: u32| match short_loca {
        true => put16(v, (off / 2) as u16),
        false => put32(v, off),
    };
    for g in &gd {
        let gap = g.gid as i64 - prev - 1;
        for j in 1..=gap {
            if prev + j == numberofhmetrics as i64 - 1 {
                put16(&mut hmtx_out, last_advw);
            } else if prev + j < numberofhmetrics as i64 {
                put16(&mut hmtx_out, 0);
            }
            put16(&mut hmtx_out, 0);
            put_loca(&mut loca_out, offset);
        }
        if g.gid < numberofhmetrics {
            put16(&mut hmtx_out, g.advw);
        }
        put16(&mut hmtx_out, g.lsb as u16);
        put_loca(&mut loca_out, offset);
        let pad = (4 - g.length % 4) % 4;
        glyf_out.extend_from_slice(&g.data);
        glyf_out.extend(std::iter::repeat_n(0u8, pad as usize));
        offset += g.length + pad;
        prev = g.gid as i64;
    }
    put_loca(&mut loca_out, offset);
    dir.set("hmtx", hmtx_out);
    dir.set("loca", loca_out);
    dir.set("glyf", glyf_out);

    let mut maxp = Vec::new();
    put32(&mut maxp, maxp_version);
    put16(&mut maxp, last_gid + 1);
    for x in &maxp_rest {
        put16(&mut maxp, *x);
    }
    dir.set("maxp", maxp);
    let mut hhea = Vec::new();
    put32(&mut hhea, hhea_version);
    for x in &hhea_shorts {
        put16(&mut hhea, *x as u16);
    }
    put16(&mut hhea, metric_format as u16);
    put16(&mut hhea, numberofhmetrics);
    dir.set("hhea", hhea);
    let mut head = Vec::new();
    put32(&mut head, head_version);
    put32(&mut head, font_revision);
    put32(&mut head, 0);
    put32(&mut head, magic);
    put16(&mut head, flags);
    put16(&mut head, units);
    head.extend_from_slice(&created);
    head.extend_from_slice(&modified);
    for b in bbox {
        put16(&mut head, b as u16);
    }
    put16(&mut head, mac_style);
    put16(&mut head, lowest);
    put16(&mut head, direction as u16);
    put16(&mut head, if short_loca { 0 } else { 1 });
    put16(&mut head, glyph_format as u16);
    dir.set("head", head);

    // the tables a CIDFontType 2 keeps (writetype2.c:176-195)
    for (tag, must) in [
        ("OS/2", false),
        ("cmap", false),
        ("head", true),
        ("hhea", true),
        ("loca", true),
        ("maxp", false),
        ("name", true),
        ("glyf", true),
        ("hmtx", true),
        ("fpgm", false),
        ("cvt ", false),
        ("prep", false),
    ] {
        dir.require(tag, must)?;
    }
    let fontfile = fontfile_stream(&dir, data)?;

    // CIDSet: bit per CID, .notdef forced
    let mut cidset = vec![0u8; last_cid as usize / 8 + 1];
    cidset[0] |= 1 << 7;
    for &cid in used.iter().filter(|&&c| c >= 1) {
        cidset[cid as usize / 8] |= 1 << (7 - (cid % 8));
    }
    Ok(TtSubset {
        fontfile,
        cidset,
        stemv,
    })
}

/// `sfnt_create_FontFile_stream` (sfnt.c:319-393): the kept tables in
/// directory order, each on a four-byte boundary, the last unpadded.
fn fontfile_stream(dir: &Directory, data: &[u8]) -> Result<Vec<u8>, String> {
    let kept = dir.num_kept as u32;
    let mut out = Vec::new();
    put32(&mut out, dir.version);
    put16(&mut out, kept as u16);
    let mut max2 = 1u32;
    let mut log2 = 0u32;
    let mut n = kept;
    while n > 1 {
        n /= 2;
        max2 *= 2;
        log2 += 1;
    }
    let sr = max2 * 16;
    put16(&mut out, sr as u16);
    put16(&mut out, log2 as u16);
    put16(&mut out, (kept * 16).wrapping_sub(sr) as u16);
    let mut offset = 12 + 16 * kept;
    for t in dir.tables.iter().filter(|t| t.required) {
        offset += (4 - offset % 4) % 4;
        out.extend_from_slice(&t.tag);
        put32(&mut out, t.check_sum);
        put32(&mut out, offset);
        put32(&mut out, t.length);
        offset += t.length;
    }
    for t in dir.tables.iter().filter(|t| t.required) {
        let pad = (4 - out.len() % 4) % 4;
        out.extend(std::iter::repeat_n(0u8, pad));
        match &t.data {
            Some(d) => out.extend_from_slice(d),
            None => {
                let s = t.offset as usize;
                let bytes = data
                    .get(s..s + t.length as usize)
                    .ok_or("ttf: reading file failed")?;
                out.extend_from_slice(bytes);
            }
        }
    }
    Ok(out)
}
