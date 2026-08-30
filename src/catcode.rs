//! TeX's category codes — the sixteen classes every character belongs to.
//!
//! `tex.web` §207 fixes the numbering, and the classification is what makes TeX
//! reconfigurable at the character level: `\catcode`\@=11` is how plain.tex makes
//! `@` a letter for the duration of a macro file. Nothing here decides what a
//! character MEANS; that is the mouth's job (`crate::lexer`). This only says
//! which of the sixteen classes it is in right now.

/// A category code, numbered as `tex.web` numbers them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Cat {
    Escape = 0,
    BeginGroup = 1,
    EndGroup = 2,
    MathShift = 3,
    AlignTab = 4,
    EndLine = 5,
    Param = 6,
    Superscript = 7,
    Subscript = 8,
    Ignored = 9,
    Space = 10,
    Letter = 11,
    Other = 12,
    Active = 13,
    Comment = 14,
    Invalid = 15,
}

impl Cat {
    /// Every category, in `tex.web`'s numbering order.
    pub const ALL: [Cat; 16] = [
        Cat::Escape,
        Cat::BeginGroup,
        Cat::EndGroup,
        Cat::MathShift,
        Cat::AlignTab,
        Cat::EndLine,
        Cat::Param,
        Cat::Superscript,
        Cat::Subscript,
        Cat::Ignored,
        Cat::Space,
        Cat::Letter,
        Cat::Other,
        Cat::Active,
        Cat::Comment,
        Cat::Invalid,
    ];

    /// The name `tex.web` gives this category, for a reference table or a
    /// diagnostic. Not the Rust variant name: `tex.web` writes "end of line",
    /// not "EndLine".
    pub fn name(self) -> &'static str {
        match self {
            Cat::Escape => "escape",
            Cat::BeginGroup => "begin group",
            Cat::EndGroup => "end group",
            Cat::MathShift => "math shift",
            Cat::AlignTab => "alignment tab",
            Cat::EndLine => "end of line",
            Cat::Param => "parameter",
            Cat::Superscript => "superscript",
            Cat::Subscript => "subscript",
            Cat::Ignored => "ignored",
            Cat::Space => "space",
            Cat::Letter => "letter",
            Cat::Other => "other",
            Cat::Active => "active",
            Cat::Comment => "comment",
            Cat::Invalid => "invalid",
        }
    }
}

/// The catcode of every character, mutable exactly as `\catcode` makes it.
///
/// INITEX's defaults (`tex.web` §232) are deliberately sparse: only `\`, `%`,
/// the letters, the space, the carriage return and the null/delete characters
/// are special. `{`, `}`, `$`, `&`, `#`, `^`, `_` and `~` get their familiar
/// meanings from plain.tex, NOT from the engine — which is why a bare INITEX
/// run treats `{` as an ordinary character. Reproducing that split matters: a
/// document that sets its own catcodes has to see the same starting point.
pub struct CatTable {
    codes: [Cat; 256],
    /// Bumped by every `set`.
    ///
    /// Lexing ahead of the expander is only valid while the table it was done
    /// under still holds. A counter is what lets a cached token stream say "I
    /// was produced under generation N" and be thrown away the moment
    /// `\catcode` moves the table on, without comparing 256 bytes each time.
    generation: u32,
}

impl Default for CatTable {
    fn default() -> Self {
        Self::new()
    }
}

impl CatTable {
    /// INITEX's table, before any format or macro file has run.
    pub fn new() -> Self {
        let mut codes = [Cat::Other; 256];
        for c in b'A'..=b'Z' {
            codes[c as usize] = Cat::Letter;
        }
        for c in b'a'..=b'z' {
            codes[c as usize] = Cat::Letter;
        }
        codes[b'\\' as usize] = Cat::Escape;
        codes[b'%' as usize] = Cat::Comment;
        codes[b'\r' as usize] = Cat::EndLine;
        codes[b'\n' as usize] = Cat::EndLine;
        codes[b' ' as usize] = Cat::Space;
        codes[0] = Cat::Ignored;
        codes[127] = Cat::Invalid;
        Self {
            codes,
            generation: 0,
        }
    }

    /// Which revision of the table this is; see [`CatTable::generation`]'s field.
    pub fn generation(&self) -> u32 {
        self.generation
    }

    pub fn get(&self, c: char) -> Cat {
        match u32::from(c) {
            // Outside Latin-1, `Other`. TeX82 reads BYTES, so a character up
            // there is a run of `Other` bytes to it and never part of a control
            // word. Calling them Letters here made them part of one: in a UTF-8
            // document `\textgreater→key` lexed as a single control sequence
            // named `textgreater→key`, so a real document full of arrows and
            // dashes failed on names that do not exist. `Other` is both closer
            // to what tex does and the only choice that ends a control word
            // where tex ends it.
            n if n > 255 => Cat::Other,
            n => self.codes[n as usize],
        }
    }

    pub fn set(&mut self, c: char, cat: Cat) {
        if let Ok(i) = usize::try_from(u32::from(c)) {
            if i < 256 {
                // Bump even when the value is unchanged: `\catcode` reassigning
                // a character its current class is still a point where anything
                // lexed ahead has to be re-checked, and pretending otherwise
                // would make correctness depend on the old value.
                self.codes[i] = cat;
                self.generation = self.generation.wrapping_add(1);
            }
        }
    }
}

/// `tex.web`'s numbering, for `\catcode` reads and writes.
pub fn cat_from_i64(n: i64) -> Option<Cat> {
    Some(match n {
        0 => Cat::Escape,
        1 => Cat::BeginGroup,
        2 => Cat::EndGroup,
        3 => Cat::MathShift,
        4 => Cat::AlignTab,
        5 => Cat::EndLine,
        6 => Cat::Param,
        7 => Cat::Superscript,
        8 => Cat::Subscript,
        9 => Cat::Ignored,
        10 => Cat::Space,
        11 => Cat::Letter,
        12 => Cat::Other,
        13 => Cat::Active,
        14 => Cat::Comment,
        15 => Cat::Invalid,
        _ => return None,
    })
}
