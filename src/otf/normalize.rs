//! luaotfload's `normalize` feature (default `nfc`,
//! luaotfload-features.lua:849-892): before a node-mode font's lookups run,
//! its characters are put into Normalization Form C, composing only into
//! characters the font has -- lua-uni-normalize.lua's `nodes_to_nfc` with
//! the font's characters as `allowed_characters` and `preserve_attr` set.
//!
//! The Unicode data is read as lua-uni-parse.lua reads it: `UnicodeData.txt`
//! found through kpathsea, once per process. Without it nothing is
//! normalized.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use super::nodes::{Ix, Kind, List};

/// lua-uni-normalize.lua:54-76.
const COMPOSITION_EXCLUSIONS: [u32; 81] = [
    0x00958, 0x00959, 0x0095A, 0x0095B, 0x0095C, 0x0095D, 0x0095E, 0x0095F, 0x009DC, 0x009DD,
    0x009DF, 0x00A33, 0x00A36, 0x00A59, 0x00A5A, 0x00A5B, 0x00A5E, 0x00B5C, 0x00B5D, 0x00F43,
    0x00F4D, 0x00F52, 0x00F57, 0x00F5C, 0x00F69, 0x00F76, 0x00F78, 0x00F93, 0x00F9D, 0x00FA2,
    0x00FA7, 0x00FAC, 0x00FB9, 0x0FB1D, 0x0FB1F, 0x0FB2A, 0x0FB2B, 0x0FB2C, 0x0FB2D, 0x0FB2E,
    0x0FB2F, 0x0FB30, 0x0FB31, 0x0FB32, 0x0FB33, 0x0FB34, 0x0FB35, 0x0FB36, 0x0FB38, 0x0FB39,
    0x0FB3A, 0x0FB3B, 0x0FB3C, 0x0FB3E, 0x0FB40, 0x0FB41, 0x0FB43, 0x0FB44, 0x0FB46, 0x0FB47,
    0x0FB48, 0x0FB49, 0x0FB4A, 0x0FB4B, 0x0FB4C, 0x0FB4D, 0x0FB4E, 0x02ADC, 0x1D15E, 0x1D15F,
    0x1D160, 0x1D161, 0x1D162, 0x1D163, 0x1D164, 0x1D1BB, 0x1D1BC, 0x1D1BD, 0x1D1BE, 0x1D1BF,
    0x1D1C0,
];

/// The tables lua-uni-normalize.lua:27-192 builds.
pub struct Tables {
    ccc: BTreeMap<u32, u32>,
    /// canonical decompositions, the first element expanded recursively
    decomposition: BTreeMap<u32, Vec<u32>>,
    composition: BTreeMap<u32, BTreeMap<u32, u32>>,
    /// NFC_Quick_Check: `false` No, `true` Maybe, absent Yes
    nfc_qc: BTreeMap<u32, bool>,
}

impl Tables {
    /// From the text of `UnicodeData.txt`.
    pub fn parse(text: &str) -> Tables {
        let mut ccc = BTreeMap::new();
        let mut decomposition: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for line in text.lines() {
            let f: Vec<&str> = line.split(';').collect();
            if f.len() < 6 {
                continue;
            }
            let Ok(cp) = u32::from_str_radix(f[0], 16) else {
                continue;
            };
            if let Ok(c) = f[3].parse::<u32>() {
                if c != 0 {
                    ccc.insert(cp, c);
                }
            }
            // a `<tag>` makes it a compatibility mapping, not canonical
            let d = f[5];
            if !d.is_empty() && !d.starts_with('<') {
                let v: Vec<u32> = d
                    .split(' ')
                    .filter_map(|x| u32::from_str_radix(x, 16).ok())
                    .collect();
                if !v.is_empty() {
                    decomposition.insert(cp, v);
                }
            }
        }
        let mut nfc_qc = BTreeMap::new();
        for (&cp, d) in &decomposition {
            if ccc.contains_key(&cp)
                || ccc.contains_key(&d[0])
                || d.len() == 1
                || COMPOSITION_EXCLUSIONS.contains(&cp)
            {
                nfc_qc.insert(cp, false);
            } else {
                nfc_qc.insert(d[1], true);
            }
        }
        for i in (0x1161..=0x1175).chain(0x11A8..=0x11C2) {
            nfc_qc.insert(i, true);
        }
        let mut composition: BTreeMap<u32, BTreeMap<u32, u32>> = BTreeMap::new();
        for (&cp, d) in &decomposition {
            if d.len() > 1 && !(COMPOSITION_EXCLUSIONS.contains(&cp) || ccc.contains_key(&d[0])) {
                composition.entry(d[0]).or_default().insert(d[1], cp);
            }
        }
        // fixup_decomp: expand the first element until it does not decompose
        let keys: Vec<u32> = decomposition.keys().copied().collect();
        for cp in keys {
            let mut d = decomposition[&cp].clone();
            while let Some(first) = decomposition.get(&d[0]) {
                let mut e = first.clone();
                e.extend_from_slice(&d[1..]);
                d = e;
            }
            decomposition.insert(cp, d);
        }
        Tables {
            ccc,
            decomposition,
            composition,
            nfc_qc,
        }
    }

    /// The tables from TeX Live's `UnicodeData.txt`, if kpathsea finds it.
    pub fn get() -> Option<&'static Tables> {
        static TABLES: OnceLock<Option<Tables>> = OnceLock::new();
        TABLES
            .get_or_init(|| {
                let path = crate::kpse::locate("UnicodeData.txt")?;
                let text = std::fs::read_to_string(path).ok()?;
                Some(Tables::parse(&text))
            })
            .as_ref()
    }

    /// The canonical combining class, `None` for 0.
    pub fn ccc(&self, c: u32) -> Option<u32> {
        self.ccc.get(&c).copied()
    }
}

/// `nodes_to_nfc(head, f, allowed_characters, true)`
/// (lua-uni-normalize.lua:349-533). `allowed` says whether the font has a
/// character.
pub fn nodes_to_nfc(
    list: &mut List,
    head: Option<Ix>,
    font: usize,
    allowed: &dyn Fn(u32) -> bool,
    t: &Tables,
) -> Option<Ix> {
    let head0 = head?;
    let is_char = |list: &List, n: Option<Ix>| list.ischar(n, Some(font)).ok().flatten();
    let tmp = list.alloc(Kind::Other { id: u32::MAX });
    // passes 1 and 2: decompose what NFC_QC says No to, reorder marks
    let mut last_ccc: Option<u32> = None;
    let mut n = Some(head0);
    let prev = list.nodes[head0].prev;
    list.link(Some(tmp), Some(head0));
    let mut require_work = false;
    while let Some(cur) = n {
        if let Some(mut ch) = is_char(list, Some(cur)) {
            let mut qc = t.nfc_qc.get(&ch).copied();
            let at = cur;
            if qc == Some(false) {
                let decomposed = t.decomposition[&ch].clone();
                if decomposed.iter().all(|c| allowed(*c)) {
                    ch = decomposed[0];
                    qc = t.nfc_qc.get(&ch).copied();
                    list.set_char(at, ch);
                    // (`local n = n`: the copies go after one another, and
                    // the loop goes on from the first, reaching them next)
                    let mut last = at;
                    for &d in &decomposed[1..] {
                        let nn = list.copy(last);
                        list.set_char(nn, d);
                        list.insert_after(Some(tmp), Some(last), nn);
                        last = nn;
                    }
                }
            }
            let this_ccc = t.ccc(ch);
            if let (Some(l), Some(c)) = (last_ccc, this_ccc) {
                if l > c {
                    let mut nn = at;
                    while nn != tmp {
                        nn = list.nodes[nn].prev.expect("the sentinel precedes the list");
                        let Some(nc) = is_char(list, Some(nn)) else {
                            break;
                        };
                        match t.ccc(nc) {
                            Some(x) if x > c => {}
                            _ => break,
                        }
                    }
                    let (before, after) = (list.nodes[at].prev, list.nodes[at].next);
                    list.insert_after(Some(tmp), Some(nn), at);
                    list.link(before, after);
                    n = after;
                    require_work = require_work || qc.unwrap_or(false);
                    continue;
                }
            }
            n = list.nodes[at].next;
            last_ccc = this_ccc;
            require_work = require_work || qc.unwrap_or(false);
        } else {
            n = list.nodes[cur].next;
            last_ccc = None;
        }
    }
    let mut head = list.nodes[tmp].next;
    list.set_prev(head, prev);
    if !require_work {
        list.set_next(Some(tmp), None);
        return head;
    }
    // pass 3: maybe decompose, then compose again
    let mut starter_n: Option<Ix> = None;
    let mut starter: Option<u32> = None;
    let mut lookup: Option<&BTreeMap<u32, u32>> = None;
    let mut starter_decomposition: Option<Vec<u32>> = None;
    let mut last_ccc: Option<u32> = None;
    let mut i: Option<usize> = None;
    let mut i_ccc = 0;
    let mut n = head;
    let tail = list.tail(head);
    list.insert_after(head, tail, tmp);
    loop {
        let cur = n.expect("the sentinel ends the list");
        let mut ch = is_char(list, Some(cur));
        let this_ccc = ch.and_then(|c| t.ccc(c)).unwrap_or(300);
        let mut is_composed = false;
        while let Some(ix) = i {
            if i_ccc > this_ccc {
                break;
            }
            let sd = starter_decomposition.as_ref().unwrap();
            let new_starter = lookup.and_then(|l| l.get(&sd[ix - 1]).copied());
            match new_starter.filter(|s| allowed(*s)) {
                Some(s) => {
                    starter = Some(s);
                    list.set_char(starter_n.unwrap(), s);
                    lookup = t.composition.get(&s);
                }
                None => {
                    let nn = list.copy(starter_n.unwrap());
                    list.set_char(nn, sd[ix - 1]);
                    head = list.insert_before(head, Some(cur), nn);
                    last_ccc = Some(i_ccc);
                }
            }
            let next = ix + 1;
            match sd.get(next - 1) {
                Some(&c) => {
                    i = Some(next);
                    i_ccc = t.ccc(c).unwrap_or(300);
                }
                None => i = None,
            }
        }
        let mut n_now = cur;
        if let Some(c) = ch {
            if lookup.is_some() && ((this_ccc == 300) == (Some(this_ccc) == last_ccc)) {
                match lookup
                    .and_then(|l| l.get(&c).copied())
                    .filter(|s| allowed(*s))
                {
                    Some(s) => {
                        let last = list.nodes[cur].prev;
                        let (h, _) = list.remove(head, Some(cur));
                        head = h;
                        n_now = last.expect("a starter precedes");
                        starter = Some(s);
                        list.set_char(starter_n.unwrap(), s);
                        ch = Some(s);
                        is_composed = true;
                        lookup = t.composition.get(&s);
                    }
                    None => last_ccc = Some(this_ccc),
                }
            } else if lookup.is_none() && this_ccc == 300 && last_ccc == Some(300) {
                let st = starter.unwrap_or(0);
                if (0x1100..=0x1112).contains(&st) && (0x1161..=0x1175).contains(&c) {
                    let s = ((st - 0x1100) * 21 + c - 0x1161) * 28 + 0xAC00;
                    if allowed(s) {
                        let (h, _) = list.remove(head, Some(cur));
                        head = h;
                        starter = Some(s);
                        list.set_char(starter_n.unwrap(), s);
                        ch = Some(s);
                        is_composed = true;
                        lookup = t.composition.get(&s);
                        n_now = starter_n.unwrap();
                    }
                } else if (0x11A8..=0x11C2).contains(&c)
                    && (0xAC00..=0xD7A3).contains(&st)
                    && (st - 0xAC00).is_multiple_of(28)
                {
                    let s = st + c - 0x11A7;
                    if allowed(s) {
                        let (h, _) = list.remove(head, Some(cur));
                        head = h;
                        starter = Some(s);
                        list.set_char(starter_n.unwrap(), s);
                        ch = Some(s);
                        is_composed = true;
                        lookup = t.composition.get(&s);
                        n_now = starter_n.unwrap();
                    }
                }
            } else {
                last_ccc = Some(this_ccc);
            }
            if this_ccc == 300 {
                starter_n = Some(n_now);
                if is_composed {
                    starter = ch;
                } else {
                    let c = ch.unwrap();
                    starter_decomposition = t
                        .decomposition
                        .get(&c)
                        .cloned()
                        .filter(|d| d.iter().all(|x| allowed(*x)));
                    let s = starter_decomposition.as_ref().map_or(c, |d| d[0]);
                    starter = Some(s);
                    list.set_char(n_now, s);
                    match &starter_decomposition {
                        Some(d) => {
                            i = Some(2);
                            i_ccc = d.get(1).and_then(|x| t.ccc(*x)).unwrap_or(300);
                        }
                        None => i = None,
                    }
                }
                lookup = starter.and_then(|s| t.composition.get(&s));
            }
        } else {
            starter = None;
            lookup = None;
            last_ccc = None;
            i = None;
        }
        if n_now == tmp {
            let (h, _) = list.remove(head, Some(tmp));
            head = h;
            break;
        }
        n = list.nodes[n_now].next;
    }
    head
}
