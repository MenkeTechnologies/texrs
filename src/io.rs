//! Where a document's bytes come from, ported in shape from Tectonic's
//! `io_base`.
//!
//! texrs reads one file today. `\input` reads many, and the moment it does the
//! question stops being "read this path" and becomes "find this name, wherever
//! it is" — a source that is a directory, a document the editor is holding
//! unsaved, or (as Tectonic does it) a bundle fetched from the network. Tectonic
//! answers that with a provider it can stack; this is the same shape, cut to
//! what texrs has.
//!
//! Two things are worth keeping from that design even at this size:
//!
//!  * **Not-found is not an error.** [`OpenResult`] separates "this provider
//!    does not have it" from "this provider has it and it is broken", because a
//!    search that treats a permission error as absence silently reads the wrong
//!    file — the next one along.
//!  * **What was read is recorded.** Every open is logged with the SHA-256 of
//!    what came back, so a caller can say exactly what a run consumed. That is
//!    what a cache keyed on more than one file needs: today's shard is valid
//!    while one document's mtime holds, and a document that `\input`s another
//!    is only as valid as both.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Where an input came from, kept with the digest so a record says what it is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputOrigin {
    /// A file, which can change under the run.
    Filesystem,
    /// Bytes handed over in memory — an editor's unsaved buffer, or a test.
    Memory,
}

/// One opened input, read whole.
///
/// TeX's mouth reads a file start to finish and the lexer here takes a `&str`,
/// so there is nothing to gain from a streaming handle and a great deal of
/// complication to lose. Tectonic streams because its engines seek.
#[derive(Clone, Debug)]
pub struct Input {
    /// The name it was opened under, as written in the document.
    pub name: String,
    /// Where it resolved to, when it came from disk.
    pub path: Option<PathBuf>,
    pub origin: InputOrigin,
    pub content: String,
}

/// The three answers to "open this", which are not two.
#[derive(Debug)]
pub enum OpenResult<T> {
    Ok(T),
    /// This provider does not have it. A stack tries the next one.
    NotAvailable,
    /// It is here and it cannot be read. A stack stops: reading past a broken
    /// file would silently use a different one.
    Err(std::io::Error),
}

impl<T> OpenResult<T> {
    pub fn is_not_available(&self) -> bool {
        matches!(self, OpenResult::NotAvailable)
    }

    /// The value, turning absence into an error for a caller that must have it.
    pub fn must_exist(self) -> std::io::Result<T> {
        match self {
            OpenResult::Ok(v) => Ok(v),
            OpenResult::NotAvailable => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "not found",
            )),
            OpenResult::Err(e) => Err(e),
        }
    }
}

/// Something that can find a document by name.
pub trait InputProvider {
    /// Open `name`, or say this provider does not have it.
    fn input_open(&mut self, name: &str) -> OpenResult<Input>;

    /// The document the run started from. A provider that has no notion of one
    /// says so, and the stack asks the next.
    fn input_open_primary(&mut self) -> OpenResult<Input> {
        OpenResult::NotAvailable
    }
}

/// A directory, plus the names TeX would try. Following tex, a name with no
/// extension is also tried with `.tex` appended, which is what makes
/// `\input macros` find `macros.tex`.
pub struct FilesystemProvider {
    roots: Vec<PathBuf>,
    primary: Option<PathBuf>,
}

impl FilesystemProvider {
    /// Search `root` for anything opened by name.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            roots: vec![root.into()],
            primary: None,
        }
    }

    /// Search these directories, in order.
    pub fn with_roots(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            roots: roots.into_iter().collect(),
            primary: None,
        }
    }

    /// The document the run started from. Its directory is searched first,
    /// since a document's `\input` names are written relative to itself.
    pub fn with_primary(mut self, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                self.roots.insert(0, dir.to_path_buf());
            }
        }
        self.primary = Some(path);
        self
    }

    /// The names to try for `name`, in tex's order.
    fn candidates(name: &str) -> Vec<String> {
        match Path::new(name).extension() {
            Some(_) => vec![name.to_string()],
            None => vec![name.to_string(), format!("{name}.tex")],
        }
    }

    fn read(path: &Path, name: &str) -> OpenResult<Input> {
        match std::fs::read_to_string(path) {
            Ok(content) => OpenResult::Ok(Input {
                name: name.to_string(),
                path: Some(path.to_path_buf()),
                origin: InputOrigin::Filesystem,
                content,
            }),
            // A file that is there and unreadable is an error, not an absence:
            // see the module comment.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => OpenResult::NotAvailable,
            Err(e) => OpenResult::Err(e),
        }
    }
}

impl InputProvider for FilesystemProvider {
    fn input_open(&mut self, name: &str) -> OpenResult<Input> {
        // An absolute path is not searched for: it says where it is.
        let as_given = Path::new(name);
        if as_given.is_absolute() {
            return Self::read(as_given, name);
        }
        for root in &self.roots {
            for candidate in Self::candidates(name) {
                let path = root.join(&candidate);
                if !path.exists() {
                    continue;
                }
                return Self::read(&path, name);
            }
        }
        OpenResult::NotAvailable
    }

    fn input_open_primary(&mut self) -> OpenResult<Input> {
        match self.primary.clone() {
            Some(path) => {
                let name = path.to_string_lossy().into_owned();
                Self::read(&path, &name)
            }
            None => OpenResult::NotAvailable,
        }
    }
}

/// Documents held in memory: what an editor has that the disk does not, and
/// what a test wants instead of a temporary directory.
#[derive(Default)]
pub struct MemoryProvider {
    files: HashMap<String, String>,
    primary: Option<String>,
}

impl MemoryProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: impl Into<String>, content: impl Into<String>) {
        self.files.insert(name.into(), content.into());
    }

    /// Hold `name` as the document the run starts from.
    pub fn set_primary(&mut self, name: impl Into<String>) {
        self.primary = Some(name.into());
    }
}

impl InputProvider for MemoryProvider {
    fn input_open(&mut self, name: &str) -> OpenResult<Input> {
        // The same `.tex` courtesy the filesystem gives, so a document reads
        // the same whether the editor has it or the disk does.
        let key = FilesystemProvider::candidates(name)
            .into_iter()
            .find(|c| self.files.contains_key(c));
        match key.and_then(|k| self.files.get(&k)) {
            Some(content) => OpenResult::Ok(Input {
                name: name.to_string(),
                path: None,
                origin: InputOrigin::Memory,
                content: content.clone(),
            }),
            None => OpenResult::NotAvailable,
        }
    }

    fn input_open_primary(&mut self) -> OpenResult<Input> {
        match self.primary.clone() {
            Some(name) => self.input_open(&name),
            None => OpenResult::NotAvailable,
        }
    }
}

/// What a run actually read: one entry per open, in order, with the digest of
/// what came back.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InputRecord {
    pub name: String,
    pub path: Option<PathBuf>,
    pub origin: InputOrigin,
    /// SHA-256 of the bytes, hex. Two runs that read the same digests read the
    /// same document, whatever the paths were.
    pub digest: String,
}

/// Providers tried in order, recording what each open returned.
///
/// The order is the policy: an editor's unsaved buffer before the file on disk,
/// the document's own directory before a shared one. A provider that does not
/// have a name defers; one that has it and cannot read it stops the search.
#[derive(Default)]
pub struct ProviderStack {
    providers: Vec<Box<dyn InputProvider>>,
    read: Vec<InputRecord>,
}

impl ProviderStack {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, provider: Box<dyn InputProvider>) {
        self.providers.push(provider);
    }

    /// Everything this run has opened, oldest first.
    pub fn inputs_read(&self) -> &[InputRecord] {
        &self.read
    }

    fn record(&mut self, input: &Input) {
        self.read.push(InputRecord {
            name: input.name.clone(),
            path: input.path.clone(),
            origin: input.origin,
            digest: digest_of(&input.content),
        });
    }
}

impl InputProvider for ProviderStack {
    fn input_open(&mut self, name: &str) -> OpenResult<Input> {
        for i in 0..self.providers.len() {
            match self.providers[i].input_open(name) {
                OpenResult::NotAvailable => continue,
                OpenResult::Ok(input) => {
                    self.record(&input);
                    return OpenResult::Ok(input);
                }
                OpenResult::Err(e) => return OpenResult::Err(e),
            }
        }
        OpenResult::NotAvailable
    }

    fn input_open_primary(&mut self) -> OpenResult<Input> {
        for i in 0..self.providers.len() {
            match self.providers[i].input_open_primary() {
                OpenResult::NotAvailable => continue,
                OpenResult::Ok(input) => {
                    self.record(&input);
                    return OpenResult::Ok(input);
                }
                OpenResult::Err(e) => return OpenResult::Err(e),
            }
        }
        OpenResult::NotAvailable
    }
}

/// SHA-256 of `content`, hex — the digest an [`InputRecord`] carries.
pub fn digest_of(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("texrs_io_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_name_without_an_extension_also_tries_dot_tex() {
        let dir = scratch("candidates");
        std::fs::write(dir.join("macros.tex"), "\\def\\a{A}").unwrap();
        let mut fs = FilesystemProvider::new(&dir);

        // `\input macros` finds macros.tex, which is what tex does.
        let opened = fs.input_open("macros").must_exist().expect("found");
        assert_eq!(opened.content, "\\def\\a{A}");
        assert_eq!(opened.name, "macros", "the name is what was asked for");
        assert_eq!(opened.path.unwrap().file_name().unwrap(), "macros.tex");

        // An explicit extension is taken as written.
        assert!(fs.input_open("macros.tex").must_exist().is_ok());
        assert!(fs.input_open("macros.sty").is_not_available());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn absence_and_breakage_are_different_answers() {
        let dir = scratch("absence");
        let mut fs = FilesystemProvider::new(&dir);
        assert!(fs.input_open("nothing").is_not_available());

        // A directory in the place of a file is not an absence: it is there and
        // it cannot be read, and a search that skipped it would read the next
        // provider's copy instead.
        std::fs::create_dir_all(dir.join("shadow.tex")).unwrap();
        let result = fs.input_open("shadow.tex");
        assert!(!result.is_not_available(), "a directory is not an absence");
        assert!(matches!(result, OpenResult::Err(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_stack_tries_providers_in_order_and_stops_at_the_first_that_has_it() {
        let dir = scratch("stack");
        std::fs::write(dir.join("shared.tex"), "ON-DISK").unwrap();

        let mut memory = MemoryProvider::new();
        memory.insert("shared.tex", "IN-MEMORY");

        // The editor's copy first: an unsaved buffer is what the user is
        // looking at, and reading the disk under it would compile the past.
        let mut stack = ProviderStack::new();
        stack.push(Box::new(memory));
        stack.push(Box::new(FilesystemProvider::new(&dir)));
        assert_eq!(
            stack.input_open("shared.tex").must_exist().unwrap().content,
            "IN-MEMORY"
        );

        // A name only the disk has falls through to it.
        std::fs::write(dir.join("only.tex"), "DISK-ONLY").unwrap();
        assert_eq!(
            stack.input_open("only").must_exist().unwrap().content,
            "DISK-ONLY"
        );
        assert!(stack.input_open("neither").is_not_available());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_stack_records_every_open_with_its_digest() {
        let dir = scratch("records");
        std::fs::write(dir.join("one.tex"), "ONE").unwrap();
        let mut memory = MemoryProvider::new();
        memory.insert("two.tex", "TWO");

        let mut stack = ProviderStack::new();
        stack.push(Box::new(memory));
        stack.push(Box::new(FilesystemProvider::new(&dir)));
        stack.input_open("one").must_exist().unwrap();
        stack.input_open("two").must_exist().unwrap();
        let _ = stack.input_open("missing");

        let read = stack.inputs_read();
        assert_eq!(read.len(), 2, "what was not found was not read");
        assert_eq!(read[0].name, "one");
        assert_eq!(read[0].origin, InputOrigin::Filesystem);
        assert!(read[0].path.is_some());
        assert_eq!(read[1].origin, InputOrigin::Memory);
        assert_eq!(read[1].path, None);

        // The digest is of the content, so it is the same wherever the bytes
        // came from — which is what makes a record comparable across runs.
        assert_eq!(read[0].digest, digest_of("ONE"));
        assert_eq!(read[0].digest.len(), 64, "sha-256, hex");
        // The known vector, so the digest is SHA-256 and not something else
        // that also produces 64 hex characters.
        assert_eq!(
            digest_of("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_ne!(digest_of("ONE"), digest_of("TWO"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_primary_document_brings_its_own_directory_with_it() {
        let dir = scratch("primary");
        let doc = dir.join("doc.tex");
        std::fs::write(&doc, "\\input sibling").unwrap();
        std::fs::write(dir.join("sibling.tex"), "SIBLING").unwrap();

        let mut fs = FilesystemProvider::with_roots(Vec::new()).with_primary(&doc);
        assert_eq!(
            fs.input_open_primary().must_exist().unwrap().content,
            "\\input sibling"
        );
        // A name the document writes is looked for beside the document, which
        // is where a document's own `\input` names point.
        assert_eq!(
            fs.input_open("sibling").must_exist().unwrap().content,
            "SIBLING"
        );

        // A provider with no primary says so rather than inventing one.
        assert!(MemoryProvider::new()
            .input_open_primary()
            .is_not_available());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
