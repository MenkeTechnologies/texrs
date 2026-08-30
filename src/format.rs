//! A format: the engine as a preamble left it, saved so the next run starts
//! there. Ported in shape from tectonic's `xetex_format`.
//!
//! TeX's own answer to "this macro file takes longer to read than the document
//! does" is a format: run the macros once, dump the engine, and start from the
//! dump ever after. `tex.web` calls it `\dump`; every LaTeX run loads one.
//!
//! What is dumped here is the state a preamble produces and nothing else — the
//! category codes, the meanings of control sequences, the count registers, the
//! escape character, and where the hidden scratch registers have got to. That
//! is a *compile-time* state, which is why a preamble may only define: if
//! lowering it produces run-time commands, there is nothing in a format to put
//! them in, and the caller compiles the whole document instead. A format that
//! silently dropped a `\message` in a macro file would be worse than no format.
//!
//! Names, not ids. `CsId` is a pointer into this process's intern table, so an
//! id means nothing to the next run; the dump stores the name and interns it
//! again on load. Getting that wrong would not fail — it would resolve to
//! whatever else had been interned into that slot.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::catcode::{Cat, CatTable};
use crate::expand::{Engine, Macro, Meaning};
use crate::token::{CsId, Token};

/// `TXFM`, big-endian, so a file says what it is in its first four bytes.
pub const FORMAT_MAGIC: u32 = 0x5458_464D;

/// Bumped whenever what is dumped changes shape. An older file is rebuilt.
pub const FORMAT_VERSION: u32 = 1;

/// One token, by name rather than by interned id.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
enum TokenRepr {
    Char(char, u8),
    Cs(String),
}

/// What a control sequence means, in the same terms.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
enum MeaningRepr {
    Macro {
        params: Vec<TokenRepr>,
        body: Vec<TokenRepr>,
    },
    Primitive(String),
    Char(char, u8),
    // Appended, not inserted: serde numbers a variant by position, and a dump
    // is only ever read back by the build that wrote it (see `Format::usable`),
    // so adding at the end cannot change how an existing one decodes.
    CharDef(i64),
    CountDef(i64),
}

/// The dump itself.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Format {
    magic: u32,
    version: u32,
    /// The texrs that wrote it. A format is only meaningful to the build that
    /// produced it, exactly as a `.fmt` is to its engine.
    texrs_version: String,
    escape: char,
    /// Only the codes that differ from INITEX's table, since most do not.
    catcodes: Vec<(u32, u8)>,
    counts: Vec<(i64, i64)>,
    meanings: Vec<(String, MeaningRepr)>,
    /// Where [`crate::lower::Lowerer::scratch_mark`] had got to.
    scratch_mark: i64,
}

impl Format {
    /// Capture `engine` as a preamble left it.
    pub fn capture(engine: &Engine, scratch_mark: i64) -> Self {
        let initex = CatTable::new();
        let mut catcodes = Vec::new();
        for code in 0u32..=0x10FFFF {
            let Some(ch) = char::from_u32(code) else {
                continue;
            };
            // Above the ASCII range nothing is assigned by INITEX and nothing
            // but a document can have changed it, so the scan stops where a
            // document's `\catcode` can still have reached.
            if code > 0xFF {
                break;
            }
            let cat = engine.cats.get(ch);
            if cat != initex.get(ch) {
                catcodes.push((code, cat_to_u8(cat)));
            }
        }
        let mut counts: Vec<(i64, i64)> = engine
            .count
            .iter()
            .filter(|(_, v)| **v != 0)
            .map(|(k, v)| (*k, *v))
            .collect();
        counts.sort_unstable();

        let mut meanings: Vec<(String, MeaningRepr)> = engine
            .meanings
            .iter()
            .map(|(id, meaning)| (id.name().to_string(), meaning_repr(meaning)))
            .collect();
        // Sorted so two captures of the same state are the same bytes: a
        // format that differed run to run could not be compared or cached.
        meanings.sort_by(|a, b| a.0.cmp(&b.0));

        Format {
            magic: FORMAT_MAGIC,
            version: FORMAT_VERSION,
            texrs_version: env!("CARGO_PKG_VERSION").to_string(),
            escape: engine.escape,
            catcodes,
            counts,
            meanings,
            scratch_mark,
        }
    }

    /// Put this state into `engine`, and say where the scratch counter was.
    pub fn apply(&self, engine: &mut Engine) -> i64 {
        engine.escape = self.escape;
        engine.cats = CatTable::new();
        for (code, cat) in &self.catcodes {
            if let Some(ch) = char::from_u32(*code) {
                engine.cats.set(ch, u8_to_cat(*cat));
            }
        }
        engine.count.clear();
        for (register, value) in &self.counts {
            engine.count.insert(*register, *value);
        }
        engine.meanings.clear();
        for (name, meaning) in &self.meanings {
            engine
                .meanings
                .insert(CsId::intern(name), meaning_of(meaning));
        }
        self.scratch_mark
    }

    /// Whether this build can use the format in `bytes`.
    fn usable(&self) -> bool {
        self.magic == FORMAT_MAGIC
            && self.version == FORMAT_VERSION
            && self.texrs_version == env!("CARGO_PKG_VERSION")
    }

    /// Write the format to `path`, through a temp file and a rename so a
    /// reader never sees half of one.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let bytes = bincode::serialize(self).map_err(|e| format!("format: {e}"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot make {}: {e}", parent.display()))?;
        }
        let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
        std::fs::write(&tmp, &bytes).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .map_err(|e| format!("cannot rename into {}: {e}", path.display()))
    }

    /// Read a format this build can use, or `None` — a missing, damaged or
    /// foreign file is a rebuild, never an error.
    pub fn load(path: &Path) -> Option<Format> {
        let bytes = std::fs::read(path).ok()?;
        let format: Format = bincode::deserialize(&bytes).ok()?;
        format.usable().then_some(format)
    }
}

fn cat_to_u8(cat: Cat) -> u8 {
    cat as u8
}

/// `tex.web` §207 numbers the categories, and so does this: the number is the
/// wire format, so a match rather than a transmute — a byte out of range is a
/// damaged file, not undefined behaviour.
fn u8_to_cat(n: u8) -> Cat {
    match n {
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
        13 => Cat::Active,
        14 => Cat::Comment,
        15 => Cat::Invalid,
        _ => Cat::Other,
    }
}

fn token_repr(token: &Token) -> TokenRepr {
    match token {
        Token::Char(c, cat) => TokenRepr::Char(*c, cat_to_u8(*cat)),
        Token::Cs(id) => TokenRepr::Cs(id.name().to_string()),
    }
}

fn token_of(repr: &TokenRepr) -> Token {
    match repr {
        TokenRepr::Char(c, cat) => Token::Char(*c, u8_to_cat(*cat)),
        TokenRepr::Cs(name) => Token::Cs(CsId::intern(name)),
    }
}

fn meaning_repr(meaning: &Meaning) -> MeaningRepr {
    match meaning {
        Meaning::Macro(m) => MeaningRepr::Macro {
            params: m.params.iter().map(token_repr).collect(),
            body: m.body.iter().map(token_repr).collect(),
        },
        Meaning::Primitive(id) => MeaningRepr::Primitive(id.name().to_string()),
        Meaning::Char(c, cat) => MeaningRepr::Char(*c, cat_to_u8(*cat)),
        Meaning::CharDef(v) => MeaningRepr::CharDef(*v),
        Meaning::CountDef(r) => MeaningRepr::CountDef(*r),
    }
}

fn meaning_of(repr: &MeaningRepr) -> Meaning {
    match repr {
        MeaningRepr::Macro { params, body } => Meaning::Macro(Macro {
            params: params.iter().map(token_of).collect(),
            body: body.iter().map(token_of).collect(),
        }),
        MeaningRepr::Primitive(name) => Meaning::Primitive(CsId::intern(name)),
        MeaningRepr::Char(c, cat) => Meaning::Char(*c, u8_to_cat(*cat)),
        MeaningRepr::CharDef(v) => Meaning::CharDef(*v),
        MeaningRepr::CountDef(r) => Meaning::CountDef(*r),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lower `src` and hand back the engine it left, with its scratch mark.
    fn preamble(src: &str) -> (Engine, i64, usize) {
        let mut lowerer = crate::lower::Lowerer::new();
        let cmds = lowerer.lower(src).expect("lowers");
        let mark = lowerer.scratch_mark();
        (lowerer.eng, mark, cmds.len())
    }

    /// A preamble that only defines: category codes, a macro, a `\let`.
    const MACROS: &str = "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6\n\
                          \\def\\greet#1{HELLO-#1}\n\\let\\hello\\greet\n";

    #[test]
    fn a_preamble_that_only_defines_lowers_to_nothing_a_format_could_lose() {
        // The premise the design rests on: what a macro file leaves behind is
        // state, not work, so a format can carry all of it.
        let (_, _, commands) = preamble(MACROS);
        assert_eq!(commands, 0, "definitions produce no run-time commands");

        // And the converse, which is why the document layer checks: a preamble
        // that DOES something lowers to commands, and those have nowhere to go
        // in a format. `\count7=42` is an assignment at run time, not a
        // compile-time fact — the engine's own register map is still empty
        // after lowering it.
        let (engine, _, commands) = preamble("\\count7=42\n");
        assert!(commands > 0, "an assignment lowers to run-time commands");
        assert_eq!(engine.count.get(&7), None, "which has not run yet");
    }

    #[test]
    fn what_a_preamble_left_is_what_a_format_puts_back() {
        let (engine, scratch, _) = preamble(MACROS);
        let format = Format::capture(&engine, scratch);

        let mut fresh = Engine::new();
        assert_ne!(fresh.cats.get('{'), engine.cats.get('{'), "before applying");
        let mark = format.apply(&mut fresh);

        assert_eq!(mark, scratch);
        assert_eq!(fresh.escape, engine.escape);
        assert_eq!(fresh.cats.get('{'), Cat::BeginGroup);
        assert_eq!(fresh.cats.get('}'), Cat::EndGroup);
        assert_eq!(fresh.cats.get('#'), Cat::Param);
        // A character the preamble never touched is INITEX's again.
        assert_eq!(fresh.cats.get('a'), Cat::Letter);
        assert_eq!(fresh.meanings.len(), engine.meanings.len());
        assert!(fresh.meanings.contains_key(&CsId::intern("greet")));
    }

    #[test]
    fn a_macro_survives_the_round_trip_with_its_parameters() {
        let (engine, scratch, _) = preamble(MACROS);
        let format = Format::capture(&engine, scratch);
        let mut fresh = Engine::new();
        format.apply(&mut fresh);

        let before = engine.meanings.get(&CsId::intern("greet")).unwrap();
        let after = fresh.meanings.get(&CsId::intern("greet")).unwrap();
        assert!(before == after, "the meaning is the same meaning");
        // And it is a macro with one parameter and a body, not an empty shell.
        match after {
            Meaning::Macro(m) => {
                assert!(!m.params.is_empty(), "the parameter text survived");
                assert!(!m.body.is_empty(), "so did the body");
            }
            _ => panic!("\\greet came back as something other than a macro"),
        }
    }

    #[test]
    fn a_format_is_the_same_bytes_for_the_same_state() {
        // Two captures of one preamble have to serialize identically, or a
        // format could not be compared, cached or checked.
        let (a, mark_a, _) = preamble(MACROS);
        let (b, mark_b, _) = preamble(MACROS);
        let first = bincode::serialize(&Format::capture(&a, mark_a)).unwrap();
        let second = bincode::serialize(&Format::capture(&b, mark_b)).unwrap();
        assert_eq!(first, second, "the dump does not depend on hash order");
    }

    #[test]
    fn a_format_this_build_cannot_use_is_a_rebuild_rather_than_an_error() {
        let dir = std::env::temp_dir().join(format!("texrs_fmt_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("preamble.fmt");

        let (engine, scratch, _) = preamble(MACROS);
        let format = Format::capture(&engine, scratch);
        format.save(&path).expect("saves");
        assert!(Format::load(&path).is_some(), "what it wrote, it reads");

        // A format from another version of texrs is not this build's bytecode
        // vocabulary, so it is ignored rather than trusted.
        let mut foreign = format.clone();
        foreign.texrs_version = "0.0.0-other".into();
        foreign.save(&path).unwrap();
        assert!(Format::load(&path).is_none());

        // So is one from another dump layout, and one that is not a format.
        let mut old = format.clone();
        old.version = FORMAT_VERSION + 1;
        old.save(&path).unwrap();
        assert!(Format::load(&path).is_none());
        std::fs::write(&path, b"not a format at all").unwrap();
        assert!(Format::load(&path).is_none());
        assert!(Format::load(&dir.join("nothing.fmt")).is_none());

        // Nothing is left behind by the writes.
        let strays: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("tmp"))
            .collect();
        assert!(strays.is_empty(), "{strays:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_category_number_survives_the_wire() {
        // The number IS the format, so each one has to come back as itself; a
        // byte outside the sixteen is a damaged file, read as Other rather
        // than as undefined behaviour.
        for cat in [
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
        ] {
            assert_eq!(u8_to_cat(cat_to_u8(cat)), cat, "{cat:?}");
        }
        assert_eq!(u8_to_cat(200), Cat::Other);
    }
}
