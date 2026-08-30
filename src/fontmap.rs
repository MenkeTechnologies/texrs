//! Reading a font map and an encoding file, ported from `fontmap.c` and
//! `t1_load_enc` in tectonic's `xdvipdfmx`.
//!
//! This is the join between everything else. A `.dvi` names a font `ptmr8r`,
//! and nothing in the file says what that is: the name is a TeX name, and the
//! map is what turns it into a real font file, an encoding to read it through,
//! and whatever slanting or extending the document asked for. Without it a
//! driver has a font name and no font.
//!
//! A map line is a small grammar with no separators:
//!
//! ```text
//! ptmr8r NimbusRomNo9L-Regu " TeXBase1Encoding ReEncodeFont " <8r.enc <utmr8a.pfb
//! cmr10 CMR10 <cmr10.pfb
//! ```
//!
//! The first word is the TeX name. A second bare word is the font's
//! PostScript name. A `<` names a file, and which kind it is comes from its
//! extension -- `.enc` is an encoding, anything else is the font. A quoted
//! run is PostScript to be run before the font is used, and the two things
//! worth pulling out of it are `SlantFont` and `ExtendFont`, which is how
//! `\textsl` gets a slanted Times without a slanted Times existing.
//!
//! An encoding file is a PostScript array of 256 glyph names, which is what
//! makes the codes in a `.dvi` mean something in a font that has never heard
//! of TeX.

use std::collections::BTreeMap;
use std::path::Path;

/// What a map says about one TeX font name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MapEntry {
    /// The name a `.dvi` uses.
    pub tex_name: String,
    /// The font's own PostScript name, when the line gives one.
    pub ps_name: Option<String>,
    /// The file the outlines are in: a `.pfb`, a `.ttf`, an `.otf`.
    pub font_file: Option<String>,
    /// The `.enc` the font is read through, when the line names one.
    pub encoding_file: Option<String>,
    /// How far to slant, as a fraction: `0.167 SlantFont` is an oblique made
    /// out of an upright.
    pub slant: f64,
    /// How far to stretch, as a multiple. One is the font as it is.
    pub extend: f64,
    /// The PostScript between the quotes, kept whole.
    pub snippet: Option<String>,
}

/// A font map.
#[derive(Debug, Clone, Default)]
pub struct FontMap {
    entries: BTreeMap<String, MapEntry>,
    /// Lines that could not be read, with what was wrong.
    pub warnings: Vec<String>,
}

impl FontMap {
    pub fn open(path: impl AsRef<Path>) -> Result<FontMap, String> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Ok(FontMap::parse(&text))
    }

    /// Read a map. A line that cannot be read is a warning, not a failure: a
    /// map is tens of thousands of lines assembled from every package
    /// installed, and one bad line is not a reason to lose the rest.
    pub fn parse(text: &str) -> FontMap {
        let mut map = FontMap::default();
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            // `%` is a comment, and so is a blank line. Some maps use `#`.
            if line.is_empty() || line.starts_with('%') || line.starts_with('#') {
                continue;
            }
            match parse_line(line) {
                Some(entry) => {
                    // A name defined twice keeps the first, as dvips does.
                    map.entries.entry(entry.tex_name.clone()).or_insert(entry);
                }
                None => map.warnings.push(format!("line {}: {line}", number + 1)),
            }
        }
        map
    }

    /// What the map says about a TeX font name.
    pub fn lookup(&self, tex_name: &str) -> Option<&MapEntry> {
        self.entries.get(tex_name)
    }

    /// Every name the map defines.
    pub fn names(&self) -> Vec<&str> {
        self.entries.keys().map(String::as_str).collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// A summary a person reads.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("fonts         {}\n", self.entries.len()));
        let encoded = self
            .entries
            .values()
            .filter(|e| e.encoding_file.is_some())
            .count();
        out.push_str(&format!("re-encoded    {encoded}\n"));
        let slanted = self.entries.values().filter(|e| e.slant != 0.0).count();
        let extended = self.entries.values().filter(|e| e.extend != 1.0).count();
        out.push_str(&format!("slanted       {slanted}\n"));
        out.push_str(&format!("extended      {extended}\n"));
        for warning in self.warnings.iter().take(10) {
            out.push_str(&format!("unreadable    {warning}\n"));
        }
        out
    }
}

/// One entry as a person reads it.
impl std::fmt::Display for MapEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "tex name      {}", self.tex_name)?;
        if let Some(name) = &self.ps_name {
            writeln!(f, "postscript    {name}")?;
        }
        if let Some(file) = &self.font_file {
            writeln!(f, "font file     {file}")?;
        }
        if let Some(file) = &self.encoding_file {
            writeln!(f, "encoding      {file}")?;
        }
        if self.slant != 0.0 {
            writeln!(f, "slant         {}", self.slant)?;
        }
        if self.extend != 1.0 {
            writeln!(f, "extend        {}", self.extend)?;
        }
        if let Some(snippet) = &self.snippet {
            writeln!(f, "instructions  \"{}\"", snippet.trim())?;
        }
        Ok(())
    }
}

/// One line of a map.
fn parse_line(line: &str) -> Option<MapEntry> {
    let mut entry = MapEntry {
        extend: 1.0,
        ..MapEntry::default()
    };
    let chars: Vec<char> = line.chars().collect();
    let mut at = 0usize;
    let word = |at: &mut usize| -> Option<String> {
        while chars.get(*at).is_some_and(|c| c.is_whitespace()) {
            *at += 1;
        }
        let start = *at;
        while chars.get(*at).is_some_and(|c| !c.is_whitespace()) {
            *at += 1;
        }
        (start < *at).then(|| chars[start..*at].iter().collect())
    };

    entry.tex_name = word(&mut at)?;
    if entry.tex_name.starts_with('<') || entry.tex_name.starts_with('"') {
        return None;
    }

    while at < chars.len() {
        while chars.get(at).is_some_and(|c| c.is_whitespace()) {
            at += 1;
        }
        match chars.get(at) {
            None => break,
            // A quoted run of PostScript, which may hold the slant and the
            // extend.
            Some('"') => {
                at += 1;
                let start = at;
                while chars.get(at).is_some_and(|&c| c != '"') {
                    at += 1;
                }
                let snippet: String = chars[start..at.min(chars.len())].iter().collect();
                at += 1;
                read_snippet(&snippet, &mut entry);
                entry.snippet = Some(snippet);
            }
            // A file. `<<` asks for the whole font rather than a subset, and
            // `<[` says the file is an encoding whatever it is called.
            Some('<') => {
                let mut name = word(&mut at)?;
                let mut is_encoding = false;
                while name.starts_with('<') || name.starts_with('[') {
                    is_encoding |= name.starts_with('[');
                    name.remove(0);
                }
                if name.is_empty() {
                    continue;
                }
                match is_encoding || name.ends_with(".enc") {
                    true => entry.encoding_file = Some(name),
                    false => entry.font_file = Some(name),
                }
            }
            // A bare word: the PostScript name if this is the first, and a
            // dvips option letter otherwise.
            _ => {
                let Some(text) = word(&mut at) else { break };
                if entry.ps_name.is_none() && entry.font_file.is_none() {
                    entry.ps_name = Some(text);
                }
            }
        }
    }
    Some(entry)
}

/// The two things worth reading out of the PostScript: `N SlantFont` and
/// `N ExtendFont`.
fn read_snippet(snippet: &str, entry: &mut MapEntry) {
    let words: Vec<&str> = snippet.split_whitespace().collect();
    for (i, word) in words.iter().enumerate() {
        let value = match i > 0 {
            true => words[i - 1].parse::<f64>().ok(),
            false => None,
        };
        match (*word, value) {
            ("SlantFont", Some(value)) => entry.slant = value,
            ("ExtendFont", Some(value)) => entry.extend = value,
            _ => {}
        }
    }
}

/// An encoding: 256 glyph names, in a PostScript array.
#[derive(Debug, Clone, PartialEq)]
pub struct Encoding {
    /// What the array is called: `/TeXBase1Encoding`, and so on.
    pub name: String,
    /// The names, by code. A code the encoding does not use is `.notdef`.
    pub glyphs: Vec<String>,
}

impl Encoding {
    pub fn open(path: impl AsRef<Path>) -> Result<Encoding, String> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Encoding::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Read `/Name [ /glyph /glyph … ] def`, with `%` comments.
    pub fn parse(text: &str) -> Result<Encoding, String> {
        // Comments run to the end of the line, and an encoding file is mostly
        // comments.
        let stripped: String = text
            .lines()
            .map(|line| match line.find('%') {
                Some(at) => &line[..at],
                None => line,
            })
            .collect::<Vec<_>>()
            .join("\n");

        let open = stripped
            .find('[')
            .ok_or("the file holds no array, so it is not an encoding")?;
        let close = stripped[open..]
            .find(']')
            .map(|at| open + at)
            .ok_or("the array is never closed")?;

        let name = stripped[..open]
            .split_whitespace()
            .next_back()
            .unwrap_or("")
            .trim_start_matches('/')
            .to_string();

        let glyphs: Vec<String> = stripped[open + 1..close]
            .split_whitespace()
            .filter(|word| word.starts_with('/'))
            .map(|word| word.trim_start_matches('/').to_string())
            .collect();
        if glyphs.is_empty() {
            return Err("the array holds no names".into());
        }
        if glyphs.len() > 256 {
            return Err(format!(
                "{} names is more than an encoding holds",
                glyphs.len()
            ));
        }
        Ok(Encoding { name, glyphs })
    }

    /// The glyph a code means, or `None` where the encoding has none.
    pub fn glyph(&self, code: u8) -> Option<&str> {
        match self.glyphs.get(code as usize).map(String::as_str) {
            Some(".notdef") | None => None,
            Some(name) => Some(name),
        }
    }

    /// How many codes the encoding really uses.
    pub fn used(&self) -> usize {
        (0..=255u8)
            .filter(|&code| self.glyph(code).is_some())
            .count()
    }

    /// A summary a person reads.
    pub fn summary(&self) -> String {
        let mut out = format!("encoding      {}\n", self.name);
        out.push_str(&format!("names         {}\n", self.glyphs.len()));
        out.push_str(&format!("used          {}\n", self.used()));
        for code in 0..=255u8 {
            if let Some(name) = self.glyph(code) {
                let shown = match (code as char).is_ascii_graphic() {
                    true => format!("'{}'", code as char),
                    false => format!("0o{code:o}"),
                };
                out.push_str(&format!("  {code:>3} {shown:<5} {name}\n"));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lines a real map is made of, and the ones that are easy to read
    /// wrongly.
    #[test]
    fn a_map_line_is_read_the_way_dvips_reads_one() {
        // The common shape: a TeX name, a PostScript name, a snippet, an
        // encoding and a font.
        let map = FontMap::parse(
            "ptmr8r NimbusRomNo9L-Regu \" TeXBase1Encoding ReEncodeFont \" <8r.enc <utmr8a.pfb\n",
        );
        let entry = map.lookup("ptmr8r").expect("the entry");
        assert_eq!(entry.ps_name.as_deref(), Some("NimbusRomNo9L-Regu"));
        assert_eq!(entry.encoding_file.as_deref(), Some("8r.enc"));
        assert_eq!(entry.font_file.as_deref(), Some("utmr8a.pfb"));
        assert_eq!(entry.slant, 0.0);
        assert_eq!(entry.extend, 1.0);

        // The shortest shape: a name and a file.
        let map = FontMap::parse("cmr10 CMR10 <cmr10.pfb\n");
        let entry = map.lookup("cmr10").expect("the entry");
        assert_eq!(entry.font_file.as_deref(), Some("cmr10.pfb"));
        assert_eq!(entry.encoding_file, None);

        // Slanting and extending, which is how a document gets an oblique
        // Times without an oblique Times existing.
        let map = FontMap::parse(
            "ptmro8r NimbusRomNo9L-Regu \" 0.167 SlantFont 1.2 ExtendFont \" <8r.enc <utmr8a.pfb\n",
        );
        let entry = map.lookup("ptmro8r").expect("the entry");
        assert_eq!(entry.slant, 0.167);
        assert_eq!(entry.extend, 1.2);

        // `<<` asks for the whole font rather than a subset, and `<[` says the
        // file is an encoding whatever it is called.
        let map = FontMap::parse("x Y <<full.pfb <[weird\n");
        let entry = map.lookup("x").expect("the entry");
        assert_eq!(entry.font_file.as_deref(), Some("full.pfb"));
        assert_eq!(entry.encoding_file.as_deref(), Some("weird"));

        // Comments and blank lines are not entries, and a name defined twice
        // keeps the first.
        let map = FontMap::parse("% a comment\n\na A <one.pfb\na B <two.pfb\n");
        assert_eq!(map.len(), 1);
        assert_eq!(
            map.lookup("a").unwrap().font_file.as_deref(),
            Some("one.pfb")
        );
    }

    /// An encoding is a PostScript array, and most of the file is comments.
    #[test]
    fn an_encoding_is_the_array_and_not_the_comments() {
        let encoding = Encoding::parse(
            "% a comment mentioning [ and /brackets\n\
             /TestEncoding [\n\
             /.notdef /A /B % two letters\n\
             /.notdef\n\
             ] def\n",
        )
        .expect("reads");
        assert_eq!(encoding.name, "TestEncoding");
        assert_eq!(encoding.glyphs.len(), 4);
        assert_eq!(encoding.glyph(1), Some("A"));
        assert_eq!(encoding.glyph(2), Some("B"));
        // `.notdef` is the absence of a glyph, not a glyph called `.notdef`.
        assert_eq!(encoding.glyph(0), None);
        assert_eq!(encoding.used(), 2);
        // A code past the end of a short array is simply not there.
        assert_eq!(encoding.glyph(200), None);

        // Only the names count: an array with a number or a bare word in it --
        // which an encoding built by a program can have -- must not turn that
        // into a glyph.
        let encoding = Encoding::parse("/E [ /a 0 /b readonly /c ] def").expect("reads");
        assert_eq!(encoding.glyphs, ["a", "b", "c"]);

        assert!(Encoding::parse("nothing here").is_err());
        assert!(Encoding::parse("/X [ ] def").is_err());
        assert!(Encoding::parse("/X [ /a").is_err());
    }
}
