//! What the frontend knows of the registers while it is lowering.
//!
//! A register is a VM slot, so its value is the run's -- except that a
//! straight-line program is run in the order it is lowered, and `\dimen0=3pt`
//! followed by `\kern\dimen0` is a kern of 3pt whatever else the program does.
//! TeX itself has no such split: `tex.web` §413's `scan_something_internal`
//! reads `eqtb` at the moment the mouth reaches the register, which is the
//! moment this follows. So this table is the engine's `eqtb` for the
//! registers, as far as it can be followed, and it is the one both halves of
//! the frontend read: the lowerer, deciding a `\ifnum` while lowering when
//! both operands are known, and the expander, freezing `\number\count2` into
//! an `\edef` body or a `\csname` as §366 does.
//!
//! Three states per register. Never assigned, so INITEX's zero (the code
//! generator writes zero into every slot first). Assigned to a value this
//! tracked. Assigned to something it could not follow -- a `\rustcall`, whose
//! value only the run has -- which is unknown and makes whatever depends on it
//! a run-time branch rather than a guess.

use crate::ir::{Arith, Cmd, Num};

#[derive(Default, Clone)]
pub struct Known {
    map: rustc_hash::FxHashMap<i64, Option<i64>>,
    /// Every change, with what it replaced, so a group or a conditional arm
    /// can put the state back without cloning the table.
    trail: Vec<(i64, Option<Option<i64>>)>,
}

impl Known {
    /// The value of `slot`, if it is known.
    pub fn get(&self, slot: i64) -> Option<i64> {
        match self.map.get(&slot) {
            Some(v) => *v,
            None => Some(0),
        }
    }

    pub fn set(&mut self, slot: i64, value: Option<i64>) {
        let old = self.map.insert(slot, value);
        self.trail.push((slot, old));
    }

    /// Where the trail stands, for [`Known::undo_to`].
    pub fn mark(&self) -> usize {
        self.trail.len()
    }

    /// Put back everything changed since `mark`, or only `only` of it.
    pub fn undo_to(&mut self, mark: usize, only: Option<&[i64]>) {
        while self.trail.len() > mark {
            let (slot, old) = self.trail.pop().expect("longer than the mark");
            if only.is_some_and(|set| !set.contains(&slot)) {
                continue;
            }
            match old {
                Some(v) => self.map.insert(slot, v),
                None => self.map.remove(&slot),
            };
        }
    }

    /// Forget every value, back to INITEX's zeros: what a format load does
    /// before it puts its own registers in.
    pub fn clear(&mut self) {
        self.map.clear();
        self.trail.clear();
    }

    /// The known count registers that are not zero, in register order -- what
    /// a format dumps and what a `\directlua` chunk's `tex.count` reads.
    pub fn counts(&self) -> Vec<(i64, i64)> {
        let mut out: Vec<(i64, i64)> = self
            .map
            .iter()
            .filter(|(slot, _)| (0..crate::compiler::DIMEN_BASE).contains(*slot))
            .filter_map(|(slot, v)| v.filter(|v| *v != 0).map(|v| (*slot, v)))
            .collect();
        out.sort_unstable();
        out
    }

    /// Every slot whose value is known and not zero: the whole slot file, so
    /// a `\directlua` chunk's `tex.dimen` and `tex.skip` read from the same
    /// table its `tex.count` does.
    pub fn values(&self) -> std::collections::HashMap<i64, i64> {
        self.map
            .iter()
            .filter_map(|(slot, v)| v.filter(|v| *v != 0).map(|v| (*slot, v)))
            .collect()
    }

    /// What a number is, if every register it reads is known.
    ///
    /// The products are the run's own (`b_scale_dimen` and the checked
    /// multiply the code generator emits for a negated factor): a value
    /// computed here that differed from the one the VM computes would decide
    /// a conditional the run then contradicts. A product the run would report
    /// as too large is not followed.
    pub fn num(&self, n: &Num) -> Option<i64> {
        match n {
            Num::Literal(v) => Some(*v),
            Num::Count(s) => self.get(*s),
            Num::Neg(s) => self.get(*s).map(|v| -v),
            Num::Scaled { int, frac, reg } => {
                crate::dimen::scale_by_factor_checked(*int, *frac, self.get(*reg)?).ok()
            }
            Num::ByCount { factor, sign, unit } => {
                let unit = self.num(unit)?;
                let factor = match *sign < 0 {
                    true => self.get(*factor)?.checked_mul(-1)?,
                    false => self.get(*factor)?,
                };
                crate::dimen::scale_by_factor_checked(factor, 0, unit).ok()
            }
            Num::Rust { .. } => None,
        }
    }

    /// Follow one lowered command.
    #[inline(never)]
    pub fn apply(&mut self, cmd: &Cmd) {
        const LIMIT: i64 = i32::MAX as i64;
        match cmd {
            Cmd::SetCount(reg, n) => {
                let v = self.num(n);
                self.set(*reg, v);
            }
            Cmd::Arith(op, reg, n) => {
                let next = match (self.get(*reg), self.num(n)) {
                    (Some(cur), Some(by)) => match op {
                        Arith::Add => Some(cur.wrapping_add(by)),
                        Arith::Mul => cur.checked_mul(by),
                        Arith::Div if by != 0 => Some(cur / by),
                        Arith::Div => None,
                    },
                    _ => None,
                };
                self.set(*reg, next.filter(|v| v.abs() <= LIMIT));
            }
            // A group puts its registers back itself, where it is lowered.
            Cmd::Group { .. } => {}
            // Which arm runs, and how often, is the run's.
            Cmd::IfNum { .. } | Cmd::IfOdd { .. } | Cmd::Loop { .. } => {
                for reg in crate::lower::assigned_counts(std::slice::from_ref(cmd)) {
                    self.set(reg, None);
                }
            }
            _ => {}
        }
    }
}
