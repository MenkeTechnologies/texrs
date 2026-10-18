//! texrs's harf shaper against LuaHBTeX's, list by list.
//!
//! `scripts/harf-freeze.sh` runs every file in `tests/harf_cases` through
//! luahbtex with luaotfload and records, for each box a case dumps, the list
//! the box was given before any font handler ran and the list it came out
//! with (`tests/data/harf_expected.txt`). This hands the first to
//! `texrs::shape::process` -- for every harf font the list uses, as
//! luaotfload's `nodepass` does -- then `post_process`, and requires the
//! second back: the same glyph slots, the same offsets and metrics, the same
//! font kerns, the same interword glue widths, the same discretionaries and
//! the same `/ActualText` literals.
//!
//! The fonts are TeX Live's, found through kpsewhich, so a machine without
//! TeX Live skips the boxes whose fonts it cannot find rather than failing
//! them: what is being compared is the shaper, and without the font file
//! there is nothing to shape.

use std::collections::BTreeMap;
use std::sync::Arc;

use texrs::shape::{self, HarfFace, HarfFont, Item, Props, ScriptLang};

struct Case {
    label: String,
    fonts: Vec<(usize, i64, String)>,
    input: Vec<String>,
    output: Vec<String>,
}

fn cases() -> Vec<Case> {
    let text = include_str!("data/harf_expected.txt");
    let mut out: Vec<Case> = Vec::new();
    let mut section = 0;
    for line in text.lines() {
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

/// One list of the dump, back into items.
fn parse_list(lines: &[String], at: &mut usize) -> Vec<Item> {
    let mut items = Vec::new();
    while *at < lines.len() {
        let line = lines[*at].trim_start();
        if line == "|" || line == "}" {
            return items;
        }
        *at += 1;
        let mut w = line.split(' ');
        let kind = w.next().unwrap();
        let mut num = || w.next().unwrap().parse::<i64>().unwrap();
        items.push(match kind {
            "glyph" => {
                let char = u32::from_str_radix(line.split(' ').nth(1).unwrap(), 16).unwrap();
                let font = line.split(' ').nth(2).unwrap().parse().unwrap();
                Item::Glyph {
                    char,
                    font,
                    xoffset: 0,
                    yoffset: 0,
                    processed: false,
                    props: Props::default(),
                }
            }
            "glue" => Item::Glue {
                subtype: num() as u16,
                width: num(),
                stretch: num(),
                shrink: num(),
                props: Props::default(),
            },
            "kern" => Item::Kern {
                subtype: num() as u16,
                kern: num(),
                props: Props::default(),
            },
            "disc" => {
                let subtype = num() as u16;
                let pre = parse_list(lines, at);
                *at += 1;
                let post = parse_list(lines, at);
                *at += 1;
                let replace = parse_list(lines, at);
                *at += 1;
                Item::Disc {
                    subtype,
                    pre,
                    post,
                    replace,
                }
            }
            "dir" => Item::Dir {
                dir: num() as u8,
                cancel: num() == 1,
            },
            "localpar" => Item::LocalPar { dir: num() as u8 },
            "literal" => Item::Literal(line["literal ".len()..].to_string()),
            "other" => Item::Other { id: num() as u32 },
            other => panic!("unknown node {other:?}"),
        });
    }
    items
}

/// The shaped list in the dump's own format, metrics from the fonts.
fn write_list(
    items: &[Item],
    fonts: &BTreeMap<usize, HarfFont>,
    depth: usize,
    out: &mut Vec<String>,
) {
    let pad = "  ".repeat(depth);
    for item in items {
        match item {
            Item::Glyph {
                char,
                font,
                xoffset,
                yoffset,
                ..
            } => {
                let m = fonts.get(font).and_then(|f| f.char_metrics(*char));
                let (w, h, d) = m.map_or((0, 0, 0), |m| (m.width, m.height, m.depth));
                out.push(format!(
                    "{pad}glyph {char:X} {font} {xoffset} {yoffset} {w} {h} {d}"
                ));
            }
            Item::Glue {
                subtype,
                width,
                stretch,
                shrink,
                ..
            } => out.push(format!("{pad}glue {subtype} {width} {stretch} {shrink}")),
            Item::Kern { subtype, kern, .. } => out.push(format!("{pad}kern {subtype} {kern}")),
            Item::Disc {
                subtype,
                pre,
                post,
                replace,
            } => {
                out.push(format!("{pad}disc {subtype} {{"));
                write_list(pre, fonts, depth + 1, out);
                out.push(format!("{pad}|"));
                write_list(post, fonts, depth + 1, out);
                out.push(format!("{pad}|"));
                write_list(replace, fonts, depth + 1, out);
                out.push(format!("{pad}}}"));
            }
            Item::Dir { dir, cancel } => out.push(format!("{pad}dir {dir} {}", *cancel as u8)),
            Item::LocalPar { dir } => out.push(format!("{pad}localpar {dir}")),
            Item::Literal(s) => out.push(format!("{pad}literal {s}")),
            Item::Other { id } => out.push(format!("{pad}other {id}")),
        }
    }
}

fn load(spec: &str, size: i64, faces: &mut BTreeMap<String, Arc<HarfFace>>) -> Option<HarfFont> {
    let spec = texrs::fontreq::analyze(spec).ok()?;
    let path = texrs::kpse::locate(&spec.name)?;
    let face = match faces.get(&spec.name) {
        Some(f) => f.clone(),
        None => {
            let f = HarfFace::open(&path, 0).ok()?;
            faces.insert(spec.name.clone(), f.clone());
            f
        }
    };
    let script_lang = ScriptLang::from_request(spec.script(), spec.language());
    Some(HarfFont::new(face, size, &spec.raw, script_lang))
}

#[test]
fn harf_mode_matches_luahbtex() {
    let mut faces = BTreeMap::new();
    let mut failures = Vec::new();
    let mut compared = 0;
    for case in cases() {
        let mut fonts = BTreeMap::new();
        let mut missing = false;
        for (id, size, spec) in &case.fonts {
            match load(spec, *size, &mut faces) {
                Some(f) => {
                    fonts.insert(*id, f);
                }
                None => missing = true,
            }
        }
        if missing {
            eprintln!("skipping {:?}: a font it uses is not installed", case.label);
            continue;
        }
        let mut at = 0;
        let mut items = parse_list(&case.input, &mut at);
        let mut unicode = shape::ToUnicode::default();
        for &id in fonts.keys() {
            items = shape::process(&items, &fonts, id, 0, &mut unicode).unwrap();
        }
        let items = shape::post_process(items);
        let mut got = Vec::new();
        write_list(&items, &fonts, 0, &mut got);
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
        "{} of {compared} boxes differ from luahbtex\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
