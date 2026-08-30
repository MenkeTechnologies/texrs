//! rkyv-backed bytecode cache for `.tex` files.
//!
//! Ported from the sibling engines' `script_cache` (zshrs, awkrs, vimlrs,
//! elisprs), which all write the same shard: one file holding a header and a
//! `path -> entry` map, where an entry carries the source's mtime and a
//! bincode-encoded `fusevm::Chunk`. On the second and later run of a document,
//! the mouth, the expander and the lowerer are all skipped — a hit is an `mmap`,
//! a zero-copy `ArchivedHashMap` lookup and a bincode decode.
//!
//! Storage layout (rkyv archived), identical to the siblings' but for the magic:
//!
//! ```text
//! ScriptShard { header: { magic, format_version, texrs_version,
//!                         pointer_width, built_at_secs },
//!               entries: HashMap<canonical_path, ScriptEntry> }
//! ScriptEntry { mtime_secs, mtime_nsecs, binary_mtime_at_cache,
//!               cached_at_secs, chunk_blob }
//! ```
//!
//! Read path: `mmap`, `rkyv::check_archived_root` validation, then the header's
//! magic, format version, pointer width and writing-binary version, then the
//! entry's own source mtime and the binary mtime — any rebuild of texrs
//! invalidates every entry silently rather than running yesterday's bytecode.
//! Write path: `flock(LOCK_EX)`, read-modify-serialize, fsync a temp file,
//! atomic rename. The format is versioned from the first release, so a document
//! that compiled yesterday never breaks on an upgrade: a shard texrs cannot read
//! is rebuilt, never misread.

use std::collections::HashMap;
use std::fs::File;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use memmap2::Mmap;
use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};

/// Magic header bytes, big-endian ASCII — `TEXR`. Registered in the shard-header
/// convention the engines share (`zdbview`'s `spec/rkyv-shard-header.md`), so a
/// reader that has never heard of texrs still names the file.
pub const SHARD_MAGIC: u32 = 0x54_45_58_52;

/// Bumped whenever the layout of an entry changes, or whenever the meaning of
/// what the blob holds changes. An older shard is rebuilt rather than read.
pub const SHARD_FORMAT_VERSION: u32 = 1;

/// Shard header: format identity plus the provenance that decides staleness.
#[derive(Archive, RkyvDeserialize, RkyvSerialize, Debug, Clone)]
#[archive(check_bytes)]
pub struct ShardHeader {
    /// [`SHARD_MAGIC`].
    pub magic: u32,
    /// [`SHARD_FORMAT_VERSION`].
    pub format_version: u32,
    /// `CARGO_PKG_VERSION` of the binary that wrote the shard.
    pub texrs_version: String,
    /// `size_of::<usize>()` of that binary — a shard is not portable across it.
    pub pointer_width: u32,
    /// Unix seconds the shard was last written.
    pub built_at_secs: u64,
}

/// One cached document: what it was compiled from, and what it compiled to.
#[derive(Archive, RkyvDeserialize, RkyvSerialize, Debug, Clone)]
#[archive(check_bytes)]
pub struct ScriptEntry {
    /// Source mtime seconds.
    pub mtime_secs: i64,
    /// Source mtime nanoseconds — seconds alone miss an edit inside one second.
    pub mtime_nsecs: i64,
    /// mtime of the texrs binary when the entry was written.
    pub binary_mtime_at_cache: i64,
    /// Unix seconds the entry was written.
    pub cached_at_secs: i64,
    /// The bincode-encoded `fusevm::Chunk` the document lowered to.
    pub chunk_blob: Vec<u8>,
}

/// The whole shard: header plus canonical path to entry.
#[derive(Archive, RkyvDeserialize, RkyvSerialize, Debug, Clone)]
#[archive(check_bytes)]
pub struct ScriptShard {
    /// Format identity.
    pub header: ShardHeader,
    /// Canonical source path to compiled entry.
    pub entries: HashMap<String, ScriptEntry>,
}

/// An mmap plus the validated archive inside it. Self-referential: the pointer
/// is valid for as long as the mmap it points into, which this owns.
pub struct MmappedShard {
    _mmap: Mmap,
    archived: *const ArchivedScriptShard,
}

// SAFETY: the pointer aliases an immutable mmap that lives as long as `Self`,
// and every read through it is immutable and rkyv-validated.
unsafe impl Send for MmappedShard {}
unsafe impl Sync for MmappedShard {}

impl MmappedShard {
    /// mmap the shard and validate its bytes. `None` for anything that is not a
    /// shard this build can read — which is a cache miss, never an error.
    pub fn open(path: &Path) -> Option<Self> {
        let file = File::open(path).ok()?;
        // SAFETY: the file is opened read-only and only read through the
        // validated archive below.
        let mmap = unsafe { Mmap::map(&file).ok()? };
        let archived = rkyv::check_archived_root::<ScriptShard>(&mmap[..]).ok()?;
        let archived = archived as *const ArchivedScriptShard;
        Some(Self {
            _mmap: mmap,
            archived,
        })
    }

    fn shard(&self) -> &ArchivedScriptShard {
        // SAFETY: see the Send/Sync note above.
        unsafe { &*self.archived }
    }

    /// Whether this shard was written by a build whose entries this one can use.
    fn header_ok(&self) -> bool {
        let h = &self.shard().header;
        let magic: u32 = h.magic.into();
        let format_version: u32 = h.format_version.into();
        let pointer_width: u32 = h.pointer_width.into();
        magic == SHARD_MAGIC
            && format_version == SHARD_FORMAT_VERSION
            && pointer_width as usize == std::mem::size_of::<usize>()
            && h.texrs_version.as_str() == env!("CARGO_PKG_VERSION")
    }

    fn lookup(&self, path: &str) -> Option<&ArchivedScriptEntry> {
        self.shard().entries.get(path)
    }
}

/// The cache key for an assembled document: what its inputs hashed to, in
/// order, and which profile compiled them.
///
/// A document built by `-X build` is not a file — it is several files joined —
/// so it cannot be keyed the way a single source is, by path and mtime. Keying
/// on the digests instead makes the entry valid exactly while the content is,
/// which is stronger than an mtime: a file touched but not edited still hits,
/// and a file restored from a backup with an older mtime does not wrongly.
///
/// The `doc:` prefix keeps these out of the way of the path-keyed entries; an
/// absolute path cannot start with it.
pub fn document_key(input_digests: &[String], profile: &str) -> String {
    let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
    sha2::Digest::update(&mut hasher, profile.as_bytes());
    for digest in input_digests {
        sha2::Digest::update(&mut hasher, b"\0");
        sha2::Digest::update(&mut hasher, digest.as_bytes());
    }
    let hex: String = sha2::Digest::finalize(hasher)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("doc:{hex}")
}

/// A handle to one shard file and the lock that serializes writers to it.
pub struct ScriptCache {
    path: PathBuf,
    lock_path: PathBuf,
    mmap: Mutex<Option<MmappedShard>>,
}

impl ScriptCache {
    /// Open a shard at `path`, creating its directory. The file itself is made
    /// on the first write, so a run that only reads leaves nothing behind.
    pub fn open(path: &Path) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let parent = path.parent().unwrap_or_else(|| Path::new("/tmp"));
        let lock_path = parent.join(format!(
            "{}.lock",
            path.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("scripts.rkyv")
        ));
        Ok(Self {
            path: path.to_path_buf(),
            lock_path,
            mmap: Mutex::new(None),
        })
    }

    fn ensure_mmap(&self) {
        let mut guard = self.mmap.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = MmappedShard::open(&self.path);
        }
    }

    fn invalidate_mmap(&self) {
        *self.mmap.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// The compiled blob for `path`, or `None` on a miss — which includes a
    /// source edited since, a texrs rebuilt since, and a shard from another
    /// build. Every one of those is a silent rebuild, never a wrong answer.
    pub fn get(&self, path: &str, mtime_secs: i64, mtime_nsecs: i64) -> Option<Vec<u8>> {
        self.ensure_mmap();
        let guard = self.mmap.lock().unwrap_or_else(|e| e.into_inner());
        let shard = guard.as_ref()?;
        if !shard.header_ok() {
            return None;
        }
        let entry = shard.lookup(path)?;
        let entry_secs: i64 = entry.mtime_secs.into();
        let entry_nsecs: i64 = entry.mtime_nsecs.into();
        if entry_secs != mtime_secs || entry_nsecs != mtime_nsecs {
            return None;
        }
        if let Some(binary_mtime) = current_binary_mtime_secs() {
            let cached: i64 = entry.binary_mtime_at_cache.into();
            if cached < binary_mtime {
                return None;
            }
        }
        Some(entry.chunk_blob.as_slice().to_vec())
    }

    /// Write one entry, replacing any entry for the same path. The whole shard
    /// is re-serialized under the writer lock and renamed into place, so a
    /// reader never sees a half-written file.
    pub fn put(
        &self,
        path: &str,
        mtime_secs: i64,
        mtime_nsecs: i64,
        chunk_blob: Vec<u8>,
    ) -> std::io::Result<()> {
        let _lock = match acquire_lock(&self.lock_path) {
            Some(lock) => lock,
            // No lock is no write: a cache is an optimization, and losing one
            // entry costs a compile rather than correctness.
            None => return Ok(()),
        };
        let mut shard = match read_owned_shard(&self.path) {
            Some(s)
                if s.header.texrs_version == env!("CARGO_PKG_VERSION")
                    && s.header.pointer_width as usize == std::mem::size_of::<usize>()
                    && s.header.format_version == SHARD_FORMAT_VERSION =>
            {
                s
            }
            // Anything else is a shard this build cannot add to; start again
            // rather than mixing entries from two formats.
            _ => fresh_shard(),
        };
        shard.entries.insert(
            path.to_string(),
            ScriptEntry {
                mtime_secs,
                mtime_nsecs,
                binary_mtime_at_cache: current_binary_mtime_secs().unwrap_or(0),
                cached_at_secs: now_secs(),
                chunk_blob,
            },
        );
        shard.header.built_at_secs = now_secs() as u64;
        write_shard_atomic(&self.path, &shard)?;
        self.invalidate_mmap();
        Ok(())
    }

    /// Drop entries whose source is gone or has changed, and report how many.
    /// Nothing depends on this — a stale entry is already ignored on read — but
    /// a shard that is never pruned grows for the life of the machine.
    pub fn evict_stale(&self) -> usize {
        let _lock = match acquire_lock(&self.lock_path) {
            Some(lock) => lock,
            None => return 0,
        };
        let mut shard = match read_owned_shard(&self.path) {
            Some(s) => s,
            None => return 0,
        };
        let before = shard.entries.len();
        shard.entries.retain(|p, e| match file_mtime(Path::new(p)) {
            Some((secs, nsecs)) => secs == e.mtime_secs && nsecs == e.mtime_nsecs,
            None => false,
        });
        let evicted = before - shard.entries.len();
        if evicted > 0 {
            let _ = write_shard_atomic(&self.path, &shard);
            self.invalidate_mmap();
        }
        evicted
    }

    /// How many documents the shard holds, for `--cache-stats`.
    pub fn len(&self) -> usize {
        self.ensure_mmap();
        let guard = self.mmap.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().map(|s| s.shard().entries.len()).unwrap_or(0)
    }

    /// Whether the shard holds nothing — a missing file included.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Where this cache lives, for a message that has to name it.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Delete the shard. Idempotent: a cache that is already gone is cleared.
    pub fn clear(&self) -> std::io::Result<()> {
        let _lock = acquire_lock(&self.lock_path);
        let result = match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        };
        self.invalidate_mmap();
        result
    }
}

fn acquire_lock(path: &Path) -> Option<nix::fcntl::Flock<File>> {
    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .ok()?;
    nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusive).ok()
}

fn fresh_shard() -> ScriptShard {
    ScriptShard {
        header: ShardHeader {
            magic: SHARD_MAGIC,
            format_version: SHARD_FORMAT_VERSION,
            texrs_version: env!("CARGO_PKG_VERSION").to_string(),
            pointer_width: std::mem::size_of::<usize>() as u32,
            built_at_secs: now_secs() as u64,
        },
        entries: HashMap::new(),
    }
}

fn read_owned_shard(path: &Path) -> Option<ScriptShard> {
    let bytes = std::fs::read(path).ok()?;
    let archived = rkyv::check_archived_root::<ScriptShard>(&bytes[..]).ok()?;
    archived.deserialize(&mut rkyv::Infallible).ok()
}

/// Serialize and rename into place. The temp name carries the pid and the clock
/// so two texrs runs writing at once cannot collide on it.
fn write_shard_atomic(path: &Path, shard: &ScriptShard) -> std::io::Result<()> {
    let bytes = rkyv::to_bytes::<_, 4096>(shard)
        .map_err(|e| std::io::Error::other(format!("rkyv serialize: {e}")))?;
    let parent = path.parent().unwrap_or_else(|| Path::new("/tmp"));
    let _ = std::fs::create_dir_all(parent);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_path = parent.join(format!(
        "{}.tmp.{}.{nanos}",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("scripts.rkyv"),
        std::process::id()
    ));
    {
        let mut f = File::create(&tmp_path)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A source file's mtime as `(seconds, nanoseconds)`.
pub fn file_mtime(path: &Path) -> Option<(i64, i64)> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.mtime(), meta.mtime_nsec()))
}

/// mtime of the running texrs, read once: every entry is invalidated by a newer
/// binary, since the bytecode a build emits is only meaningful to that build.
fn current_binary_mtime_secs() -> Option<i64> {
    static BINARY_MTIME: OnceLock<Option<i64>> = OnceLock::new();
    *BINARY_MTIME.get_or_init(|| {
        let exe = std::env::current_exe().ok()?;
        let (secs, _) = file_mtime(&exe)?;
        Some(secs)
    })
}

/// `$XDG_CACHE_HOME/texrs/scripts.rkyv`, falling back to `~/.cache` and then to
/// `/tmp` — the path the sibling engines use for their own shards.
pub fn default_cache_path() -> PathBuf {
    dirs::cache_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("texrs/scripts.rkyv")
}

/// `TEXRS_CACHE=0|false|no` turns the cache off for a run.
pub fn cache_enabled() -> bool {
    !matches!(
        std::env::var("TEXRS_CACHE").as_deref(),
        Ok("0") | Ok("false") | Ok("no")
    )
}

/// The process's cache, or `None` when it is disabled or cannot be opened.
pub static CACHE: once_cell::sync::Lazy<Option<ScriptCache>> = once_cell::sync::Lazy::new(|| {
    if !cache_enabled() {
        return None;
    }
    ScriptCache::open(&default_cache_path()).ok()
});

/// The chunk cached under `key`, if this build wrote one.
///
/// The mtime guards do not apply: the key IS the content, so an entry either
/// describes these bytes or belongs to different ones. The binary's own mtime
/// still does, since bytecode is only meaningful to the build that emitted it.
pub fn try_load_keyed(key: &str) -> Option<fusevm::Chunk> {
    let cache = CACHE.as_ref()?;
    let blob = cache.get(key, DIGEST_KEYED, DIGEST_KEYED)?;
    bincode::deserialize::<fusevm::Chunk>(&blob).ok()
}

/// Remember what the document under `key` compiled to. Best-effort, as the
/// path-keyed store is.
pub fn store_keyed(key: &str, chunk: &fusevm::Chunk) {
    let Some(cache) = CACHE.as_ref() else {
        return;
    };
    let Ok(blob) = bincode::serialize(chunk) else {
        return;
    };
    let _ = cache.put(key, DIGEST_KEYED, DIGEST_KEYED, blob);
}

/// The mtime a digest-keyed entry carries. Any fixed pair does: the key already
/// says what the content is, and this is what makes the guard a no-op rather
/// than a comparison against a file that does not exist.
const DIGEST_KEYED: i64 = 0;

/// The compiled chunk for `path`, if the cache still has one that matches it.
/// The chunk for a document compiled in a particular MODE.
///
/// The .tex file itself is the key, so a document lives in `scripts.rkyv` under
/// its own name and is guarded by its own mtime -- edit the file and the entry
/// goes stale, as it should. The mode is a suffix on that key because one
/// document compiles to more than one chunk: a `--text` run carries the
/// document's characters and an ordinary run does not, and serving one where
/// the other was asked for would be a silently wrong answer rather than a slow
/// one.
pub fn try_load_mode(path: &Path, mode: &str) -> Option<fusevm::Chunk> {
    let cache = CACHE.as_ref()?;
    let canonical = path.canonicalize().ok()?;
    let key = format!("{}#{mode}", canonical.to_str()?);
    let (mtime_secs, mtime_nsecs) = file_mtime(&canonical)?;
    let blob = cache.get(&key, mtime_secs, mtime_nsecs)?;
    bincode::deserialize::<fusevm::Chunk>(&blob).ok()
}

/// Remember what a document compiled to in `mode`. Best-effort, as the
/// path-keyed store is.
pub fn store_mode(path: &Path, mode: &str, chunk: &fusevm::Chunk) {
    let Some(cache) = CACHE.as_ref() else {
        return;
    };
    let Ok(canonical) = path.canonicalize() else {
        return;
    };
    let Some(base) = canonical.to_str() else {
        return;
    };
    let Some((mtime_secs, mtime_nsecs)) = file_mtime(&canonical) else {
        return;
    };
    let Ok(blob) = bincode::serialize(chunk) else {
        return;
    };
    let _ = cache.put(&format!("{base}#{mode}"), mtime_secs, mtime_nsecs, blob);
}

pub fn try_load(path: &Path) -> Option<fusevm::Chunk> {
    let cache = CACHE.as_ref()?;
    let canonical = path.canonicalize().ok()?;
    let key = canonical.to_str()?;
    let (mtime_secs, mtime_nsecs) = file_mtime(&canonical)?;
    let blob = cache.get(key, mtime_secs, mtime_nsecs)?;
    bincode::deserialize::<fusevm::Chunk>(&blob).ok()
}

/// Remember what `path` compiled to. Best-effort: a cache that cannot be written
/// costs a compile next time and nothing else, so every failure here is silent.
pub fn store(path: &Path, chunk: &fusevm::Chunk) {
    let Some(cache) = CACHE.as_ref() else {
        return;
    };
    let Ok(canonical) = path.canonicalize() else {
        return;
    };
    let Some(key) = canonical.to_str() else {
        return;
    };
    let Some((mtime_secs, mtime_nsecs)) = file_mtime(&canonical) else {
        return;
    };
    let Ok(blob) = bincode::serialize(chunk) else {
        return;
    };
    let _ = cache.put(key, mtime_secs, mtime_nsecs, blob);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shard of its own, in a directory of its own, so tests do not read or
    /// write the cache the user's own runs are using.
    fn scratch(name: &str) -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("texrs_cache_{}_{seq}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("scripts.rkyv")
    }

    /// A `.tex` file with a known mtime, standing in for a document.
    fn source(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn a_stored_chunk_comes_back_byte_for_byte() {
        let shard = scratch("roundtrip");
        let cache = ScriptCache::open(&shard).unwrap();
        assert!(cache.is_empty(), "a cache with no file holds nothing");

        cache.put("/tmp/doc.tex", 7, 11, vec![1, 2, 3]).unwrap();
        assert_eq!(cache.get("/tmp/doc.tex", 7, 11), Some(vec![1, 2, 3]));
        assert_eq!(cache.len(), 1);

        // A second entry joins the first rather than replacing the shard.
        cache.put("/tmp/other.tex", 1, 2, vec![9]).unwrap();
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get("/tmp/doc.tex", 7, 11), Some(vec![1, 2, 3]));

        // Writing the same path again replaces that entry alone.
        cache.put("/tmp/doc.tex", 7, 11, vec![4, 5]).unwrap();
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get("/tmp/doc.tex", 7, 11), Some(vec![4, 5]));
    }

    #[test]
    fn an_edited_source_is_a_miss_rather_than_a_stale_hit() {
        let shard = scratch("mtime");
        let cache = ScriptCache::open(&shard).unwrap();
        cache.put("/tmp/doc.tex", 100, 0, vec![1]).unwrap();

        assert_eq!(cache.get("/tmp/doc.tex", 100, 0), Some(vec![1]));
        assert_eq!(cache.get("/tmp/doc.tex", 101, 0), None, "a later second");
        assert_eq!(
            cache.get("/tmp/doc.tex", 100, 1),
            None,
            "an edit inside the same second, which seconds alone would miss"
        );
        assert_eq!(cache.get("/tmp/elsewhere.tex", 100, 0), None);
    }

    #[test]
    fn a_shard_this_build_cannot_read_is_rebuilt_not_misread() {
        let shard = scratch("foreign");

        // A file that is not a shard at all.
        std::fs::write(&shard, b"not an rkyv archive").unwrap();
        let cache = ScriptCache::open(&shard).unwrap();
        assert_eq!(cache.get("/tmp/doc.tex", 1, 2), None);
        assert!(cache.is_empty());

        // Writing over it produces a shard this build does read.
        cache.put("/tmp/doc.tex", 1, 2, vec![7]).unwrap();
        assert_eq!(cache.get("/tmp/doc.tex", 1, 2), Some(vec![7]));

        // A shard whose header says another format version is not read either,
        // and the entry that replaces it is this build's.
        let mut owned = read_owned_shard(&shard).expect("readable");
        owned.header.format_version = SHARD_FORMAT_VERSION + 1;
        write_shard_atomic(&shard, &owned).unwrap();
        let cache = ScriptCache::open(&shard).unwrap();
        assert_eq!(
            cache.get("/tmp/doc.tex", 1, 2),
            None,
            "a newer format is a miss"
        );
        cache.put("/tmp/doc.tex", 1, 2, vec![8]).unwrap();
        let back = read_owned_shard(&shard).expect("readable");
        assert_eq!(back.header.format_version, SHARD_FORMAT_VERSION);
        assert_eq!(
            back.entries.len(),
            1,
            "the old entries went with the format"
        );
    }

    #[test]
    fn the_header_says_what_wrote_the_shard() {
        let shard = scratch("header");
        let cache = ScriptCache::open(&shard).unwrap();
        cache.put("/tmp/doc.tex", 1, 2, vec![1]).unwrap();

        let owned = read_owned_shard(&shard).expect("readable");
        assert_eq!(owned.header.magic, SHARD_MAGIC);
        assert_eq!(
            &SHARD_MAGIC.to_be_bytes(),
            b"TEXR",
            "the tag reads as itself"
        );
        assert_eq!(owned.header.format_version, SHARD_FORMAT_VERSION);
        assert_eq!(owned.header.texrs_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(
            owned.header.pointer_width as usize,
            std::mem::size_of::<usize>()
        );
        assert!(owned.header.built_at_secs > 0, "and when it was written");
    }

    #[test]
    fn eviction_drops_what_the_disk_no_longer_backs() {
        let shard = scratch("evict");
        let dir = shard.parent().unwrap().to_path_buf();
        let kept = source(&dir, "kept.tex", "\\message{a}");
        let edited = source(&dir, "edited.tex", "\\message{b}");
        let removed = source(&dir, "removed.tex", "\\message{c}");

        let cache = ScriptCache::open(&shard).unwrap();
        for path in [&kept, &edited, &removed] {
            let (secs, nsecs) = file_mtime(path).unwrap();
            cache
                .put(path.to_str().unwrap(), secs, nsecs, vec![1])
                .unwrap();
        }
        assert_eq!(cache.len(), 3);

        // One file edited, one gone: both entries are stale, the third is not.
        std::fs::write(&edited, "\\message{different}").unwrap();
        std::fs::remove_file(&removed).unwrap();
        assert_eq!(cache.evict_stale(), 2);
        assert_eq!(cache.len(), 1);
        let (secs, nsecs) = file_mtime(&kept).unwrap();
        assert!(cache.get(kept.to_str().unwrap(), secs, nsecs).is_some());

        // Evicting again has nothing left to do.
        assert_eq!(cache.evict_stale(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clearing_is_idempotent_and_leaves_nothing_behind() {
        let shard = scratch("clear");
        let cache = ScriptCache::open(&shard).unwrap();
        cache.put("/tmp/doc.tex", 1, 2, vec![1]).unwrap();
        assert!(shard.exists());

        cache.clear().unwrap();
        assert!(!shard.exists(), "the file is gone");
        assert!(cache.is_empty(), "and so is what it holds");
        cache.clear().unwrap();

        // A cleared cache is a working cache, not a broken one.
        cache.put("/tmp/doc.tex", 1, 2, vec![2]).unwrap();
        assert_eq!(cache.get("/tmp/doc.tex", 1, 2), Some(vec![2]));

        // Nothing is left in the directory but the shard and its lock.
        let dir = shard.parent().unwrap();
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec!["scripts.rkyv", "scripts.rkyv.lock"],
            "no temp files"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_real_document_compiles_once_and_is_read_back_from_the_shard() {
        let shard = scratch("chunk");
        let dir = shard.parent().unwrap().to_path_buf();
        // INITEX category codes: `{` is an ordinary character until a document
        // says otherwise, so a document that wants a group opens by saying so.
        let src = "\\catcode`\\{=1 \\catcode`\\}=2 \\message{hello}";
        let doc = source(&dir, "doc.tex", src);
        let chunk = crate::compile(src).expect("compiles");
        let blob = bincode::serialize(&chunk).unwrap();

        let cache = ScriptCache::open(&shard).unwrap();
        let (secs, nsecs) = file_mtime(&doc).unwrap();
        cache
            .put(doc.to_str().unwrap(), secs, nsecs, blob.clone())
            .unwrap();

        // What comes back out is the bytecode that went in, and it still decodes
        // to a chunk the VM would run.
        let back = cache
            .get(doc.to_str().unwrap(), secs, nsecs)
            .expect("a hit");
        assert_eq!(back, blob);
        let decoded: fusevm::Chunk = bincode::deserialize(&back).unwrap();
        assert_eq!(decoded.ops.len(), chunk.ops.len());
        assert_eq!(
            crate::runtime::run(decoded).unwrap(),
            vec!["hello".to_string()]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_cache_can_be_turned_off_by_the_environment() {
        // The reader is the one thing that must honour it, since a disabled
        // cache is how a user works around a cache they suspect.
        for value in ["0", "false", "no"] {
            std::env::set_var("TEXRS_CACHE", value);
            assert!(!cache_enabled(), "TEXRS_CACHE={value} turns it off");
        }
        for value in ["1", "true", "yes", ""] {
            std::env::set_var("TEXRS_CACHE", value);
            assert!(cache_enabled(), "TEXRS_CACHE={value:?} leaves it on");
        }
        std::env::remove_var("TEXRS_CACHE");
        assert!(cache_enabled(), "and unset is on");
    }

    #[test]
    fn a_document_key_is_its_content_and_its_profile() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        // Same inputs, same order, same profile: the same key.
        assert_eq!(
            document_key(&[a.clone(), b.clone()], "default"),
            document_key(&[a.clone(), b.clone()], "default")
        );
        // Order is content: two documents made of the same files in a
        // different order are different documents.
        assert_ne!(
            document_key(&[a.clone(), b.clone()], "default"),
            document_key(&[b.clone(), a.clone()], "default")
        );
        // The profile is part of the key, or a `disasm` build would be served
        // the chunk a `messages` build cached — which is the same chunk, but
        // the entry must still say which one asked for it.
        assert_ne!(
            document_key(std::slice::from_ref(&a), "default"),
            document_key(std::slice::from_ref(&a), "bytes")
        );
        // A key cannot be mistaken for a path.
        assert!(document_key(&[a], "default").starts_with("doc:"));
    }

    #[test]
    fn the_default_path_is_under_the_cache_directory() {
        let path = default_cache_path();
        assert!(path.ends_with("texrs/scripts.rkyv"), "{path:?}");
        assert!(path.is_absolute(), "{path:?}");
    }
}
