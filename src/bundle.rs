//! A bundle: a document's support files as one archive, ported from the local
//! half of tectonic's `bundles`.
//!
//! Tectonic's bundle is where a document's TeX files come from when they are not
//! beside it — a zip, indexed by name, fetched or local. The fetching half needs
//! a package universe to fetch from, and texrs has none; the local half is
//! useful the moment a document is shared, because "here is my paper" is one
//! file rather than a directory the recipient has to keep in order.
//!
//! A bundle is an [`InputProvider`](crate::io::InputProvider) like any other, so
//! it layers into the same stack: the document's own directory first, then its
//! extra paths, then the bundle. That order is the policy — a file a document
//! carries beside itself overrides the one in its bundle, which is what lets a
//! recipient change one macro without unpacking the archive.
//!
//! What is NOT here, deliberately: fetching. A build that reaches the network is
//! a build that fails on an aeroplane, and one that silently depends on a server
//! being up years from now. If a bundle has to come from somewhere, it can be
//! fetched by whatever already fetches things and handed over as a file.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::io::{Input, InputOrigin, InputProvider, OpenResult};

/// One archive of support files, read into memory.
///
/// A TeX bundle is small — macro files, not images — and the whole of it is read
/// at once so the archive is opened once rather than per lookup. Tectonic
/// streams because its bundles are TeX Live, which is gigabytes.
#[derive(Debug)]
pub struct Bundle {
    path: PathBuf,
    files: BTreeMap<String, String>,
}

impl Bundle {
    /// Open the zip at `path` and index what it holds.
    ///
    /// Names are indexed by their last component as well as in full, so a
    /// document can say `\input macros` whether the bundle stores `macros.tex`
    /// at its root or under `tex/latex/mine/macros.tex`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        let file = std::fs::File::open(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| format!("{} is not a bundle: {e}", path.display()))?;

        let mut files = BTreeMap::new();
        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            if entry.is_dir() {
                continue;
            }
            let name = entry.name().to_string();
            let mut content = String::new();
            // A bundle holds TeX, which is text. Anything that is not gets
            // skipped rather than failing the whole bundle: an archive with a
            // stray binary in it is still a usable bundle.
            if entry.read_to_string(&mut content).is_err() {
                continue;
            }
            if let Some(base) = name.rsplit('/').next() {
                // The full path wins over a bare name, so two files with the
                // same basename do not silently shadow each other.
                files.entry(base.to_string()).or_insert(content.clone());
            }
            files.insert(name, content);
        }
        Ok(Bundle { path, files })
    }

    /// Every file the bundle holds, in order — tectonic's `all_files`, and what
    /// `-X show` prints for a document that has one.
    pub fn all_files(&self) -> Vec<&str> {
        self.files.keys().map(String::as_str).collect()
    }

    /// A digest of what the bundle holds, so two bundles with the same files
    /// are the same bundle whatever they are called or when they were made.
    pub fn digest(&self) -> String {
        let mut joined = String::new();
        for (name, content) in &self.files {
            joined.push_str(name);
            joined.push('\0');
            joined.push_str(&crate::io::digest_of(content));
            joined.push('\n');
        }
        crate::io::digest_of(&joined)
    }

    /// Where the bundle came from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

impl InputProvider for Bundle {
    fn input_open(&mut self, name: &str) -> OpenResult<Input> {
        // The same `.tex` courtesy the filesystem gives, so a name reads the
        // same whether it resolves beside the document or inside the bundle.
        let candidates: Vec<String> = match Path::new(name).extension() {
            Some(_) => vec![name.to_string()],
            None => vec![name.to_string(), format!("{name}.tex")],
        };
        for candidate in candidates {
            if let Some(content) = self.files.get(&candidate) {
                return OpenResult::Ok(Input {
                    name: name.to_string(),
                    // A bundle entry is not a path on this machine: saying it
                    // was would send an editor to a file that is not there.
                    path: None,
                    origin: InputOrigin::Memory,
                    content: content.clone(),
                });
            }
        }
        OpenResult::NotAvailable
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A zip holding `files`, written where a test can open it.
    fn bundle_of(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("texrs_bundle_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.zip"));
        let file = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for (entry, content) in files {
            zip.start_file(*entry, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    #[test]
    fn a_bundle_answers_by_name_and_by_the_name_with_tex_on_it() {
        let path = bundle_of("simple", &[("macros.tex", "\\def\\a{A}")]);
        let mut bundle = Bundle::open(&path).expect("opens");

        assert_eq!(
            bundle
                .input_open("macros.tex")
                .must_exist()
                .unwrap()
                .content,
            "\\def\\a{A}"
        );
        // `\input macros` finds macros.tex, as it does on the filesystem.
        assert_eq!(
            bundle.input_open("macros").must_exist().unwrap().content,
            "\\def\\a{A}"
        );
        assert!(bundle.input_open("absent").is_not_available());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_file_deep_in_a_bundle_is_reachable_by_its_bare_name() {
        // TeX Live puts a macro file under a long path and documents refer to
        // it by name alone; a bundle that only answered full paths would be
        // unusable for the thing bundles are for.
        let path = bundle_of(
            "deep",
            &[("tex/latex/mine/macros.tex", "DEEP"), ("top.tex", "TOP")],
        );
        let mut bundle = Bundle::open(&path).expect("opens");

        assert_eq!(
            bundle.input_open("macros").must_exist().unwrap().content,
            "DEEP"
        );
        assert_eq!(
            bundle
                .input_open("tex/latex/mine/macros.tex")
                .must_exist()
                .unwrap()
                .content,
            "DEEP"
        );
        assert_eq!(
            bundle.input_open("top").must_exist().unwrap().content,
            "TOP"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_entry_is_not_a_path_on_this_machine() {
        let path = bundle_of("origin", &[("macros.tex", "X")]);
        let mut bundle = Bundle::open(&path).expect("opens");
        let opened = bundle.input_open("macros.tex").must_exist().unwrap();
        assert_eq!(opened.origin, InputOrigin::Memory);
        assert_eq!(
            opened.path, None,
            "a bundle entry has no path an editor could open"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_digest_is_of_what_the_bundle_holds_not_of_the_file_it_came_in() {
        let one = bundle_of("digest_a", &[("a.tex", "A"), ("b.tex", "B")]);
        // The same files, in the other order, written to another name.
        let two = bundle_of("digest_b", &[("b.tex", "B"), ("a.tex", "A")]);
        let three = bundle_of("digest_c", &[("a.tex", "A"), ("b.tex", "CHANGED")]);

        let a = Bundle::open(&one).unwrap();
        let b = Bundle::open(&two).unwrap();
        let c = Bundle::open(&three).unwrap();
        assert_eq!(a.digest(), b.digest(), "the same files are the same bundle");
        assert_ne!(a.digest(), c.digest(), "a changed file is a changed bundle");
        assert_eq!(a.all_files(), vec!["a.tex", "b.tex"]);
        for path in [&one, &two, &three] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn what_is_not_a_bundle_is_refused_by_name() {
        let dir = std::env::temp_dir().join(format!("texrs_bundle_bad_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("not.zip");
        std::fs::write(&path, b"not a zip at all").unwrap();
        let err = Bundle::open(&path).unwrap_err();
        assert!(err.contains("is not a bundle"), "{err}");
        assert!(Bundle::open(dir.join("absent.zip"))
            .unwrap_err()
            .contains("cannot read"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
