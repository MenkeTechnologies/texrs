//! The extra features the fontloader adds to every font it loads
//! (font-otc.lua's `addfeature`, applied as the "check extra features"
//! enhancer, fontloader-2023-12-28.lua:29519-30232), for the ones a request
//! can switch on: ConTeXt's `trep`, `tquo` and `anum` (font-imp-tex.lua,
//! merged at :33122-33211) and luaotfload's `tlig`, `rot13` and `!!??`
//! (luaotfload-features.lua:548-669), whose `tlig` takes the place of the
//! fontloader's own.
//!
//! Each becomes an ordinary lookup over the glyphs the font has, enabled for
//! every script and language and placed at the front of the GSUB lookups
//! (`prepend`), so the one registered last runs first, and within a feature
//! the later part before the earlier.
//!
//! Not here: `blockligatures` (no rules unless configured, so no lookup)
//! and `collapse` (fonts-lig.lua's composition ligatures).

use std::collections::{BTreeMap, BTreeSet};

use super::dsp::{Coverage, Flags, Layout, LigTree, Sequence, Step};
use super::oup::{Description, Expanded};

enum Data {
    Substitution(Vec<(u32, u32)>),
    Ligature(&'static [(u32, &'static [u32])]),
}

struct Part {
    data: Data,
    /// `features`: script -> languages; `None` is `everywhere`.
    features: Option<&'static [(&'static str, &'static [&'static str])]>,
}

/// The extra features in registration order (`extrafeatures` slots).
fn registered() -> Vec<(&'static str, Vec<Part>, bool)> {
    const ANUM_ARABIC: &[(u32, u32)] = &[
        (0x0030, 0x0660),
        (0x0031, 0x0661),
        (0x0032, 0x0662),
        (0x0033, 0x0663),
        (0x0034, 0x0664),
        (0x0035, 0x0665),
        (0x0036, 0x0666),
        (0x0037, 0x0667),
        (0x0038, 0x0668),
        (0x0039, 0x0669),
    ];
    const ANUM_PERSIAN: &[(u32, u32)] = &[
        (0x0030, 0x06F0),
        (0x0031, 0x06F1),
        (0x0032, 0x06F2),
        (0x0033, 0x06F3),
        (0x0034, 0x06F4),
        (0x0035, 0x06F5),
        (0x0036, 0x06F6),
        (0x0037, 0x06F7),
        (0x0038, 0x06F8),
        (0x0039, 0x06F9),
    ];
    let mut rot13 = Vec::new();
    for (a, b) in (b'A'..=b'M').zip(b'N'..=b'Z') {
        rot13.push((a as u32, b as u32));
        rot13.push((b as u32, a as u32));
        rot13.push((a.to_ascii_lowercase() as u32, b.to_ascii_lowercase() as u32));
        rot13.push((b.to_ascii_lowercase() as u32, a.to_ascii_lowercase() as u32));
    }
    vec![
        (
            "trep",
            vec![Part {
                data: Data::Substitution(vec![(0x0027, 0x2019)]),
                features: None,
            }],
            false,
        ),
        // luaotfload's tlig, in the fontloader's slot
        (
            "tlig",
            vec![
                Part {
                    data: Data::Substitution(vec![
                        (0x0022, 0x201D),
                        (0x0027, 0x2019),
                        (0x0060, 0x2018),
                    ]),
                    features: None,
                },
                Part {
                    data: Data::Ligature(&[
                        (0x2013, &[0x002D, 0x002D]),
                        (0x2014, &[0x002D, 0x002D, 0x002D]),
                        (0x201C, &[0x0060, 0x0060]),
                        (0x201D, &[0x0027, 0x0027]),
                        (0x00A1, &[0x0021, 0x0060]),
                        (0x00BF, &[0x003F, 0x0060]),
                        (0x201E, &[0x002C, 0x002C]),
                        (0x00AB, &[0x003C, 0x003C]),
                        (0x00BB, &[0x003E, 0x003E]),
                    ]),
                    features: None,
                },
            ],
            false,
        ),
        (
            "tquo",
            vec![Part {
                data: Data::Ligature(&[
                    (0x201C, &[0x0060, 0x0060]),
                    (0x201D, &[0x0027, 0x0027]),
                    (0x201E, &[0x002C, 0x002C]),
                ]),
                features: None,
            }],
            false,
        ),
        (
            "anum",
            vec![
                Part {
                    data: Data::Substitution(ANUM_ARABIC.to_vec()),
                    features: Some(&[("arab", &["urd", "dflt"])]),
                },
                Part {
                    data: Data::Substitution(ANUM_PERSIAN.to_vec()),
                    features: Some(&[("arab", &["urd"])]),
                },
            ],
            true,
        ),
        (
            "rot13",
            vec![Part {
                data: Data::Substitution(rot13),
                features: None,
            }],
            false,
        ),
        (
            "!!??",
            vec![
                Part {
                    data: Data::Ligature(&[(0x203D, &[0x21, 0x3F]), (0x2E18, &[0xA1, 0xBF])]),
                    features: None,
                },
                Part {
                    data: Data::Ligature(&[(0x203D, &[0x3F, 0x21]), (0x2E18, &[0xBF, 0xA1])]),
                    features: None,
                },
            ],
            false,
        ),
    ]
}

/// `getrange(sequences, "gsub")`'s first: where `prepend` inserts.
fn first_gsub(sequences: &[Sequence]) -> usize {
    sequences
        .iter()
        .position(|s| s.typ.starts_with("gsub_"))
        .unwrap_or(0)
}

/// font-imp-tex.lua's `valid` for anum: some feature has an `arab` script.
fn has_arab(layout: &Layout) -> bool {
    layout
        .features
        .values()
        .any(|f| f.values().any(|s| s.contains_key("arab")))
}

/// The "check extra features" enhancer over a loaded face.
pub fn enhance(
    descriptions: &BTreeMap<u32, Description>,
    layout: &mut Layout,
    expanded: &mut Expanded,
) {
    for (name, parts, needs_arab) in registered() {
        if needs_arab && !has_arab(layout) {
            continue;
        }
        for part in parts {
            let (typ, coverage) = match part.data {
                // prepare_substitution: both glyphs present
                Data::Substitution(list) => {
                    let map: BTreeMap<u32, u32> = list
                        .into_iter()
                        .filter(|(a, b)| {
                            descriptions.contains_key(a) && descriptions.contains_key(b)
                        })
                        .collect();
                    (
                        "gsub_single",
                        (!map.is_empty()).then_some(Coverage::Single(map)),
                    )
                }
                // prepare_ligature + coverup's ligature action
                Data::Ligature(list) => {
                    let mut trees: BTreeMap<u32, LigTree> = BTreeMap::new();
                    for (lig, parts) in list {
                        if !descriptions.contains_key(lig)
                            || !parts.iter().all(|p| descriptions.contains_key(p))
                        {
                            continue;
                        }
                        let mut tree = trees.entry(parts[0]).or_default();
                        for p in &parts[1..] {
                            tree = tree.next.entry(*p).or_default();
                        }
                        tree.ligature = Some(*lig);
                    }
                    (
                        "gsub_ligature",
                        (!trees.is_empty()).then_some(Coverage::Ligature(trees)),
                    )
                }
            };
            let Some(coverage) = coverage else {
                continue;
            };
            let scripts: BTreeMap<String, BTreeSet<String>> = match part.features {
                None => BTreeMap::from([("*".to_string(), BTreeSet::from(["*".to_string()]))]),
                Some(f) => f
                    .iter()
                    .map(|(s, l)| (s.to_string(), l.iter().map(|l| l.to_string()).collect()))
                    .collect(),
            };
            let sequence = Sequence {
                index: 0,
                name: name.to_string(),
                typ,
                steps: vec![Step {
                    coverage,
                    format: None,
                    rules: None,
                    baseclasses: None,
                }],
                markclass: None,
                flags: Flags::default(),
                order: vec![name.to_string()],
                features: BTreeMap::from([(name.to_string(), scripts.clone())]),
                merged: false,
            };
            let at = first_gsub(&layout.sequences);
            layout.sequences.insert(at, sequence);
            expanded.rules.insert(at, vec![Vec::new()]);
            expanded.markclass.insert(at, None);
            let k = layout
                .features
                .entry("gsub".into())
                .or_default()
                .entry(name.to_string())
                .or_default();
            for (script, languages) in scripts {
                k.entry(script).or_default().extend(languages);
            }
        }
    }
}
