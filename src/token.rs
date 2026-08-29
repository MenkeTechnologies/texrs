//! What the mouth produces: TeX has exactly two kinds of token.

use crate::catcode::Cat;

/// A token, as `tex.web` §289 defines one.
///
/// A character token carries the catcode it had WHEN IT WAS READ, not the
/// current one — that is why `\catcode`\{=1` after a macro was defined does not
/// retroactively change that macro's body. Keeping the pair together is what
/// makes the distinction representable at all.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Token {
    Char(char, Cat),
    /// A control sequence, stored WITHOUT its escape character.
    Cs(String),
}

impl Token {
    /// The text `\string` and `\message` produce for this token.
    ///
    /// A multi-letter control sequence prints with a trailing space and a
    /// single-character one does not (`tex.web` §294's `print_cs`) — the rule
    /// that makes `\message{\foo}` read `\foo ` and `\message{\!}` read `\!`.
    pub fn to_text(&self, escape: char) -> String {
        match self {
            Token::Char(c, _) => c.to_string(),
            Token::Cs(name) => {
                let single = name.chars().count() == 1;
                match single {
                    true => format!("{escape}{name}"),
                    false => format!("{escape}{name} "),
                }
            }
        }
    }

    pub fn is_space(&self) -> bool {
        matches!(self, Token::Char(_, Cat::Space))
    }
}
