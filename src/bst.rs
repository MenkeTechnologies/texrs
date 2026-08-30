//! Reading a `.bst` style, ported from the front of tectonic's `engine_bibtex`.
//!
//! A `.bst` is a program in Patashnik's stack language: it declares the fields
//! an entry may have, the variables it keeps, and the functions that turn one
//! into a line of a bibliography, and then says what to do — `READ`, `SORT`,
//! `ITERATE {call.type$}`. bibtex is the interpreter for it.
//!
//! This is the reader. It takes a style apart into its commands and its
//! function bodies, and answers the question that costs a user an afternoon
//! otherwise: does this style call something nothing defines? bibtex reports
//! that at run time, one undefined name per run, in the middle of a build.
//!
//! The interpreter is the next piece and is deliberately not here. It needs one
//! thing texrs has no way to provide yet: `width$` measures a string in the
//! current font, which means reading a `.tfm`. A style that never calls it —
//! most do, through `format.lab.names` — would run without, but an interpreter
//! that was right for some styles and silently wrong for others is worse than
//! none.

use std::collections::BTreeSet;
use std::path::Path;

/// One token of the language.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// `#12` — an integer literal.
    Integer(i64),
    /// `"text"` — a string literal.
    String(String),
    /// `'name` — a function pushed rather than called.
    Quoted(String),
    /// `{ … }` — a block, which is a function without a name.
    Block(Vec<Token>),
    /// Anything else: a call, a field, a variable.
    Name(String),
}

/// A command at the top of a style.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// `ENTRY {fields} {integers} {strings}`.
    Entry {
        fields: Vec<String>,
        integers: Vec<String>,
        strings: Vec<String>,
    },
    Integers(Vec<String>),
    Strings(Vec<String>),
    /// `MACRO {name} {"value"}` — the month and journal abbreviations.
    Macro {
        name: String,
        value: String,
    },
    Function {
        name: String,
        body: Vec<Token>,
    },
    Read,
    Sort,
    Execute(Vec<Token>),
    Iterate(Vec<Token>),
    Reverse(Vec<Token>),
}

/// A parsed style.
#[derive(Debug, Clone, Default)]
pub struct Style {
    pub commands: Vec<Command>,
    pub warnings: Vec<String>,
}

impl Style {
    pub fn open(path: impl AsRef<Path>) -> Result<Style, String> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Ok(Style::parse(&text))
    }

    /// Parse `text`. Never fails: what could not be read is a warning, as it is
    /// for a database.
    pub fn parse(text: &str) -> Style {
        let tokens = tokenize(text);
        let mut style = Style::default();
        let mut at = 0usize;
        while at < tokens.len() {
            let Token::Name(word) = &tokens[at] else {
                style
                    .warnings
                    .push(format!("{:?} is not a command", tokens[at]));
                at += 1;
                continue;
            };
            let command = word.to_ascii_lowercase();
            at += 1;
            match command.as_str() {
                "entry" => {
                    let fields = take_names(&tokens, &mut at);
                    let integers = take_names(&tokens, &mut at);
                    let strings = take_names(&tokens, &mut at);
                    style.commands.push(Command::Entry {
                        fields,
                        integers,
                        strings,
                    });
                }
                "integers" => style
                    .commands
                    .push(Command::Integers(take_names(&tokens, &mut at))),
                "strings" => style
                    .commands
                    .push(Command::Strings(take_names(&tokens, &mut at))),
                "macro" => {
                    let name = take_names(&tokens, &mut at).first().cloned();
                    let value = match tokens.get(at) {
                        Some(Token::Block(body)) => {
                            at += 1;
                            match body.first() {
                                Some(Token::String(text)) => text.clone(),
                                _ => String::new(),
                            }
                        }
                        _ => String::new(),
                    };
                    match name {
                        Some(name) => style.commands.push(Command::Macro { name, value }),
                        None => style.warnings.push("MACRO with no name".into()),
                    }
                }
                "function" => {
                    let name = take_names(&tokens, &mut at).first().cloned();
                    let body = match tokens.get(at) {
                        Some(Token::Block(body)) => {
                            at += 1;
                            body.clone()
                        }
                        _ => Vec::new(),
                    };
                    match name {
                        Some(name) => style.commands.push(Command::Function { name, body }),
                        None => style.warnings.push("FUNCTION with no name".into()),
                    }
                }
                "read" => style.commands.push(Command::Read),
                "sort" => style.commands.push(Command::Sort),
                "execute" | "iterate" | "reverse" => {
                    let body = match tokens.get(at) {
                        Some(Token::Block(body)) => {
                            at += 1;
                            body.clone()
                        }
                        _ => Vec::new(),
                    };
                    style.commands.push(match command.as_str() {
                        "execute" => Command::Execute(body),
                        "iterate" => Command::Iterate(body),
                        _ => Command::Reverse(body),
                    });
                }
                other => style.warnings.push(format!("{other} is not a command")),
            }
        }
        style
    }

    /// Every name the style defines: its functions, variables, fields and
    /// macros.
    pub fn defined(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for command in &self.commands {
            match command {
                Command::Entry {
                    fields,
                    integers,
                    strings,
                } => {
                    out.extend(fields.iter().cloned());
                    out.extend(integers.iter().cloned());
                    out.extend(strings.iter().cloned());
                }
                Command::Integers(names) | Command::Strings(names) => {
                    out.extend(names.iter().cloned())
                }
                Command::Macro { name, .. } => {
                    out.insert(name.clone());
                }
                Command::Function { name, .. } => {
                    out.insert(name.clone());
                }
                _ => {}
            }
        }
        out
    }

    /// Names the style calls that nothing defines and that are not builtins.
    ///
    /// bibtex reports these one per run, in the middle of a build; a style can
    /// be read for them in one pass instead.
    pub fn undefined(&self) -> Vec<String> {
        let defined = self.defined();
        let mut out = BTreeSet::new();
        let mut look = |body: &[Token]| {
            for name in called(body) {
                if !defined.contains(&name) && !BUILTINS.contains(&name.as_str()) {
                    out.insert(name);
                }
            }
        };
        for command in &self.commands {
            match command {
                Command::Function { body, .. }
                | Command::Execute(body)
                | Command::Iterate(body)
                | Command::Reverse(body) => look(body),
                _ => {}
            }
        }
        out.into_iter().collect()
    }

    /// The fields an entry may carry, as `ENTRY` declares them.
    pub fn fields(&self) -> Vec<String> {
        self.commands
            .iter()
            .find_map(|command| match command {
                Command::Entry { fields, .. } => Some(fields.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// The `MACRO` abbreviations, which are what a database's month names mean.
    pub fn macros(&self) -> Vec<(String, String)> {
        self.commands
            .iter()
            .filter_map(|command| match command {
                Command::Macro { name, value } => Some((name.clone(), value.clone())),
                _ => None,
            })
            .collect()
    }

    /// A summary a person reads.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        let fields = self.fields();
        if !fields.is_empty() {
            out.push_str(&format!("fields     {}\n", fields.join(" ")));
        }
        let macros = self.macros();
        if !macros.is_empty() {
            let names: Vec<&str> = macros.iter().map(|(n, _)| n.as_str()).collect();
            out.push_str(&format!("macros     {}\n", names.join(" ")));
        }
        let functions: Vec<&str> = self
            .commands
            .iter()
            .filter_map(|c| match c {
                Command::Function { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        out.push_str(&format!("functions  {}\n", functions.len()));
        for command in &self.commands {
            match command {
                Command::Read => out.push_str("READ\n"),
                Command::Sort => out.push_str("SORT\n"),
                Command::Execute(body) => {
                    out.push_str(&format!("EXECUTE    {}\n", called(body).join(" ")))
                }
                Command::Iterate(body) => {
                    out.push_str(&format!("ITERATE    {}\n", called(body).join(" ")))
                }
                Command::Reverse(body) => {
                    out.push_str(&format!("REVERSE    {}\n", called(body).join(" ")))
                }
                _ => {}
            }
        }
        for name in self.undefined() {
            out.push_str(&format!("UNDEFINED  {name}\n"));
        }
        for warning in &self.warnings {
            out.push_str(&format!("warning    {warning}\n"));
        }
        out
    }
}

/// The names called in a body, including inside its blocks and its `'quoted`
/// references — a function pushed by name is still a function used.
fn called(body: &[Token]) -> Vec<String> {
    let mut out = Vec::new();
    for token in body {
        match token {
            Token::Name(name) => out.push(name.clone()),
            Token::Quoted(name) => out.push(name.clone()),
            Token::Block(inner) => out.extend(called(inner)),
            _ => {}
        }
    }
    out
}

/// The next block's names, for `ENTRY`, `INTEGERS`, `FUNCTION` and friends.
fn take_names(tokens: &[Token], at: &mut usize) -> Vec<String> {
    match tokens.get(*at) {
        Some(Token::Block(body)) => {
            *at += 1;
            body.iter()
                .filter_map(|token| match token {
                    Token::Name(name) => Some(name.clone()),
                    _ => None,
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Split `text` into tokens, blocks nested.
fn tokenize(text: &str) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let mut at = 0usize;
    let (tokens, _) = tokenize_block(&chars, &mut at, false);
    tokens
}

fn tokenize_block(chars: &[char], at: &mut usize, nested: bool) -> (Vec<Token>, bool) {
    let mut out = Vec::new();
    while *at < chars.len() {
        let c = chars[*at];
        match c {
            // A comment runs to the end of the line, as it does in TeX.
            '%' => {
                while *at < chars.len() && chars[*at] != '\n' {
                    *at += 1;
                }
            }
            c if c.is_whitespace() => *at += 1,
            '{' => {
                *at += 1;
                let (body, _) = tokenize_block(chars, at, true);
                out.push(Token::Block(body));
            }
            '}' => {
                *at += 1;
                return (out, true);
            }
            '"' => {
                *at += 1;
                let mut text = String::new();
                while *at < chars.len() && chars[*at] != '"' {
                    text.push(chars[*at]);
                    *at += 1;
                }
                *at += 1;
                out.push(Token::String(text));
            }
            '#' => {
                *at += 1;
                let mut digits = String::new();
                if chars.get(*at) == Some(&'-') {
                    digits.push('-');
                    *at += 1;
                }
                while *at < chars.len() && chars[*at].is_ascii_digit() {
                    digits.push(chars[*at]);
                    *at += 1;
                }
                out.push(Token::Integer(digits.parse().unwrap_or(0)));
            }
            '\'' => {
                *at += 1;
                out.push(Token::Quoted(read_word(chars, at)));
            }
            _ => {
                let word = read_word(chars, at);
                if word.is_empty() {
                    *at += 1;
                    continue;
                }
                out.push(Token::Name(word));
            }
        }
    }
    (out, nested)
}

/// A word runs to whitespace or to one of the characters that start something
/// else. `$` is part of a name — every builtin ends in one.
fn read_word(chars: &[char], at: &mut usize) -> String {
    let mut out = String::new();
    while *at < chars.len() {
        let c = chars[*at];
        if c.is_whitespace() || "{}\"%'#".contains(c) {
            break;
        }
        out.push(c);
        *at += 1;
    }
    out
}

/// The builtins of the language, as `bibtex.web` §106 lists them.
pub const BUILTINS: &[&str] = &[
    ">",
    "<",
    "=",
    "+",
    "-",
    "*",
    ":=",
    "add.period$",
    "call.type$",
    "change.case$",
    "chr.to.int$",
    "cite$",
    "duplicate$",
    "empty$",
    "format.name$",
    "if$",
    "int.to.chr$",
    "int.to.str$",
    "missing$",
    "newline$",
    "num.names$",
    "pop$",
    "preamble$",
    "purify$",
    "quote$",
    "skip$",
    "stack$",
    "substring$",
    "swap$",
    "text.length$",
    "text.prefix$",
    "top$",
    "type$",
    "warning$",
    "while$",
    "width$",
    "write$",
    "entry.max$",
    "global.max$",
    "sort.key$",
    "crossref",
    "entry.max",
    "global.max",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// A style from TeX Live, or `None` when there is none here.
    fn installed(name: &str) -> Option<String> {
        let found = std::process::Command::new("kpsewhich")
            .arg(name)
            .output()
            .ok()?;
        let path = String::from_utf8_lossy(&found.stdout).trim().to_string();
        std::fs::read_to_string(path).ok()
    }

    #[test]
    fn the_pieces_of_the_language_are_told_apart() {
        let style = Style::parse(
            "% a comment\n\
             ENTRY { author title } { } { label }\n\
             INTEGERS { state }\n\
             MACRO {jan} {\"January\"}\n\
             FUNCTION {greet} { #1 \"text\" 'other if$ }\n\
             READ\n SORT\n EXECUTE {greet}\n ITERATE {call.type$}\n",
        );

        assert_eq!(style.fields(), vec!["author", "title"]);
        assert_eq!(
            style.macros(),
            vec![("jan".to_string(), "January".to_string())]
        );
        assert!(
            style.defined().contains("state"),
            "an INTEGERS name is defined"
        );
        assert!(style.defined().contains("label"), "so is an ENTRY string");

        // The body keeps its literals apart from its calls.
        let body = style.commands.iter().find_map(|c| match c {
            Command::Function { name, body } if name == "greet" => Some(body.clone()),
            _ => None,
        });
        let body = body.expect("the function");
        assert_eq!(body[0], Token::Integer(1));
        assert_eq!(body[1], Token::String("text".into()));
        assert_eq!(body[2], Token::Quoted("other".into()));
        assert_eq!(body[3], Token::Name("if$".into()));

        // The commands that say what to do, in order.
        assert!(style.commands.contains(&Command::Read));
        assert!(style.commands.contains(&Command::Sort));
    }

    #[test]
    fn a_name_nothing_defines_is_reported_without_running_the_style() {
        let style = Style::parse(
            "FUNCTION {a} { b call.type$ }\n\
             FUNCTION {c} { 'nosuchthing if$ }\n\
             FUNCTION {b} { skip$ }\n",
        );
        // `b` is defined after it is called, which is legal and common; `if$`
        // and `call.type$` are builtins; the one real gap is reported.
        assert_eq!(style.undefined(), vec!["nosuchthing"]);
        assert!(style.summary().contains("UNDEFINED  nosuchthing"));

        // A function pushed by name is still a use, so a quoted name that is
        // defined is not reported.
        let style = Style::parse("FUNCTION {x} { skip$ }\nFUNCTION {y} { 'x while$ }\n");
        assert!(style.undefined().is_empty(), "{:?}", style.undefined());
    }

    #[test]
    fn the_standard_styles_read_without_a_gap() {
        // The test that matters: four styles shipped with TeX Live, read whole.
        // A reader that mistook one construct would report a name nothing
        // defines in a file that bibtex runs every day.
        for name in ["plain.bst", "unsrt.bst", "abbrv.bst", "alpha.bst"] {
            let Some(text) = installed(name) else {
                return;
            };
            let style = Style::parse(&text);
            assert!(style.warnings.is_empty(), "{name}: {:?}", style.warnings);
            assert!(
                style.undefined().is_empty(),
                "{name} calls {:?}",
                style.undefined()
            );
            // And it really read the thing: plain.bst declares its fields, and
            // defines the functions that build an entry.
            assert!(style.fields().contains(&"author".to_string()), "{name}");
            assert!(
                style.defined().contains("format.names"),
                "{name} defines the name formatter"
            );
            assert!(
                style.commands.iter().any(|c| matches!(c, Command::Read)),
                "{name} reads the database"
            );
        }
    }

    #[test]
    fn a_style_that_is_not_one_is_reported_rather_than_guessed_at() {
        let style = Style::parse("this is not a style at all");
        assert!(!style.warnings.is_empty());
        assert!(style.commands.is_empty());

        // A FUNCTION with no name is a warning, not a panic.
        let style = Style::parse("FUNCTION");
        assert!(!style.warnings.is_empty(), "{:?}", style.summary());
    }
}
