//! The colours a document names, and the models it names them in.
//!
//! `\textcolor[rgb]{1,0,0}{...}` carries its colour with it, but almost nothing
//! written by hand does that. A real document defines a palette once and refers
//! to it by name from then on -- and across the publication corpus every one of
//! the 2,119 definitions is `\definecolor{name}{HTML}{RRGGBB}`. Without the
//! name, `\color{neonCyan}` says nothing and the page comes out black.
//!
//! Everything is kept as three components in 0..=1, which is what PDF's `rg`
//! operator and a DVI `color push rgb` special both want.

use std::collections::BTreeMap;

/// A colour as PDF wants it: red, green and blue, each in 0..=1.
pub type Rgb = (f64, f64, f64);

/// The palette a document has defined so far.
#[derive(Debug, Clone)]
pub struct Colours {
    by_name: BTreeMap<String, Rgb>,
}

impl Default for Colours {
    fn default() -> Self {
        Colours::new()
    }
}

impl Colours {
    /// The palette every LaTeX document starts with.
    ///
    /// xcolor predefines these, so a document may write `\color{red}` without
    /// having defined anything -- and one that redefines `red` gets its own,
    /// because a definition overwrites.
    pub fn new() -> Colours {
        let mut by_name = BTreeMap::new();
        for (name, rgb) in [
            ("black", (0.0, 0.0, 0.0)),
            ("white", (1.0, 1.0, 1.0)),
            ("red", (1.0, 0.0, 0.0)),
            ("green", (0.0, 1.0, 0.0)),
            ("blue", (0.0, 0.0, 1.0)),
            ("cyan", (0.0, 1.0, 1.0)),
            ("magenta", (1.0, 0.0, 1.0)),
            ("yellow", (1.0, 1.0, 0.0)),
            ("gray", (0.5, 0.5, 0.5)),
            ("grey", (0.5, 0.5, 0.5)),
            ("darkgray", (0.25, 0.25, 0.25)),
            ("lightgray", (0.75, 0.75, 0.75)),
            ("brown", (0.75, 0.5, 0.25)),
            ("orange", (1.0, 0.5, 0.0)),
            ("pink", (1.0, 0.75, 0.75)),
            ("purple", (0.75, 0.0, 0.25)),
            ("violet", (0.5, 0.0, 0.5)),
            ("olive", (0.5, 0.5, 0.0)),
            ("teal", (0.0, 0.5, 0.5)),
            ("lime", (0.75, 1.0, 0.0)),
        ] {
            by_name.insert(name.to_string(), rgb);
        }
        Colours { by_name }
    }

    /// Name a colour that is already known, for `\colorlet`.
    pub fn define_rgb(&mut self, name: &str, rgb: Rgb) {
        self.by_name.insert(name.trim().to_string(), rgb);
    }

    /// Record `\definecolor{name}{model}{spec}`.
    ///
    /// A model this does not know is not recorded: a colour guessed from a
    /// spec read the wrong way is worse than one that stays the default, since
    /// the page still reads either way and only one of them is silently wrong.
    pub fn define(&mut self, name: &str, model: &str, spec: &str) -> bool {
        match parse(model, spec) {
            Some(rgb) => {
                self.by_name.insert(name.trim().to_string(), rgb);
                true
            }
            None => false,
        }
    }

    /// What a name means, if the document has said.
    pub fn get(&self, name: &str) -> Option<Rgb> {
        self.by_name.get(name.trim()).copied()
    }

    /// A colour written either way: `\color{name}` or `\color[model]{spec}`.
    pub fn resolve(&self, model: Option<&str>, spec: &str) -> Option<Rgb> {
        match model {
            Some(model) => parse(model, spec),
            None => self.get(spec),
        }
    }
}

/// A colour spec read in the model it was written in.
///
/// The models are xcolor's, and the two that matter are the two documents
/// actually write: `HTML`, which is what a palette lifted from a stylesheet
/// looks like, and `rgb`, which is what a package writes for itself.
pub fn parse(model: &str, spec: &str) -> Option<Rgb> {
    let spec = spec.trim();
    let numbers = || -> Vec<f64> {
        spec.split(',')
            .filter_map(|p| p.trim().parse::<f64>().ok())
            .collect()
    };
    match model.trim() {
        // Six hex digits, the way a stylesheet writes a colour.
        "HTML" => {
            let digits = spec.trim_start_matches('#');
            if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            let channel = |at: usize| {
                u8::from_str_radix(&digits[at..at + 2], 16)
                    .ok()
                    .map(|v| f64::from(v) / 255.0)
            };
            Some((channel(0)?, channel(2)?, channel(4)?))
        }
        "rgb" => match numbers()[..] {
            [r, g, b] => Some((clamp(r), clamp(g), clamp(b))),
            _ => None,
        },
        // The same three components written 0..=255 instead of 0..=1.
        "RGB" => match numbers()[..] {
            [r, g, b] => Some((clamp(r / 255.0), clamp(g / 255.0), clamp(b / 255.0))),
            _ => None,
        },
        "gray" | "grey" => match numbers()[..] {
            [v] => Some((clamp(v), clamp(v), clamp(v))),
            _ => None,
        },
        // Print colour. The conversion is the naive one, which is what every
        // reader does with a cmyk fill it has no profile for.
        "cmyk" => match numbers()[..] {
            [c, m, y, k] => Some((
                clamp((1.0 - c) * (1.0 - k)),
                clamp((1.0 - m) * (1.0 - k)),
                clamp((1.0 - y) * (1.0 - k)),
            )),
            _ => None,
        },
        _ => None,
    }
}

fn clamp(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_every_document_in_the_corpus_writes() {
        // 2,119 of 2,119 `\definecolor`s in the publication corpus are HTML.
        assert_eq!(
            parse("HTML", "FF2A6D"),
            Some((1.0, 42.0 / 255.0, 109.0 / 255.0))
        );
        assert_eq!(parse("HTML", "000000"), Some((0.0, 0.0, 0.0)));
        assert_eq!(parse("HTML", "FFFFFF"), Some((1.0, 1.0, 1.0)));
        // A leading `#` is not xcolor's spelling, but it is what gets pasted.
        assert_eq!(parse("HTML", "#05D9E8"), parse("HTML", "05D9E8"));
        // Wrong length or a non-digit is not a colour, and must not be read as
        // one: a mis-read palette is wrong on every page it reaches.
        assert_eq!(parse("HTML", "FFF"), None);
        assert_eq!(parse("HTML", "GGGGGG"), None);
    }

    #[test]
    fn the_other_models_a_package_writes() {
        assert_eq!(parse("rgb", "1,0,0"), Some((1.0, 0.0, 0.0)));
        assert_eq!(parse("RGB", "255,128,0"), Some((1.0, 128.0 / 255.0, 0.0)));
        assert_eq!(parse("gray", "0.5"), Some((0.5, 0.5, 0.5)));
        // Out of range is clamped rather than refused: the document still says
        // which colour it wants, and a reader clamps too.
        assert_eq!(parse("rgb", "2,-1,0.5"), Some((1.0, 0.0, 0.5)));
        assert_eq!(parse("cmyk", "0,0,0,0"), Some((1.0, 1.0, 1.0)));
        assert_eq!(parse("cmyk", "0,0,0,1"), Some((0.0, 0.0, 0.0)));
        // A model nothing knows is refused rather than guessed.
        assert_eq!(parse("wave", "580"), None);
    }

    #[test]
    fn a_name_means_what_the_document_defined_it_as() {
        let mut colours = Colours::new();
        assert_eq!(colours.get("red"), Some((1.0, 0.0, 0.0)));
        assert!(colours.define("neonCyan", "HTML", "05D9E8"));
        assert_eq!(colours.get("neonCyan"), parse("HTML", "05D9E8"));
        // A definition overwrites, including one of xcolor's own.
        assert!(colours.define("red", "HTML", "000000"));
        assert_eq!(colours.get("red"), Some((0.0, 0.0, 0.0)));
        // A name nothing defined stays unknown, so the caller can leave the
        // text in the colour it already had.
        assert_eq!(colours.get("nosuchcolour"), None);
        // A spec that does not parse defines nothing at all.
        assert!(!colours.define("bad", "HTML", "nope"));
        assert_eq!(colours.get("bad"), None);
    }

    #[test]
    fn both_ways_a_colour_reaches_a_command() {
        let mut colours = Colours::new();
        colours.define("bgPrimary", "HTML", "05050A");
        // `\color{bgPrimary}` -- by name.
        assert_eq!(colours.resolve(None, "bgPrimary"), parse("HTML", "05050A"));
        // `\color[rgb]{1,0,0}` -- carried with the command.
        assert_eq!(colours.resolve(Some("rgb"), "1,0,0"), Some((1.0, 0.0, 0.0)));
    }
}
