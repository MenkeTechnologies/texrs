//! texrs's node-mode OpenType processing against luaotfload's, list by list.
//!
//! `scripts/node-freeze.sh` runs every file in `tests/node_cases` through
//! luahbtex with luaotfload and records, for each box a case dumps, the list
//! the box was given before any font handler ran and the list it came out
//! with (`tests/data/node_expected.txt`). This loads the same fonts with
//! `texrs::otf`, works out their features from the requests as luaotfload
//! does, hands the first list to `texrs::otf::ots::process` and requires the
//! second back: the same characters (ligatures, substitutions), the same
//! font kerns and glyph offsets the injections make, the same interword glue
//! widths (space kerns), and the same discretionaries.
//!
//! The fonts are TeX Live's, found through kpsewhich, so a machine without
//! them skips the boxes that use them rather than failing.

use std::collections::BTreeMap;

use texrs::otf::nodes::{Ix, Kind, List};
use texrs::otf::ots::{self, Font};
use texrs::otf::tfm::{self, Scaled};
use texrs::otf::{features, otj, Face};

struct Case {
    label: String,
    fonts: Vec<(usize, i64, String)>,
    input: Vec<String>,
    output: Vec<String>,
}

fn cases() -> Vec<Case> {
    let mut out: Vec<Case> = Vec::new();
    let mut section = 0;
    for line in include_str!("data/node_expected.txt").lines() {
        if line.starts_with('#') {
            continue;
        }
        if let Some(label) = line.strip_prefix("=== ") {
            out.push(Case {
                label: label.to_string(),
                fonts: Vec::new(),
                input: Vec::new(),
                output: Vec::new(),
            });
            section = 0;
            continue;
        }
        let case = out.last_mut().expect("a box header comes first");
        match line {
            "in" => section = 1,
            "out" => section = 2,
            _ if section == 0 => {
                let mut f = line.splitn(4, ' ');
                assert_eq!(f.next(), Some("font"));
                let id = f.next().unwrap().parse().unwrap();
                let size = f.next().unwrap().parse().unwrap();
                case.fonts.push((id, size, f.next().unwrap().to_string()));
            }
            _ if section == 1 => case.input.push(line.to_string()),
            _ => case.output.push(line.to_string()),
        }
    }
    out
}

/// One list of the dump, into the arena; returns its head.
fn parse_list(lines: &[String], at: &mut usize, list: &mut List) -> Option<Ix> {
    let mut head = None;
    let mut last: Option<Ix> = None;
    while *at < lines.len() {
        let line = lines[*at].trim_start();
        if line == "|" || line == "}" {
            break;
        }
        *at += 1;
        let w: Vec<&str> = line.split(' ').collect();
        let num = |i: usize| w[i].parse::<i64>().unwrap();
        let kind = match w[0] {
            "glyph" => Kind::Glyph {
                char: u32::from_str_radix(w[1], 16).unwrap(),
                font: w[2].parse().unwrap(),
                subtype: 1,
                components: None,
                xoffset: 0,
                yoffset: 0,
            },
            "glue" => Kind::Glue {
                subtype: num(1) as u16,
                width: num(2),
                stretch: num(3),
                shrink: num(4),
            },
            "kern" => Kind::Kern {
                subtype: num(1) as u16,
                kern: num(2),
            },
            "disc" => {
                let subtype = num(1) as u16;
                let pre = parse_list(lines, at, list);
                *at += 1;
                let post = parse_list(lines, at, list);
                *at += 1;
                let replace = parse_list(lines, at, list);
                *at += 1;
                let d = list.alloc(Kind::Disc {
                    subtype,
                    pre: None,
                    post: None,
                    replace: None,
                });
                list.set_disc(d, pre, post, replace);
                list.link(last, Some(d));
                head = head.or(Some(d));
                last = Some(d);
                continue;
            }
            "dir" => Kind::Dir {
                dir: num(1) as u8,
                cancel: num(2) == 1,
            },
            "localpar" => Kind::Par { dir: num(1) as u8 },
            "other" => Kind::Other { id: num(1) as u32 },
            other => panic!("unknown node {other:?}"),
        };
        let n = list.alloc(kind);
        list.link(last, Some(n));
        head = head.or(Some(n));
        last = Some(n);
    }
    head
}

/// The processed list in the dump's format, glyph metrics from the fonts.
fn write_list(
    list: &List,
    head: Option<Ix>,
    scaled: &BTreeMap<usize, Scaled>,
    depth: usize,
    out: &mut Vec<String>,
) {
    let pad = "  ".repeat(depth);
    let mut n = head;
    while let Some(i) = n {
        match &list.nodes[i].kind {
            Kind::Glyph {
                char,
                font,
                xoffset,
                yoffset,
                ..
            } => {
                let c = scaled.get(font).and_then(|s| s.characters.get(char));
                let (w, h, d) = c.map_or((0, 0, 0), |c| {
                    (
                        tfm::round(c.width),
                        tfm::round(c.height),
                        c.depth.map_or(0, tfm::round),
                    )
                });
                out.push(format!(
                    "{pad}glyph {char:X} {font} {xoffset} {yoffset} {w} {h} {d}"
                ));
            }
            Kind::Glue {
                subtype,
                width,
                stretch,
                shrink,
            } => out.push(format!("{pad}glue {subtype} {width} {stretch} {shrink}")),
            Kind::Kern { subtype, kern } => out.push(format!("{pad}kern {subtype} {kern}")),
            Kind::Disc {
                subtype,
                pre,
                post,
                replace,
            } => {
                out.push(format!("{pad}disc {subtype} {{"));
                write_list(list, *pre, scaled, depth + 1, out);
                out.push(format!("{pad}|"));
                write_list(list, *post, scaled, depth + 1, out);
                out.push(format!("{pad}|"));
                write_list(list, *replace, scaled, depth + 1, out);
                out.push(format!("{pad}}}"));
            }
            Kind::Dir { dir, cancel } => out.push(format!("{pad}dir {dir} {}", *cancel as u8)),
            Kind::Par { dir } => out.push(format!("{pad}localpar {dir}")),
            Kind::Math { .. } => out.push(format!("{pad}other 11")),
            Kind::Other { id } => out.push(format!("{pad}other {id}")),
        }
        n = list.nodes[i].next;
    }
}

struct Widths<'a>(&'a BTreeMap<usize, Scaled>);

impl otj::Widths for Widths<'_> {
    fn glyph_width(&self, font: usize, char: u32) -> i64 {
        self.0
            .get(&font)
            .and_then(|s| s.characters.get(&char))
            .map_or(0, |c| tfm::round(c.width))
    }
}

#[test]
fn node_mode_matches_luaotfload() {
    let mut faces: BTreeMap<String, std::sync::Arc<Face>> = BTreeMap::new();
    let mut failures = Vec::new();
    let mut compared = 0;
    for case in cases() {
        let mut specs = Vec::new();
        let mut missing = false;
        for (id, size, request) in &case.fonts {
            let spec = texrs::fontreq::analyze(request).unwrap();
            if !faces.contains_key(&spec.name) {
                match texrs::kpse::locate(&spec.name) {
                    Some(path) => {
                        faces.insert(spec.name.clone(), Face::open(&path, 0).unwrap());
                    }
                    None => missing = true,
                }
            }
            specs.push((*id, *size, spec));
        }
        if missing {
            eprintln!("skipping {:?}: a font it uses is not installed", case.label);
            continue;
        }
        let mut scaled = BTreeMap::new();
        for (id, size, spec) in &specs {
            scaled.insert(*id, faces[&spec.name].scale(*size));
        }
        let mut fonts = BTreeMap::new();
        for (id, _, spec) in &specs {
            let s = &scaled[id];
            let f = features::node_features(&spec.raw);
            fonts.insert(
                *id,
                Font {
                    face: &faces[&spec.name],
                    id: *id,
                    factor: s.parameters.factor,
                    threshold: 0.0,
                    widths: s.characters.iter().map(|(c, ch)| (*c, ch.width)).collect(),
                    checkmarks: f.features.contains_key("checkmarks"),
                    features: f.features,
                    script: f.script,
                    language: f.language,
                },
            );
        }
        let mut list = List::default();
        let mut at = 0;
        let head = parse_list(&case.input, &mut at, &mut list);
        let head = ots::process(&mut list, head, &fonts, 0, &Widths(&scaled));
        let mut got = Vec::new();
        write_list(&list, head, &scaled, 0, &mut got);
        compared += 1;
        if got != case.output {
            let first = got
                .iter()
                .zip(&case.output)
                .position(|(a, b)| a != b)
                .unwrap_or(got.len().min(case.output.len()));
            let window = |v: &[String]| {
                v.iter()
                    .skip(first.saturating_sub(2))
                    .take(6)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n    ")
            };
            failures.push(format!(
                "{}: first difference at line {first}\n  luahbtex:\n    {}\n  texrs:\n    {}",
                case.label,
                window(&case.output),
                window(&got)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {compared} boxes differ from luaotfload\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
