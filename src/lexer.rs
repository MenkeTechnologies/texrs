//! TeX's mouth: bytes to tokens.
//!
//! The mouth is a three-state machine per line (`tex.web` §303): `N` at the
//! start of a line, `M` in the middle, `S` while skipping blanks. The states are
//! not decoration — they are what makes a blank line a `\par`, what collapses a
//! run of spaces to one, and what makes a space after a control word vanish.
//! Getting them wrong changes what documents mean, so they are modelled
//! directly rather than approximated with trimming.

use crate::catcode::{Cat, CatTable};
use crate::token::Token;

#[derive(Clone, Copy, PartialEq)]
enum State {
    NewLine,
    MidLine,
    SkipBlanks,
}

pub struct Lexer {
    chars: Vec<char>,
    pos: usize,
    state: State,
    /// Pushed-back tokens (`\expandafter` and macro expansion feed these).
    pub pending: Vec<Token>,
}

impl Lexer {
    pub fn new(src: &str) -> Self {
        Self {
            chars: src.chars().collect(),
            pos: 0,
            state: State::NewLine,
            pending: Vec::new(),
        }
    }

    /// Put a token stream back in front of the input, to be read next.
    ///
    /// Stored reversed so `next` can pop from the end — expansion pushes whole
    /// macro bodies here and they must come back out in order.
    pub fn push_back(&mut self, toks: &[Token]) {
        for t in toks.iter().rev() {
            self.pending.push(t.clone());
        }
    }

    /// The 1-based line the mouth has reached, for a diagnostic that has to
    /// point somewhere. Counted from the characters already consumed rather
    /// than tracked incrementally: the scanner rewinds and pushes back, and a
    /// counter that has to be maintained through both is a counter that drifts.
    pub fn line(&self) -> u32 {
        1 + self.chars[..self.pos.min(self.chars.len())]
            .iter()
            .filter(|c| **c == '\n')
            .count() as u32
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    /// `^^X` notation (`tex.web` §352): a superscript character twice, then one
    /// character, denotes a control character. `^^M` is a carriage return, which
    /// is how a line end gets written inside a macro body.
    fn double_superscript(&mut self, cats: &CatTable, c: char) -> Option<char> {
        if cats.get(c) != Cat::Superscript {
            return None;
        }
        if self.chars.get(self.pos + 1).copied() != Some(c) {
            return None;
        }
        let third = self.chars.get(self.pos + 2).copied()?;
        let code = u32::from(third);
        if code >= 128 {
            return None;
        }
        self.pos += 3;
        let shifted = match code < 64 {
            true => code + 64,
            false => code - 64,
        };
        char::from_u32(shifted)
    }

    /// One token, or `None` at end of input.
    pub fn next_token(&mut self, cats: &CatTable) -> Option<Token> {
        if let Some(t) = self.pending.pop() {
            return Some(t);
        }
        loop {
            let c = self.peek()?;
            if let Some(decoded) = self.double_superscript(cats, c) {
                // Re-read the decoded character in place of the `^^X` triple.
                self.chars.splice(self.pos..self.pos, [decoded]);
                continue;
            }
            let cat = cats.get(c);
            self.pos += 1;
            match cat {
                Cat::Escape => return Some(self.control_sequence(cats)),
                Cat::Comment => {
                    // The comment and the line end it eats produce nothing, and
                    // the next line starts in state N.
                    while let Some(ch) = self.peek() {
                        self.pos += 1;
                        if cats.get(ch) == Cat::EndLine {
                            break;
                        }
                    }
                    self.state = State::NewLine;
                }
                Cat::EndLine => {
                    // §304: a line end is a space in state M, a `\par` in state
                    // N (the blank-line rule), and nothing while skipping blanks.
                    let out = match self.state {
                        State::MidLine => Some(Token::Char(' ', Cat::Space)),
                        State::NewLine => Some(Token::Cs("par".to_string())),
                        State::SkipBlanks => None,
                    };
                    self.state = State::NewLine;
                    if out.is_some() {
                        return out;
                    }
                }
                Cat::Space => {
                    // A run of spaces is one space, and only in state M.
                    if self.state == State::MidLine {
                        self.state = State::SkipBlanks;
                        return Some(Token::Char(' ', Cat::Space));
                    }
                }
                Cat::Ignored => {}
                _ => {
                    self.state = State::MidLine;
                    return Some(Token::Char(c, cat));
                }
            }
        }
    }

    /// The escape character has been consumed; read the control sequence.
    ///
    /// A control WORD is a maximal run of letters and swallows the spaces after
    /// it (state S); a control SYMBOL is exactly one character and does not.
    /// That asymmetry is why `\foo bar` loses its space and `\! bar` keeps it.
    fn control_sequence(&mut self, cats: &CatTable) -> Token {
        let Some(first) = self.peek() else {
            return Token::Cs(String::new());
        };
        self.pos += 1;
        if cats.get(first) != Cat::Letter {
            // A control space (`\ `) leaves the state mid-line, like any other
            // single-character control sequence.
            self.state = match cats.get(first) == Cat::Space {
                true => State::SkipBlanks,
                false => State::MidLine,
            };
            return Token::Cs(first.to_string());
        }
        let mut name = String::from(first);
        while let Some(c) = self.peek() {
            if cats.get(c) != Cat::Letter {
                break;
            }
            name.push(c);
            self.pos += 1;
        }
        self.state = State::SkipBlanks;
        Token::Cs(name)
    }
}
