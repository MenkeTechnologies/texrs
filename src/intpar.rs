//! TeX's integer parameters (`tex.web` §236), as frontend state.
//!
//! The 55 names are the `int_pars` of §236, in §236's order, spelt as §238's
//! `primitive` calls spell them. Each is an internal integer (§413): it is
//! assigned with or without `=`, read wherever a number is scanned, written by
//! `\the` and `\number`, changed by `\advance`/`\multiply`/`\divide`, and
//! restored at the end of a group like any other `eqtb` entry (§283).
//!
//! They live beside the category codes and the five code tables rather than in
//! a VM slot, because the one of them that changes what the frontend WRITES --
//! `\escapechar`, which every `\string`, `\meaning` and `\message` of a control
//! sequence prints in front of its name (§63's `print_esc`) -- has to be known
//! while lowering, and a table that holds one of them the lowerer's way and the
//! rest another would answer `\the` two different ways. The cost is the same
//! one the catcode table already has: a right-hand side that is only known at
//! run time (`\tolerance=\count1` after a run-time `\advance`) is read as the
//! value the lowerer holds.
//!
//! What each parameter is READ by is a separate question from whether it can be
//! set and read back: `\escapechar` is honoured by everything that prints a
//! control sequence; `\endlinechar` by the mouth, through the copy
//! `crate::catcode::CatTable` carries; `\newlinechar` by `\message`'s
//! printer; the line breaker's eight by `crate::linebreak::Params`, as the
//! document left them when lowering finished; the page builder's are still the
//! constants `crate::page` was written with, which `BUGS.md` records.

/// §236's integer parameters, in order.
pub const NAMES: [&str; 55] = [
    "pretolerance",
    "tolerance",
    "linepenalty",
    "hyphenpenalty",
    "exhyphenpenalty",
    "clubpenalty",
    "widowpenalty",
    "displaywidowpenalty",
    "brokenpenalty",
    "binoppenalty",
    "relpenalty",
    "predisplaypenalty",
    "postdisplaypenalty",
    "interlinepenalty",
    "doublehyphendemerits",
    "finalhyphendemerits",
    "adjdemerits",
    "mag",
    "delimiterfactor",
    "looseness",
    "time",
    "day",
    "month",
    "year",
    "showboxbreadth",
    "showboxdepth",
    "hbadness",
    "vbadness",
    "pausing",
    "tracingonline",
    "tracingmacros",
    "tracingstats",
    "tracingparagraphs",
    "tracingpages",
    "tracingoutput",
    "tracinglostchars",
    "tracingcommands",
    "tracingrestores",
    "uchyph",
    "outputpenalty",
    "maxdeadcycles",
    "hangafter",
    "floatingpenalty",
    "globaldefs",
    "fam",
    "escapechar",
    "defaulthyphenchar",
    "defaultskewchar",
    "endlinechar",
    "newlinechar",
    "language",
    "lefthyphenmin",
    "righthyphenmin",
    "holdinginserts",
    "errorcontextlines",
];

/// Where `name` is in [`NAMES`], if it is one of them.
pub fn index(name: &str) -> Option<usize> {
    NAMES.iter().position(|n| *n == name)
}

/// The position of `\mag`, which a `true` dimension is divided by (§457).
pub const MAG: usize = 17;

/// The position of `\escapechar`, which the printer reads on every control
/// sequence it writes.
pub const ESCAPE_CHAR: usize = 45;

/// The position of `\endlinechar`, which the mouth appends to every line it
/// reads; `crate::catcode::CatTable` carries a copy for the mouth.
pub const END_LINE_CHAR: usize = 48;

/// The position of `\newlinechar`, which the printer writes as a line end.
pub const NEW_LINE_CHAR: usize = 49;

/// The table itself.
#[derive(Clone, Debug, PartialEq)]
pub struct IntPars {
    values: [i64; NAMES.len()],
    /// Which parameters an assignment has written, one bit per [`NAMES`]
    /// position (55 fit in 64). The typesetting path reads a parameter a
    /// document set and keeps plain.tex's value for one it did not, because
    /// texrs loads no format and INITEX's values (`\tolerance=10000`,
    /// `\pretolerance=0`) are not what any document is set with.
    written: u64,
}

impl IntPars {
    /// INITEX's values: §240 zeroes the table and then sets six of them, and
    /// §241's `fix_date_and_time` fills in the four date parameters from the
    /// clock before the first line is read.
    pub fn new() -> Self {
        let mut values = [0; NAMES.len()];
        for (name, v) in [
            ("mag", 1000),
            ("tolerance", 10000),
            ("hangafter", 1),
            ("maxdeadcycles", 25),
            ("escapechar", i64::from(b'\\')),
            ("endlinechar", 13),
        ] {
            values[index(name).expect("a §236 name")] = v;
        }
        let (time, day, month, year) = date_and_time();
        for (name, v) in [
            ("time", time),
            ("day", day),
            ("month", month),
            ("year", year),
        ] {
            values[index(name).expect("a §236 name")] = v;
        }
        Self { values, written: 0 }
    }

    pub fn get(&self, i: usize) -> i64 {
        self.values[i]
    }

    pub fn set(&mut self, i: usize, v: i64) {
        self.values[i] = v;
        self.written |= 1 << i;
    }

    /// The value of parameter `i` if anything has assigned it, else `None`.
    pub fn assigned(&self, i: usize) -> Option<i64> {
        (self.written & (1 << i) != 0).then(|| self.values[i])
    }

    /// The parameters that differ from `other`, by name, for a format dump.
    pub fn differences(&self, other: &IntPars) -> Vec<(String, i64)> {
        NAMES
            .iter()
            .enumerate()
            .filter(|(i, _)| self.values[*i] != other.values[*i])
            .map(|(i, n)| ((*n).to_string(), self.values[i]))
            .collect()
    }
}

impl Default for IntPars {
    fn default() -> Self {
        Self::new()
    }
}

/// §241's `fix_date_and_time`, as TeX Live implements it: the LOCAL wall clock,
/// as minutes since midnight, day, month and year. Measured: `tex -ini` at
/// 10:25 on 26 September 2026 answers `[625][26][9][2026]`, and a
/// `SOURCE_DATE_EPOCH` in the environment does not change what `tex` says.
fn date_and_time() -> (i64, i64, i64, i64) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as libc::time_t)
        .unwrap_or(0);
    // SAFETY: `localtime_r` writes only into the `tm` it is handed and reads
    // only `now`; both outlive the call.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let ok = unsafe { !libc::localtime_r(&now, &mut tm).is_null() };
    if !ok {
        // §241's own fallback when there is no clock: noon on 4 July 1776.
        return (12 * 60, 4, 7, 1776);
    }
    (
        i64::from(tm.tm_hour) * 60 + i64::from(tm.tm_min),
        i64::from(tm.tm_mday),
        i64::from(tm.tm_mon) + 1,
        i64::from(tm.tm_year) + 1900,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapechar_index_is_the_one_the_printer_reads() {
        assert_eq!(index("escapechar"), Some(ESCAPE_CHAR));
    }

    #[test]
    fn line_char_indices_name_their_parameters() {
        assert_eq!(index("endlinechar"), Some(END_LINE_CHAR));
        assert_eq!(index("newlinechar"), Some(NEW_LINE_CHAR));
    }

    #[test]
    fn mag_index_names_mag() {
        assert_eq!(index("mag"), Some(MAG));
    }

    #[test]
    fn initex_values() {
        let p = IntPars::new();
        let at = |n| p.get(index(n).unwrap());
        assert_eq!(at("mag"), 1000);
        assert_eq!(at("tolerance"), 10000);
        assert_eq!(at("escapechar"), 92);
        assert_eq!(at("endlinechar"), 13);
        assert_eq!(at("newlinechar"), 0);
        assert_eq!(at("pretolerance"), 0);
        assert!((1..=12).contains(&at("month")));
    }
}
