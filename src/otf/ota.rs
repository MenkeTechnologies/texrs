//! The script analyzers of fontloader-font-ota.lua: before the lookups run,
//! the `analyze` processor gives each glyph of the font a `state` -- init,
//! medi, fina, isol, mark or rest -- which the init/medi/fina/isol lookups
//! test (`getstate(start, attribute)`). `methods.latn` is the generic
//! marks-and-runs analyzer (font-ota.lua:132-201), `methods.arab` (also
//! Syriac, Mandaic, N'Ko) the joining analyzer (:321-457) over the
//! precomputed joining classes.
//!
//! Not ported: `methods.mong` (needs char-def.lua's Mongolian joining data)
//! and the Indic analyzers of font-osd.lua; `analyze` refuses those scripts.

use super::nodes::{Ix, Kind, List};
use super::Face;

pub const S_INIT: u8 = 1;
pub const S_MEDI: u8 = 2;
pub const S_FINA: u8 = 3;
pub const S_ISOL: u8 = 4;
pub const S_MARK: u8 = 5;
pub const S_REST: u8 = 6;

/// `analyzers.features`: the state an analyzer-dependent feature's lookups
/// require.
pub fn feature_state(kind: &str) -> Option<u8> {
    Some(match kind {
        "init" => S_INIT,
        "medi" | "med2" => S_MEDI,
        "fina" | "fin2" | "fin3" => S_FINA,
        "isol" => S_ISOL,
        "rphf" => 7,
        "half" => 8,
        "pref" => 9,
        "blwf" => 10,
        "pstf" => 11,
        _ => return None,
    })
}

/// `characters.classifiers` (fontloader-basics-chr.lua, merged as
/// fontloader-2023-12-28.lua:467-2599): first, last, state.
const CLASSIFIERS: &[(u32, u32, u8)] = &[
    (0x0300, 0x036F, 5),
    (0x0483, 0x0487, 5),
    (0x0591, 0x05BD, 5),
    (0x05BF, 0x05BF, 5),
    (0x05C1, 0x05C2, 5),
    (0x05C4, 0x05C5, 5),
    (0x05C7, 0x05C7, 5),
    (0x0600, 0x0605, 4),
    (0x0606, 0x0607, 6),
    (0x0608, 0x0608, 4),
    (0x0609, 0x060A, 6),
    (0x060B, 0x060B, 4),
    (0x060C, 0x060F, 6),
    (0x0610, 0x061A, 5),
    (0x061B, 0x061F, 6),
    (0x0620, 0x0620, 2),
    (0x0621, 0x0621, 4),
    (0x0622, 0x0625, 3),
    (0x0626, 0x0626, 2),
    (0x0627, 0x0627, 3),
    (0x0628, 0x0628, 2),
    (0x0629, 0x0629, 3),
    (0x062A, 0x062E, 2),
    (0x062F, 0x0632, 3),
    (0x0633, 0x0647, 2),
    (0x0648, 0x0648, 3),
    (0x0649, 0x064A, 2),
    (0x064B, 0x065F, 5),
    (0x0660, 0x066D, 6),
    (0x066E, 0x066F, 2),
    (0x0670, 0x0670, 5),
    (0x0671, 0x0673, 3),
    (0x0674, 0x0674, 4),
    (0x0675, 0x0677, 3),
    (0x0678, 0x0687, 2),
    (0x0688, 0x0699, 3),
    (0x069A, 0x06BF, 2),
    (0x06C0, 0x06C0, 3),
    (0x06C1, 0x06C2, 2),
    (0x06C3, 0x06CB, 3),
    (0x06CC, 0x06CC, 2),
    (0x06CD, 0x06CD, 3),
    (0x06CE, 0x06CE, 2),
    (0x06CF, 0x06CF, 3),
    (0x06D0, 0x06D1, 2),
    (0x06D2, 0x06D3, 3),
    (0x06D4, 0x06D4, 6),
    (0x06D5, 0x06D5, 3),
    (0x06D6, 0x06DC, 5),
    (0x06DD, 0x06DD, 4),
    (0x06DE, 0x06DE, 6),
    (0x06DF, 0x06E4, 5),
    (0x06E5, 0x06E6, 6),
    (0x06E7, 0x06E8, 5),
    (0x06E9, 0x06E9, 6),
    (0x06EA, 0x06ED, 5),
    (0x06EE, 0x06EF, 3),
    (0x06F0, 0x06F9, 6),
    (0x06FA, 0x06FC, 2),
    (0x06FD, 0x06FE, 6),
    (0x06FF, 0x06FF, 2),
    (0x0700, 0x070D, 6),
    (0x0710, 0x0710, 3),
    (0x0711, 0x0711, 5),
    (0x0712, 0x0714, 2),
    (0x0715, 0x0719, 3),
    (0x071A, 0x071D, 2),
    (0x071E, 0x071E, 3),
    (0x071F, 0x0727, 2),
    (0x0728, 0x0728, 3),
    (0x0729, 0x0729, 2),
    (0x072A, 0x072A, 3),
    (0x072B, 0x072B, 2),
    (0x072C, 0x072C, 3),
    (0x072D, 0x072E, 2),
    (0x072F, 0x072F, 3),
    (0x0730, 0x074A, 5),
    (0x074D, 0x074D, 3),
    (0x074E, 0x0758, 2),
    (0x0759, 0x075B, 3),
    (0x075C, 0x076A, 2),
    (0x076B, 0x076C, 3),
    (0x076D, 0x0770, 2),
    (0x0771, 0x0771, 3),
    (0x0772, 0x0772, 2),
    (0x0773, 0x0774, 3),
    (0x0775, 0x0777, 2),
    (0x0778, 0x0779, 3),
    (0x077A, 0x077F, 2),
    (0x07A6, 0x07B0, 5),
    (0x07C0, 0x07C9, 6),
    (0x07CA, 0x07EA, 2),
    (0x07EB, 0x07F3, 5),
    (0x07F4, 0x07F9, 6),
    (0x07FA, 0x07FA, 2),
    (0x07FD, 0x07FD, 5),
    (0x07FE, 0x07FF, 6),
    (0x0816, 0x0819, 5),
    (0x081B, 0x0823, 5),
    (0x0825, 0x0827, 5),
    (0x0829, 0x082D, 5),
    (0x0840, 0x0840, 3),
    (0x0841, 0x0845, 2),
    (0x0846, 0x0847, 3),
    (0x0848, 0x0848, 2),
    (0x0849, 0x0849, 3),
    (0x084A, 0x0853, 2),
    (0x0854, 0x0854, 3),
    (0x0855, 0x0855, 2),
    (0x0856, 0x0858, 3),
    (0x0859, 0x085B, 5),
    (0x0860, 0x0860, 2),
    (0x0861, 0x0861, 4),
    (0x0862, 0x0865, 2),
    (0x0866, 0x0866, 4),
    (0x0867, 0x0867, 3),
    (0x0868, 0x0868, 2),
    (0x0869, 0x086A, 3),
    (0x0870, 0x0882, 3),
    (0x0883, 0x0886, 2),
    (0x0887, 0x0888, 4),
    (0x0889, 0x088D, 2),
    (0x088E, 0x088E, 3),
    (0x0890, 0x0891, 4),
    (0x0898, 0x089F, 5),
    (0x08A0, 0x08A9, 2),
    (0x08AA, 0x08AC, 3),
    (0x08AD, 0x08AD, 4),
    (0x08AE, 0x08AE, 3),
    (0x08AF, 0x08B0, 2),
    (0x08B1, 0x08B2, 3),
    (0x08B3, 0x08B8, 2),
    (0x08B9, 0x08B9, 3),
    (0x08BA, 0x08C8, 2),
    (0x08CA, 0x08E1, 5),
    (0x08E2, 0x08E2, 4),
    (0x08E3, 0x0902, 5),
    (0x093A, 0x093A, 5),
    (0x093C, 0x093C, 5),
    (0x0941, 0x0948, 5),
    (0x094D, 0x094D, 5),
    (0x0951, 0x0957, 5),
    (0x0962, 0x0963, 5),
    (0x0981, 0x0981, 5),
    (0x09BC, 0x09BC, 5),
    (0x09C1, 0x09C4, 5),
    (0x09CD, 0x09CD, 5),
    (0x09E2, 0x09E3, 5),
    (0x09FE, 0x09FE, 5),
    (0x0A01, 0x0A02, 5),
    (0x0A3C, 0x0A3C, 5),
    (0x0A41, 0x0A42, 5),
    (0x0A47, 0x0A48, 5),
    (0x0A4B, 0x0A4D, 5),
    (0x0A51, 0x0A51, 5),
    (0x0A70, 0x0A71, 5),
    (0x0A75, 0x0A75, 5),
    (0x0A81, 0x0A82, 5),
    (0x0ABC, 0x0ABC, 5),
    (0x0AC1, 0x0AC5, 5),
    (0x0AC7, 0x0AC8, 5),
    (0x0ACD, 0x0ACD, 5),
    (0x0AE2, 0x0AE3, 5),
    (0x0AFA, 0x0AFF, 5),
    (0x0B01, 0x0B01, 5),
    (0x0B3C, 0x0B3C, 5),
    (0x0B3F, 0x0B3F, 5),
    (0x0B41, 0x0B44, 5),
    (0x0B4D, 0x0B4D, 5),
    (0x0B55, 0x0B56, 5),
    (0x0B62, 0x0B63, 5),
    (0x0B82, 0x0B82, 5),
    (0x0BC0, 0x0BC0, 5),
    (0x0BCD, 0x0BCD, 5),
    (0x0C00, 0x0C00, 5),
    (0x0C04, 0x0C04, 5),
    (0x0C3C, 0x0C3C, 5),
    (0x0C3E, 0x0C40, 5),
    (0x0C46, 0x0C48, 5),
    (0x0C4A, 0x0C4D, 5),
    (0x0C55, 0x0C56, 5),
    (0x0C62, 0x0C63, 5),
    (0x0C81, 0x0C81, 5),
    (0x0CBC, 0x0CBC, 5),
    (0x0CBF, 0x0CBF, 5),
    (0x0CC6, 0x0CC6, 5),
    (0x0CCC, 0x0CCD, 5),
    (0x0CE2, 0x0CE3, 5),
    (0x0D00, 0x0D01, 5),
    (0x0D3B, 0x0D3C, 5),
    (0x0D41, 0x0D44, 5),
    (0x0D4D, 0x0D4D, 5),
    (0x0D62, 0x0D63, 5),
    (0x0D81, 0x0D81, 5),
    (0x0DCA, 0x0DCA, 5),
    (0x0DD2, 0x0DD4, 5),
    (0x0DD6, 0x0DD6, 5),
    (0x0E31, 0x0E31, 5),
    (0x0E34, 0x0E3A, 5),
    (0x0E47, 0x0E4E, 5),
    (0x0EB1, 0x0EB1, 5),
    (0x0EB4, 0x0EBC, 5),
    (0x0EC8, 0x0ECD, 5),
    (0x0F18, 0x0F19, 5),
    (0x0F35, 0x0F35, 5),
    (0x0F37, 0x0F37, 5),
    (0x0F39, 0x0F39, 5),
    (0x0F71, 0x0F7E, 5),
    (0x0F80, 0x0F84, 5),
    (0x0F86, 0x0F87, 5),
    (0x0F8D, 0x0F97, 5),
    (0x0F99, 0x0FBC, 5),
    (0x0FC6, 0x0FC6, 5),
    (0x102D, 0x1030, 5),
    (0x1032, 0x1037, 5),
    (0x1039, 0x103A, 5),
    (0x103D, 0x103E, 5),
    (0x1058, 0x1059, 5),
    (0x105E, 0x1060, 5),
    (0x1071, 0x1074, 5),
    (0x1082, 0x1082, 5),
    (0x1085, 0x1086, 5),
    (0x108D, 0x108D, 5),
    (0x109D, 0x109D, 5),
    (0x135D, 0x135F, 5),
    (0x1712, 0x1714, 5),
    (0x1732, 0x1734, 5),
    (0x1752, 0x1753, 5),
    (0x1772, 0x1773, 5),
    (0x17B7, 0x17BD, 5),
    (0x17C6, 0x17C6, 5),
    (0x17C9, 0x17D3, 5),
    (0x17DD, 0x17DD, 5),
    (0x1806, 0x1806, 4),
    (0x1807, 0x1807, 2),
    (0x180A, 0x180A, 2),
    (0x180B, 0x180D, 5),
    (0x180E, 0x180E, 4),
    (0x180F, 0x180F, 5),
    (0x1820, 0x1878, 2),
    (0x1880, 0x1884, 4),
    (0x1887, 0x18A8, 2),
    (0x18A9, 0x18A9, 5),
    (0x18AA, 0x18AA, 2),
    (0x1920, 0x1922, 5),
    (0x1927, 0x1928, 5),
    (0x1932, 0x1932, 5),
    (0x1939, 0x193B, 5),
    (0x1A17, 0x1A18, 5),
    (0x1A56, 0x1A56, 5),
    (0x1A58, 0x1A5E, 5),
    (0x1A60, 0x1A60, 5),
    (0x1A62, 0x1A62, 5),
    (0x1A65, 0x1A6C, 5),
    (0x1A73, 0x1A7C, 5),
    (0x1A7F, 0x1A7F, 5),
    (0x1AB0, 0x1ABD, 5),
    (0x1ABF, 0x1ACE, 5),
    (0x1B00, 0x1B03, 5),
    (0x1B34, 0x1B34, 5),
    (0x1B36, 0x1B3A, 5),
    (0x1B3C, 0x1B3C, 5),
    (0x1B42, 0x1B42, 5),
    (0x1B6B, 0x1B73, 5),
    (0x1B80, 0x1B81, 5),
    (0x1BA2, 0x1BA5, 5),
    (0x1BA8, 0x1BA9, 5),
    (0x1BAB, 0x1BAB, 5),
    (0x1BE6, 0x1BE6, 5),
    (0x1BE8, 0x1BE9, 5),
    (0x1BED, 0x1BED, 5),
    (0x1BEF, 0x1BF1, 5),
    (0x1C2C, 0x1C33, 5),
    (0x1C36, 0x1C37, 5),
    (0x1CD0, 0x1CD2, 5),
    (0x1CD4, 0x1CE0, 5),
    (0x1CE2, 0x1CE8, 5),
    (0x1CED, 0x1CED, 5),
    (0x1CF4, 0x1CF4, 5),
    (0x1CF8, 0x1CF9, 5),
    (0x1DC0, 0x1DFF, 5),
    (0x200C, 0x200C, 4),
    (0x200D, 0x200D, 2),
    (0x202F, 0x202F, 4),
    (0x2066, 0x2069, 4),
    (0x20D0, 0x20DC, 5),
    (0x20E1, 0x20E1, 5),
    (0x20E5, 0x20F0, 5),
    (0x2CEF, 0x2CF1, 5),
    (0x2D7F, 0x2D7F, 5),
    (0x2DE0, 0x2DFF, 5),
    (0x302A, 0x302F, 5),
    (0x3099, 0x309A, 5),
    (0xA66F, 0xA66F, 5),
    (0xA674, 0xA67D, 5),
    (0xA69E, 0xA69F, 5),
    (0xA6F0, 0xA6F1, 5),
    (0xA806, 0xA806, 5),
    (0xA80B, 0xA80B, 5),
    (0xA825, 0xA826, 5),
    (0xA82C, 0xA82C, 5),
    (0xA840, 0xA871, 2),
    (0xA872, 0xA872, 1),
    (0xA873, 0xA873, 4),
    (0xA8C4, 0xA8C5, 5),
    (0xA8E0, 0xA8F1, 5),
    (0xA8FF, 0xA8FF, 5),
    (0xA926, 0xA92D, 5),
    (0xA947, 0xA951, 5),
    (0xA980, 0xA982, 5),
    (0xA9B3, 0xA9B3, 5),
    (0xA9B6, 0xA9B9, 5),
    (0xA9BC, 0xA9BC, 5),
    (0xA9E5, 0xA9E5, 5),
    (0xAA29, 0xAA2E, 5),
    (0xAA31, 0xAA32, 5),
    (0xAA35, 0xAA36, 5),
    (0xAA43, 0xAA43, 5),
    (0xAA4C, 0xAA4C, 5),
    (0xAA7C, 0xAA7C, 5),
    (0xAAB0, 0xAAB0, 5),
    (0xAAB2, 0xAAB4, 5),
    (0xAAB7, 0xAAB8, 5),
    (0xAABE, 0xAABF, 5),
    (0xAAC1, 0xAAC1, 5),
    (0xAAEC, 0xAAED, 5),
    (0xAAF6, 0xAAF6, 5),
    (0xABE5, 0xABE5, 5),
    (0xABE8, 0xABE8, 5),
    (0xABED, 0xABED, 5),
    (0xFB1E, 0xFB1E, 5),
    (0xFE20, 0xFE2F, 5),
    (0x101FD, 0x101FD, 5),
    (0x102E0, 0x102E0, 5),
    (0x10376, 0x1037A, 5),
    (0x10A01, 0x10A03, 5),
    (0x10A05, 0x10A06, 5),
    (0x10A0C, 0x10A0F, 5),
    (0x10A38, 0x10A3A, 5),
    (0x10A3F, 0x10A3F, 5),
    (0x10AC0, 0x10AC4, 2),
    (0x10AC5, 0x10AC5, 3),
    (0x10AC6, 0x10AC6, 4),
    (0x10AC7, 0x10AC7, 3),
    (0x10AC8, 0x10AC8, 4),
    (0x10AC9, 0x10ACA, 3),
    (0x10ACB, 0x10ACC, 4),
    (0x10ACD, 0x10ACD, 1),
    (0x10ACE, 0x10AD2, 3),
    (0x10AD3, 0x10AD6, 2),
    (0x10AD7, 0x10AD7, 1),
    (0x10AD8, 0x10ADC, 2),
    (0x10ADD, 0x10ADD, 3),
    (0x10ADE, 0x10AE0, 2),
    (0x10AE1, 0x10AE1, 3),
    (0x10AE2, 0x10AE3, 4),
    (0x10AE4, 0x10AE4, 3),
    (0x10AE5, 0x10AE6, 5),
    (0x10AEB, 0x10AEE, 2),
    (0x10AEF, 0x10AEF, 3),
    (0x10B80, 0x10B80, 2),
    (0x10B81, 0x10B81, 3),
    (0x10B82, 0x10B82, 2),
    (0x10B83, 0x10B85, 3),
    (0x10B86, 0x10B88, 2),
    (0x10B89, 0x10B89, 3),
    (0x10B8A, 0x10B8B, 2),
    (0x10B8C, 0x10B8C, 3),
    (0x10B8D, 0x10B8D, 2),
    (0x10B8E, 0x10B8F, 3),
    (0x10B90, 0x10B90, 2),
    (0x10B91, 0x10B91, 3),
    (0x10BA9, 0x10BAC, 3),
    (0x10BAD, 0x10BAE, 2),
    (0x10BAF, 0x10BAF, 4),
    (0x10D00, 0x10D00, 1),
    (0x10D01, 0x10D21, 2),
    (0x10D22, 0x10D22, 3),
    (0x10D23, 0x10D23, 2),
    (0x10D24, 0x10D27, 5),
    (0x10EAB, 0x10EAC, 5),
    (0x10F30, 0x10F32, 2),
    (0x10F33, 0x10F33, 3),
    (0x10F34, 0x10F44, 2),
    (0x10F45, 0x10F45, 4),
    (0x10F46, 0x10F50, 5),
    (0x10F51, 0x10F53, 2),
    (0x10F54, 0x10F54, 3),
    (0x10F70, 0x10F73, 2),
    (0x10F74, 0x10F75, 3),
    (0x10F76, 0x10F81, 2),
    (0x10F82, 0x10F85, 5),
    (0x10FB0, 0x10FB0, 2),
    (0x10FB1, 0x10FB1, 4),
    (0x10FB2, 0x10FB3, 2),
    (0x10FB4, 0x10FB6, 3),
    (0x10FB7, 0x10FB7, 4),
    (0x10FB8, 0x10FB8, 2),
    (0x10FB9, 0x10FBA, 3),
    (0x10FBB, 0x10FBC, 2),
    (0x10FBD, 0x10FBD, 3),
    (0x10FBE, 0x10FBF, 2),
    (0x10FC0, 0x10FC0, 4),
    (0x10FC1, 0x10FC1, 2),
    (0x10FC2, 0x10FC3, 3),
    (0x10FC4, 0x10FC4, 2),
    (0x10FC5, 0x10FC8, 4),
    (0x10FC9, 0x10FC9, 3),
    (0x10FCA, 0x10FCA, 2),
    (0x10FCB, 0x10FCB, 1),
    (0x11001, 0x11001, 5),
    (0x11038, 0x11046, 5),
    (0x11070, 0x11070, 5),
    (0x11073, 0x11074, 5),
    (0x1107F, 0x11081, 5),
    (0x110B3, 0x110B6, 5),
    (0x110B9, 0x110BA, 5),
    (0x110BD, 0x110BD, 4),
    (0x110C2, 0x110C2, 5),
    (0x110CD, 0x110CD, 4),
    (0x11100, 0x11102, 5),
    (0x11127, 0x1112B, 5),
    (0x1112D, 0x11134, 5),
    (0x11173, 0x11173, 5),
    (0x11180, 0x11181, 5),
    (0x111B6, 0x111BE, 5),
    (0x111CA, 0x111CC, 5),
    (0x111CF, 0x111CF, 5),
    (0x1122F, 0x11231, 5),
    (0x11234, 0x11234, 5),
    (0x11236, 0x11237, 5),
    (0x1123E, 0x1123E, 5),
    (0x112DF, 0x112DF, 5),
    (0x112E3, 0x112EA, 5),
    (0x11300, 0x11301, 5),
    (0x1133B, 0x1133C, 5),
    (0x11340, 0x11340, 5),
    (0x11366, 0x1136C, 5),
    (0x11370, 0x11374, 5),
    (0x11438, 0x1143F, 5),
    (0x11442, 0x11444, 5),
    (0x11446, 0x11446, 5),
    (0x1145E, 0x1145E, 5),
    (0x114B3, 0x114B8, 5),
    (0x114BA, 0x114BA, 5),
    (0x114BF, 0x114C0, 5),
    (0x114C2, 0x114C3, 5),
    (0x115B2, 0x115B5, 5),
    (0x115BC, 0x115BD, 5),
    (0x115BF, 0x115C0, 5),
    (0x115DC, 0x115DD, 5),
    (0x11633, 0x1163A, 5),
    (0x1163D, 0x1163D, 5),
    (0x1163F, 0x11640, 5),
    (0x116AB, 0x116AB, 5),
    (0x116AD, 0x116AD, 5),
    (0x116B0, 0x116B5, 5),
    (0x116B7, 0x116B7, 5),
    (0x1171D, 0x1171F, 5),
    (0x11722, 0x11725, 5),
    (0x11727, 0x1172B, 5),
    (0x1182F, 0x11837, 5),
    (0x11839, 0x1183A, 5),
    (0x1193B, 0x1193C, 5),
    (0x1193E, 0x1193E, 5),
    (0x11943, 0x11943, 5),
    (0x119D4, 0x119D7, 5),
    (0x119DA, 0x119DB, 5),
    (0x119E0, 0x119E0, 5),
    (0x11A01, 0x11A06, 5),
    (0x11A09, 0x11A0A, 5),
    (0x11A33, 0x11A38, 5),
    (0x11A3B, 0x11A3E, 5),
    (0x11A47, 0x11A47, 5),
    (0x11A51, 0x11A56, 5),
    (0x11A59, 0x11A5B, 5),
    (0x11A8A, 0x11A96, 5),
    (0x11A98, 0x11A99, 5),
    (0x11C30, 0x11C36, 5),
    (0x11C38, 0x11C3D, 5),
    (0x11C3F, 0x11C3F, 5),
    (0x11C92, 0x11CA7, 5),
    (0x11CAA, 0x11CB0, 5),
    (0x11CB2, 0x11CB3, 5),
    (0x11CB5, 0x11CB6, 5),
    (0x11D31, 0x11D36, 5),
    (0x11D3A, 0x11D3A, 5),
    (0x11D3C, 0x11D3D, 5),
    (0x11D3F, 0x11D45, 5),
    (0x11D47, 0x11D47, 5),
    (0x11D90, 0x11D91, 5),
    (0x11D95, 0x11D95, 5),
    (0x11D97, 0x11D97, 5),
    (0x11EF3, 0x11EF4, 5),
    (0x16AF0, 0x16AF4, 5),
    (0x16B30, 0x16B36, 5),
    (0x16F4F, 0x16F4F, 5),
    (0x16F8F, 0x16F92, 5),
    (0x16FE4, 0x16FE4, 5),
    (0x1BC9D, 0x1BC9E, 5),
    (0x1CF00, 0x1CF2D, 5),
    (0x1CF30, 0x1CF46, 5),
    (0x1D167, 0x1D169, 5),
    (0x1D17B, 0x1D182, 5),
    (0x1D185, 0x1D18B, 5),
    (0x1D1AA, 0x1D1AD, 5),
    (0x1D242, 0x1D244, 5),
    (0x1DA00, 0x1DA36, 5),
    (0x1DA3B, 0x1DA6C, 5),
    (0x1DA75, 0x1DA75, 5),
    (0x1DA84, 0x1DA84, 5),
    (0x1DA9B, 0x1DA9F, 5),
    (0x1DAA1, 0x1DAAF, 5),
    (0x1E000, 0x1E006, 5),
    (0x1E008, 0x1E018, 5),
    (0x1E01B, 0x1E021, 5),
    (0x1E023, 0x1E024, 5),
    (0x1E026, 0x1E02A, 5),
    (0x1E130, 0x1E136, 5),
    (0x1E2AE, 0x1E2AE, 5),
    (0x1E2EC, 0x1E2EF, 5),
    (0x1E8D0, 0x1E8D6, 5),
    (0x1E900, 0x1E943, 2),
    (0x1E944, 0x1E94A, 5),
    (0xFE940, 0xFE940, 5),
];

fn classifier(c: u32) -> Option<u8> {
    let i = CLASSIFIERS.partition_point(|r| r.1 < c);
    CLASSIFIERS.get(i).filter(|r| r.0 <= c).map(|r| r.2)
}

/// The scripts `methods` has no port for here.
pub fn unsupported(script: &str) -> bool {
    matches!(
        script,
        "mong"
            | "deva"
            | "dev2"
            | "beng"
            | "bng2"
            | "gujr"
            | "gjr2"
            | "guru"
            | "gur2"
            | "knda"
            | "knd2"
            | "mlym"
            | "mlm2"
            | "orya"
            | "ory2"
            | "taml"
            | "tml2"
            | "telu"
            | "tel2"
            | "sinh"
            | "khmr"
            | "mym2"
            | "mymr"
    )
}

/// `analyzeprocessor` (font-ota.lua:222-237) for the font's script.
pub fn analyze(list: &mut List, head: Option<Ix>, font: usize, face: &Face, script: &str) {
    match script {
        "latn" => setstate(list, head, font, face),
        "arab" | "syrc" | "mand" | "nko" => arab(list, head, font),
        _ => {}
    }
}

fn endofmath(list: &List, start: Ix) -> Ix {
    let mut n = list.nodes[start].next;
    while let Some(i) = n {
        if let Kind::Math { subtype: 1 } = list.nodes[i].kind {
            return i;
        }
        n = list.nodes[i].next;
    }
    start
}

/// `analyzers.setstate` (font-ota.lua:132-201).
fn setstate(list: &mut List, head: Option<Ix>, font: usize, face: &Face) {
    let (mut first, mut last): (Option<Ix>, Option<Ix>) = (None, None);
    let mut n = 0;
    let mut current = head;
    let finish = |list: &mut List, first: Option<Ix>, last: Option<Ix>| {
        if first.is_some() && first == last {
            list.set_state(last.unwrap(), Some(S_ISOL));
        } else if let Some(l) = last {
            list.set_state(l, Some(S_FINA));
        }
    };
    while let Some(cur) = current {
        match list.ischar(Some(cur), Some(font)) {
            Ok(Some(c)) if list.state(cur).is_none() => match face.descriptions.get(&c) {
                Some(d) => {
                    if d.class.as_deref() == Some("mark") {
                        list.set_state(cur, Some(S_MARK));
                    } else if n == 0 {
                        first = Some(cur);
                        last = Some(cur);
                        n = 1;
                        list.set_state(cur, Some(S_INIT));
                    } else {
                        last = Some(cur);
                        n += 1;
                        list.set_state(cur, Some(S_MEDI));
                    }
                }
                None => {
                    finish(list, first, last);
                    first = None;
                    last = None;
                    n = 0;
                }
            },
            // another font, or (falling through to the Lua's last branch) a
            // glyph that already has a state
            Ok(_) => {
                finish(list, first, last);
                first = None;
                last = None;
                n = 0;
            }
            Err(()) if list.is_disc(Some(cur)) => {
                list.set_state(cur, Some(S_MEDI));
                last = Some(cur);
            }
            Err(()) => {
                finish(list, first, last);
                first = None;
                last = None;
                n = 0;
                if matches!(list.nodes[cur].kind, Kind::Math { .. }) {
                    current = Some(endofmath(list, cur));
                }
            }
        }
        current = current.and_then(|c| list.nodes[c].next);
    }
    finish(list, first, last);
}

/// `methods.arab` (font-ota.lua:321-457). The Lua's `s_error` is an
/// undefined name, so "setting" it leaves the state alone.
fn arab(list: &mut List, head: Option<Ix>, font: usize) {
    let (mut first, mut last): (Option<Ix>, Option<Ix>) = (None, None);
    let (mut c_first, mut c_last): (Option<u8>, Option<u8>) = (None, None);
    // what every non-joining break does with the run so far
    fn close(
        list: &mut List,
        first: &mut Option<Ix>,
        last: &mut Option<Ix>,
        c_first: Option<u8>,
        c_last: Option<u8>,
    ) {
        if let Some(l) = *last {
            if matches!(c_last, Some(S_MEDI) | Some(S_FINA)) {
                list.set_state(l, Some(S_FINA));
            }
            *first = None;
            *last = None;
        } else if let Some(f) = *first {
            if matches!(c_first, Some(S_MEDI) | Some(S_FINA)) {
                list.set_state(f, Some(S_ISOL));
            }
            *first = None;
        }
    }
    let mut current = head;
    while let Some(cur) = current {
        match list.ischar(Some(cur), Some(font)) {
            Ok(Some(c)) if list.state(cur).is_none() => match classifier(c) {
                None => close(list, &mut first, &mut last, c_first, c_last),
                Some(S_MARK) => list.set_state(cur, Some(S_MARK)),
                Some(S_ISOL) => {
                    close(list, &mut first, &mut last, c_first, c_last);
                    list.set_state(cur, Some(S_ISOL));
                }
                Some(S_MEDI) => {
                    if first.is_some() {
                        last = Some(cur);
                        c_last = Some(S_MEDI);
                        list.set_state(cur, Some(S_MEDI));
                    } else {
                        list.set_state(cur, Some(S_INIT));
                        first = Some(cur);
                        c_first = Some(S_MEDI);
                    }
                }
                Some(S_FINA) => {
                    if let Some(l) = last {
                        if list.state(l) != Some(S_INIT) {
                            list.set_state(l, Some(S_MEDI));
                        }
                        list.set_state(cur, Some(S_FINA));
                        first = None;
                        last = None;
                    } else if first.is_some() {
                        list.set_state(cur, Some(S_FINA));
                        first = None;
                    } else {
                        list.set_state(cur, Some(S_ISOL));
                    }
                }
                // S_REST, and the init class no classifier has
                Some(_) => {
                    list.set_state(cur, Some(S_REST));
                    close(list, &mut first, &mut last, c_first, c_last);
                }
            },
            _ => {
                close(list, &mut first, &mut last, c_first, c_last);
                if matches!(list.nodes[cur].kind, Kind::Math { .. }) {
                    current = Some(endofmath(list, cur));
                }
            }
        }
        current = current.and_then(|c| list.nodes[c].next);
    }
    close(list, &mut first, &mut last, c_first, c_last);
}
