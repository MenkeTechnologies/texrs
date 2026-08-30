//! An indexed tar bundle, ported from `tectonic_bundles::itar`.
//!
//! [`crate::bundle`] reads a zip and keeps it in memory, which is right for a
//! paper's own support files. It is wrong for the other kind of bundle: TeX
//! Live is gigabytes, a document reads a few dozen files out of it, and nothing
//! should have to hold the rest.
//!
//! Tectonic's answer is a plain tar with a separate index: a line per file
//! saying its name, where in the archive it begins and how long it is. That is
//! all the structure a tar lacks -- a tar has no directory, so finding a file
//! means reading every header up to it -- and with the index a file is a seek
//! and a read. The same index makes the archive fetchable over HTTP a file at a
//! time, with a range request per file, which is how tectonic ships TeX Live
//! without shipping TeX Live.
//!
//! What is here is the local half: reading a tar, building the index that makes
//! it seekable, and reading a file out of one by seeking. What is not here is
//! the fetching, for the reason [`crate::bundle`] gives -- a build that reaches
//! the network is a build that fails on an aeroplane.

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::io::{Input, InputOrigin, InputProvider, OpenResult};

/// Where one file sits in the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// The offset of the file's *data*, past its header.
    pub offset: u64,
    pub length: u64,
}

/// A tar with an index.
#[derive(Debug)]
pub struct Itar {
    path: PathBuf,
    entries: Vec<Entry>,
    by_name: BTreeMap<String, usize>,
    /// The last component of each name, for a document that says `\input
    /// macros` rather than naming the path in the archive.
    by_basename: BTreeMap<String, usize>,
}

/// A tar header is 512 bytes, and so is every block after it.
const BLOCK: u64 = 512;

/// The octal number in a header field, which is written as text and padded
/// with spaces or NULs.
fn octal(field: &[u8]) -> Option<u64> {
    let text: String = field
        .iter()
        .take_while(|&&b| b != 0 && b != b' ')
        .map(|&b| b as char)
        .collect();
    match text.is_empty() {
        true => Some(0),
        false => u64::from_str_radix(text.trim(), 8).ok(),
    }
}

/// A NUL-terminated name out of a header field.
fn text(field: &[u8]) -> String {
    field
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as char)
        .collect()
}

/// The checksum a tar header carries: the sum of its bytes with the checksum
/// field itself read as spaces. It is the only thing in a tar that says a
/// block is a header rather than somebody's data.
fn checksum(block: &[u8]) -> u64 {
    block
        .iter()
        .enumerate()
        .map(|(i, &b)| match (148..156).contains(&i) {
            true => b' ' as u64,
            false => b as u64,
        })
        .sum()
}

impl Itar {
    /// Index the tar at `path` by reading its headers.
    ///
    /// This is what makes the index in the first place: a tar has no
    /// directory, so the only way to learn what is in one is to walk it,
    /// jumping over each file's data to the next header.
    pub fn open(path: impl AsRef<Path>) -> Result<Itar, String> {
        let path = path.as_ref().to_path_buf();
        let mut file = std::fs::File::open(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let entries = Itar::index_of(&mut file, &path)?;
        Ok(Itar::with_entries(path, entries))
    }

    /// Open a tar with an index already written, which is the point of the
    /// format: no walk, and nothing read but the files a document asks for.
    pub fn open_indexed(path: impl AsRef<Path>, index: &str) -> Result<Itar, String> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Err(format!("{} is not there", path.display()));
        }
        Ok(Itar::with_entries(path, Itar::parse_index(index)?))
    }

    fn with_entries(path: PathBuf, entries: Vec<Entry>) -> Itar {
        let mut by_name = BTreeMap::new();
        let mut by_basename = BTreeMap::new();
        for (i, entry) in entries.iter().enumerate() {
            by_name.insert(entry.name.clone(), i);
            if let Some(base) = entry.name.rsplit('/').next() {
                // The first of a repeated basename wins, as it does in the
                // zip bundle: a search path is a policy, not a guess.
                by_basename.entry(base.to_string()).or_insert(i);
            }
        }
        Itar {
            path,
            entries,
            by_name,
            by_basename,
        }
    }

    /// Walk a tar's headers.
    fn index_of(file: &mut std::fs::File, path: &Path) -> Result<Vec<Entry>, String> {
        let length = file
            .metadata()
            .map_err(|e| format!("{}: {e}", path.display()))?
            .len();
        let mut entries = Vec::new();
        let mut at = 0u64;
        let mut header = [0u8; BLOCK as usize];
        while at + BLOCK <= length {
            file.seek(SeekFrom::Start(at))
                .map_err(|e| format!("{}: {e}", path.display()))?;
            file.read_exact(&mut header)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            // Two blocks of zeros end an archive, and one is enough to stop.
            if header.iter().all(|&b| b == 0) {
                break;
            }
            // A checksum field that is not a number is not a checksum, so
            // the block is not a header either.
            let stored = octal(&header[148..156]).unwrap_or(u64::MAX);
            if checksum(&header) != stored {
                return Err(format!(
                    "{}: the block at {at} is not a tar header",
                    path.display()
                ));
            }
            let size = octal(&header[124..136])
                .ok_or_else(|| format!("{}: a header with no size", path.display()))?;
            // `ustar` splits a long name into a prefix and a name.
            let name = text(&header[0..100]);
            let prefix = text(&header[345..500]);
            let name = match prefix.is_empty() {
                true => name,
                false => format!("{prefix}/{name}"),
            };
            let kind = header[156];
            // 0 and '0' are a plain file; everything else -- a directory, a
            // link, a long-name extension -- is not a file to read.
            if matches!(kind, 0 | b'0') && !name.ends_with('/') {
                entries.push(Entry {
                    name,
                    offset: at + BLOCK,
                    length: size,
                });
            }
            // The data is padded to a whole number of blocks.
            at += BLOCK + size.div_ceil(BLOCK) * BLOCK;
        }
        Ok(entries)
    }

    /// The index as tectonic writes it: `name offset length`, one file a line.
    pub fn index(&self) -> String {
        let mut out = String::new();
        for entry in &self.entries {
            out.push_str(&format!(
                "{} {} {}\n",
                entry.name, entry.offset, entry.length
            ));
        }
        out
    }

    fn parse_index(text: &str) -> Result<Vec<Entry>, String> {
        let mut entries = Vec::new();
        for (number, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            // The name may hold spaces, so the two numbers are taken from the
            // end rather than the name from the front.
            let mut words = line.rsplitn(3, ' ');
            let length = words.next().and_then(|w| w.parse().ok());
            let offset = words.next().and_then(|w| w.parse().ok());
            let name = words.next().map(str::to_string);
            match (name, offset, length) {
                (Some(name), Some(offset), Some(length)) if !name.is_empty() => {
                    entries.push(Entry {
                        name,
                        offset,
                        length,
                    })
                }
                _ => return Err(format!("index line {}: {line}", number + 1)),
            }
        }
        Ok(entries)
    }

    /// Every file in the archive.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn names(&self) -> Vec<&str> {
        self.entries.iter().map(|e| e.name.as_str()).collect()
    }

    /// Where a file is, by full name or by its last component.
    pub fn entry(&self, name: &str) -> Option<&Entry> {
        self.by_name
            .get(name)
            .or_else(|| self.by_basename.get(name))
            .map(|&i| &self.entries[i])
    }

    /// Read one file: a seek and a read, and nothing else touched.
    pub fn read(&self, name: &str) -> Result<Vec<u8>, String> {
        let entry = self
            .entry(name)
            .ok_or_else(|| format!("{}: no {name}", self.path.display()))?;
        let mut file = std::fs::File::open(&self.path)
            .map_err(|e| format!("cannot read {}: {e}", self.path.display()))?;
        file.seek(SeekFrom::Start(entry.offset))
            .map_err(|e| format!("{}: {e}", self.path.display()))?;
        let mut out = vec![0u8; entry.length as usize];
        file.read_exact(&mut out).map_err(|e| {
            format!(
                "{}: {name} runs past the end of the archive: {e}",
                self.path.display()
            )
        })?;
        Ok(out)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// A summary a person reads.
    pub fn summary(&self) -> String {
        let bytes: u64 = self.entries.iter().map(|e| e.length).sum();
        let mut out = format!("archive       {}\n", self.path.display());
        out.push_str(&format!("files         {}\n", self.entries.len()));
        out.push_str(&format!("bytes         {bytes}\n"));
        out
    }
}

impl InputProvider for Itar {
    fn input_open(&mut self, name: &str) -> OpenResult<Input> {
        // The same `.tex` courtesy the filesystem and the zip bundle give, so
        // a name reads the same whichever it resolves in.
        let candidates: Vec<String> = match Path::new(name).extension() {
            Some(_) => vec![name.to_string()],
            None => vec![name.to_string(), format!("{name}.tex")],
        };
        for candidate in candidates {
            if self.entry(&candidate).is_none() {
                continue;
            }
            return match self.read(&candidate) {
                Ok(bytes) => OpenResult::Ok(Input {
                    name: name.to_string(),
                    // An archive entry is not a path on this machine: saying
                    // it was would send an editor to a file that is not there.
                    path: None,
                    origin: InputOrigin::Memory,
                    content: String::from_utf8_lossy(&bytes).into_owned(),
                }),
                Err(e) => OpenResult::Err(std::io::Error::other(e)),
            };
        }
        OpenResult::NotAvailable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An archive built by the system's own `tar`, which is the oracle: this
    /// reads what another program wrote.
    fn built(name: &str, files: &[(&str, &str)]) -> Option<(PathBuf, PathBuf)> {
        let dir = std::env::temp_dir().join(format!("texrs_itar_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("work")).ok()?;
        for (name, content) in files {
            let path = dir.join("work").join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).ok()?;
            }
            std::fs::write(path, content).ok()?;
        }
        let archive = dir.join("bundle.tar");
        let made = std::process::Command::new("tar")
            // macOS's tar writes an AppleDouble `._name` beside every file
            // unless told not to; they are real entries in the archive, and
            // this is asking for an archive of the files it named.
            .env("COPYFILE_DISABLE", "1")
            .arg("cf")
            .arg(&archive)
            .args(files.iter().map(|(name, _)| *name))
            .current_dir(dir.join("work"))
            .status()
            .ok()?;
        made.success().then_some((dir, archive))
    }

    #[test]
    fn a_tar_is_indexed_and_read_a_file_at_a_time() {
        let files = [
            ("macros.tex", "\\def\\hi{HI}\n"),
            ("tex/deep/other.tex", "% deeper\n"),
            ("empty.tex", ""),
        ];
        let Some((dir, archive)) = built("read", &files) else {
            return;
        };
        let itar = Itar::open(&archive).expect("the archive reads");

        // The same files tar put in, and their contents byte for byte.
        let mut names = itar.names();
        names.sort();
        assert_eq!(names, ["empty.tex", "macros.tex", "tex/deep/other.tex"]);
        for (name, content) in files {
            assert_eq!(
                itar.read(name).expect("the file"),
                content.as_bytes(),
                "{name}"
            );
        }
        // A file is found by its last component too, which is what lets a
        // document say `\input other`.
        assert_eq!(itar.read("other.tex").expect("by basename"), b"% deeper\n");
        assert!(itar.read("nosuch.tex").is_err());

        // The index says where each file is, and reading through it gives the
        // same answers without walking the archive again.
        let index = itar.index();
        assert_eq!(index.lines().count(), 3);
        let indexed = Itar::open_indexed(&archive, &index).expect("the index reads");
        assert_eq!(indexed.entries(), itar.entries());
        assert_eq!(
            indexed.read("macros.tex").expect("the file"),
            b"\\def\\hi{HI}\n"
        );

        // And the offsets are real offsets into the file: a header is 512
        // bytes, so the first file's data begins there.
        assert_eq!(itar.entry("macros.tex").expect("an entry").offset % 512, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What is not a tar, and an index that does not match one.
    #[test]
    fn what_is_not_a_tar_is_refused() {
        let dir = std::env::temp_dir().join(format!("texrs_itar_bad_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // A file of the right size and the wrong contents: the checksum is
        // what says a block is a header.
        let rubbish = dir.join("rubbish.tar");
        std::fs::write(&rubbish, vec![b'x'; 2048]).unwrap();
        let e = Itar::open(&rubbish).unwrap_err();
        assert!(e.contains("not a tar header"), "{e}");

        // A file too short to hold a header is an empty archive rather than an
        // error, as it is to tar.
        let tiny = dir.join("tiny.tar");
        std::fs::write(&tiny, b"hi").unwrap();
        assert!(Itar::open(&tiny).expect("reads").is_empty());

        // An index that names a length the archive cannot supply.
        std::fs::write(&tiny, vec![0u8; 1024]).unwrap();
        let itar = Itar::open_indexed(&tiny, "a.tex 512 4096\n").expect("the index reads");
        assert!(itar.read("a.tex").unwrap_err().contains("past the end"));

        // A line that is not an index line.
        assert!(Itar::open_indexed(&tiny, "not an index\n").is_err());
        assert!(Itar::open_indexed(&tiny, "a.tex 512\n").is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
