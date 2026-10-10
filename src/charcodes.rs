//! TeX's other per-character tables.
//!
//! `\catcode` is in `catcode.rs` because the lexer reads it on every character;
//! these four are read only when something asks for them, so they live together
//! here. All of them are 256-entry tables with INITEX's defaults, saved and
//! restored by a group exactly as the category codes are — measured against
//! `tex -ini`, which is the only oracle for a default, since plain `tex` has
//! already loaded a format that changes several of them.

/// Which table an assignment or a `\the` is talking about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Table {
    /// `\mathcode` — how a character is set in math mode.
    Math,
    /// `\lccode` — the character's lowercase form, for `\lowercase`.
    Lower,
    /// `\uccode` — its uppercase form, for `\uppercase`.
    Upper,
    /// `\sfcode` — the space factor a character leaves behind.
    Space,
    /// `\delcode` — the character's meaning as a delimiter.
    Delimiter,
}

impl Table {
    /// The primitive that reads and writes it.
    pub fn name(self) -> &'static str {
        match self {
            Table::Math => "mathcode",
            Table::Lower => "lccode",
            Table::Upper => "uccode",
            Table::Space => "sfcode",
            Table::Delimiter => "delcode",
        }
    }

    /// The primitive spelling, if it is one of these.
    pub fn from_name(name: &str) -> Option<Table> {
        match name {
            "mathcode" => Some(Table::Math),
            "lccode" => Some(Table::Lower),
            "uccode" => Some(Table::Upper),
            "sfcode" => Some(Table::Space),
            "delcode" => Some(Table::Delimiter),
            _ => None,
        }
    }

    /// The largest value TeX accepts (`tex.web` §1232's `n`).
    fn largest(self) -> i64 {
        match self {
            // "8000 is the largest, and it means "active in math".
            Table::Math => 0x8000,
            Table::Lower | Table::Upper => 255,
            Table::Space => 0x7FFF,
            Table::Delimiter => 0xFF_FFFF,
        }
    }

    /// §1232's complaint about `v`, or `None` when the table may hold it.
    pub fn invalid(self, v: i64) -> Option<String> {
        let n = self.largest();
        match self {
            // Any negative delimiter code is legal: it means "not a
            // delimiter", which is what INITEX's -1 everywhere says.
            Table::Delimiter => {
                (v > n).then(|| format!("Invalid code ({v}), should be at most {n}"))
            }
            _ => invalid_in_range(v, n),
        }
    }
}

/// §1232's complaint for a table whose codes run 0 to `n`, the catcode table
/// among them, or `None` when `v` is one of them.
pub fn invalid_in_range(v: i64, n: i64) -> Option<String> {
    (!(0..=n).contains(&v)).then(|| format!("Invalid code ({v}), should be in the range 0..{n}"))
}

/// The four tables, with INITEX's defaults.
#[derive(Clone, PartialEq, Debug)]
pub struct CharCodes {
    math: [i64; 256],
    lower: [i64; 256],
    upper: [i64; 256],
    space: [i64; 256],
    delimiter: [i64; 256],
}

impl Default for CharCodes {
    fn default() -> Self {
        let mut it = CharCodes {
            // A character is set as itself in math unless it is a letter or a
            // digit: those carry the "variable family" class, and a letter also
            // carries family 1, which is why `\mathcode`\A` is "7141 and
            // `\mathcode`\0` is "7030 while `\mathcode`\+` is 43 (tex.web §232).
            math: std::array::from_fn(|k| k as i64),
            // Zero, not the character: a non-letter has no case.
            lower: [0; 256],
            upper: [0; 256],
            // Every character leaves the space factor alone at 1000, except an
            // uppercase letter, which lowers it to 999 so a sentence does not
            // end at "N.A.S.A.".
            space: [1000; 256],
            // -1 everywhere means "not a delimiter"; the period is the one
            // character INITEX gives a real code, and that code is 0.
            delimiter: [-1; 256],
        };
        const VAR_CODE: i64 = 0x7000;
        for k in 0..256usize {
            let c = k as u8 as char;
            if c.is_ascii_digit() {
                it.math[k] = k as i64 + VAR_CODE;
            }
            if c.is_ascii_alphabetic() {
                it.math[k] = k as i64 + VAR_CODE + 0x100;
                it.lower[k] = c.to_ascii_lowercase() as i64;
                it.upper[k] = c.to_ascii_uppercase() as i64;
                if c.is_ascii_uppercase() {
                    it.space[k] = 999;
                }
            }
        }
        it.delimiter[b'.' as usize] = 0;
        it
    }
}

impl CharCodes {
    fn slot(&self, table: Table) -> &[i64; 256] {
        match table {
            Table::Math => &self.math,
            Table::Lower => &self.lower,
            Table::Upper => &self.upper,
            Table::Space => &self.space,
            Table::Delimiter => &self.delimiter,
        }
    }

    fn slot_mut(&mut self, table: Table) -> &mut [i64; 256] {
        match table {
            Table::Math => &mut self.math,
            Table::Lower => &mut self.lower,
            Table::Upper => &mut self.upper,
            Table::Space => &mut self.space,
            Table::Delimiter => &mut self.delimiter,
        }
    }

    /// What `table` says about `c`. A character outside the 256 the tables
    /// cover reads as the default rather than as an error, which is what an
    /// engine reading bytes would have given.
    pub fn get(&self, table: Table, c: char) -> i64 {
        match (c as u32) < 256 {
            true => self.slot(table)[c as usize],
            false => match table {
                Table::Math => c as i64,
                Table::Space => 1000,
                Table::Delimiter => -1,
                _ => 0,
            },
        }
    }

    /// Set it, refusing a value the table cannot hold.
    pub fn set(&mut self, table: Table, c: char, v: i64) -> Result<(), String> {
        if let Some(msg) = table.invalid(v) {
            return Err(msg);
        }
        if (c as u32) < 256 {
            self.slot_mut(table)[c as usize] = v;
        }
        Ok(())
    }
}
