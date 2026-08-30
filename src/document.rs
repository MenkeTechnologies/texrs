//! `Texrs.toml`: a document that is more than one file, ported in shape from
//! tectonic's `docmodel`.
//!
//! A TeX document of any size is several files, and today texrs compiles one.
//! Tectonic answers that with a file at the root of the document saying what
//! the document is made of and what should come out of it; the engine stays a
//! thing that compiles what it is handed. That division is why this is driver
//! work and not expander work — nothing here touches the mouth.
//!
//! ```toml
//! [doc]
//! name = "thesis"
//! inputs = ["macros.tex", "body.tex"]
//!
//! [[output]]
//! name = "default"
//! type = "messages"
//! ```
//!
//! The inputs are read in order through [`crate::io`]'s provider stack and
//! compiled as one document, which is what a TeX file that has not yet met
//! `\input` can honestly do: the engine sees one source, and the record of what
//! was read says which files it came from and what each of them hashed to.
//!
//! What is deliberately not carried over from tectonic's schema: `bundle` (there
//! is nothing to fetch — texrs has no package universe), `tex_format` (there is
//! one format, INITEX's), `shell_escape` and `synctex` (no shell, no page).
//! Those fields would describe machinery that does not exist.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::io::{InputProvider, OpenResult, ProviderStack};

/// The file at the root of a document.
pub const DOCUMENT_FILE: &str = "Texrs.toml";

/// Where a build writes what it produced.
pub const BUILD_DIR: &str = "build";

/// What a profile asks the engine for.
///
/// Each is a pipeline texrs already has, named: an output type here exists
/// because the binary can produce it, not because tectonic has one like it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputType {
    /// The `\message` stream — what an ordinary run prints.
    Messages,
    /// The mouth's token stream, before anything expands.
    Tokens,
    /// The fusevm bytecode the document lowers to.
    Disasm,
}

impl OutputType {
    pub fn extension(self) -> &'static str {
        match self {
            OutputType::Messages => "txt",
            OutputType::Tokens => "tokens",
            OutputType::Disasm => "disasm",
        }
    }
}

/// One way of building the document.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Output {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: OutputType,
}

/// The `[doc]` table.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DocSection {
    pub name: String,
    /// The files that make up the document, in the order they are read.
    #[serde(default = "default_inputs")]
    pub inputs: Vec<String>,
    /// Extra directories searched for an input, after the document's own.
    #[serde(default)]
    pub extra_paths: Vec<PathBuf>,
    /// Anything the user wants to keep here. texrs does not read it.
    #[serde(default)]
    pub metadata: Option<toml::Value>,
}

fn default_inputs() -> Vec<String> {
    vec!["index.tex".to_string()]
}

/// A whole `Texrs.toml`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DocumentFile {
    pub doc: DocSection,
    #[serde(rename = "output", default)]
    pub outputs: Vec<Output>,
}

/// A document on disk: its file, and where it lives.
#[derive(Clone, Debug)]
pub struct Document {
    /// The directory holding `Texrs.toml`.
    pub src_dir: PathBuf,
    /// Where builds write, `build/` under the source directory.
    pub build_dir: PathBuf,
    pub file: DocumentFile,
}

/// What a build produced.
#[derive(Debug)]
pub struct Built {
    pub profile: String,
    /// Where the output was written.
    pub path: PathBuf,
    /// Every input the build read, in order, with its digest.
    pub inputs: Vec<crate::io::InputRecord>,
}

impl Document {
    /// Read the document rooted at `dir`.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, String> {
        let src_dir = dir.as_ref().to_path_buf();
        let path = src_dir.join(DOCUMENT_FILE);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Self::parse(&src_dir, &text)
    }

    /// Read `text` as the document file for `src_dir`.
    pub fn parse(src_dir: impl AsRef<Path>, text: &str) -> Result<Self, String> {
        let file: DocumentFile =
            toml::from_str(text).map_err(|e| format!("{DOCUMENT_FILE}: {e}"))?;
        if file.doc.name.trim().is_empty() {
            return Err(format!("{DOCUMENT_FILE}: the document needs a name"));
        }
        if file.doc.inputs.is_empty() {
            return Err(format!("{DOCUMENT_FILE}: the document needs an input"));
        }
        if file.outputs.is_empty() {
            return Err(format!("{DOCUMENT_FILE}: the document needs an output"));
        }
        // Two profiles under one name is a build whose result depends on which
        // one was found first.
        let mut seen = BTreeMap::new();
        for output in &file.outputs {
            if seen.insert(output.name.clone(), ()).is_some() {
                return Err(format!(
                    "{DOCUMENT_FILE}: two outputs are named {:?}",
                    output.name
                ));
            }
        }
        let src_dir = src_dir.as_ref().to_path_buf();
        Ok(Document {
            build_dir: src_dir.join(BUILD_DIR),
            src_dir,
            file,
        })
    }

    /// The document containing `start`, found by walking up as tectonic does —
    /// so a build works from anywhere inside the tree, not only its root.
    pub fn find_from(start: impl AsRef<Path>) -> Result<Self, String> {
        let mut at = start.as_ref().to_path_buf();
        if at.is_file() {
            at.pop();
        }
        loop {
            if at.join(DOCUMENT_FILE).is_file() {
                return Self::open(&at);
            }
            if !at.pop() {
                return Err(format!(
                    "no {DOCUMENT_FILE} here or in any directory above it"
                ));
            }
        }
    }

    /// The profile named, or the only one, or `default`.
    pub fn profile(&self, name: Option<&str>) -> Result<&Output, String> {
        match name {
            Some(name) => self
                .file
                .outputs
                .iter()
                .find(|o| o.name == name)
                .ok_or_else(|| {
                    let known: Vec<&str> =
                        self.file.outputs.iter().map(|o| o.name.as_str()).collect();
                    format!("no output named {name:?}; this document has {known:?}")
                }),
            None if self.file.outputs.len() == 1 => Ok(&self.file.outputs[0]),
            None => self
                .file
                .outputs
                .iter()
                .find(|o| o.name == "default")
                .ok_or_else(|| {
                    "this document has several outputs and none is `default`; \
                     name one with --profile"
                        .to_string()
                }),
        }
    }

    /// Read every input in order, and say what was read.
    ///
    /// The inputs are concatenated with a newline between them, because a file
    /// that does not end in one would otherwise join the next file's first line
    /// — and in TeX that changes what the line means.
    pub fn assemble(&self) -> Result<(String, Vec<crate::io::InputRecord>), String> {
        let mut roots = vec![self.src_dir.clone()];
        roots.extend(self.file.doc.extra_paths.iter().map(|p| {
            if p.is_absolute() {
                p.clone()
            } else {
                self.src_dir.join(p)
            }
        }));
        let mut stack = ProviderStack::new();
        stack.push(Box::new(crate::io::FilesystemProvider::with_roots(roots)));

        let mut source = String::new();
        for name in &self.file.doc.inputs {
            match stack.input_open(name) {
                OpenResult::Ok(input) => {
                    source.push_str(&input.content);
                    if !input.content.ends_with('\n') {
                        source.push('\n');
                    }
                }
                OpenResult::NotAvailable => {
                    return Err(format!("input {name:?} is not in this document"))
                }
                OpenResult::Err(e) => return Err(format!("input {name:?}: {e}")),
            }
        }
        Ok((source, stack.inputs_read().to_vec()))
    }

    /// Build one profile, writing its output under `build/`.
    pub fn build(&self, profile: Option<&str>) -> Result<Built, String> {
        let output = self.profile(profile)?.clone();
        let (source, inputs) = self.assemble()?;
        let body = match output.kind {
            OutputType::Messages => crate::run_messages(&source).map_err(|e| e.0)?,
            OutputType::Tokens => {
                let cats = crate::catcode::CatTable::new();
                let mut lexer = crate::lexer::Lexer::new(&source);
                let mut out = String::new();
                while let Some(token) = lexer.next_token(&cats) {
                    out.push_str(&format!("{token:?}\n"));
                }
                out
            }
            OutputType::Disasm => crate::compile(&source).map_err(|e| e.0)?.disassemble(),
        };
        std::fs::create_dir_all(&self.build_dir)
            .map_err(|e| format!("cannot make {}: {e}", self.build_dir.display()))?;
        let path = self.build_dir.join(format!(
            "{}.{}",
            self.file.doc.name,
            output.kind.extension()
        ));
        let mut text = body;
        if !text.ends_with('\n') {
            text.push('\n');
        }
        std::fs::write(&path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        Ok(Built {
            profile: output.name,
            path,
            inputs,
        })
    }
}

impl Document {
    /// The document's shape, as `-X show` prints it: where it is, what it is
    /// made of, and what it can produce.
    ///
    /// Everything here is read from the document file and the inputs
    /// themselves, so what it prints is what a build would use — not a
    /// remembered summary that can be out of date.
    pub fn show(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("name       {}\n", self.file.doc.name));
        out.push_str(&format!("source     {}\n", self.src_dir.display()));
        out.push_str(&format!("build      {}\n", self.build_dir.display()));
        for path in &self.file.doc.extra_paths {
            out.push_str(&format!("extra path {}\n", path.display()));
        }
        out.push_str("\ninputs\n");
        match self.assemble() {
            Ok((_, read)) => {
                for record in read {
                    // Twelve characters of the digest: enough to tell two
                    // versions of a file apart at a glance, and not so much
                    // that the name is pushed off the line.
                    out.push_str(&format!(
                        "  {}  {}\n",
                        &record.digest[..12.min(record.digest.len())],
                        record.name
                    ));
                }
            }
            Err(e) => out.push_str(&format!("  (cannot be read: {e})\n")),
        }
        out.push_str("\noutputs\n");
        for output in &self.file.outputs {
            out.push_str(&format!(
                "  {:<12} {:?} → {}.{}\n",
                output.name,
                output.kind,
                self.file.doc.name,
                output.kind.extension()
            ));
        }
        out
    }

    /// Build a profile and hand back what it produced, writing nothing.
    ///
    /// tectonic's `-X dump` exists so an intermediate can be looked at without
    /// hunting for it in the build directory; this is the same, and it is also
    /// what a pipeline wants — `texrs -X dump | grep` needs the bytes on
    /// stdout, not a path to them.
    pub fn dump(&self, profile: Option<&str>) -> Result<String, String> {
        let output = self.profile(profile)?.clone();
        let (source, _) = self.assemble()?;
        match output.kind {
            OutputType::Messages => crate::run_messages(&source).map_err(|e| e.0),
            OutputType::Tokens => {
                let cats = crate::catcode::CatTable::new();
                let mut lexer = crate::lexer::Lexer::new(&source);
                let mut out = String::new();
                while let Some(token) = lexer.next_token(&cats) {
                    out.push_str(&format!("{token:?}\n"));
                }
                Ok(out)
            }
            OutputType::Disasm => Ok(crate::compile(&source).map_err(|e| e.0)?.disassemble()),
        }
    }
}

/// How often [`Document::watch`] looks, when the caller does not say.
pub const WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

impl Document {
    /// Rebuild whenever an input changes, until `stop` says otherwise.
    ///
    /// Change is decided by the digests a build already records, not by mtimes:
    /// a file touched but not edited is not a rebuild, and an editor that
    /// writes through a temporary file — which most do, so the mtime jumps
    /// while the content does not — does not cause one either. The cost is a
    /// read per input per tick, which for a document is nothing next to
    /// compiling it.
    ///
    /// Polling rather than the operating system's watch API is deliberate:
    /// a poll is fifty lines that behave the same on every platform, where the
    /// APIs disagree about directories, renames and network mounts, and the
    /// crates that paper over them are exactly the kind of dependency that
    /// stops building three years from now.
    pub fn watch(
        &self,
        profile: Option<&str>,
        interval: std::time::Duration,
        status: &mut dyn crate::status::StatusBackend,
        stop: &dyn Fn() -> bool,
    ) -> Result<usize, String> {
        // The profile is resolved once: a name that is not there should fail
        // now rather than on the first change, when the user has looked away.
        let profile = self.profile(profile)?.name.clone();
        let mut builds = 0usize;
        let mut last: Option<Vec<crate::io::InputRecord>> = None;

        while !stop() {
            let (_, current) = match self.assemble() {
                Ok(v) => v,
                Err(e) => {
                    // A missing input is a state to wait out, not a reason to
                    // stop watching: it is what a rename looks like halfway.
                    status.warning(&e);
                    std::thread::sleep(interval);
                    continue;
                }
            };
            let changed = last.as_ref().is_none_or(|previous| {
                previous.len() != current.len()
                    || previous
                        .iter()
                        .zip(&current)
                        .any(|(a, b)| a.digest != b.digest || a.name != b.name)
            });
            if changed {
                match self.build(Some(&profile)) {
                    Ok(built) => {
                        builds += 1;
                        status.note(&format!(
                            "built {} from {} input(s) → {}",
                            built.profile,
                            built.inputs.len(),
                            built.path.display()
                        ));
                    }
                    // A document that does not compile is the normal state of
                    // one being edited; report it and keep watching.
                    Err(e) => status.error(&e),
                }
                last = Some(current);
            }
            std::thread::sleep(interval);
        }
        Ok(builds)
    }
}

/// Write a new document into `dir`: the file, and an `index.tex` that runs.
///
/// The starting document sets the category codes INITEX leaves ordinary,
/// because one that did not would fail on its first group — the same reason the
/// editor templates do it.
pub fn scaffold(dir: impl AsRef<Path>, name: &str) -> Result<PathBuf, String> {
    let dir = dir.as_ref();
    let path = dir.join(DOCUMENT_FILE);
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    let document = format!(
        "# What this document is made of, and what comes out of it.\n\
         # `texrs -X build` reads this; see texrs(1).\n\
         [doc]\n\
         name = {name:?}\n\
         inputs = [\"index.tex\"]\n\
         \n\
         [[output]]\n\
         name = \"default\"\n\
         type = \"messages\"\n"
    );
    std::fs::write(&path, document).map_err(|e| format!("cannot write {}: {e}", path.display()))?;

    let index = dir.join("index.tex");
    if !index.exists() {
        let body = "% INITEX leaves `{` and `}` ordinary, so a document that wants\n\
                    % groups says so first.\n\
                    \\catcode`\\{=1 \\catcode`\\}=2\n\
                    \n\
                    \\message{hello from texrs}\n\
                    \\end\n";
        std::fs::write(&index, body)
            .map_err(|e| format!("cannot write {}: {e}", index.display()))?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("texrs_doc_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const HELLO: &str = "\\catcode`\\{=1 \\catcode`\\}=2 \\message{HI}\n\\end\n";

    #[test]
    fn a_document_file_says_what_it_is_made_of() {
        let dir = scratch("parse");
        let doc = Document::parse(
            &dir,
            r#"
            [doc]
            name = "thesis"
            inputs = ["macros.tex", "body.tex"]

            [[output]]
            name = "default"
            type = "messages"

            [[output]]
            name = "bytecode"
            type = "disasm"
            "#,
        )
        .expect("parses");
        assert_eq!(doc.file.doc.name, "thesis");
        assert_eq!(doc.file.doc.inputs, vec!["macros.tex", "body.tex"]);
        assert_eq!(doc.build_dir, dir.join("build"));
        assert_eq!(
            doc.profile(Some("bytecode")).unwrap().kind,
            OutputType::Disasm
        );
        // With several outputs and no name, `default` is the one.
        assert_eq!(doc.profile(None).unwrap().name, "default");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_document_that_cannot_be_built_is_refused_rather_than_half_read() {
        let dir = scratch("refuse");
        let cases = [
            ("[doc]\nname = \"\"\ninputs = [\"a.tex\"]\n[[output]]\nname=\"d\"\ntype=\"messages\"\n", "needs a name"),
            ("[doc]\nname = \"x\"\ninputs = []\n[[output]]\nname=\"d\"\ntype=\"messages\"\n", "needs an input"),
            ("[doc]\nname = \"x\"\n", "needs an output"),
            (
                "[doc]\nname=\"x\"\n[[output]]\nname=\"d\"\ntype=\"messages\"\n[[output]]\nname=\"d\"\ntype=\"tokens\"\n",
                "two outputs are named",
            ),
            ("[doc]\nname=\"x\"\n[[output]]\nname=\"d\"\ntype=\"pdf\"\n", "unknown variant"),
        ];
        for (text, expected) in cases {
            let err = Document::parse(&dir, text).unwrap_err();
            assert!(err.contains(expected), "{text:?} gave {err:?}");
        }
        // A named profile that is not there says what is.
        let doc = Document::parse(
            &dir,
            "[doc]\nname=\"x\"\n[[output]]\nname=\"only\"\ntype=\"messages\"\n",
        )
        .unwrap();
        let err = doc.profile(Some("nope")).unwrap_err();
        assert!(
            err.contains("no output named") && err.contains("only"),
            "{err}"
        );
        // One output and no name needs no `default`.
        assert_eq!(doc.profile(None).unwrap().name, "only");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_document_is_found_by_walking_up() {
        let dir = scratch("find");
        let nested = dir.join("chapters/one");
        std::fs::create_dir_all(&nested).unwrap();
        scaffold(&dir, "thesis").unwrap();

        let found = Document::find_from(&nested).expect("found from below");
        assert_eq!(found.file.doc.name, "thesis");
        assert_eq!(found.src_dir, dir);
        // A file works as well as a directory, since that is what an editor has.
        let found = Document::find_from(dir.join("index.tex")).expect("found from a file");
        assert_eq!(found.src_dir, dir);
        // Nowhere to find one is an error that says so.
        let empty = scratch("find_none");
        assert!(Document::find_from(&empty)
            .unwrap_err()
            .contains("no Texrs.toml"));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&empty);
    }

    #[test]
    fn several_inputs_are_read_in_order_and_recorded() {
        let dir = scratch("assemble");
        std::fs::write(dir.join("macros.tex"), "\\catcode`\\{=1 \\catcode`\\}=2\n").unwrap();
        // No trailing newline: the join has to add one, or `\def` would run
        // into the next file's first line.
        std::fs::write(dir.join("body.tex"), "\\message{FROM-BODY}\n\\end").unwrap();
        let doc = Document::parse(
            &dir,
            "[doc]\nname=\"multi\"\ninputs=[\"macros.tex\",\"body.tex\"]\n\
             [[output]]\nname=\"default\"\ntype=\"messages\"\n",
        )
        .unwrap();

        let (source, read) = doc.assemble().expect("assembles");
        assert!(source.starts_with("\\catcode"), "{source}");
        assert!(source.contains("FROM-BODY"), "{source}");
        assert!(source.ends_with('\n'), "the join ends the last line");
        assert_eq!(read.len(), 2, "both inputs are recorded");
        assert_eq!(read[0].name, "macros.tex");
        assert_eq!(read[1].digest.len(), 64);

        // An input that is not there names itself.
        let missing = Document::parse(
            &dir,
            "[doc]\nname=\"x\"\ninputs=[\"nope.tex\"]\n\
             [[output]]\nname=\"default\"\ntype=\"messages\"\n",
        )
        .unwrap();
        assert!(missing.assemble().unwrap_err().contains("nope.tex"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_build_writes_what_the_profile_asked_for() {
        let dir = scratch("build");
        std::fs::write(dir.join("index.tex"), HELLO).unwrap();
        let doc = Document::parse(
            &dir,
            "[doc]\nname=\"doc\"\n\
             [[output]]\nname=\"default\"\ntype=\"messages\"\n\
             [[output]]\nname=\"tok\"\ntype=\"tokens\"\n\
             [[output]]\nname=\"bytes\"\ntype=\"disasm\"\n",
        )
        .unwrap();

        let built = doc.build(None).expect("builds");
        assert_eq!(built.profile, "default");
        assert_eq!(built.path, dir.join("build/doc.txt"));
        assert_eq!(std::fs::read_to_string(&built.path).unwrap().trim(), "HI");
        assert_eq!(built.inputs.len(), 1, "what it read is reported");

        let tokens = doc.build(Some("tok")).expect("builds");
        assert!(std::fs::read_to_string(&tokens.path)
            .unwrap()
            .contains("Cs("));
        let disasm = doc.build(Some("bytes")).expect("builds");
        // A listing is one instruction per line, addressed from zero.
        let listing = std::fs::read_to_string(&disasm.path).unwrap();
        assert!(listing.starts_with("0000"), "{listing}");
        assert!(listing.contains("LoadInt"), "{listing}");

        // Each profile writes its own file, so one does not overwrite another.
        assert_ne!(built.path, tokens.path);
        assert_ne!(tokens.path, disasm.path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn showing_a_document_reads_it_rather_than_remembering_it() {
        let dir = scratch("show");
        std::fs::write(dir.join("index.tex"), HELLO).unwrap();
        let doc = Document::parse(
            &dir,
            "[doc]\nname=\"shown\"\n\
             [[output]]\nname=\"default\"\ntype=\"messages\"\n\
             [[output]]\nname=\"bytes\"\ntype=\"disasm\"\n",
        )
        .unwrap();

        let shown = doc.show();
        assert!(shown.contains("name       shown"), "{shown}");
        assert!(shown.contains(&dir.display().to_string()), "{shown}");
        assert!(shown.contains("index.tex"), "{shown}");
        assert!(
            shown.contains("default") && shown.contains("bytes"),
            "{shown}"
        );
        assert!(
            shown.contains("shown.disasm"),
            "the file each profile writes: {shown}"
        );

        // The digest shown is the file's, so editing it changes what is shown.
        let before = shown.clone();
        std::fs::write(dir.join("index.tex"), "\\message{OTHER}\n\\end\n").unwrap();
        assert_ne!(doc.show(), before, "it reads the input, not a memory of it");

        // A document whose input has gone says so instead of pretending.
        std::fs::remove_file(dir.join("index.tex")).unwrap();
        assert!(doc.show().contains("cannot be read"), "{}", doc.show());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dumping_produces_what_a_build_would_write_without_writing_it() {
        let dir = scratch("dump");
        std::fs::write(dir.join("index.tex"), HELLO).unwrap();
        let doc = Document::parse(
            &dir,
            "[doc]\nname=\"d\"\n\
             [[output]]\nname=\"default\"\ntype=\"messages\"\n\
             [[output]]\nname=\"tok\"\ntype=\"tokens\"\n",
        )
        .unwrap();

        assert_eq!(doc.dump(None).unwrap().trim(), "HI");
        assert!(doc.dump(Some("tok")).unwrap().contains("Cs("));
        assert!(
            !dir.join("build").exists(),
            "a dump writes nothing: the build directory was not made"
        );

        // What it dumps is what a build writes, so the two cannot disagree.
        let built = doc.build(None).unwrap();
        assert_eq!(
            std::fs::read_to_string(&built.path).unwrap().trim(),
            doc.dump(None).unwrap().trim()
        );
        assert!(doc
            .dump(Some("nope"))
            .unwrap_err()
            .contains("no output named"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn watching_rebuilds_when_an_input_changes_and_not_otherwise() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;

        let dir = scratch("watch");
        std::fs::write(dir.join("index.tex"), HELLO).unwrap();
        let doc = Document::parse(
            &dir,
            "[doc]\nname=\"w\"\n[[output]]\nname=\"default\"\ntype=\"messages\"\n",
        )
        .unwrap();

        // Tick 0 builds (nothing has been built yet); tick 1 sees no change;
        // tick 2 sees the edit; the fourth tick stops the loop.
        let ticks = AtomicUsize::new(0);
        let edited = dir.join("index.tex");
        let stop = || {
            let n = ticks.fetch_add(1, Ordering::SeqCst);
            if n == 2 {
                std::fs::write(
                    &edited,
                    "\\catcode`\\{=1 \\catcode`\\}=2 \\message{EDITED}\n\\end\n",
                )
                .unwrap();
            }
            n >= 4
        };
        let mut status = crate::status::CollectingStatus::new();
        let builds = doc
            .watch(None, Duration::from_millis(1), &mut status, &stop)
            .expect("watches");

        assert_eq!(builds, 2, "the first build, and the one the edit caused");
        let output = std::fs::read_to_string(dir.join("build/w.txt")).unwrap();
        assert_eq!(output.trim(), "EDITED", "the rebuild used the new content");
        assert_eq!(
            status
                .messages()
                .iter()
                .filter(|(level, _)| *level == crate::status::Level::Note)
                .count(),
            2,
            "one report per build: {:?}",
            status.messages()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn watching_survives_a_document_that_does_not_compile() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;

        let dir = scratch("watch_broken");
        std::fs::write(dir.join("index.tex"), "\\undefined@thing\n").unwrap();
        let doc = Document::parse(
            &dir,
            "[doc]\nname=\"w\"\n[[output]]\nname=\"default\"\ntype=\"messages\"\n",
        )
        .unwrap();

        let ticks = AtomicUsize::new(0);
        let fixed = dir.join("index.tex");
        let stop = || {
            let n = ticks.fetch_add(1, Ordering::SeqCst);
            if n == 1 {
                std::fs::write(&fixed, HELLO).unwrap();
            }
            n >= 3
        };
        let mut status = crate::status::CollectingStatus::new();
        doc.watch(None, Duration::from_millis(1), &mut status, &stop)
            .expect("keeps watching");

        // The broken state was reported and the loop carried on to the fix.
        assert!(status.had_error(), "the failure is reported");
        assert_eq!(
            std::fs::read_to_string(dir.join("build/w.txt"))
                .unwrap()
                .trim(),
            "HI",
            "and the document that compiles is built"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn watching_refuses_a_profile_that_is_not_there_before_it_starts() {
        let dir = scratch("watch_profile");
        std::fs::write(dir.join("index.tex"), HELLO).unwrap();
        let doc = Document::parse(
            &dir,
            "[doc]\nname=\"w\"\n[[output]]\nname=\"default\"\ntype=\"messages\"\n",
        )
        .unwrap();
        let mut status = crate::status::SilentStatus;
        let err = doc
            .watch(
                Some("nope"),
                std::time::Duration::from_millis(1),
                &mut status,
                &|| true,
            )
            .unwrap_err();
        assert!(err.contains("no output named"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_scaffolded_document_builds_as_it_stands() {
        let dir = scratch("scaffold");
        let path = scaffold(&dir, "fresh").expect("scaffolds");
        assert!(path.ends_with(DOCUMENT_FILE));
        assert!(dir.join("index.tex").is_file(), "and a document to build");

        let doc = Document::open(&dir).expect("reads back");
        let built = doc.build(None).expect("builds");
        assert_eq!(
            std::fs::read_to_string(&built.path).unwrap().trim(),
            "hello from texrs"
        );

        // Scaffolding over a document is refused rather than overwriting one.
        assert!(scaffold(&dir, "again")
            .unwrap_err()
            .contains("already exists"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
