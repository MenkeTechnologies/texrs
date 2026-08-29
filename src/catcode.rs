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
        Self { codes }
    }

    pub fn get(&self, c: char) -> Cat {
        match u32::from(c) {
            // Outside Latin-1 TeX82 has no opinion; treat as a letter so a
            // UTF-8 source at least tokenises rather than aborting. Recorded in
            // BUGS.md: real TeX reads bytes, so `é` is two `Other` characters
            // there and one Letter here.
            n if n > 255 => Cat::Letter,
            n => self.codes[n as usize],
        }
    }

    pub fn set(&mut self, c: char, cat: Cat) {
        if let Ok(i) = usize::try_from(u32::from(c)) {
            if i < 256 {
                self.codes[i] = cat;
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
