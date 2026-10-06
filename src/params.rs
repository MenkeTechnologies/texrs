//! TeX's dimension, glue and token parameters (`tex.web` §247, §224, §230).
//!
//! The names are §247's `dimen_pars`, §224's glue parameters and §230's token
//! parameters, in those sections' order, spelt as the `primitive` calls of
//! §248, §226 and §230 spell them. INITEX starts every one at zero -- a zero
//! dimension, `zero_glue`, an empty list -- and nothing in §240 changes that.
//!
//! A dimension or glue parameter is a register in every way a document can
//! see (§413's `assign_dimen` and `assign_glue`): assigned with or without
//! `=`, read by `\the` and wherever a dimension or glue is scanned, changed by
//! `\advance`/`\multiply`/`\divide`, and restored at the end of a group. So
//! each one is a slot in the same file as `\dimen` and `\skip` (see
//! `crate::compiler`'s `DIMPAR_BASE` and `GLUEPAR_BASE`), and everything a
//! register already does works for it, including a right-hand side that is
//! only known at run time.
//!
//! A token parameter is a token register in the same sense (§1226 assigns both
//! through `assign_toks`), so it is one of the expander's token lists, numbered
//! past the 256 `\toks` registers.
//!
//! What each parameter is READ by is a separate question: these are stores.
//! The line breaker takes its measure from `crate::typeset::Layout`, not from
//! `\hsize`, and `\everypar` is not inserted when a paragraph starts.

/// §247's dimension parameters, in order.
pub const DIMEN_NAMES: [&str; 21] = [
    "parindent",
    "mathsurround",
    "lineskiplimit",
    "hsize",
    "vsize",
    "maxdepth",
    "splitmaxdepth",
    "boxmaxdepth",
    "hfuzz",
    "vfuzz",
    "delimitershortfall",
    "nulldelimiterspace",
    "scriptspace",
    "predisplaysize",
    "displaywidth",
    "displayindent",
    "overfullrule",
    "hangindent",
    "hoffset",
    "voffset",
    "emergencystretch",
];

/// §224's glue parameters, in order. The last three are MATH glue (§1228
/// scans them in mu), and the rest are ordinary glue.
pub const GLUE_NAMES: [&str; 18] = [
    "lineskip",
    "baselineskip",
    "parskip",
    "abovedisplayskip",
    "belowdisplayskip",
    "abovedisplayshortskip",
    "belowdisplayshortskip",
    "leftskip",
    "rightskip",
    "topskip",
    "splittopskip",
    "tabskip",
    "spaceskip",
    "xspaceskip",
    "parfillskip",
    "thinmuskip",
    "medmuskip",
    "thickmuskip",
];

/// How many of [`GLUE_NAMES`] are ordinary glue; the rest are math glue.
pub const ORDINARY_GLUE: usize = 15;

/// §230's token parameters, in order.
pub const TOKS_NAMES: [&str; 9] = [
    "output",
    "everypar",
    "everymath",
    "everydisplay",
    "everyhbox",
    "everyvbox",
    "everyjob",
    "everycr",
    "errhelp",
];

/// Where the token parameters are numbered among the expander's token lists:
/// past the 256 `\toks` registers, so none can be reached as `\toks<n>`.
pub const TOKS_BASE: i64 = 256;

/// The slot a dimension or glue parameter named `name` lives in, if it is one.
pub fn register_slot(name: &str) -> Option<i64> {
    use crate::compiler::{DIMPAR_BASE, GLUEPAR_BASE, MUGLUEPAR_BASE, SKIP_STRIDE};
    if let Some(i) = DIMEN_NAMES.iter().position(|n| *n == name) {
        return Some(DIMPAR_BASE + i as i64);
    }
    let i = GLUE_NAMES.iter().position(|n| *n == name)?;
    Some(match i < ORDINARY_GLUE {
        true => GLUEPAR_BASE + i as i64 * SKIP_STRIDE,
        false => MUGLUEPAR_BASE + (i - ORDINARY_GLUE) as i64 * SKIP_STRIDE,
    })
}

/// The parameter whose (first) slot is `slot`, by name, if it is one.
pub fn name_of_slot(slot: i64) -> Option<&'static str> {
    use crate::compiler::{DIMPAR_BASE, GLUEPAR_BASE, MUGLUEPAR_BASE, SKIP_STRIDE};
    if (DIMPAR_BASE..GLUEPAR_BASE).contains(&slot) {
        return DIMEN_NAMES.get((slot - DIMPAR_BASE) as usize).copied();
    }
    let (base, first) = match slot >= MUGLUEPAR_BASE {
        true => (MUGLUEPAR_BASE, ORDINARY_GLUE),
        false => (GLUEPAR_BASE, 0),
    };
    if slot < GLUEPAR_BASE || (slot - base) % SKIP_STRIDE != 0 {
        return None;
    }
    GLUE_NAMES
        .get(first + ((slot - base) / SKIP_STRIDE) as usize)
        .copied()
}

/// The token list a token parameter named `name` is, if it is one.
pub fn toks_register(name: &str) -> Option<i64> {
    TOKS_NAMES
        .iter()
        .position(|n| *n == name)
        .map(|i| TOKS_BASE + i as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_slot_names_its_parameter_back() {
        for name in DIMEN_NAMES.iter().chain(GLUE_NAMES.iter()) {
            let slot = register_slot(name).expect("a parameter");
            assert_eq!(name_of_slot(slot), Some(*name));
        }
    }

    #[test]
    fn the_math_glue_is_in_the_math_glue_file() {
        for name in &GLUE_NAMES[ORDINARY_GLUE..] {
            assert!(
                crate::compiler::is_mu_slot(register_slot(name).unwrap()),
                "{name}"
            );
        }
        for name in &GLUE_NAMES[..ORDINARY_GLUE] {
            let slot = register_slot(name).unwrap();
            assert!(crate::compiler::is_glue_slot(slot) && !crate::compiler::is_mu_slot(slot));
        }
        for name in DIMEN_NAMES {
            let slot = register_slot(name).unwrap();
            assert!(!crate::compiler::is_glue_slot(slot), "{name}");
        }
    }
}
