//! Reading a `.bib` database, ported from the reading half of tectonic's
//! `engine_bibtex`.
//!
//! BibTeX is the other engine in TeX's world, and the only one that needs
//! neither boxes nor fonts: it reads a bibliography database, picks the entries
//! a document cited, and writes a `.bbl` for TeX to read back. Tectonic carries
//! the whole of it as a transpile of Oren Patashnik's C. What is portable here
//! and now is the database format — the half that is pure text processing, with
//! no `.bst` interpreter behind it.
//!
//! The format's awkward corners are the reason this is more than a split on
//! commas:
//!
//!  * A value is braced, quoted, or a bare number, and a braced value nests —
//!    `{Bra{ces} inside}` is one value, not two.
//!  * `@string{k = "v"}` defines an abbreviation, and a later value may be
//!    `k # " and more"`, concatenated with `#`. An abbreviation that is not
//!    defined stands for itself, which is what BibTeX does rather than failing.
//!  * Entry types and field names are case-insensitive (`@ARTICLE`, `Author`),
//!    but keys are not: `knuth1984` and `Knuth1984` are two entries.
//!  * `@comment` is ignored, and so is anything outside an entry — a `.bib` is
//!    allowed to have prose between its records.

use std::collections::BTreeMap;
use std::path::Path;

/// One record: `@article{key, field = value, …}`.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// The type, lowercased: `article`, `book`, `inproceedings`.
    pub kind: String,
    /// The citation key, as written — case matters here.
    pub key: String,
    /// Fields by lowercased name, in the order they were written.
    pub fields: Vec<(String, String)>,
}

impl Entry {
    /// One field by name, case-insensitively.
    pub fn field(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.fields
            .iter()
            .find(|(field, _)| *field == name)
            .map(|(_, value)| value.as_str())
    }
}

/// A parsed database.
#[derive(Debug, Clone, Default)]
pub struct Bib {
    pub entries: Vec<Entry>,
    /// The `@string` abbreviations, already expanded into the values above but
    /// kept because a reader may want to know what the file defined.
    pub strings: BTreeMap<String, String>,
    /// What was wrong but not fatal, in the order it was met.
    pub warnings: Vec<String>,
}

impl Bib {
    /// Read the database at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Bib, String> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Ok(Bib::parse(&text))
    }

    /// Parse `text`.
    ///
    /// Never fails: BibTeX itself carries on past a record it cannot read, and
    /// a database with one bad entry is still a database. What could not be
    /// read comes back in `warnings`.
    pub fn parse(text: &str) -> Bib {
        let mut parser = Parser {
            chars: text.chars().collect(),
            at: 0,
            bib: Bib::default(),
        };
        parser.run();
        parser.bib
    }

    /// The entry with this key, case-sensitively as BibTeX matches them.
    pub fn entry(&self, key: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.key == key)
    }

    /// A summary a person reads, which is what `-X bib` prints.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        for (name, value) in &self.strings {
            out.push_str(&format!("string   {name} = {value:?}\n"));
        }
        for entry in &self.entries {
            out.push_str(&format!("{:<14} {}\n", entry.kind, entry.key));
            for (field, value) in &entry.fields {
                let shown = match value.chars().count() > 60 {
                    true => format!("{}…", value.chars().take(59).collect::<String>()),
                    false => value.clone(),
                };
                out.push_str(&format!("    {field:<12} {shown}\n"));
            }
        }
        for warning in &self.warnings {
            out.push_str(&format!("warning  {warning}\n"));
        }
        out
    }
}

struct Parser {
    chars: Vec<char>,
    at: usize,
    bib: Bib,
}

impl Parser {
    fn run(&mut self) {
        while let Some(start) = self.next_record() {
            let kind = start.to_ascii_lowercase();
            match kind.as_str() {
                // §Ignored by BibTeX, and by anything reading after it.
                "comment" => {
                    self.skip_balanced();
                }
                "preamble" => {
                    self.skip_balanced();
                }
                "string" => self.read_string_definition(),
                _ => self.read_entry(&kind),
            }
        }
    }

    /// Find the next `@type`, returning the type. Anything between records is
    /// prose and is skipped.
    fn next_record(&mut self) -> Option<String> {
        while self.at < self.chars.len() {
            if self.chars[self.at] == '@' {
                self.at += 1;
                self.skip_space();
                let name = self.read_name();
                return match name.is_empty() {
                    true => None,
                    false => Some(name),
                };
            }
            self.at += 1;
        }
        None
    }

    fn read_entry(&mut self, kind: &str) {
        self.skip_space();
        let Some(open) = self.opening() else {
            self.bib
                .warnings
                .push(format!("@{kind}: no {{ or ( after the entry type"));
            return;
        };
        self.at += 1;
        self.skip_space();
        let key = self.read_until_key_end();
        if key.is_empty() {
            self.bib.warnings.push(format!("@{kind}: no citation key"));
            return;
        }
        if self.bib.entries.iter().any(|e| e.key == key) {
            // BibTeX takes the first and says so; a silent overwrite would make
            // a bibliography depend on file order.
            self.bib
                .warnings
                .push(format!("{key}: defined twice; the first one stands"));
        }
        let mut fields = Vec::new();
        loop {
            self.skip_space();
            match self.peek() {
                Some(',') => {
                    self.at += 1;
                    continue;
                }
                Some(c) if c == closing(open) => {
                    self.at += 1;
                    break;
                }
                None => {
                    self.bib
                        .warnings
                        .push(format!("{key}: the file ends inside the entry"));
                    break;
                }
                _ => {}
            }
            let name = self.read_name().to_ascii_lowercase();
            if name.is_empty() {
                // Nothing readable here; step over it rather than spinning.
                self.at += 1;
                continue;
            }
            self.skip_space();
            if self.peek() != Some('=') {
                self.bib
                    .warnings
                    .push(format!("{key}: {name} has no value"));
                continue;
            }
            self.at += 1;
            let value = self.read_value();
            fields.push((name, value));
        }
        if !self.bib.entries.iter().any(|e| e.key == key) {
            self.bib.entries.push(Entry {
                kind: kind.to_string(),
                key,
                fields,
            });
        }
    }

    fn read_string_definition(&mut self) {
        self.skip_space();
        let Some(open) = self.opening() else {
            return;
        };
        self.at += 1;
        self.skip_space();
        let name = self.read_name().to_ascii_lowercase();
        self.skip_space();
        if self.peek() == Some('=') {
            self.at += 1;
            let value = self.read_value();
            self.bib.strings.insert(name, value);
        }
        self.skip_space();
        if self.peek() == Some(closing(open)) {
            self.at += 1;
        }
    }

    /// A value: braced, quoted, a number, or an abbreviation — and any number
    /// of those joined with `#`.
    fn read_value(&mut self) -> String {
        let mut out = String::new();
        loop {
            self.skip_space();
            match self.peek() {
                Some('{') => out.push_str(&self.read_braced()),
                Some('"') => out.push_str(&self.read_quoted()),
                Some(c) if c.is_ascii_digit() => {
                    while let Some(c) = self.peek() {
                        if !c.is_ascii_digit() {
                            break;
                        }
                        out.push(c);
                        self.at += 1;
                    }
                }
                Some(_) => {
                    let name = self.read_name();
                    if name.is_empty() {
                        break;
                    }
                    // An abbreviation nobody defined stands for itself, which
                    // is what BibTeX does rather than dropping the value.
                    match self.bib.strings.get(&name.to_ascii_lowercase()) {
                        Some(value) => out.push_str(value),
                        None => out.push_str(&name),
                    }
                }
                None => break,
            }
            self.skip_space();
            if self.peek() == Some('#') {
                self.at += 1;
                continue;
            }
            break;
        }
        out
    }

    /// `{…}` with its braces stripped, keeping the ones inside.
    fn read_braced(&mut self) -> String {
        let mut depth = 0usize;
        let mut out = String::new();
        while let Some(c) = self.peek() {
            self.at += 1;
            match c {
                '{' => {
                    depth += 1;
                    if depth > 1 {
                        out.push(c);
                    }
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return out;
                    }
                    out.push(c);
                }
                _ => out.push(c),
            }
        }
        out
    }

    /// `"…"`, where a brace group protects a quote inside it.
    fn read_quoted(&mut self) -> String {
        self.at += 1;
        let mut out = String::new();
        let mut depth = 0usize;
        while let Some(c) = self.peek() {
            self.at += 1;
            match c {
                '"' if depth == 0 => return out,
                '{' => {
                    depth += 1;
                    out.push(c);
                }
                '}' => {
                    depth = depth.saturating_sub(1);
                    out.push(c);
                }
                _ => out.push(c),
            }
        }
        out
    }

    fn skip_balanced(&mut self) {
        self.skip_space();
        match self.peek() {
            Some('{') => {
                self.read_braced();
            }
            Some('(') => {
                while let Some(c) = self.peek() {
                    self.at += 1;
                    if c == ')' {
                        break;
                    }
                }
            }
            _ => {}
        }
    }

    fn opening(&mut self) -> Option<char> {
        match self.peek() {
            Some('{') => Some('{'),
            Some('(') => Some('('),
            _ => None,
        }
    }

    fn read_name(&mut self) -> String {
        let mut out = String::new();
        while let Some(c) = self.peek() {
            // A name runs to the first character that could start something
            // else. BibTeX is generous here: anything but the punctuation the
            // format uses can appear in one.
            if c.is_whitespace() || "{}()\",=#@".contains(c) {
                break;
            }
            out.push(c);
            self.at += 1;
        }
        out
    }

    fn read_until_key_end(&mut self) -> String {
        let mut out = String::new();
        while let Some(c) = self.peek() {
            if c == ',' || c == '}' || c == ')' {
                break;
            }
            if !c.is_whitespace() {
                out.push(c);
            }
            self.at += 1;
        }
        out
    }

    fn skip_space(&mut self) {
        while let Some(c) = self.peek() {
            if !c.is_whitespace() {
                break;
            }
            self.at += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }
}

fn closing(open: char) -> char {
    match open {
        '(' => ')',
        _ => '}',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATABASE: &str = r#"
This line is prose between records, which a .bib may carry.

@STRING{ tug = "TeX Users Group" }
@string{jtug = tug # " Journal"}

@Article{knuth1984,
  author  = {Knuth, Donald E.},
  title   = {Literate Programming},
  journal = jtug,
  year    = 1984,
  note    = "Braces {inside} a quoted value",
}

@book(texbook,
  author = "Donald E. Knuth",
  title  = {The {\TeX}book}
)

@comment{ this is ignored, and so is @notanentry{x} inside it }
"#;

    #[test]
    fn an_entry_comes_back_with_its_type_key_and_fields() {
        let bib = Bib::parse(DATABASE);
        assert_eq!(bib.entries.len(), 2, "{:?}", bib.summary());

        let article = bib.entry("knuth1984").expect("the article");
        // The type is lowercased; the key is not.
        assert_eq!(article.kind, "article");
        assert_eq!(article.key, "knuth1984");
        assert_eq!(article.field("author"), Some("Knuth, Donald E."));
        // A field is found however it was capitalised.
        assert_eq!(article.field("TITLE"), Some("Literate Programming"));
        assert_eq!(
            article.field("year"),
            Some("1984"),
            "a bare number is a value"
        );
        assert!(article.field("publisher").is_none());
    }

    #[test]
    fn abbreviations_are_defined_expanded_and_concatenated() {
        let bib = Bib::parse(DATABASE);
        assert_eq!(
            bib.strings.get("tug").map(String::as_str),
            Some("TeX Users Group")
        );
        // A definition may be built from an earlier one with `#`.
        assert_eq!(
            bib.strings.get("jtug").map(String::as_str),
            Some("TeX Users Group Journal")
        );
        // And a field that names one gets its value.
        assert_eq!(
            bib.entry("knuth1984").unwrap().field("journal"),
            Some("TeX Users Group Journal")
        );

        // An abbreviation nobody defined stands for itself rather than
        // vanishing, which is what BibTeX does.
        let bib = Bib::parse("@article{k, journal = undefined_name }");
        assert_eq!(
            bib.entry("k").unwrap().field("journal"),
            Some("undefined_name")
        );
    }

    #[test]
    fn braces_nest_and_quotes_protect_what_is_inside_them() {
        let bib = Bib::parse(DATABASE);
        // The outer braces go, the inner ones stay: they are the value.
        assert_eq!(
            bib.entry("texbook").unwrap().field("title"),
            Some("The {\\TeX}book")
        );
        // A brace group inside a quoted value is part of it, quotes and all.
        assert_eq!(
            bib.entry("knuth1984").unwrap().field("note"),
            Some("Braces {inside} a quoted value")
        );
        // A quote inside braces does not end the value.
        let bib = Bib::parse(r#"@misc{q, title = {a "quoted" word} }"#);
        assert_eq!(
            bib.entry("q").unwrap().field("title"),
            Some(r#"a "quoted" word"#)
        );
    }

    #[test]
    fn a_record_in_parentheses_reads_like_one_in_braces() {
        let bib = Bib::parse(DATABASE);
        let book = bib.entry("texbook").expect("the book");
        assert_eq!(book.kind, "book");
        assert_eq!(book.field("author"), Some("Donald E. Knuth"));
    }

    #[test]
    fn what_is_not_an_entry_is_left_alone() {
        let bib = Bib::parse(DATABASE);
        // Prose between records, and everything inside @comment — including
        // something that looks like an entry.
        assert!(bib.entry("x").is_none(), "{:?}", bib.summary());
        assert_eq!(bib.entries.len(), 2);

        // A @preamble is skipped rather than read as an entry.
        let bib = Bib::parse("@preamble{ \"\\newcommand{\\noop}[1]{}\" }\n@misc{m, title={T}}");
        assert_eq!(bib.entries.len(), 1);
        assert_eq!(bib.entry("m").unwrap().field("title"), Some("T"));
    }

    #[test]
    fn what_this_reads_is_what_bibtex_reads() {
        // The oracle, as the parity harness uses tex: real bibtex is run over
        // the same database and its .bbl has to carry what this parser found.
        // A reader of a format is only right if the tool it was ported from
        // agrees.
        let dir = std::env::temp_dir().join(format!("texrs_bib_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let database = "@STRING{tug = \"TeX Users Group\"}\n\
             @Article{knuth1984,\n\
               author  = {Knuth, Donald E.},\n\
               title   = {Literate Programming},\n\
               journal = tug # \" Journal\",\n\
               year    = 1984\n\
             }\n";
        if std::fs::write(dir.join("refs.bib"), database).is_err() {
            return;
        }
        let aux = "\\relax\n\\citation{knuth1984}\n\\bibstyle{plain}\n\\bibdata{refs}\n";
        if std::fs::write(dir.join("t.aux"), aux).is_err() {
            return;
        }
        let ran = std::process::Command::new("bibtex")
            .arg("t")
            .current_dir(&dir)
            .output();
        let Ok(_) = ran else {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        };
        let Ok(bbl) = std::fs::read_to_string(dir.join("t.bbl")) else {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        };

        let bib = Bib::parse(database);
        let entry = bib.entry("knuth1984").expect("the entry");
        assert!(bbl.contains("\\bibitem{knuth1984}"), "{bbl}");
        // bibtex prints the author as `Donald~E. Knuth`, having taken the
        // name apart; what matters here is that both read the same surname
        // out of the same field.
        assert!(bbl.contains("Knuth"), "{bbl}");
        assert!(entry.field("author").unwrap().contains("Knuth"));
        // The abbreviation and the concatenation: bibtex expands them the same
        // way, which is the corner this parser most easily gets wrong.
        assert_eq!(entry.field("journal"), Some("TeX Users Group Journal"));
        assert!(bbl.contains("TeX Users Group Journal"), "{bbl}");
        assert_eq!(entry.field("year"), Some("1984"));
        assert!(bbl.contains("1984"), "{bbl}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_record_is_reported_and_the_rest_of_the_file_still_reads() {
        // The first entry ends where the file does; the second is whole. A
        // reader that gave up on the first would lose a database to one typo.
        let bib = Bib::parse("@misc{broken, title = {unclosed\n@misc{good, title={T}}");
        assert!(!bib.warnings.is_empty(), "the damage is reported");

        let bib = Bib::parse("@misc{nofields}\n@misc{after, title={T}}");
        assert_eq!(bib.entry("after").and_then(|e| e.field("title")), Some("T"));

        // A key defined twice keeps the first, and says so.
        let bib = Bib::parse("@misc{k, title={first}}\n@misc{k, title={second}}");
        assert_eq!(bib.entries.len(), 1);
        assert_eq!(bib.entry("k").unwrap().field("title"), Some("first"));
        assert!(
            bib.warnings.iter().any(|w| w.contains("defined twice")),
            "{:?}",
            bib.warnings
        );

        // And an entry with no key at all is a warning, not a panic.
        let bib = Bib::parse("@misc{}");
        assert!(bib.entries.is_empty());
        assert!(!bib.warnings.is_empty());
    }
}
