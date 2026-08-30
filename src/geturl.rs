//! Fetching a bundle, ported from tectonic's `geturl`.
//!
//! Tectonic downloads its TeX Live bundle on demand, which is the feature that
//! makes it self-contained. texrs fetches on one command and never during a
//! build: `-X bundle fetch URL` puts an archive in the cache, and a document
//! that names a URL resolves it to what is already there. A build that reached
//! the network would fail on an aeroplane, would depend on a server still
//! answering years from now, and would turn a compile into something that can
//! hang.
//!
//! What is fetched is written under the digest of its own bytes, so:
//!
//!  * two URLs serving the same archive share one file;
//!  * a bundle already fetched is never fetched again;
//!  * a `Texrs.toml` can say which bundle it means by digest rather than by URL,
//!    which is the difference between "the file I built against" and "whatever
//!    that address serves today".

use std::io::Read;
use std::path::PathBuf;

/// The largest archive this will accept, in bytes.
///
/// A bundle is macro files; a hundred megabytes of them would be a mistake or a
/// hostile server, and either way it should stop rather than fill a disk.
pub const MAX_BUNDLE_BYTES: u64 = 100 * 1024 * 1024;

/// What a fetch produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// Where it was written.
    pub path: PathBuf,
    /// SHA-256 of the bytes, hex — the name it is filed under.
    pub digest: String,
    pub bytes: usize,
}

/// Where fetched bundles live: `<cache>/texrs/bundles`.
pub fn bundle_dir() -> Option<PathBuf> {
    let cache = crate::script_cache::default_cache_path();
    Some(cache.parent()?.join("bundles"))
}

/// The file a bundle with this digest is filed under.
pub fn path_for(digest: &str) -> Option<PathBuf> {
    Some(bundle_dir()?.join(format!("{digest}.zip")))
}

/// Every bundle in the cache, by digest.
pub fn fetched() -> Vec<String> {
    let Some(dir) = bundle_dir() else {
        return Vec::new();
    };
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".zip").map(str::to_string)
        })
        .collect();
    out.sort();
    out
}

/// File `bytes` under their digest, and say where they went.
///
/// Separate from the download so the storing half is testable without a server,
/// and so a bundle that arrived by other means can be put in the same place.
pub fn store(bytes: &[u8]) -> Result<Fetched, String> {
    // The digest is of the BYTES, not of their text: an archive is binary, and
    // two different archives must not share a name because both held an
    // invalid sequence that read as the same replacement characters.
    let digest = sha256_hex(bytes);
    let path = path_for(&digest).ok_or("no cache directory")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot make {}: {e}", parent.display()))?;
    }
    if !path.exists() {
        let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
        std::fs::write(&tmp, bytes).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path)
            .map_err(|e| format!("cannot rename into {}: {e}", path.display()))?;
    }
    Ok(Fetched {
        path,
        digest,
        bytes: bytes.len(),
    })
}

/// SHA-256 of bytes, hex.
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Download `url` and file it under its digest.
///
/// Nothing else in texrs calls this: it runs when a user asks for it and at no
/// other time.
pub fn fetch(url: &str) -> Result<Fetched, String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(format!("{url}: only http and https are fetched"));
    }
    let mut response = ureq::get(url).call().map_err(|e| format!("{url}: {e}"))?;

    // Read with a ceiling rather than to the end: a server that never stops
    // sending should not be able to fill the disk.
    let mut body = Vec::new();
    let mut reader = response.body_mut().as_reader().take(MAX_BUNDLE_BYTES + 1);
    reader
        .read_to_end(&mut body)
        .map_err(|e| format!("{url}: {e}"))?;
    if body.len() as u64 > MAX_BUNDLE_BYTES {
        return Err(format!(
            "{url}: larger than the {MAX_BUNDLE_BYTES}-byte limit for a bundle"
        ));
    }
    store(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_urls_are_fetched() {
        // A path is not a URL, and `file://` would make a build's behaviour
        // depend on which of the two spellings a document used.
        for url in ["/tmp/bundle.zip", "file:///tmp/bundle.zip", "ftp://x/y.zip"] {
            let err = fetch(url).unwrap_err();
            assert!(err.contains("only http and https"), "{url}: {err}");
        }
    }

    #[test]
    fn what_is_stored_is_filed_under_the_digest_of_its_bytes() {
        let one = store(b"a bundle's bytes").expect("stores");
        let same = store(b"a bundle's bytes").expect("stores");
        let other = store(b"different bytes").expect("stores");

        assert_eq!(
            one.digest, same.digest,
            "the same bytes are the same bundle"
        );
        assert_eq!(one.path, same.path, "and share one file");
        assert_ne!(one.digest, other.digest);
        assert_eq!(one.digest.len(), 64, "sha-256, hex");
        assert!(one.path.ends_with(format!("{}.zip", one.digest)));
        assert_eq!(one.bytes, 16);

        // Filed where the listing looks for it.
        assert!(fetched().contains(&one.digest));
        for f in [one, other] {
            let _ = std::fs::remove_file(f.path);
        }
    }

    #[test]
    fn a_digest_is_of_the_bytes_and_not_of_their_lossy_text() {
        // Two archives that differ only outside valid UTF-8 must not collide:
        // reading them as text first would map both to the same replacement
        // characters and file them under one name.
        let a = store(&[0xff, 0xfe, 0x00]).expect("stores");
        let b = store(&[0xff, 0xfd, 0x00]).expect("stores");
        assert_ne!(a.digest, b.digest, "different bytes, different bundles");
        for f in [a, b] {
            let _ = std::fs::remove_file(f.path);
        }
    }

    #[test]
    fn the_cache_path_is_beside_the_bytecode_cache() {
        let dir = bundle_dir().expect("a cache directory");
        assert!(dir.ends_with("texrs/bundles"), "{dir:?}");
        let path = path_for("abc").unwrap();
        assert!(path.ends_with("abc.zip"), "{path:?}");
    }
}
