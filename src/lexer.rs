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

/// Tokens read ahead of the expander, and the table revision they assume.
struct Ahead {
    /// Generation of the catcode table these were lexed under.
    generation: u32,
    /// Tokens with the character index each ended at, in source order.
    toks: Vec<crate::parallel::Placed>,
    /// How many have been handed out.
    at: usize,
}

pub struct Lexer {
    chars: Vec<char>,
    pos: usize,
    /// The last answer [`Lexer::line`] gave, as `(position, line)`.
    ///
    /// Counting newlines from the start of the file on every call is O(n) per
    /// token and so O(n²) per document — which the scaling benchmark caught:
    /// quadrupling the input cost 27x. Counting only what has been consumed
    /// SINCE the last answer makes the whole walk amortized O(n), and a rewind
    /// (the scanner pushes tokens back) simply recounts from the start.
    line_cache: std::cell::Cell<(usize, u32)>,
    /// Tokens lexed ahead of the expander, with the generation of the catcode
    /// table they were produced under.
    ///
    /// The mouth is the largest single cost of compiling a document and it is
    /// the one stage that parallelises: its states are per-line, so line-
    /// aligned slices lex independently. Reading ahead is a GUESS that the
    /// table will not move, and `\catcode` is allowed to move it at any point,
    /// so the guess carries the generation it was made under and is dropped the
    /// moment that stops matching. Dropping it costs only the work already
    /// done, never correctness.
    ahead: Option<Ahead>,
    /// Set once read-ahead has been abandoned often enough that it is not
    /// paying: a document rewriting catcodes throughout would otherwise re-lex
    /// its tail on every change.
    ahead_disabled: bool,
    /// How many times a stale read-ahead has been thrown away.
    ahead_misses: u32,
    /// Tokens to scan normally before speculating again.
    ///
    /// A document opens by setting catcodes -- `\catcode`\{=1` and friends --
    /// and each one invalidates whatever was read ahead. Speculating from the
    /// first token therefore lexes the whole file once per catcode in the
    /// preamble and throws all of it away. Waiting until the table has been
    /// still for a while costs a few hundred tokens of ordinary scanning and
    /// saves re-lexing the document.
    ahead_cooldown: usize,
    state: State,
    /// Pushed-back tokens (`\expandafter` and macro expansion feed these).
    pub pending: Vec<Token>,
}

impl Lexer {
    /// A mouth over an already-decoded slice of characters.
    ///
    /// The parallel pre-lexer hands each worker one line-aligned slice of the
    /// document's characters; decoding UTF-8 once for the whole file and
    /// slicing it is what keeps the split from paying for the decode N times.
    ///
    /// Read-ahead is OFF here, and that is load-bearing rather than an
    /// optimisation: this constructor is what the workers themselves run, so a
    /// worker that read ahead would fan out again, and again, each level
    /// re-chunking at different offsets. That nesting is what made the split
    /// slower than the scan it replaced AND made it disagree with the mouth --
    /// a worker's own sub-chunks split at boundaries the outer level never
    /// accounted for, so state that should have been `S` began as `N` and
    /// invented a `\par`.
    pub fn from_chars(chars: Vec<char>) -> Self {
        Self {
            chars,
            pos: 0,
            state: State::NewLine,
            pending: Vec::new(),
            line_cache: std::cell::Cell::new((0, 1)),
            ahead: None,
            ahead_disabled: true,
            ahead_misses: 0,
            ahead_cooldown: Self::AHEAD_COOLDOWN,
        }
    }

    /// How far the mouth has read, as an index into the characters.
    ///
    /// The pre-lexer records this after each token so a cached stream can be
    /// abandoned mid-way and scanning resumed at exactly the right character.
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// The document's characters, for a caller that wants to slice them.
    pub fn chars(&self) -> &[char] {
        &self.chars
    }

    pub fn new(src: &str) -> Self {
        Self {
            chars: src.chars().collect(),
            pos: 0,
            line_cache: std::cell::Cell::new((0, 1)),
            state: State::NewLine,
            pending: Vec::new(),
            ahead: None,
            ahead_disabled: false,
            ahead_misses: 0,
            ahead_cooldown: Self::AHEAD_COOLDOWN,
        }
    }

    /// Put a token stream back in front of the input, to be read next.
    ///
    /// Stored reversed so `next` can pop from the end — expansion pushes whole
    /// macro bodies here and they must come back out in order.
    pub fn push_back(&mut self, toks: &[Token]) {
        for t in toks.iter().rev() {
            self.pending.push(*t);
        }
    }

    /// The 1-based line the mouth has reached, for a diagnostic that has to
    /// point somewhere. Counted from the characters already consumed rather
    /// than tracked incrementally: the scanner rewinds and pushes back, and a
    /// counter that has to be maintained through both is a counter that drifts.
    pub fn line(&self) -> u32 {
        let pos = self.pos.min(self.chars.len());
        let (from, base) = match self.line_cache.get() {
            (cached_pos, line) if cached_pos <= pos => (cached_pos, line),
            // The scanner moved backwards: recount.
            _ => (0, 1),
        };
        let line = base + self.chars[from..pos].iter().filter(|c| **c == '\n').count() as u32;
        self.line_cache.set((pos, line));
        line
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

    /// Smallest tail worth handing to other cores.
    ///
    /// Below this the threads cost more than the scan they replace; the figure
    /// is a slice per core of a few tens of kilobytes, which is where the split
    /// starts to pay on the documents in `bench/`.
    const AHEAD_MIN_CHARS: usize = 65_536;

    /// How many times a stale read-ahead is tolerated before giving up on it.
    ///
    /// A document that rewrites catcodes throughout would otherwise re-lex its
    /// tail after every change, turning a speedup into quadratic work. Three
    /// strikes is enough to tell a preamble (which settles) from a document
    /// that keeps moving the table.
    const AHEAD_MAX_MISSES: u32 = 3;

    /// Tokens of quiet required before (re)building the read-ahead.
    const AHEAD_COOLDOWN: usize = 512;

    /// Serve the next token from the speculative read-ahead, filling it first
    /// if that is worthwhile.
    ///
    /// Returns `None` when there is nothing cached and nothing worth caching,
    /// in which case the caller scans as it always did.
    fn next_from_ahead(&mut self, cats: &CatTable) -> Option<Token> {
        if self.ahead_disabled {
            return None;
        }
        // OFF by default, and that is a measured decision rather than caution.
        //
        // Splitting the mouth across cores is correct (tests/parallel_lex.rs
        // pins it against the sequential mouth) but it LOSES: the sequential
        // mouth streams, never materialising a token list, while reading ahead
        // has to copy the characters per worker, build a token-and-position
        // vector for the whole tail, and concatenate the pieces. On a 5.6 MB
        // document that is tens of megabytes of traffic to save a scan worth
        // about 0.03s, and it measured 3x SLOWER end to end than not doing it.
        //
        // The lexer is simply not where a texrs run spends its time: it
        // compiles to bytecode, so the mouth is walked once and the VM does the
        // repeated work. Left in, off, and honest -- `TEXRS_PARALLEL=1` turns
        // it on for anyone measuring a document shaped differently.
        if std::env::var_os("TEXRS_PARALLEL").is_none() {
            self.ahead_disabled = true;
            return None;
        }
        // Anything cached under a superseded table is wrong from here on.
        if let Some(a) = &self.ahead {
            if a.generation != cats.generation() {
                self.ahead = None;
                crate::parallel::DROP_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                self.ahead_misses += 1;
                self.ahead_cooldown = Self::AHEAD_COOLDOWN;
                if self.ahead_misses >= Self::AHEAD_MAX_MISSES {
                    self.ahead_disabled = true;
                    return None;
                }
            }
        }
        if self.ahead.is_none() {
            if self.ahead_cooldown > 0 {
                self.ahead_cooldown -= 1;
                return None;
            }
            if self.chars.len().saturating_sub(self.pos) < Self::AHEAD_MIN_CHARS {
                return None;
            }
            let threads = crate::parallel::default_threads();
            if threads < 2 {
                return None;
            }
            let toks = crate::parallel::prelex(&self.chars, self.pos, cats, threads);
            if toks.is_empty() {
                return None;
            }
            self.ahead = Some(Ahead {
                generation: cats.generation(),
                toks,
                at: 0,
            });
        }
        let a = self.ahead.as_mut()?;
        let (tok, end) = *a.toks.get(a.at)?;
        a.at += 1;
        // Keep `pos` exact: `line()` reads it, and abandoning the cache has to
        // resume scanning at the character this token ended on.
        self.pos = end;
        if a.at == a.toks.len() {
            crate::parallel::DROP_EXHAUST.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.ahead = None;
        }
        Some(tok)
    }

    /// One token, or `None` at end of input.
    pub fn next_token(&mut self, cats: &CatTable) -> Option<Token> {
        if let Some(t) = self.pending.pop() {
            return Some(t);
        }
        if let Some(t) = self.next_from_ahead(cats) {
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
                        State::NewLine => Some(Token::cs("par")),
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
            return Token::cs("");
        };
        self.pos += 1;
        if cats.get(first) != Cat::Letter {
            // A control space (`\ `) leaves the state mid-line, like any other
            // single-character control sequence.
            self.state = match cats.get(first) == Cat::Space {
                true => State::SkipBlanks,
                false => State::MidLine,
            };
            return Token::cs(first.encode_utf8(&mut [0u8; 4]));
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
        Token::cs(&name)
    }
}
