//! The indexed tar against `tar` itself.
//!
//! A tar is written by every unix in the world and read by this, so the oracle
//! is the program that wrote the archive: `tar tf` says what is in it and
//! `tar xf` says what each file holds. The cases that matter are the ones a
//! bundle really has -- a name too long for a header field, a file whose length
//! is not a whole number of blocks, a file of nothing at all, and an archive
//! big enough that reading a file out of it must not mean reading the archive.

use std::path::{Path, PathBuf};
use std::process::Command;

use texrs::itar::Itar;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("texrs_itar_it_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Build an archive with the system's `tar`, and hand back where it is.
fn archive(dir: &Path, files: &[(String, Vec<u8>)]) -> Option<PathBuf> {
    let work = dir.join("work");
    std::fs::create_dir_all(&work).ok()?;
    for (name, content) in files {
        let path = work.join(name);
        std::fs::create_dir_all(path.parent()?).ok()?;
        std::fs::write(path, content).ok()?;
    }
    let out = dir.join("bundle.tar");
    let made = Command::new("tar")
        .env("COPYFILE_DISABLE", "1")
        .arg("cf")
        .arg(&out)
        .args(files.iter().map(|(name, _)| name))
        .current_dir(&work)
        .status()
        .ok()?;
    made.success().then_some(out)
}

/// What `tar tf` says is in the archive.
fn listed(archive: &Path) -> Vec<String> {
    let out = Command::new("tar")
        .arg("tf")
        .arg(archive)
        .output()
        .expect("tar tf");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .filter(|name| !name.ends_with('/'))
        .collect()
}

/// The files a bundle really holds: awkward lengths, deep names, a name too
/// long for a tar header's hundred bytes, and one of nothing.
fn corpus() -> Vec<(String, Vec<u8>)> {
    let deep = format!(
        "texmf-dist/tex/latex/{}/a-rather-long-package-name.sty",
        "a-directory-with-a-long-name/".repeat(3)
    );
    vec![
        ("macros.tex".into(), b"\\def\\hi{HI}\n".to_vec()),
        // 511, 512 and 513 bytes: a tar pads to 512, and the boundary is where
        // an offset goes wrong.
        ("just-under.tex".into(), vec![b'x'; 511]),
        ("exactly.tex".into(), vec![b'y'; 512]),
        ("just-over.tex".into(), vec![b'z'; 513]),
        ("empty.tex".into(), Vec::new()),
        (deep, b"% deep\n".to_vec()),
        ("binary.bin".into(), (0u8..=255).collect()),
    ]
}

/// Every file, against `tar`.
#[test]
fn every_file_is_the_one_tar_stored() {
    let dir = scratch("compare");
    let files = corpus();
    let Some(path) = archive(&dir, &files) else {
        return;
    };
    let itar = Itar::open(&path).expect("the archive reads");

    // The same names, in the order tar wrote them.
    assert_eq!(itar.names(), listed(&path), "a different set of files");

    // And the same bytes, read one at a time out of the middle of the archive.
    for (name, content) in &files {
        assert_eq!(&itar.read(name).expect(name), content, "{name}");
    }

    // A name longer than a tar header's hundred bytes is stored split across
    // the prefix field or in an extension record; either way it comes back
    // whole, which is what a bundle of TeX Live needs.
    let long = files
        .iter()
        .map(|(name, _)| name)
        .max_by_key(|name| name.len())
        .expect("a name");
    assert!(long.len() > 100, "{} characters", long.len());
    assert_eq!(
        &itar.read(long).expect("the long name"),
        &b"% deep\n".to_vec()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The index is the point: with one, a file is a seek and a read.
#[test]
fn the_index_makes_the_archive_seekable() {
    let dir = scratch("index");
    let files = corpus();
    let Some(path) = archive(&dir, &files) else {
        return;
    };

    // Walking the archive builds the index; the index alone reads the archive.
    let walked = Itar::open(&path).expect("reads");
    let index = walked.index();
    let seeking = Itar::open_indexed(&path, &index).expect("the index reads");
    assert_eq!(seeking.entries(), walked.entries());
    for (name, content) in &files {
        assert_eq!(&seeking.read(name).expect(name), content, "{name}");
    }

    // The index says the truth about the file: every entry's data really is at
    // the offset it names, which is what a range request would ask for.
    let bytes = std::fs::read(&path).expect("the archive");
    for entry in walked.entries() {
        let at = entry.offset as usize;
        let end = at + entry.length as usize;
        assert!(end <= bytes.len(), "{}: past the end", entry.name);
        let content = files
            .iter()
            .find(|(name, _)| name == &entry.name)
            .map(|(_, content)| content)
            .unwrap_or_else(|| panic!("{} is not one of the files", entry.name));
        assert_eq!(&bytes[at..end], content.as_slice(), "{}", entry.name);
        // A file's data begins on a block boundary, because a header is a
        // whole block.
        assert_eq!(entry.offset % 512, 0, "{}", entry.name);
    }

    // The index survives a round trip through text, which is how it is shipped.
    let again = Itar::open_indexed(&path, &seeking.index()).expect("reads");
    assert_eq!(again.entries(), walked.entries());

    let _ = std::fs::remove_dir_all(&dir);
}

/// Reading a file does not read the archive.
///
/// This is the whole reason for the format: tectonic's bundle is TeX Live, and
/// a document reads a few dozen files out of gigabytes. A reader that loaded
/// the archive to answer a question would pass every test above and be useless.
#[test]
fn a_file_is_read_without_reading_the_archive() {
    let dir = scratch("big");
    // Twenty megabytes of archive, one small file at the far end of it.
    let mut files: Vec<(String, Vec<u8>)> = (0..20)
        .map(|i| (format!("filler{i:02}.bin"), vec![b'.'; 1_000_000]))
        .collect();
    files.push(("last.tex".into(), b"\\def\\last{LAST}\n".to_vec()));
    let Some(path) = archive(&dir, &files) else {
        return;
    };
    let size = std::fs::metadata(&path).expect("the archive").len();
    assert!(size > 20_000_000, "{size} bytes");

    let itar = Itar::open(&path).expect("reads");
    let entry = itar.entry("last.tex").expect("the last file");
    assert!(
        entry.offset > 19_000_000,
        "the file should be at the far end, not {}",
        entry.offset
    );
    assert_eq!(
        itar.read("last.tex").expect("the file"),
        b"\\def\\last{LAST}\n"
    );

    // The proof that it seeks: reading the last file is not slower than
    // reading the first, which it would be if the archive were being walked.
    let time = |name: &str| {
        let start = std::time::Instant::now();
        for _ in 0..20 {
            itar.read(name).expect(name);
        }
        start.elapsed()
    };
    let first = time("filler00.bin");
    let last = time("last.tex");
    assert!(
        last < first,
        "reading a 16-byte file at the end took {last:?}, and a megabyte at the start {first:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
